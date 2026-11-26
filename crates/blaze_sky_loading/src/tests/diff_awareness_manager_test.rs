//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java`:
//! the manager works in a sequential manner and is correct in the presence of unprocessed diffs.
//!
//! PORT: `ProcessableModifiedFileSet.markProcessed()` is `DiffAwarenessManager::mark_processed`;
//! the Mockito mocks are hand-written stubs; the `OptionsProvider.EMPTY` is the default
//! `LocalDiffAwarenessOptions`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use blaze_events::{EventCollector, EventKind};
use blaze_optdefs::command::local_diff_awareness::LocalDiffAwarenessOptions;
use blaze_vfs::{ModifiedFileSet, PathFragment, Root};
use indexmap::IndexSet;
use parking_lot::Mutex;

use crate::diff_awareness::{
    BrokenDiffAwarenessException, DiffAwareness, DiffAwarenessFactory, DiffAwarenessManager,
    DiffError, IncompatibleViewException, View, WorkspaceInfoFromDiff,
};
use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::test_util::Scratch;

fn options() -> LocalDiffAwarenessOptions {
    blaze_options::Options::<LocalDiffAwarenessOptions>::get_defaults()
}

fn modified_file_set(paths: &[&str]) -> ModifiedFileSet {
    let mut builder = ModifiedFileSet::builder();
    for p in paths {
        builder = builder.modify(PathFragment::create(p));
    }
    builder.build()
}

struct Fixture {
    _scratch: Scratch,
    events: Arc<EventCollector>,
    root: Root,
}

impl Fixture {
    fn new(path: &str) -> Fixture {
        let scratch = Scratch::new();
        let root = Root::from_path(scratch.path(path));
        Fixture {
            _scratch: scratch,
            events: Arc::new(EventCollector::new()),
            root,
        }
    }

    fn assert_no_warnings_or_errors(&self) {
        assert!(
            self.events.filtered(EventKind::Warning).is_empty()
                && self.events.filtered(EventKind::Error).is_empty(),
            "{:?}",
            self.events
                .iter()
                .map(|e| e.get_message().into_owned())
                .collect::<Vec<_>>()
        );
    }

    fn count_containing(&self, text: &str) -> usize {
        self.events
            .iter()
            .filter(|e| e.get_message().contains(text))
            .count()
    }
}

// Java: DiffAwarenessManagerTest.DiffAwarenessFactoryStub
#[derive(Default)]
struct DiffAwarenessFactoryStub {
    diff_awarenesses: Mutex<HashMap<String, Box<dyn DiffAwareness>>>,
}

impl DiffAwarenessFactoryStub {
    fn inject(&self, path_entry: &Root, diff_awareness: Box<dyn DiffAwareness>) {
        self.diff_awarenesses
            .lock()
            .insert(path_entry.to_string(), diff_awareness);
    }

    fn remove(&self, path_entry: &Root) {
        self.diff_awarenesses.lock().remove(&path_entry.to_string());
    }
}

/// A factory shared with the test (the manager owns an `Arc`).
struct SharedFactory(Arc<DiffAwarenessFactoryStub>);

impl DiffAwarenessFactory for SharedFactory {
    fn maybe_create(
        &self,
        path_entry: &Root,
        _ignored_paths: &IgnoredSubdirectories,
        _options: &LocalDiffAwarenessOptions,
    ) -> Option<Box<dyn DiffAwareness>> {
        // Java's stub returns the same instance on each call; the manager calls it only when it
        // has no state for the entry, and every test injects a fresh stub before that.
        self.0
            .diff_awarenesses
            .lock()
            .remove(&path_entry.to_string())
    }
}

// Java: DiffAwarenessManagerTest.DiffAwarenessStub.ViewStub
struct ViewStub {
    sequence_num: usize,
}

impl std::fmt::Display for ViewStub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ViewStub({})", self.sequence_num)
    }
}

