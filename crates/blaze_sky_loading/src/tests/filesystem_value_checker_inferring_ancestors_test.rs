//! Ports of `FileSystemValueCheckerInferringAncestorsTestBase.java` and
//! `FileSystemValueCheckerInferringAncestorsTest.java`.
//!
//! PORT: the graph is an evaluator without functions whose nodes are injected values
//! (`addDoneNode`) and invalidated through an empty evaluation (`markDirty(CHANGE)`); the
//! `@TestParameter fsvcThreads` is not a parameter of the port, which visits the entries one
//! after the other (the expectations of `fsvcThreads == 1` hold).

use std::io::{self, Read, Write};
use std::sync::Arc;

use blaze_skyframe::{EvaluationContext, Evaluator, EvaluatorBuilder, SkyKey, SkyValue};
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemExt, FileSystemRef,
    PathFragment, Root, RootedPath,
};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::actions::FileStateValue;
use crate::directory_listing::{DirectoryListingStateKey, DirectoryListingStateValue};
use crate::file_state::FileStateKey;
use crate::filesystem_value_checker::FileDirtinessChecker;
use crate::filesystem_value_checker_inferring_ancestors::{
    InferredDiff, StatFailedException, get_diff_with_inferred_ancestors,
};
use crate::test_util::Scratch;

/// What the tracking file system does on its next stat.
#[derive(Clone)]
enum ThrowOnStat {
    Io(String),
    Runtime(String),
}

/// `new DelegateFileSystem(scratch.getFileSystem()) { statIfFound ... }`: records the stated
/// paths relative to the source root and can throw on the next stat.
#[derive(Debug)]
struct TrackingFileSystem {
    delegate: FileSystemRef,
    src_root: PathFragment,
    stated_paths: Arc<Mutex<Vec<String>>>,
    #[allow(clippy::type_complexity)]
    throw_on_stat: Arc<Mutex<Option<ThrowOnStatDebug>>>,
}

#[derive(Debug, Clone)]
struct ThrowOnStatDebug(String, bool);

