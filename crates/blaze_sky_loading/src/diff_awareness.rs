//! Ports of `skyframe/DiffAwareness.java`, `BrokenDiffAwarenessException.java`,
//! `WorkspaceInfoFromDiff.java`, `WorkspaceInfoFromDiffReceiver.java`,
//! `DiffAwarenessManager.java`, `LocalDiffAwareness.java` and `WatchServiceDiffAwareness.java`:
//! the computation of the files modified under a package path entry between two commands
//! (`--watchfs`).
//!
//! PORT: the `OptionsProvider` that `getCurrentView` reads is the one options class Bazel's diff
//! awarenesses read (`LocalDiffAwareness.Options`). `MacOSXFsEventsDiffAwareness` and the Windows
//! branches are not ported (Linux only).

use std::any::Any;
use std::collections::HashSet;
use std::path::{Path as OsPath, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::local_diff_awareness::LocalDiffAwarenessOptions;
use blaze_util::string_encoding::{internal_to_platform, platform_to_internal};
use blaze_vfs::{ModifiedFileSet, PathFragment, Root};

use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::watch_service::{WatchEventKind, WatchKey, WatchService};

/// Thrown on `getDiff` to indicate that something is wrong with the `DiffAwareness` instance and
/// it should not be used again.
// Java: com/google/devtools/build/lib/skyframe/BrokenDiffAwarenessException.java:22 BrokenDiffAwarenessException
#[derive(Clone, Debug)]
pub struct BrokenDiffAwarenessException(pub String);

impl BrokenDiffAwarenessException {
    pub fn get_message(&self) -> &str {
        &self.0
    }
}

/// `IncompatibleViewException`: the views given to `getDiff` are not from this instance.
// Java: com/google/devtools/build/lib/skyframe/IncompatibleViewException.java:17 IncompatibleViewException
#[derive(Clone, Debug)]
pub struct IncompatibleViewException(pub String);

/// The errors of `DiffAwareness.getDiff`.
#[derive(Clone, Debug)]
pub enum DiffError {
    Broken(BrokenDiffAwarenessException),
    IncompatibleView(IncompatibleViewException),
}

/// Information for a workspace computed at the time of collecting diff.
///
/// PORT: its two methods (the evaluating version and the snapshot) serve external version
/// control systems, which Bazel does not have; nothing reads them.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceInfoFromDiff.java:22 WorkspaceInfoFromDiff
pub trait WorkspaceInfoFromDiff: Send + Sync {}

/// Callback interface to receive `WorkspaceInfoFromDiff` when synchronizing the workspace.
// Java: com/google/devtools/build/lib/skyframe/WorkspaceInfoFromDiffReceiver.java:20 WorkspaceInfoFromDiffReceiver
pub trait WorkspaceInfoFromDiffReceiver: Send + Sync {
    fn sync_workspace_info_from_diff(
        &self,
        package_path: &PathFragment,
        workspace_info_from_diff: Option<Arc<dyn WorkspaceInfoFromDiff>>,
    );
}

/// Opaque view of the filesystem under a package path entry at a specific point in time.
// Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:52 DiffAwareness.View
pub trait View: Any + Send + Sync + std::fmt::Display {
    /// Returns workspace info unanimously associated with the package path or null.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:55 View.getWorkspaceInfo
    fn get_workspace_info(&self) -> Option<Arc<dyn WorkspaceInfoFromDiff>> {
        None
    }

    fn as_any(&self) -> &dyn Any;
}

/// Interface for computing modifications of files under a package path entry.
// Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:33 DiffAwareness
pub trait DiffAwareness: Send {
    /// Returns the live view of the filesystem under the package path entry.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:69 DiffAwareness.getCurrentView
    fn get_current_view(
        &mut self,
        options: &LocalDiffAwarenessOptions,
    ) -> Result<Arc<dyn View>, BrokenDiffAwarenessException>;

    /// Returns the set of files of interest that have been modified between the given two views.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:83 DiffAwareness.getDiff
    fn get_diff(
        &mut self,
        old_view: Option<&Arc<dyn View>>,
        new_view: &Arc<dyn View>,
    ) -> Result<ModifiedFileSet, DiffError>;

    /// The name of this implementation.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:87 DiffAwareness.name
    fn name(&self) -> &'static str;

    /// Must be called whenever the `DiffAwareness` object is to be discarded.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:94 DiffAwareness.close
    fn close(&mut self);
}

/// Factory for creating `DiffAwareness` instances.
// Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:35 DiffAwareness.Factory
pub trait DiffAwarenessFactory: Send + Sync {
    /// Returns a `DiffAwareness` instance suitable for managing changes to files under the given
    /// package path entry, or `None` if this factory cannot create such an instance.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwareness.java:44 Factory.maybeCreate
    fn maybe_create(
        &self,
        path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        options: &LocalDiffAwarenessOptions,
    ) -> Option<Box<dyn DiffAwareness>>;
}

// -------------------------------------------------------------------------------------------
// DiffAwarenessManager
// -------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:58 DiffAwarenessManager.DiffAwarenessState
struct DiffAwarenessState {
    root: Root,
    ignored_paths: IgnoredSubdirectories,
    diff_awareness: Box<dyn DiffAwareness>,
    /// The `View` that should be the baseline for the next `getDiff` call, or `None` if the next
    /// `getDiff` will be the first incremental one.
    baseline_view: Option<Arc<dyn View>>,
}

/// Helper class to make it easier to correctly use the `DiffAwareness` interface in a sequential
/// manner.
///
/// PORT: the states are kept in a list keyed by `(root, ignored paths)` (Java's `StateKey` in a
/// hash map); the list is as long as the package path.
// Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:38 DiffAwarenessManager
pub struct DiffAwarenessManager {
    // The manager attempts to instantiate these in the order in which they are passed to the
    // constructor; this is critical in the case where a factory always succeeds.
    diff_awareness_factories: Vec<Arc<dyn DiffAwarenessFactory>>,
    current_diff_awareness_states: Vec<DiffAwarenessState>,
}

/// A set of modified files that should be marked as processed.
// Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:89 DiffAwarenessManager.ProcessableModifiedFileSet
pub struct ProcessableModifiedFileSet {
    modified_file_set: ModifiedFileSet,
    /// `None` for `BrokenProcessableModifiedFileSet`.
    processable: Option<(Root, IgnoredSubdirectories, Arc<dyn View>)>,
}

impl ProcessableModifiedFileSet {
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:90 ProcessableModifiedFileSet.getModifiedFileSet
    pub fn get_modified_file_set(&self) -> &ModifiedFileSet {
        &self.modified_file_set
    }

    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:93 ProcessableModifiedFileSet.getWorkspaceInfo
    pub fn get_workspace_info(&self) -> Option<Arc<dyn WorkspaceInfoFromDiff>> {
        self.processable
            .as_ref()
            .and_then(|(_, _, next_view)| next_view.get_workspace_info())
    }

    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:221 BrokenProcessableModifiedFileSet
    fn broken() -> ProcessableModifiedFileSet {
        ProcessableModifiedFileSet {
            modified_file_set: ModifiedFileSet::EVERYTHING_MODIFIED,
            processable: None,
        }
    }
}

impl DiffAwarenessManager {
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:53 DiffAwarenessManager.<init>
    pub fn new(diff_awareness_factories: Vec<Arc<dyn DiffAwarenessFactory>>) -> Self {
        DiffAwarenessManager {
            diff_awareness_factories,
            current_diff_awareness_states: Vec::new(),
        }
    }

    /// Reset internal `DiffAwareness` state.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:80 DiffAwarenessManager.reset
    pub fn reset(&mut self) {
        for state in &mut self.current_diff_awareness_states {
            state.diff_awareness.close();
        }
        self.current_diff_awareness_states.clear();
    }

    fn index_of(&self, path_entry: &Root, ignored_paths: &IgnoredSubdirectories) -> Option<usize> {
        self.current_diff_awareness_states
            .iter()
            .position(|s| s.root == *path_entry && s.ignored_paths == *ignored_paths)
    }

    /// Gets the set of changed files since the last call with this path entry, or
    /// `ModifiedFileSet.EVERYTHING_MODIFIED` if this is the first such call.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:107 DiffAwarenessManager.getDiff
    pub fn get_diff(
        &mut self,
        event_handler: &dyn EventHandler,
        path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        options: &LocalDiffAwarenessOptions,
    ) -> ProcessableModifiedFileSet {
        let Some(index) = self.maybe_get_diff_awareness_state(path_entry, ignored_paths, options)
        else {
            return ProcessableModifiedFileSet::broken();
        };
        let new_view = {
            let _c = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::INFO,
                "diffAwareness.getCurrentView",
            );
            self.current_diff_awareness_states[index]
                .diff_awareness
                .get_current_view(options)
        };
        let new_view = match new_view {
            Ok(v) => v,
            Err(e) => {
                self.handle_broken_diff_awareness(event_handler, path_entry, ignored_paths, &e);
                return ProcessableModifiedFileSet::broken();
            }
        };
        let state = &mut self.current_diff_awareness_states[index];
        let baseline_view = state.baseline_view.clone();
        // PORT: logger.atInfo() "About to compute diff between ..." is not ported.
        let diff = {
            let _c = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::INFO,
                "diffAwareness.getDiff",
            );
            state
                .diff_awareness
                .get_diff(baseline_view.as_ref(), &new_view)
        };
        match diff {
            Ok(diff) => ProcessableModifiedFileSet {
                modified_file_set: diff,
                processable: Some((path_entry.clone(), ignored_paths.clone(), new_view)),
            },
            Err(DiffError::Broken(e)) => {
                self.handle_broken_diff_awareness(event_handler, path_entry, ignored_paths, &e);
                ProcessableModifiedFileSet::broken()
            }
            Err(DiffError::IncompatibleView(e)) => panic!(
                "IllegalStateException: {} {} {}: {}",
                path_entry,
                baseline_view
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "null".to_string()),
                new_view,
                e.0
            ),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:139 DiffAwarenessManager.handleBrokenDiffAwareness
    fn handle_broken_diff_awareness(
        &mut self,
        event_handler: &dyn EventHandler,
        path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        e: &BrokenDiffAwarenessException,
    ) {
        if let Some(i) = self.index_of(path_entry, ignored_paths) {
            self.current_diff_awareness_states.remove(i);
        }
        // PORT: logger.atInfo() "Broken diff awareness for ..." is not ported.
        event_handler.handle(&Event::warn(format!(
            "{}... temporarily falling back to manually checking files for changes",
            e.get_message()
        )));
    }

    /// Returns the current diff awareness for the given path entry, or a fresh one if there is no
    /// current one, or otherwise `None` if no factory could make a fresh one.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:155 DiffAwarenessManager.maybeGetDiffAwarenessState
    fn maybe_get_diff_awareness_state(
        &mut self,
        path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        options: &LocalDiffAwarenessOptions,
    ) -> Option<usize> {
        if let Some(i) = self.index_of(path_entry, ignored_paths) {
            return Some(i);
        }
        for factory in &self.diff_awareness_factories {
            if let Some(new_diff_awareness) =
                factory.maybe_create(path_entry, ignored_paths, options)
            {
                // PORT: logger.atInfo() "Using %s DiffAwareness strategy for %s" is not ported.
                self.current_diff_awareness_states.push(DiffAwarenessState {
                    root: path_entry.clone(),
                    ignored_paths: ignored_paths.clone(),
                    diff_awareness: new_diff_awareness,
                    baseline_view: None,
                });
                return Some(self.current_diff_awareness_states.len() - 1);
            }
        }
        None
    }

    /// `ProcessableModifiedFileSet.markProcessed()`: the next `getDiff` of the path entry starts
    /// from this set's view. Nothing for a broken set.
    // Java: com/google/devtools/build/lib/skyframe/DiffAwarenessManager.java:208 ProcessableModifiedFileSetImpl.markProcessed
    pub fn mark_processed(&mut self, modified_file_set: &ProcessableModifiedFileSet) {
        let Some((path_entry, ignored_paths, next_view)) = &modified_file_set.processable else {
            return;
        };
        if let Some(i) = self.index_of(path_entry, ignored_paths) {
            self.current_diff_awareness_states[i].baseline_view = Some(next_view.clone());
        }
    }
}