impl View for ViewStub {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

const BROKEN: &str = "special broken marker";

// Java: DiffAwarenessManagerTest.DiffAwarenessStub
struct DiffAwarenessStub {
    closed: Arc<Mutex<bool>>,
    cur_sequence_num: usize,
    sequential_diffs: Vec<ModifiedFileSet>,
    broken_view_num: Option<usize>,
}

impl DiffAwarenessStub {
    fn new(sequential_diffs: Vec<ModifiedFileSet>) -> (Box<DiffAwarenessStub>, Arc<Mutex<bool>>) {
        Self::with_broken_view(sequential_diffs, None)
    }

    fn with_broken_view(
        sequential_diffs: Vec<ModifiedFileSet>,
        broken_view_num: Option<usize>,
    ) -> (Box<DiffAwarenessStub>, Arc<Mutex<bool>>) {
        assert!(
            sequential_diffs
                .iter()
                .all(|d| !d.treat_everything_as_modified()),
            "Merging of diffs treating everything as modified is not implemented"
        );
        let closed = Arc::new(Mutex::new(false));
        (
            Box::new(DiffAwarenessStub {
                closed: closed.clone(),
                cur_sequence_num: 0,
                sequential_diffs,
                broken_view_num,
            }),
            closed,
        )
    }
}

impl DiffAwareness for DiffAwarenessStub {
    fn get_current_view(
        &mut self,
        _options: &LocalDiffAwarenessOptions,
    ) -> Result<Arc<dyn View>, BrokenDiffAwarenessException> {
        if Some(self.cur_sequence_num) == self.broken_view_num {
            return Err(BrokenDiffAwarenessException(
                "error in getCurrentView".to_string(),
            ));
        }
        let view = ViewStub {
            sequence_num: self.cur_sequence_num,
        };
        self.cur_sequence_num += 1;
        Ok(Arc::new(view))
    }

    fn get_diff(
        &mut self,
        old_view: Option<&Arc<dyn View>>,
        new_view: &Arc<dyn View>,
    ) -> Result<ModifiedFileSet, DiffError> {
        let Some(old_view) = old_view else {
            return Ok(ModifiedFileSet::EVERYTHING_MODIFIED);
        };
        let old = old_view.as_any().downcast_ref::<ViewStub>().unwrap();
        let new = new_view.as_any().downcast_ref::<ViewStub>().unwrap();
        assert!(new.sequence_num >= old.sequence_num);
        let mut diff = ModifiedFileSet::builder();
        for num in old.sequence_num..new.sequence_num {
            let incremental_diff = &self.sequential_diffs[num];
            if incremental_diff
                .modified_source_files()
                .contains(&PathFragment::create(BROKEN))
            {
                return Err(DiffError::Broken(BrokenDiffAwarenessException(
                    "error in getDiff".to_string(),
                )));
            }
            for path in incremental_diff.modified_source_files() {
                diff = diff.modify(path.clone());
            }
        }
        Ok(diff.build())
    }

    fn name(&self) -> &'static str {
        "testingstub"
    }

    fn close(&mut self) {
        *self.closed.lock() = true;
    }
}