impl FileSystem for TrackingFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.delegate.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.delegate.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.delegate.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.create_directory_and_parents(path)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.delegate.get_file_size(path, follow_symlinks)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.delete(path)
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.delegate.get_last_modified_time(path, follow_symlinks)
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.delegate.set_last_modified_time(path, new_time)
    }
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.delegate.get_fast_digest(path)
    }
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        self.delegate.get_digest(path)
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        self.delegate.resolve_one_link(path)
    }
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.resolve_symbolic_links(path)
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.delegate.stat(path, follow_symlinks)
    }
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        self.delegate.stat_nullable(path, follow_symlinks)
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        if let Some(ThrowOnStatDebug(message, runtime)) = self.throw_on_stat.lock().take() {
            if runtime {
                panic!("RuntimeException: {message}");
            }
            return Err(io::Error::other(message));
        }
        self.stated_paths.lock().push(
            path.relative_to(&self.src_root)
                .get_path_string()
                .to_string(),
        );
        self.delegate.stat_if_found(path, follow_symlinks)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.read_symbolic_link(path)
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.delegate.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.delegate.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.delegate.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.delegate.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.delegate.set_executable(path, executable)
    }
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.delegate.chmod(path, mode)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.delegate.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.delegate.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        self.delegate.rename_to(source_path, target_path)
    }
    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate.create_hard_link(link_path, original_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

// Java: FileSystemValueCheckerInferringAncestorsTestBase
struct Fixture {
    scratch: Scratch,
    stated_paths: Arc<Mutex<Vec<String>>>,
    throw_on_stat: Arc<Mutex<Option<ThrowOnStatDebug>>>,
    root: Root,
    untracked_root: Root,
    evaluator: Evaluator,
}

impl Fixture {
    // Java: FileSystemValueCheckerInferringAncestorsTestBase.setUpGraphAndRoot
    fn new() -> Fixture {
        let scratch = Scratch::new();
        let src_root_path = scratch.dir("src");
        let src_root = src_root_path.as_fragment().clone();
        let stated_paths = Arc::new(Mutex::new(Vec::new()));
        let throw_on_stat = Arc::new(Mutex::new(None));
        let tracking: FileSystemRef = Arc::new(TrackingFileSystem {
            delegate: scratch.fs.clone(),
            src_root: src_root.clone(),
            stated_paths: stated_paths.clone(),
            throw_on_stat: throw_on_stat.clone(),
        });
        let root = Root::from_path(tracking.get_path_fragment(&src_root));
        Fixture {
            untracked_root: Root::from_path(src_root_path),
            scratch,
            stated_paths,
            throw_on_stat,
            root,
            evaluator: EvaluatorBuilder::new().build(),
        }
    }

    fn set_throw_on_stat(&self, t: ThrowOnStat) {
        *self.throw_on_stat.lock() = Some(match t {
            ThrowOnStat::Io(m) => ThrowOnStatDebug(m, false),
            ThrowOnStat::Runtime(m) => ThrowOnStatDebug(m, true),
        });
    }

    fn file(&self, rel: &str) -> std::path::PathBuf {
        self.file_with(rel, "")
    }

    fn file_with(&self, rel: &str, contents: &str) -> std::path::PathBuf {
        let p = self.scratch.file(&format!("src/{rel}"), contents);
        crate::test_util::platform(&p)
    }

    fn dir(&self, rel: &str) -> std::path::PathBuf {
        crate::test_util::platform(&self.scratch.dir(&format!("src/{rel}")))
    }

    fn file_state_value_key(&self, relative_path: &str) -> SkyKey {
        SkyKey::new(FileStateKey::key(RootedPath::to_rooted_path(
            self.root.clone(),
            &PathFragment::create(relative_path),
        )))
    }

    fn directory_listing_state_value_key(&self, relative_path: &str) -> SkyKey {
        SkyKey::new(DirectoryListingStateKey::key(RootedPath::to_rooted_path(
            self.root.clone(),
            &PathFragment::create(relative_path),
        )))
    }

    fn file_state_value(&self, relative_path: &str) -> SkyValue {
        Arc::new(
            FileStateValue::create_with_tsgm(
                &RootedPath::to_rooted_path(
                    self.untracked_root.clone(),
                    &PathFragment::create(relative_path),
                ),
                None,
            )
            .unwrap(),
        )
    }

    fn ctx() -> EvaluationContext {
        EvaluationContext::default()
    }

    // Java: FileSystemValueCheckerInferringAncestorsTest.addDoneNodes
    fn add_done_nodes(&self, values: Vec<(SkyKey, SkyValue)>) {
        for (key, value) in values {
            self.evaluator.inject_dyn(key, value);
        }
        self.evaluator.evaluate(&[], &Self::ctx()).unwrap();
    }

    // Java: FileSystemValueCheckerInferringAncestorsTest.addDoneNodesAndThenMarkChanged
    fn add_done_nodes_and_then_mark_changed(&self, values: Vec<(SkyKey, SkyValue)>) {
        let keys: Vec<SkyKey> = values.iter().map(|(k, _)| k.clone()).collect();
        self.add_done_nodes(values);
        self.evaluator.invalidate(keys);
        self.evaluator.evaluate(&[], &Self::ctx()).unwrap();
    }

    fn get_diff(&self, modified_keys: &[SkyKey]) -> Result<InferredDiff, StatFailedException> {
        get_diff_with_inferred_ancestors(
            None,
            &self.evaluator,
            modified_keys
                .iter()
                .map(|k| k.downcast::<FileStateKey>().unwrap().clone()),
            &FileDirtinessChecker,
        )
    }

    fn stated(&self) -> Vec<String> {
        let mut v = self.stated_paths.lock().clone();
        v.sort();
        v
    }
}

impl Drop for Fixture {
    // Java: FileSystemValueCheckerInferringAncestorsTestBase.checkExceptionThrown
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert!(self.throw_on_stat.lock().is_none());
        }
    }
}

fn nonexistent() -> SkyValue {
    Arc::new(FileStateValue::NONEXISTENT_FILE_STATE_NODE)
}

fn directory() -> SkyValue {
    Arc::new(FileStateValue::DIRECTORY_FILE_STATE_NODE)
}

fn dirent_file(name: &str) -> Dirent {
    Dirent::new(name, DirentType::File)
}

fn dirent_symlink(name: &str) -> Dirent {
    Dirent::new(name, DirentType::Symlink)
}

fn listing(dirents: Vec<Dirent>) -> SkyValue {
    Arc::new(DirectoryListingStateValue::create(dirents))
}