// -------------------------------------------------------------------------------------------
// LocalDiffAwareness
// -------------------------------------------------------------------------------------------

/// Factory for creating `LocalDiffAwareness` instances.
// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:72 LocalDiffAwareness.Factory
pub struct LocalDiffAwarenessFactory {
    excluded_network_file_systems_prefixes: Vec<String>,
}

impl LocalDiffAwarenessFactory {
    /// Creates a new factory; the file system watcher may not work on all file systems,
    /// particularly for network file systems. The prefix list can be used to exclude known paths
    /// that point to network file systems.
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:80 Factory.<init>
    pub fn new(excluded_network_file_systems_prefixes: Vec<String>) -> Self {
        LocalDiffAwarenessFactory {
            excluded_network_file_systems_prefixes,
        }
    }
}

impl DiffAwarenessFactory for LocalDiffAwarenessFactory {
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:84 Factory.maybeCreate
    fn maybe_create(
        &self,
        path_entry: &Root,
        ignored_paths: &IgnoredSubdirectories,
        _options: &LocalDiffAwarenessOptions,
    ) -> Option<Box<dyn DiffAwareness>> {
        let resolved_path_entry = path_entry.as_path()?.resolve_symbolic_links().ok()?;
        let resolved_path_entry_fragment = resolved_path_entry.as_fragment();
        // There's no good way to automatically detect network file systems. We rely on a list of
        // paths to exclude for now (and maybe add a command-line option in the future?).
        for prefix in &self.excluded_network_file_systems_prefixes {
            if resolved_path_entry_fragment.starts_with(&PathFragment::create(prefix)) {
                return None;
            }
        }
        let watch_root = PathBuf::from(
            internal_to_platform(resolved_path_entry_fragment.get_path_string()).into_owned(),
        );
        // PORT: on OSX Java uses MacOSXFsEventsDiffAwareness (not ported).
        Some(Box::new(WatchServiceDiffAwareness::new(
            watch_root,
            ignored_paths.clone(),
        )))
    }
}

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// The `WatchService` is inherently sequential and side-effectful, so we enforce this by only
/// supporting `getDiff` calls that happen to be sequential.
// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:140 LocalDiffAwareness.SequentialView
pub struct SequentialView {
    /// The owning `LocalDiffAwareness` (`None` for `EVERYTHING_MODIFIED`).
    owner: Option<u64>,
    position: i64,
    modified_absolute_paths: HashSet<PathBuf>,
}