fn broken_diff() -> ModifiedFileSet {
    modified_file_set(&[BROKEN])
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:61 DiffAwarenessManagerTest.testEverythingModifiedIfNoDiffAwareness
#[test]
fn test_everything_modified_if_no_diff_awareness() {
    let f = Fixture::new("pathEntry");
    let factory = Arc::new(DiffAwarenessFactoryStub::default());
    let mut manager = DiffAwarenessManager::new(vec![Arc::new(SharedFactory(factory))]);
    assert_eq!(
        *manager
            .get_diff(
                f.events.as_ref(),
                &f.root,
                &IgnoredSubdirectories::empty(),
                &options()
            )
            .get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED since there are no factories"
    );
    f.assert_no_warnings_or_errors();
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:76 DiffAwarenessManagerTest.testResetAndSetPathEntriesCallClose
#[test]
fn test_reset_and_set_path_entries_call_close() {
    let f = Fixture::new("pathEntry");
    let diff = ModifiedFileSet::nothing_modified();
    let (diff_awareness1, closed1) = DiffAwarenessStub::new(vec![diff.clone()]);
    let (diff_awareness2, closed2) = DiffAwarenessStub::new(vec![diff]);
    let factory = Arc::new(DiffAwarenessFactoryStub::default());
    factory.inject(&f.root, diff_awareness1);
    let mut manager = DiffAwarenessManager::new(vec![Arc::new(SharedFactory(factory.clone()))]);
    let empty = IgnoredSubdirectories::empty();
    let _ = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(
        !*closed1.lock(),
        "diffAwareness1 shouldn't have been closed yet"
    );
    manager.reset();
    assert!(
        *closed1.lock(),
        "diffAwareness1 should have been closed by reset"
    );
    factory.inject(&f.root, diff_awareness2);
    let _ = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(
        !*closed2.lock(),
        "diffAwareness2 shouldn't have been closed yet"
    );
    f.assert_no_warnings_or_errors();
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:106 DiffAwarenessManagerTest.testHandlesUnprocessedDiffs
#[test]
fn test_handles_unprocessed_diffs() {
    let f = Fixture::new("pathEntry");
    let diff1 = modified_file_set(&["file1"]);
    let diff2 = modified_file_set(&["file2"]);
    let diff3 = modified_file_set(&["file3"]);
    let (diff_awareness, _) =
        DiffAwarenessStub::new(vec![diff1.clone(), diff2, diff3.clone(), broken_diff()]);
    let factory = Arc::new(DiffAwarenessFactoryStub::default());
    factory.inject(&f.root, diff_awareness);
    let mut manager = DiffAwarenessManager::new(vec![Arc::new(SharedFactory(factory))]);
    let empty = IgnoredSubdirectories::empty();
    let first_processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(
        *first_processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED on first call to getDiff"
    );
    manager.mark_processed(&first_processable_diff);
    let processable_diff1 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(*processable_diff1.get_modified_file_set(), diff1);
    let processable_diff2 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(
        *processable_diff2.get_modified_file_set(),
        modified_file_set(&["file1", "file2"])
    );
    manager.mark_processed(&processable_diff2);
    let processable_diff3 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(*processable_diff3.get_modified_file_set(), diff3);
    f.assert_no_warnings_or_errors();
    let processable_diff4 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(
        *processable_diff4.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED
    );
    assert!(f.count_containing("error") > 0);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:149 DiffAwarenessManagerTest.testHandlesBrokenDiffs
#[test]
fn test_handles_broken_diffs() {
    let f = Fixture::new("pathEntry");
    let factory1 = Arc::new(DiffAwarenessFactoryStub::default());
    let (diff_awareness1, _) = DiffAwarenessStub::with_broken_view(vec![], Some(1));
    factory1.inject(&f.root, diff_awareness1);
    let factory2 = Arc::new(DiffAwarenessFactoryStub::default());
    let diff2 = modified_file_set(&["file2"]);
    let (diff_awareness2, _) = DiffAwarenessStub::new(vec![diff2.clone(), broken_diff()]);
    factory2.inject(&f.root, diff_awareness2);
    let factory3 = Arc::new(DiffAwarenessFactoryStub::default());
    let diff3 = modified_file_set(&["file3"]);
    let (diff_awareness3, _) = DiffAwarenessStub::new(vec![diff3.clone()]);
    factory3.inject(&f.root, diff_awareness3);
    let mut manager = DiffAwarenessManager::new(vec![
        Arc::new(SharedFactory(factory1.clone())),
        Arc::new(SharedFactory(factory2.clone())),
        Arc::new(SharedFactory(factory3)),
    ]);
    let empty = IgnoredSubdirectories::empty();

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    f.assert_no_warnings_or_errors();
    assert_eq!(
        *processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED on first call to getDiff for diffAwareness1"
    );
    manager.mark_processed(&processable_diff);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(f.count_containing("error in getCurrentView"), 1);
    assert_eq!(
        *processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED because of broken getCurrentView"
    );
    manager.mark_processed(&processable_diff);
    factory1.remove(&f.root);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(
        *processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED on first call to getDiff for diffAwareness2"
    );
    manager.mark_processed(&processable_diff);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(*processable_diff.get_modified_file_set(), diff2);
    manager.mark_processed(&processable_diff);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(f.count_containing("error in getDiff"), 1);
    assert_eq!(
        *processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED because of broken getDiff"
    );
    manager.mark_processed(&processable_diff);
    factory2.remove(&f.root);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(
        *processable_diff.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED,
        "Expected EVERYTHING_MODIFIED on first call to getDiff for diffAwareness3"
    );
    manager.mark_processed(&processable_diff);

    let processable_diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert_eq!(*processable_diff.get_modified_file_set(), diff3);
    manager.mark_processed(&processable_diff);
}

/// The `mock(DiffAwareness.Factory.class)` of `testIndependentAwarenessPerIgnoredPaths`: a diff
/// awareness per ignored paths.
struct PerIgnoredPathsFactory(Mutex<Vec<(IgnoredSubdirectories, Box<dyn DiffAwareness>)>>);

impl DiffAwarenessFactory for PerIgnoredPathsFactory {
    fn maybe_create(
        &self,
        _path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        _options: &LocalDiffAwarenessOptions,
    ) -> Option<Box<dyn DiffAwareness>> {
        let mut v = self.0.lock();
        let i = v.iter().position(|(p, _)| p == ignored_paths)?;
        Some(v.remove(i).1)
    }
}

fn ignored(path: &str) -> IgnoredSubdirectories {
    let mut prefixes = IndexSet::new();
    prefixes.insert(PathFragment::create(path));
    IgnoredSubdirectories::of(prefixes, vec![])
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:220 DiffAwarenessManagerTest.testIndependentAwarenessPerIgnoredPaths
#[test]
fn test_independent_awareness_per_ignored_paths() {
    let f = Fixture::new("path");
    let diff1 = modified_file_set(&["path/ignored-path-2/foo"]);
    let (diff_awareness1, _) = DiffAwarenessStub::new(vec![diff1.clone()]);
    let diff2 = modified_file_set(&["path/ignored-path-1/foo"]);
    let (diff_awareness2, _) = DiffAwarenessStub::new(vec![diff2.clone()]);
    let factory = PerIgnoredPathsFactory(Mutex::new(vec![
        (
            ignored("path/ignored-path-1"),
            diff_awareness1 as Box<dyn DiffAwareness>,
        ),
        (
            ignored("path/ignored-path-2"),
            diff_awareness2 as Box<dyn DiffAwareness>,
        ),
    ]));
    let mut manager = DiffAwarenessManager::new(vec![Arc::new(factory)]);

    let processed_diff1 = manager.get_diff(
        f.events.as_ref(),
        &f.root,
        &ignored("path/ignored-path-1"),
        &options(),
    );
    manager.mark_processed(&processed_diff1);
    assert_eq!(
        *processed_diff1.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED
    );
    let processed_diff1 = manager.get_diff(
        f.events.as_ref(),
        &f.root,
        &ignored("path/ignored-path-1"),
        &options(),
    );
    manager.mark_processed(&processed_diff1);
    assert_eq!(*processed_diff1.get_modified_file_set(), diff1);

    let processed_diff2 = manager.get_diff(
        f.events.as_ref(),
        &f.root,
        &ignored("path/ignored-path-2"),
        &options(),
    );
    manager.mark_processed(&processed_diff2);
    assert_eq!(
        *processed_diff2.get_modified_file_set(),
        ModifiedFileSet::EVERYTHING_MODIFIED
    );
    let processed_diff2 = manager.get_diff(
        f.events.as_ref(),
        &f.root,
        &ignored("path/ignored-path-2"),
        &options(),
    );
    manager.mark_processed(&processed_diff2);
    assert_eq!(*processed_diff2.get_modified_file_set(), diff2);
}

struct WorkspaceInfo;
impl WorkspaceInfoFromDiff for WorkspaceInfo {}

// Java: DiffAwarenessManagerTest.createView
struct InfoView(Option<Arc<dyn WorkspaceInfoFromDiff>>);

impl std::fmt::Display for InfoView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InfoView")
    }
}

impl View for InfoView {
    fn get_workspace_info(&self) -> Option<Arc<dyn WorkspaceInfoFromDiff>> {
        self.0.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The `mock(DiffAwareness.class)` of the workspace info tests: returns the given views in turn
/// and the given outcome of `getDiff(view1, view2)`.
struct ScriptedDiffAwareness {
    views: std::collections::VecDeque<Arc<dyn View>>,
    diff: Result<ModifiedFileSet, DiffError>,
}

impl DiffAwareness for ScriptedDiffAwareness {
    fn get_current_view(
        &mut self,
        _options: &LocalDiffAwarenessOptions,
    ) -> Result<Arc<dyn View>, BrokenDiffAwarenessException> {
        // Mockito's thenReturn(view1, view2) repeats the last value.
        if self.views.len() > 1 {
            Ok(self.views.pop_front().unwrap())
        } else {
            Ok(self.views[0].clone())
        }
    }

    fn get_diff(
        &mut self,
        old_view: Option<&Arc<dyn View>>,
        _new_view: &Arc<dyn View>,
    ) -> Result<ModifiedFileSet, DiffError> {
        match old_view {
            // An unstubbed mock returns null for getDiff(null, view1); the manager hands it on.
            None => Ok(ModifiedFileSet::EVERYTHING_MODIFIED),
            Some(_) => self.diff.clone(),
        }
    }

    fn name(&self) -> &'static str {
        "mock"
    }

    fn close(&mut self) {}
}

struct OnceFactory(Mutex<Option<Box<dyn DiffAwareness>>>);

impl DiffAwarenessFactory for OnceFactory {
    fn maybe_create(
        &self,
        _path_entry: &Root,
        _ignored_paths: &IgnoredSubdirectories,
        _options: &LocalDiffAwarenessOptions,
    ) -> Option<Box<dyn DiffAwareness>> {
        self.0.lock().take()
    }
}

fn manager_with(
    views: Vec<Arc<dyn View>>,
    diff: Result<ModifiedFileSet, DiffError>,
) -> DiffAwarenessManager {
    DiffAwarenessManager::new(vec![Arc::new(OnceFactory(Mutex::new(Some(Box::new(
        ScriptedDiffAwareness {
            views: views.into(),
            diff,
        },
    )))))])
}

fn same(a: Option<Arc<dyn WorkspaceInfoFromDiff>>, b: &Arc<dyn WorkspaceInfoFromDiff>) -> bool {
    a.is_some_and(|a| std::ptr::addr_eq(Arc::as_ptr(&a), Arc::as_ptr(b)))
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:267 DiffAwarenessManagerTest.getDiff_cleanBuild_propagatesWorkspaceInfo
#[test]
fn get_diff_clean_build_propagates_workspace_info() {
    let f = Fixture::new("path");
    let workspace_info: Arc<dyn WorkspaceInfoFromDiff> = Arc::new(WorkspaceInfo);
    let mut manager = manager_with(
        vec![Arc::new(InfoView(Some(workspace_info.clone())))],
        Ok(ModifiedFileSet::nothing_modified()),
    );
    let diff = manager.get_diff(
        f.events.as_ref(),
        &f.root,
        &IgnoredSubdirectories::empty(),
        &options(),
    );
    assert!(same(diff.get_workspace_info(), &workspace_info));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:283 DiffAwarenessManagerTest.getDiff_incrementalBuild_propagatesLatestWorkspaceInfo
#[test]
fn get_diff_incremental_build_propagates_latest_workspace_info() {
    let f = Fixture::new("path");
    let workspace_info1: Arc<dyn WorkspaceInfoFromDiff> = Arc::new(WorkspaceInfo);
    let workspace_info2: Arc<dyn WorkspaceInfoFromDiff> = Arc::new(WorkspaceInfo);
    let mut manager = manager_with(
        vec![
            Arc::new(InfoView(Some(workspace_info1))),
            Arc::new(InfoView(Some(workspace_info2.clone()))),
        ],
        Ok(modified_file_set(&["file"])),
    );
    let empty = IgnoredSubdirectories::empty();
    let _ = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    let diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(same(diff.get_workspace_info(), &workspace_info2));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:305 DiffAwarenessManagerTest.getDiff_incrementalBuildNoChange_propagatesNewWorkspaceInfo
#[test]
fn get_diff_incremental_build_no_change_propagates_new_workspace_info() {
    let f = Fixture::new("path");
    let workspace_info1: Arc<dyn WorkspaceInfoFromDiff> = Arc::new(WorkspaceInfo);
    let workspace_info2: Arc<dyn WorkspaceInfoFromDiff> = Arc::new(WorkspaceInfo);
    let mut manager = manager_with(
        vec![
            Arc::new(InfoView(Some(workspace_info1))),
            Arc::new(InfoView(Some(workspace_info2.clone()))),
        ],
        Ok(ModifiedFileSet::nothing_modified()),
    );
    let empty = IgnoredSubdirectories::empty();
    let _ = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    let diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(same(diff.get_workspace_info(), &workspace_info2));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:326 DiffAwarenessManagerTest.getDiff_incrementalBuildWithNoWorkspaceInfo_returnsDiffWithNullWorkspaceInfo
#[test]
fn get_diff_incremental_build_with_no_workspace_info_returns_diff_with_null_workspace_info() {
    let f = Fixture::new("path");
    let mut manager = manager_with(
        vec![
            Arc::new(InfoView(Some(Arc::new(WorkspaceInfo)))),
            Arc::new(InfoView(None)),
        ],
        Ok(modified_file_set(&["file"])),
    );
    let empty = IgnoredSubdirectories::empty();
    let _ = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    let diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(diff.get_workspace_info().is_none());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:347 DiffAwarenessManagerTest.getDiff_brokenDiffAwareness_returnsDiffWithNullWorkspaceInfo
#[test]
fn get_diff_broken_diff_awareness_returns_diff_with_null_workspace_info() {
    let f = Fixture::new("path");
    let mut manager = manager_with(
        vec![
            Arc::new(InfoView(Some(Arc::new(WorkspaceInfo)))),
            Arc::new(InfoView(Some(Arc::new(WorkspaceInfo)))),
        ],
        Err(DiffError::Broken(BrokenDiffAwarenessException(
            String::new(),
        ))),
    );
    let empty = IgnoredSubdirectories::empty();
    let diff1 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    manager.mark_processed(&diff1);
    let diff = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    assert!(diff.get_workspace_info().is_none());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/DiffAwarenessManagerTest.java:368 DiffAwarenessManagerTest.getDiff_incompatibleDiff_fails
#[test]
fn get_diff_incompatible_diff_fails() {
    let f = Fixture::new("path");
    let mut manager = manager_with(
        vec![Arc::new(InfoView(None)), Arc::new(InfoView(None))],
        Err(DiffError::IncompatibleView(IncompatibleViewException(
            String::new(),
        ))),
    );
    let empty = IgnoredSubdirectories::empty();
    let diff1 = manager.get_diff(f.events.as_ref(), &f.root, &empty, &options());
    manager.mark_processed(&diff1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        manager.get_diff(f.events.as_ref(), &f.root, &empty, &options())
    }));
    let message = result.err().expect("IllegalStateException");
    let message = message
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.starts_with("IllegalStateException"), "{message}");
}