/// `containsExactly` on keys (any order).
fn assert_keys(actual: &IndexSet<SkyKey>, expected: &[SkyKey]) {
    let a: std::collections::HashSet<&SkyKey> = actual.iter().collect();
    let e: std::collections::HashSet<&SkyKey> = expected.iter().collect();
    assert_eq!(a, e, "actual {actual:?} expected {expected:?}");
    assert_eq!(actual.len(), expected.len());
}

/// `containsExactly` on the injected values (any order), comparing values by `Debug`.
fn assert_values(actual: &IndexMap<SkyKey, SkyValue>, expected: &[(SkyKey, SkyValue)]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "actual {:?} expected {:?}",
        actual.keys().collect::<Vec<_>>(),
        expected.iter().map(|(k, _)| k).collect::<Vec<_>>()
    );
    for (k, v) in expected {
        let a = actual
            .get(k)
            .unwrap_or_else(|| panic!("missing {k:?} in {:?}", actual.keys()));
        assert_eq!(
            format!("{:?}", a.downcast_ref::<FileStateValue>().unwrap()),
            format!("{:?}", v.downcast_ref::<FileStateValue>().unwrap()),
            "{k:?}"
        );
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:72 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_unknownFileChanged_returnsFileAndDirs
#[test]
fn get_diff_with_inferred_ancestors_unknown_file_changed_returns_file_and_dirs() {
    let f = Fixture::new();
    let diff = f.get_diff(&[f.file_state_value_key("foo/file")]).unwrap();
    assert_keys(
        &diff.values_to_invalidate,
        &[
            f.file_state_value_key(""),
            f.file_state_value_key("foo"),
            f.file_state_value_key("foo/file"),
            f.directory_listing_state_value_key(""),
            f.directory_listing_state_value_key("foo"),
        ],
    );
    assert!(diff.values_to_inject.is_empty());
    assert!(f.stated().is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:93 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_fileModified_returnsFileWithValues
#[test]
fn get_diff_with_inferred_ancestors_file_modified_returns_file_with_values() {
    let f = Fixture::new();
    f.file_with("file", "hello");
    let key = f.file_state_value_key("file");
    let value = f.file_state_value("file");
    // PORT: Java's Scratch is an InMemoryFileSystem; on disk the file contents proxy has
    // millisecond resolution (the timestamp granularity monitor covers this in builds).
    std::thread::sleep(std::time::Duration::from_millis(10));
    f.file_with("file", "there");
    f.add_done_nodes_and_then_mark_changed(vec![(key.clone(), value)]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, f.file_state_value("file"))]);
    assert!(diff.values_to_invalidate.is_empty());
    assert_eq!(f.stated(), strs(&["file"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:116 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_fileAdded_returnsFileAndDirListing
#[test]
fn get_diff_with_inferred_ancestors_file_added_returns_file_and_dir_listing() {
    let f = Fixture::new();
    f.file("file");
    let key = f.file_state_value_key("file");
    f.add_done_nodes_and_then_mark_changed(vec![
        (key.clone(), nonexistent()),
        (f.file_state_value_key(""), f.file_state_value("")),
    ]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, f.file_state_value("file"))]);
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("")],
    );
    assert_eq!(f.stated(), strs(&["file"]));
}

fn file_with_dirs_added(report_a: bool) {
    let f = Fixture::new();
    f.file("a/b/file");
    let file_key = f.file_state_value_key("a/b/file");
    f.add_done_nodes_and_then_mark_changed(vec![
        (f.file_state_value_key(""), f.file_state_value("")),
        (f.file_state_value_key("a"), nonexistent()),
        (f.file_state_value_key("a/b"), nonexistent()),
        (file_key.clone(), nonexistent()),
    ]);
    let mut modified = vec![file_key.clone()];
    if report_a {
        modified.push(f.file_state_value_key("a"));
    }
    let diff = f.get_diff(&modified).unwrap();
    assert_values(
        &diff.values_to_inject,
        &[
            (file_key, f.file_state_value("a/b/file")),
            (f.file_state_value_key("a"), directory()),
            (f.file_state_value_key("a/b"), directory()),
        ],
    );
    assert_keys(
        &diff.values_to_invalidate,
        &[
            f.directory_listing_state_value_key(""),
            f.directory_listing_state_value_key("a"),
            f.directory_listing_state_value_key("a/b"),
        ],
    );
    assert_eq!(f.stated(), strs(&["a/b/file"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:137 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_fileWithDirsAdded_returnsFileAndInjectsDirs
#[test]
fn get_diff_with_inferred_ancestors_file_with_dirs_added_returns_file_and_injects_dirs() {
    file_with_dirs_added(false);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:173 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_addedFileWithReportedDirs_returnsFileAndInjectsDirs
#[test]
fn get_diff_with_inferred_ancestors_added_file_with_reported_dirs_returns_file_and_injects_dirs() {
    file_with_dirs_added(true);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:213 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_fileWithUnknownDirsAdded_returnsFileAndDirs
#[test]
fn get_diff_with_inferred_ancestors_file_with_unknown_dirs_added_returns_file_and_dirs() {
    let f = Fixture::new();
    f.file("a/b/c/d");
    f.add_done_nodes_and_then_mark_changed(vec![
        (f.file_state_value_key(""), f.file_state_value("")),
        (f.file_state_value_key("a/b/c/d"), nonexistent()),
    ]);
    let diff = f.get_diff(&[f.file_state_value_key("a/b/c/d")]).unwrap();
    assert_keys(
        &diff.values_to_invalidate,
        &[
            f.file_state_value_key("a"),
            f.file_state_value_key("a/b"),
            f.file_state_value_key("a/b/c"),
            f.directory_listing_state_value_key(""),
            f.directory_listing_state_value_key("a"),
            f.directory_listing_state_value_key("a/b"),
            f.directory_listing_state_value_key("a/b/c"),
        ],
    );
    assert_values(
        &diff.values_to_inject,
        &[(
            f.file_state_value_key("a/b/c/d"),
            f.file_state_value("a/b/c/d"),
        )],
    );
    assert_eq!(f.stated(), strs(&["a/b/c/d"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:244 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_addEmptyDir_returnsDirAndParentListing
#[test]
fn get_diff_with_inferred_ancestors_add_empty_dir_returns_dir_and_parent_listing() {
    let f = Fixture::new();
    f.dir("dir");
    let key = f.file_state_value_key("dir");
    f.add_done_nodes_and_then_mark_changed(vec![
        (key.clone(), nonexistent()),
        (f.file_state_value_key(""), f.file_state_value("")),
    ]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, f.file_state_value("dir"))]);
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("")],
    );
    assert_eq!(f.stated(), strs(&["dir"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:266 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteFile_returnsFileParentListing
#[test]
fn get_diff_with_inferred_ancestors_delete_file_returns_file_parent_listing() {
    let f = Fixture::new();
    let file = f.file("dir/file1");
    f.file("dir/file2");
    let key = f.file_state_value_key("dir/file1");
    let old_value = f.file_state_value("dir/file1");
    std::fs::remove_file(file).unwrap();
    f.add_done_nodes_and_then_mark_changed(vec![
        (key.clone(), old_value),
        (f.file_state_value_key("dir"), f.file_state_value("dir")),
    ]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, nonexistent())]);
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("dir")],
    );
    assert_eq!(f.stated(), strs(&["dir/file1", "dir"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:289 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteFileFromDirWithListing_skipsDirStat
#[test]
fn get_diff_with_inferred_ancestors_delete_file_from_dir_with_listing_skips_dir_stat() {
    let f = Fixture::new();
    let file1 = f.file("dir/file1");
    let key = f.file_state_value_key("dir/file1");
    let old_value = f.file_state_value("dir/file1");
    std::fs::remove_file(file1).unwrap();
    f.add_done_nodes_and_then_mark_changed(vec![
        (key.clone(), old_value),
        (f.file_state_value_key("dir"), f.file_state_value("dir")),
    ]);
    f.add_done_nodes(vec![(
        f.directory_listing_state_value_key("dir"),
        listing(vec![dirent_file("file1"), dirent_file("file2")]),
    )]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, nonexistent())]);
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("dir")],
    );
    assert_eq!(f.stated(), strs(&["dir/file1"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:314 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteLastFileFromDir_ignoresInvalidatedListing
#[test]
fn get_diff_with_inferred_ancestors_delete_last_file_from_dir_ignores_invalidated_listing() {
    let f = Fixture::new();
    let file1 = f.file("dir/file1");
    let key = f.file_state_value_key("dir/file1");
    let old_value = f.file_state_value("dir/file1");
    std::fs::remove_file(file1).unwrap();
    f.add_done_nodes_and_then_mark_changed(vec![
        (key.clone(), old_value),
        (f.file_state_value_key("dir"), f.file_state_value("dir")),
        (
            f.directory_listing_state_value_key("dir"),
            listing(vec![dirent_file("file1"), dirent_file("file2")]),
        ),
    ]);
    let diff = f.get_diff(std::slice::from_ref(&key)).unwrap();
    assert_values(&diff.values_to_inject, &[(key, nonexistent())]);
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("dir")],
    );
    assert_eq!(f.stated(), strs(&["dir/file1", "dir"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:340 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_modifyAllUnknownEntriesInDirWithListing_skipsDir
#[test]
fn get_diff_with_inferred_ancestors_modify_all_unknown_entries_in_dir_with_listing_skips_dir() {
    let f = Fixture::new();
    let file = f.file("dir/file");
    std::os::unix::fs::symlink("file", file.parent().unwrap().join("symlink")).unwrap();
    let file_key = f.file_state_value_key("dir/file");
    let symlink_key = f.file_state_value_key("dir/symlink");
    f.add_done_nodes_and_then_mark_changed(vec![(
        f.file_state_value_key("dir"),
        f.file_state_value("dir"),
    )]);
    f.add_done_nodes(vec![(
        f.directory_listing_state_value_key("dir"),
        listing(vec![dirent_file("file"), dirent_symlink("symlink")]),
    )]);
    let diff = f
        .get_diff(&[file_key.clone(), symlink_key.clone()])
        .unwrap();
    assert_values(
        &diff.values_to_inject,
        &[
            (file_key, f.file_state_value("dir/file")),
            (symlink_key, f.file_state_value("dir/symlink")),
        ],
    );
    assert!(diff.values_to_invalidate.is_empty());
    assert_eq!(f.stated(), strs(&["dir/file", "dir/symlink"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:368 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_replaceUnknownEntriesInDirWithListing_skipsSiblingStat
#[test]
fn get_diff_with_inferred_ancestors_replace_unknown_entries_in_dir_with_listing_skips_sibling_stat()
{
    let f = Fixture::new();
    f.dir("dir/file1");
    f.dir("dir/file2");
    let file1_key = f.file_state_value_key("dir/file1");
    let file2_key = f.file_state_value_key("dir/file2");
    let dir_key = f.directory_listing_state_value_key("dir");
    f.add_done_nodes_and_then_mark_changed(vec![(
        f.file_state_value_key("dir"),
        f.file_state_value("dir"),
    )]);
    f.add_done_nodes(vec![(
        dir_key.clone(),
        listing(vec![dirent_file("file1"), dirent_file("file2")]),
    )]);
    let diff = f.get_diff(&[file1_key.clone(), file2_key.clone()]).unwrap();
    assert!(diff.values_to_invalidate.contains(&dir_key));
    let mut all: Vec<SkyKey> = diff.values_to_invalidate.iter().cloned().collect();
    all.extend(diff.values_to_inject.keys().cloned());
    let all: IndexSet<SkyKey> = all.into_iter().collect();
    assert_keys(&all, &[file1_key.clone(), file2_key.clone(), dir_key]);
    let stated = f.stated();
    assert!(!stated.is_empty());
    assert!(stated.iter().all(|p| p == "dir/file1" || p == "dir/file2"));
    // In case of single-threaded execution, we know that once we check dir/file1 or dir/file2,
    // we will be able to skip stat on the other one.
    assert_eq!(diff.values_to_inject.len(), 1);
    for (k, v) in &diff.values_to_inject {
        let rel = if *k == file1_key {
            "dir/file1"
        } else {
            "dir/file2"
        };
        assert_values(
            &IndexMap::from([(k.clone(), v.clone())]),
            &[(k.clone(), f.file_state_value(rel))],
        );
    }
    assert_eq!(diff.values_to_invalidate.len(), 2);
    assert_eq!(stated.len(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:407 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteAllFilesFromDir_returnsFilesAndDirListing
#[test]
fn get_diff_with_inferred_ancestors_delete_all_files_from_dir_returns_files_and_dir_listing() {
    let f = Fixture::new();
    let file1 = f.file("dir/file1");
    let file2 = f.file("dir/file2");
    let file3 = f.file("dir/file3");
    let key1 = f.file_state_value_key("dir/file1");
    let old_value1 = f.file_state_value("dir/file1");
    let key2 = f.file_state_value_key("dir/file2");
    let old_value2 = f.file_state_value("dir/file2");
    let key3 = f.file_state_value_key("dir/file3");
    let old_value3 = f.file_state_value("dir/file3");
    for p in [file1, file2, file3] {
        std::fs::remove_file(p).unwrap();
    }
    f.add_done_nodes_and_then_mark_changed(vec![
        (key1.clone(), old_value1),
        (key2.clone(), old_value2),
        (key3.clone(), old_value3),
        (f.file_state_value_key("dir"), f.file_state_value("dir")),
    ]);
    let diff = f
        .get_diff(&[key1.clone(), key2.clone(), key3.clone()])
        .unwrap();
    assert_values(
        &diff.values_to_inject,
        &[
            (key1, nonexistent()),
            (key2, nonexistent()),
            (key3, nonexistent()),
        ],
    );
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("dir")],
    );
    assert_eq!(
        f.stated(),
        strs(&["dir", "dir/file1", "dir/file2", "dir/file3"])
    );
}

fn delete_file_with_dirs(report_ab: bool) {
    let f = Fixture::new();
    f.file("a/b/c/file");
    let ab_key = f.file_state_value_key("a/b");
    let ab_value = f.file_state_value("a/b");
    let abc_key = f.file_state_value_key("a/b/c");
    let abc_value = f.file_state_value("a/b/c");
    let abc_file_key = f.file_state_value_key("a/b/c/file");
    let abc_file_value = f.file_state_value("a/b/c/file");
    std::fs::remove_dir_all(f.dir("a/b")).unwrap();
    f.add_done_nodes_and_then_mark_changed(vec![
        (f.file_state_value_key("a"), f.file_state_value("a")),
        (ab_key.clone(), ab_value),
        (abc_key.clone(), abc_value),
        (abc_file_key.clone(), abc_file_value),
    ]);
    let mut modified = vec![abc_file_key.clone()];
    if report_ab {
        modified.push(ab_key.clone());
    }
    let diff = f.get_diff(&modified).unwrap();
    assert_values(
        &diff.values_to_inject,
        &[
            (ab_key, nonexistent()),
            (abc_key, nonexistent()),
            (abc_file_key, nonexistent()),
        ],
    );
    assert_keys(
        &diff.values_to_invalidate,
        &[
            f.directory_listing_state_value_key("a"),
            f.directory_listing_state_value_key("a/b"),
            f.directory_listing_state_value_key("a/b/c"),
        ],
    );
    assert_eq!(f.stated(), strs(&["a", "a/b", "a/b/c", "a/b/c/file"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:445 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteFileWithDirs_returnsFileAndDirs
#[test]
fn get_diff_with_inferred_ancestors_delete_file_with_dirs_returns_file_and_dirs() {
    delete_file_with_dirs(false);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:483 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteFileWithReportedDirs_returnsFileAndDirListings
#[test]
fn get_diff_with_inferred_ancestors_delete_file_with_reported_dirs_returns_file_and_dir_listings() {
    delete_file_with_dirs(true);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:521 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteFile_infersDirFromModifiedSibling
#[test]
fn get_diff_with_inferred_ancestors_delete_file_infers_dir_from_modified_sibling() {
    let f = Fixture::new();
    let file1 = f.file("dir/file1");
    f.file_with("dir/file2", "1");
    let file1_key = f.file_state_value_key("dir/file1");
    let file1_value = f.file_state_value("dir/file1");
    let file2_key = f.file_state_value_key("dir/file2");
    let file2_value = f.file_state_value("dir/file2");
    std::fs::remove_file(file1).unwrap();
    f.file_with("dir/file2", "12");
    f.add_done_nodes_and_then_mark_changed(vec![
        (f.file_state_value_key("dir"), f.file_state_value("dir")),
        (file1_key.clone(), file1_value),
        (file2_key.clone(), file2_value),
    ]);
    let diff = f
        .get_diff(&[
            file1_key.clone(),
            file2_key.clone(),
            f.file_state_value_key("dir"),
        ])
        .unwrap();
    assert_values(
        &diff.values_to_inject,
        &[
            (file1_key, nonexistent()),
            (file2_key, f.file_state_value("dir/file2")),
        ],
    );
    assert_keys(
        &diff.values_to_invalidate,
        &[f.directory_listing_state_value_key("dir")],
    );
    assert_eq!(f.stated(), strs(&["dir/file1", "dir/file2"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:554 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_deleteDirReportDirOnly_returnsDir
#[test]
fn get_diff_with_inferred_ancestors_delete_dir_report_dir_only_returns_dir() {
    let f = Fixture::new();
    let file1 = f.file("dir/file1");
    f.file("dir/file2");
    let file1_key = f.file_state_value_key("dir/file1");
    let file1_value = f.file_state_value("dir/file1");
    let file2_key = f.file_state_value_key("dir/file2");
    let file2_value = f.file_state_value("dir/file2");
    let subdir_file_key = f.file_state_value_key("dir/subdir/file");
    let subdir_file_value = f.file_state_value("dir/subdir/file");
    let dir_key = f.file_state_value_key("dir");
    let dir_value = f.file_state_value("dir");
    let subdir_key = f.file_state_value_key("dir/subdir");
    let subdir_value = f.file_state_value("dir/subdir");
    let subdir_listing_key = f.directory_listing_state_value_key("dir/subdir");
    let subdir_listing_value = listing(vec![dirent_file("file")]);
    std::fs::remove_dir_all(file1.parent().unwrap()).unwrap();
    f.add_done_nodes_and_then_mark_changed(vec![
        (file1_key.clone(), file1_value),
        (file2_key.clone(), file2_value),
        (subdir_file_key.clone(), subdir_file_value),
        (dir_key.clone(), dir_value),
        (subdir_key.clone(), subdir_value),
        (subdir_listing_key.clone(), subdir_listing_value),
        (f.file_state_value_key(""), f.file_state_value("")),
    ]);
    let diff = f.get_diff(std::slice::from_ref(&dir_key)).unwrap();
    assert_values(&diff.values_to_inject, &[(dir_key, nonexistent())]);
    assert_keys(
        &diff.values_to_invalidate,
        &[
            f.directory_listing_state_value_key(""),
            file1_key,
            file2_key,
            subdir_key,
            subdir_listing_key,
            subdir_file_key,
        ],
    );
    assert_eq!(f.stated(), strs(&["dir", ""]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:606 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_phantomChangeForNonexistentEntry_returnsEmptyDiff
#[test]
fn get_diff_with_inferred_ancestors_phantom_change_for_nonexistent_entry_returns_empty_diff() {
    let f = Fixture::new();
    f.add_done_nodes_and_then_mark_changed(vec![(f.file_state_value_key("file"), nonexistent())]);
    let diff = f.get_diff(&[f.file_state_value_key("file")]).unwrap();
    assert!(diff.values_to_invalidate.is_empty());
    assert!(diff.values_to_inject.is_empty());
    assert_eq!(f.stated(), strs(&["file"]));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:622 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_statFails_fails
#[test]
fn get_diff_with_inferred_ancestors_stat_fails_fails() {
    let f = Fixture::new();
    f.set_throw_on_stat(ThrowOnStat::Io("oh no".to_string()));
    f.add_done_nodes_and_then_mark_changed(vec![(f.file_state_value_key("file"), nonexistent())]);
    let e = match f.get_diff(&[f.file_state_value_key("file")]) {
        Err(e) => e,
        Ok(_) => panic!("expected AbruptExitException"),
    };
    let failure_detail = e.failure_detail();
    match failure_detail.category {
        Some(blaze_protos::failure_details::failure_detail::Category::DiffAwareness(d)) => {
            assert_eq!(
                d.code,
                blaze_protos::failure_details::diff_awareness::Code::DiffStatFailed as i32
            )
        }
        other => panic!("{other:?}"),
    }
    let file = f.root.as_path().unwrap().get_relative_str("file");
    assert!(
        failure_detail.message.contains(&format!(
            "Failed to check dirtiness of {}",
            file.get_path_string()
        )),
        "{}",
        failure_detail.message
    );
    assert!(failure_detail.message.contains("oh no"));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/FileSystemValueCheckerInferringAncestorsTest.java:647 FileSystemValueCheckerInferringAncestorsTest.getDiffWithInferredAncestors_statCrashes_fails
#[test]
fn get_diff_with_inferred_ancestors_stat_crashes_fails() {
    let f = Fixture::new();
    f.set_throw_on_stat(ThrowOnStat::Runtime("oh no".to_string()));
    f.add_done_nodes_and_then_mark_changed(vec![(f.file_state_value_key("file"), nonexistent())]);
    // Java: IllegalStateException wrapping the RuntimeException; Blaze: the panic propagates.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = f.get_diff(&[f.file_state_value_key("file")]);
    }));
    assert!(result.is_err());
}