impl std::fmt::Display for SequentialView {
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:153 SequentialView.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let owner = match self.owner {
            Some(o) => format!("LocalDiffAwareness#{o}"),
            None => "null".to_string(),
        };
        let mut paths: Vec<String> = self
            .modified_absolute_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        paths.sort();
        write!(
            f,
            "SequentialView[owner={}, position={}, modifiedAbsolutePaths=[{}]]",
            owner,
            self.position,
            paths.join(", ")
        )
    }
}

impl View for SequentialView {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A view that results in any subsequent getDiff calls returning
/// `ModifiedFileSet::EVERYTHING_MODIFIED`. Use this if --watchFs is disabled.
///
/// The position is set to -2 in order for `areInSequence` below to always return false if this
/// view is passed to it.
// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:120 LocalDiffAwareness.EVERYTHING_MODIFIED
pub fn everything_modified_view() -> Arc<dyn View> {
    Arc::new(SequentialView {
        owner: None,
        position: -2,
        modified_absolute_paths: HashSet::new(),
    })
}

// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:123 LocalDiffAwareness.areInSequence
pub fn are_in_sequence(old_view: &SequentialView, new_view: &SequentialView) -> bool {
    // Keep this in sync with the EVERYTHING_MODIFIED View above.
    old_view.owner == new_view.owner && (old_view.position + 1) == new_view.position
}

/// The state shared by local diff awarenesses (Java's abstract class `LocalDiffAwareness`).
// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:40 LocalDiffAwareness
struct LocalDiffAwareness {
    owner: u64,
    num_get_current_view_calls: i64,
    /// Root directory to watch. This is an absolute path.
    watch_root: PathBuf,
}

impl LocalDiffAwareness {
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:133 LocalDiffAwareness.<init>
    fn new(watch_root: PathBuf) -> Self {
        LocalDiffAwareness {
            owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
            num_get_current_view_calls: 0,
            watch_root,
        }
    }

    /// Returns true on any call before first call to `newView`.
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:162 LocalDiffAwareness.isFirstCall
    fn is_first_call(&self) -> bool {
        self.num_get_current_view_calls == 0
    }

    /// Create a new views using a list of modified absolute paths. This will increase the view
    /// counter.
    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:170 LocalDiffAwareness.newView
    fn new_view(&mut self, modified_absolute_paths: HashSet<PathBuf>) -> Arc<dyn View> {
        self.num_get_current_view_calls += 1;
        Arc::new(SequentialView {
            owner: Some(self.owner),
            position: self.num_get_current_view_calls,
            modified_absolute_paths,
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:176 LocalDiffAwareness.getDiff
    fn get_diff(
        &self,
        old_view: Option<&Arc<dyn View>>,
        new_view: &Arc<dyn View>,
    ) -> Result<ModifiedFileSet, DiffError> {
        let Some(old_view) = old_view else {
            return Ok(ModifiedFileSet::EVERYTHING_MODIFIED);
        };
        let (Some(old_sequential_view), Some(new_sequential_view)) = (
            old_view.as_any().downcast_ref::<SequentialView>(),
            new_view.as_any().downcast_ref::<SequentialView>(),
        ) else {
            return Err(DiffError::IncompatibleView(IncompatibleViewException(
                "Given views are not from LocalDiffAwareness".to_string(),
            )));
        };
        if !are_in_sequence(old_sequential_view, new_sequential_view) {
            return Ok(ModifiedFileSet::EVERYTHING_MODIFIED);
        }

        let mut result_builder = ModifiedFileSet::builder();
        // PORT: Java iterates a HashSet; ModifiedFileSet keeps its paths in a sorted set, so the
        // order does not matter.
        for modified_path in &new_sequential_view.modified_absolute_paths {
            let Ok(relative) = modified_path.strip_prefix(&self.watch_root) else {
                return Err(DiffError::Broken(BrokenDiffAwarenessException(format!(
                    "{} is not under {}",
                    modified_path.display(),
                    self.watch_root.display()
                ))));
            };
            let relative_path = PathFragment::create(&platform_to_internal(relative.as_os_str()));
            if !relative_path.is_empty() {
                result_builder = result_builder.modify(relative_path);
            }
        }
        Ok(result_builder.build())
    }
}

/// File system watcher for local filesystems. It's able to provide a list of changed files
/// between two consecutive calls. Uses the watch service (inotify).
// Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:47 WatchServiceDiffAwareness
pub struct WatchServiceDiffAwareness {
    local: LocalDiffAwareness,
    /// Bijection from WatchKey to the (absolute) Path being watched.
    watch_key_to_dir: std::collections::HashMap<WatchKey, PathBuf>,
    dir_to_watch_key: std::collections::HashMap<PathBuf, WatchKey>,
    /// Every directory is registered under this watch service.
    watch_service: Option<WatchService>,
    ignored_paths: IgnoredSubdirectories,
}

impl WatchServiceDiffAwareness {
    /// `new SequentialView(this, position, modifiedAbsolutePaths)` (LocalDiffAwarenessTest).
    pub fn sequential_view_for_testing(
        &self,
        position: i64,
        modified_absolute_paths: HashSet<PathBuf>,
    ) -> Arc<dyn View> {
        Arc::new(SequentialView {
            owner: Some(self.local.owner),
            position,
            modified_absolute_paths,
        })
    }

    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:61 WatchServiceDiffAwareness.<init>
    pub fn new(watch_root: PathBuf, ignored_paths: IgnoredSubdirectories) -> Self {
        WatchServiceDiffAwareness {
            local: LocalDiffAwareness::new(watch_root),
            watch_key_to_dir: Default::default(),
            dir_to_watch_key: Default::default(),
            watch_service: None,
            ignored_paths,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:66 WatchServiceDiffAwareness.init
    fn init(&mut self) {
        blaze_util::check_state!(self.watch_service.is_none());
        // According to the docs, this can never happen with the default file system provider.
        self.watch_service = WatchService::new().ok();
    }

    fn put_key(&mut self, key: WatchKey, dir: PathBuf) {
        // HashBiMap.put: a value already bound to another key is an error in Guava; inotify
        // returns the existing key for a directory that is already watched.
        if let Some(old) = self.dir_to_watch_key.insert(dir.clone(), key)
            && old != key
        {
            self.watch_key_to_dir.remove(&old);
        }
        self.watch_key_to_dir.insert(key, dir);
    }

    fn remove_key(&mut self, key: WatchKey) {
        if let Some(dir) = self.watch_key_to_dir.remove(&key) {
            self.dir_to_watch_key.remove(&dir);
        }
    }

    /// Returns the changed files caught by the watch service.
    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:162 WatchServiceDiffAwareness.collectChanges
    fn collect_changes(&mut self) -> Result<HashSet<PathBuf>, CollectError> {
        let mut created_files_and_directories: HashSet<PathBuf> = HashSet::new();
        let mut deleted_or_modified_files_and_directories: HashSet<PathBuf> = HashSet::new();
        let mut deleted_tracked_directories: HashSet<PathBuf> = HashSet::new();

        loop {
            let watch_service = self.watch_service.as_ref().expect("initialized");
            let watch_key = match watch_service.poll() {
                Ok(Some(k)) => k,
                Ok(None) => break,
                Err(e) => return Err(CollectError::Closed(e.to_string())),
            };
            let dir = self.watch_key_to_dir.get(&watch_key).cloned();
            blaze_util::check_argument!(dir.is_some());
            let dir = dir.unwrap();
            // We replay all the events for this watched directory in chronological order and
            // construct the diff of this directory since the last #collectChanges call.
            for event in watch_service.poll_events(watch_key) {
                let kind = event.kind;
                if kind == WatchEventKind::Overflow {
                    // TODO(bazel-team): find out when an overflow might happen, and maybe handle
                    // it more gently.
                    return Err(CollectError::Broken(BrokenDiffAwarenessException(
                        "Overflow when watching local filesystem for changes".to_string(),
                    )));
                }
                let Some(relative_path) = event.context else {
                    return Err(CollectError::Broken(BrokenDiffAwarenessException(
                        "Insufficient information from local file system watcher".to_string(),
                    )));
                };
                // For the events we've registered, the context given is a relative path.
                let path = dir.join(relative_path);
                blaze_util::check_state!(path.is_absolute(), "{}", path.display());
                match kind {
                    WatchEventKind::Create => {
                        created_files_and_directories.insert(path.clone());
                        deleted_or_modified_files_and_directories.remove(&path);
                    }
                    WatchEventKind::Delete => {
                        created_files_and_directories.remove(&path);
                        deleted_or_modified_files_and_directories.insert(path.clone());
                        if let Some(&deleted_directory_key) = self.dir_to_watch_key.get(&path) {
                            // If the deleted directory has children, then there will also be
                            // events for the WatchKey of the directory itself; we don't want to
                            // remove the deleted directory from our bimap just yet.
                            deleted_tracked_directories.insert(path.clone());
                            // Since inotify uses inodes under the covers we cancel our
                            // registration on this key to avoid getting WatchEvents from a new
                            // directory that happens to have the same inode.
                            watch_service.cancel(deleted_directory_key);
                        }
                    }
                    WatchEventKind::Modify => {
                        // If a file was created and then modified, then the net diff is that it
                        // was created.
                        if !created_files_and_directories.contains(&path) {
                            deleted_or_modified_files_and_directories.insert(path);
                        }
                    }
                    WatchEventKind::Overflow => unreachable!(),
                }
            }
            if !watch_service.reset(watch_key) {
                // Watcher got deleted, directory no longer valid.
                self.remove_key(watch_key);
            }
        }

        for path in &deleted_tracked_directories {
            if let Some(stale_key) = self.dir_to_watch_key.get(path).copied() {
                self.remove_key(stale_key);
            }
        }
        if self.watch_key_to_dir.is_empty() {
            // No more directories to watch, something happened the root directory being watched.
            return Err(CollectError::Io(format!(
                "java.io.IOException: Root directory {} became inaccessible.",
                self.local.watch_root.display()
            )));
        }

        let mut changed_paths: HashSet<PathBuf> = HashSet::new();
        for path in created_files_and_directories {
            if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
                // This is a new directory, so changes to it since its creation have not been
                // watched. We manually traverse the directory tree to register all the new
                // subdirectories and find all the new subdirectories and files.
                changed_paths.extend(
                    self.register_sub_directories_and_return_contents(&path)
                        .map_err(|e| CollectError::Io(java_io_exception_string(&e)))?,
                );
            } else {
                changed_paths.insert(path);
            }
        }
        changed_paths.extend(deleted_or_modified_files_and_directories);
        Ok(changed_paths)
    }

    /// Traverses directory tree to register subdirectories.
    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:271 WatchServiceDiffAwareness.registerSubDirectories
    fn register_sub_directories(&mut self, root_dir: &OsPath) -> std::io::Result<()> {
        self.register_sub_directories_and_return_contents(root_dir)
            .map(|_| ())
    }

    /// Traverses directory tree to register subdirectories. Returns all paths traversed (as
    /// absolute paths).
    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:281 WatchServiceDiffAwareness.registerSubDirectoriesAndReturnContents
    fn register_sub_directories_and_return_contents(
        &mut self,
        root_dir: &OsPath,
    ) -> std::io::Result<HashSet<PathBuf>> {
        let mut visited_absolute_paths = HashSet::new();
        // Note that this does not follow symlinks.
        self.walk_file_tree(root_dir, &mut visited_absolute_paths)?;
        Ok(visited_absolute_paths)
    }

    /// `Files.walkFileTree(start, visitor)` with `WatcherFileVisitor` (no symlinks followed; a
    /// failure to read a directory or its entries is an `IOException`, as
    /// `SimpleFileVisitor.visitFileFailed` rethrows it).
    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:291 WatcherFileVisitor
    fn walk_file_tree(
        &mut self,
        path: &OsPath,
        visited_absolute_paths: &mut HashSet<PathBuf>,
    ) -> std::io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            // WatcherFileVisitor.visitFile
            blaze_util::check_state!(path.is_absolute(), "{}", path.display());
            visited_absolute_paths.insert(path.to_path_buf());
            return Ok(());
        }
        // WatcherFileVisitor.preVisitDirectory
        // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:314 WatcherFileVisitor.preVisitDirectory
        let path_fragment =
            PathFragment::create(&platform_to_internal(path.as_os_str())).to_relative();
        if self.ignored_paths.matching_entry(&path_fragment).is_some() {
            return Ok(());
        }
        // It's important that we register the directory before we visit its children. This way
        // we are guaranteed to see new files/directories either on this #getDiff or the next one.
        blaze_util::check_state!(path.is_absolute(), "{}", path.display());
        let key = self
            .watch_service
            .as_ref()
            .expect("initialized")
            .register(path)?;
        self.put_key(key, path.to_path_buf());
        visited_absolute_paths.insert(path.to_path_buf());
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            self.walk_file_tree(&entry.path(), visited_absolute_paths)?;
        }
        Ok(())
    }
}

/// The failures of `collectChanges`.
enum CollectError {
    Broken(BrokenDiffAwarenessException),
    /// An `IOException`: its `toString()`.
    Io(String),
    /// A `ClosedWatchServiceException`: its `toString()`.
    Closed(String),
}

/// `IOException.toString()` of a Java I/O failure.
fn java_io_exception_string(e: &std::io::Error) -> String {
    format!("java.io.IOException: {e}")
}

impl DiffAwareness for WatchServiceDiffAwareness {
    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:75 WatchServiceDiffAwareness.getCurrentView
    fn get_current_view(
        &mut self,
        options: &LocalDiffAwarenessOptions,
    ) -> Result<Arc<dyn View>, BrokenDiffAwarenessException> {
        // We need to consider 4 cases for watchFs:
        // previous view    current view
        //  disabled         disabled  -> EVERYTHING_MODIFIED
        //  disabled         enabled   -> valid View (1)
        //  enabled          disabled  -> throw BrokenDiffAwarenessException
        //  enabled          enabled   -> valid View
        // PORT: Windows is not ported (`!isWindows || windowsWatchFS` is true).
        let watch_fs = options.watch_fs;
        if watch_fs && self.watch_service.is_none() {
            self.init();
        } else if !watch_fs && self.watch_service.is_some() {
            self.close();
            // The contract is that throwing BrokenDiffAwarenessException prevents reuse of the
            // same diff awareness object.
            return Err(BrokenDiffAwarenessException(
                "Switched off --watchfs again".to_string(),
            ));
        }
        // If init() failed, then this if also applies.
        if self.watch_service.is_none() {
            return Ok(everything_modified_view());
        }
        let modified_absolute_paths = if self.local.is_first_call() {
            let watch_root = self.local.watch_root.clone();
            if let Err(e) = self.register_sub_directories(&watch_root) {
                self.close();
                return Err(BrokenDiffAwarenessException(format!(
                    "Error encountered with local file system watcher {}",
                    java_io_exception_string(&e)
                )));
            }
            HashSet::new()
        } else {
            match self.collect_changes() {
                Ok(paths) => paths,
                Err(CollectError::Broken(e)) => {
                    self.close();
                    return Err(e);
                }
                Err(CollectError::Io(e)) => {
                    self.close();
                    return Err(BrokenDiffAwarenessException(format!(
                        "Error encountered with local file system watcher {e}"
                    )));
                }
                Err(CollectError::Closed(e)) => {
                    return Err(BrokenDiffAwarenessException(format!(
                        "Internal error with the local file system watcher {e}"
                    )));
                }
            }
        };
        Ok(self.local.new_view(modified_absolute_paths))
    }

    fn get_diff(
        &mut self,
        old_view: Option<&Arc<dyn View>>,
        new_view: &Arc<dyn View>,
    ) -> Result<ModifiedFileSet, DiffError> {
        self.local.get_diff(old_view, new_view)
    }

    // Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:216 LocalDiffAwareness.name
    fn name(&self) -> &'static str {
        "local"
    }

    // Java: com/google/devtools/build/lib/skyframe/WatchServiceDiffAwareness.java:150 WatchServiceDiffAwareness.close
    fn close(&mut self) {
        if let Some(watch_service) = &mut self.watch_service {
            watch_service.close();
        }
    }
}
