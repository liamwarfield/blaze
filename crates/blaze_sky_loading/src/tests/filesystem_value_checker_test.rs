//! Port of the key part of `FilesystemValueCheckerTest` and of `DirtinessCheckerUtilsTest`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/FilesystemValueCheckerTest.java,
//! DirtinessCheckerUtilsTest.java. The action-output tests (`getDirtyActionValues_*`,
//! `testDirtyActions*`, the remote ones) are in blaze_sky_exec (`output_checker`).
//!
//! PORT: Java's tests run on an `InMemoryFileSystem` under `Root.absoluteRoot(fs)`; here they run
//! on the real file system under the scratch workspace (a package root), whose files the
//! `ExternalFilesHelper` classifies as internal.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_events::StoredEventHandler;
use blaze_skyframe::{Evaluator, SkyKey};
use blaze_vfs::{PathFragment, Root, RootedPath};

use crate::file::FileKey;
use crate::file_state::{ExternalFileAction, ExternalFilesHelper, FileStateKey, FileType};
use crate::filesystem_value_checker::{
    ExternalDirtinessChecker, FilesystemValueChecker, ImmutableBatchDirtyResult,
    MissingDiffDirtinessChecker, SkyValueDirtinessChecker,
    create_basic_filesystem_dirtiness_checker,
};
use crate::test_util::{FakeRepos, TestSetup, evaluate};

// Java: FilesystemValueCheckerTest.FSVC_THREADS_FOR_TEST
const FSVC_THREADS_FOR_TEST: usize = 200;

struct Fixture {
    setup: TestSetup,
    pkg_root: Root,
    evaluator: Evaluator,
}

impl Fixture {
    fn new() -> Fixture {
        let setup = TestSetup::new(FakeRepos::default());
        let pkg_root = Root::from_path(setup.workspace().clone());
        let evaluator = setup.evaluator();
        Fixture {
            setup,
            pkg_root,
            evaluator,
        }
    }

    fn rooted_path(&self, rel: &str) -> RootedPath {
        RootedPath::to_rooted_path(self.pkg_root.clone(), &PathFragment::create(rel))
    }

    fn file(&self, rel: &str, contents: &str) {
        self.setup.scratch.file(
            self.pkg_root
                .get_relative(&PathFragment::create(rel))
                .get_path_string(),
            contents,
        );
    }

    fn evaluate(&self, keys: Vec<SkyKey>) -> blaze_skyframe::EvaluationResult {
        evaluate(
            &self.evaluator,
            keys,
            false,
            Arc::new(StoredEventHandler::new()),
        )
    }

    // Java: FilesystemValueCheckerTest.getDirtyFilesystemKeys
    fn get_dirty_filesystem_keys(
        &self,
        checker: &FilesystemValueChecker,
    ) -> ImmutableBatchDirtyResult {
        checker.get_dirty_keys(
            &self.evaluator.get_values(),
            &create_basic_filesystem_dirtiness_checker(),
        )
    }
}

fn checker() -> FilesystemValueChecker {
    FilesystemValueChecker::new(/* tsgm= */ None, FSVC_THREADS_FOR_TEST)
}

// Java: FilesystemValueCheckerTest.assertEmptyDiff
fn assert_empty_diff(diff: &ImmutableBatchDirtyResult) {
    assert_diff_with_new_values(diff, &[]);
}

// Java: FilesystemValueCheckerTest.assertDiffWithNewValues
fn assert_diff_with_new_values(diff: &ImmutableBatchDirtyResult, keys_with_new_values: &[SkyKey]) {
    assert!(
        diff.changed_keys_without_new_values().is_empty(),
        "{:?}",
        diff.changed_keys_without_new_values()
    );
    let actual: HashSet<SkyKey> = diff
        .changed_keys_with_new_values()
        .keys()
        .cloned()
        .collect();
    let expected: HashSet<SkyKey> = keys_with_new_values.iter().cloned().collect();
    assert_eq!(actual, expected);
}

/// A later write to a file must change its contents proxy: wait past the millisecond (and the
/// kernel's coarse timestamp tick) of the last write.
fn wait_for_timestamp_granularity() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

// Java: FilesystemValueCheckerTest.testEmpty
#[test]
fn test_empty() {
    let f = Fixture::new();
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker()));
}

// Java: FilesystemValueCheckerTest.testSimple
#[test]
fn test_simple() {
    let f = Fixture::new();
    let checker = checker();
    f.file("foo", "");
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));

    let sky_key = SkyKey::new(FileStateKey::key(f.rooted_path("foo")));
    let result = f.evaluate(vec![sky_key.clone()]);
    assert!(!result.has_error());

    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));

    wait_for_timestamp_granularity();
    f.file("foo", "hello");
    assert_diff_with_new_values(&f.get_dirty_filesystem_keys(&checker), &[sky_key.clone()]);

    // The dirty bits are not reset until the FileValues are actually revalidated.
    assert_diff_with_new_values(&f.get_dirty_filesystem_keys(&checker), &[sky_key.clone()]);

    f.evaluator.invalidate([sky_key.clone()]);
    let result = f.evaluate(vec![sky_key]);
    assert!(!result.has_error());
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));
}

/// Tests that an already-invalidated value can still be marked changed: symlink points at sym1.
/// Invalidate symlink by changing sym1 from pointing at path to point to sym2. This only dirties
/// (rather than changes) symlink because sym2 still points at path, so all symlink stats remain
/// the same. Then do a null build, change sym1 back to point at path, and change symlink to not
/// be a symlink anymore. The fact that it is not a symlink should be detected.
// Java: FilesystemValueCheckerTest.testDirtySymlink
#[test]
fn test_dirty_symlink() {
    let f = Fixture::new();
    let checker = checker();
    let workspace = f.setup.workspace().clone();
    f.file("foo", "foo contents");
    let path = workspace.get_relative(&PathFragment::create("foo"));
    let sym1 = workspace.get_relative(&PathFragment::create("sym1"));
    let sym2 = workspace.get_relative(&PathFragment::create("sym2"));
    let symlink = workspace.get_relative(&PathFragment::create("bar"));
    f.setup
        .scratch
        .symlink(symlink.get_path_string(), sym1.get_path_string());
    f.setup
        .scratch
        .symlink(sym1.get_path_string(), path.get_path_string());
    f.setup
        .scratch
        .symlink(sym2.get_path_string(), path.get_path_string());
    let foo_key = FileKey::key(f.rooted_path("foo"));
    let symlink_rooted_path = f.rooted_path("bar");
    let symlink_key = FileKey::key(symlink_rooted_path.clone());
    let symlink_file_state_key = SkyKey::new(FileStateKey::key(symlink_rooted_path));
    let sym1_file_state_key = SkyKey::new(FileStateKey::key(f.rooted_path("sym1")));
    let all_keys = vec![
        SkyKey::new(symlink_key.clone()),
        SkyKey::new(foo_key.clone()),
    ];

    // First build -- prime the graph.
    let result = f.evaluate(all_keys.clone());
    assert!(!result.has_error());
    let symlink_value = result.get(&symlink_key).unwrap();
    let foo_value = result.get(&foo_key).unwrap();
    assert!(symlink_value.is_symlink(), "{:?}", symlink_value);
    // Digest is not always available, so use size as a proxy for contents.
    assert_eq!(symlink_value.get_size(), foo_value.get_size());
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));

    // Before second build, move sym1 to point to sym2.
    assert!(sym1.delete().unwrap());
    f.setup
        .scratch
        .symlink(sym1.get_path_string(), sym2.get_path_string());
    assert_diff_with_new_values(
        &f.get_dirty_filesystem_keys(&checker),
        &[sym1_file_state_key.clone()],
    );

    f.evaluator.invalidate([sym1_file_state_key.clone()]);
    let result = f.evaluate(vec![]);
    assert!(!result.has_error());
    assert_diff_with_new_values(
        &f.get_dirty_filesystem_keys(&checker),
        &[sym1_file_state_key.clone()],
    );

    // Before third build, move sym1 back to original (so change pruning will prevent signaling
    // of its parents, but change symlink for real.
    assert!(sym1.delete().unwrap());
    f.setup
        .scratch
        .symlink(sym1.get_path_string(), path.get_path_string());
    assert!(symlink.delete().unwrap());
    f.file("bar", "new symlink contents");
    assert_diff_with_new_values(
        &f.get_dirty_filesystem_keys(&checker),
        &[symlink_file_state_key.clone()],
    );
    f.evaluator.invalidate([symlink_file_state_key]);
    let result = f.evaluate(all_keys);
    assert!(!result.has_error());
    let symlink_value = result.get(&symlink_key).unwrap();
    assert!(!symlink_value.is_symlink(), "{:?}", symlink_value);
    assert_eq!(*result.get(&foo_key).unwrap(), *foo_value);
    assert_ne!(symlink_value.get_size(), foo_value.get_size());
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));
}

// Java: FilesystemValueCheckerTest.testExplicitFiles
#[test]
fn test_explicit_files() {
    let f = Fixture::new();
    let checker = checker();
    f.file("foo1", "");
    f.file("foo2", "");
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));

    let key1 = SkyKey::new(FileStateKey::key(f.rooted_path("foo1")));
    let key2 = SkyKey::new(FileStateKey::key(f.rooted_path("foo2")));
    let sky_keys = vec![key1.clone(), key2.clone()];
    let result = f.evaluate(sky_keys.clone());
    assert!(!result.has_error());

    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));

    // Wait for the timestamp granularity to elapse, so updating the files will observably
    // advance their ctime.
    wait_for_timestamp_granularity();
    // Update path1's contents. This will update the file's ctime with current time indicated by
    // the clock.
    f.file("foo1", "hello1");
    // Update path2's mtime but not its contents. We expect that an mtime change suffices to
    // update the ctime.
    f.pkg_root
        .get_relative(&PathFragment::create("foo2"))
        .set_last_modified_time(42)
        .unwrap();
    // Assert that both files changed. The change detection relies, among other things, on ctime
    // change.
    assert_diff_with_new_values(
        &f.get_dirty_filesystem_keys(&checker),
        &[key1.clone(), key2.clone()],
    );

    f.evaluator.invalidate(sky_keys.clone());
    let result = f.evaluate(sky_keys);
    assert!(!result.has_error());
    assert_empty_diff(&f.get_dirty_filesystem_keys(&checker));
}

// Java: FilesystemValueCheckerTest.testFileWithIOExceptionNotConsideredDirty
#[test]
#[ignore = "needs a file system whose readlink throws (Java's InMemoryFileSystem \
            readlinkThrowsIoException); the real file system cannot be made to fail so"]
fn test_file_with_io_exception_not_considered_dirty() {}

// Java: FilesystemValueCheckerTest.testFilesInCycleNotConsideredDirty
#[test]
fn test_files_in_cycle_not_considered_dirty() {
    let f = Fixture::new();
    let workspace = f.setup.workspace().clone();
    let p = |n: &str| workspace.get_relative(&PathFragment::create(n));
    f.setup
        .scratch
        .symlink(p("foo1").get_path_string(), p("foo2").get_path_string());
    f.setup
        .scratch
        .symlink(p("foo2").get_path_string(), p("foo3").get_path_string());
    f.setup
        .scratch
        .symlink(p("foo3").get_path_string(), p("foo1").get_path_string());
    let file_key1 = SkyKey::new(FileKey::key(f.rooted_path("foo1")));

    let result = f.evaluate(vec![file_key1]);
    assert!(result.has_error());

    let diff = f.get_dirty_filesystem_keys(&checker());
    assert!(diff.changed_keys_without_new_values().is_empty());
    assert!(diff.changed_keys_with_new_values().is_empty());
}

// Java: DirtinessCheckerUtilsTest.missingDiffChecker_matchesInsideRoot
#[test]
fn missing_diff_checker_matches_inside_root() {
    let f = Fixture::new();
    let checker = MissingDiffDirtinessChecker::new([f.pkg_root.clone()].into_iter().collect());
    assert!(checker.applies(&SkyKey::new(FileStateKey::key(f.rooted_path("foo")))));
}

// Java: DirtinessCheckerUtilsTest.missingDiffChecker_doesntMatchIfRootDoesntMatch
#[test]
fn missing_diff_checker_doesnt_match_if_root_doesnt_match() {
    let f = Fixture::new();
    let other_root = Root::from_path(f.setup.scratch.dir("other"));
    let checker = MissingDiffDirtinessChecker::new([other_root].into_iter().collect());
    assert!(!checker.applies(&SkyKey::new(FileStateKey::key(f.rooted_path("foo")))));
}

// Java: DirtinessCheckerUtilsTest.check_usesSyscallCache_andReturnsNewValue
#[test]
fn check_uses_syscall_cache_and_returns_new_value() {
    // PORT: no syscall cache: the part of the Java test that remains is that the checker
    // returns the new value of a changed file.
    let f = Fixture::new();
    f.file("foo", "a");
    let key = SkyKey::new(FileStateKey::key(f.rooted_path("foo")));
    let result = f.evaluate(vec![key.clone()]);
    assert!(!result.has_error());
    let old_value = f.evaluator.get_existing_value(&key);
    wait_for_timestamp_granularity();
    f.file("foo", "bb");
    let checker = MissingDiffDirtinessChecker::new([f.pkg_root.clone()].into_iter().collect());
    let dirty = checker.check(&key, old_value.as_ref(), None).unwrap();
    assert!(dirty.is_dirty());
    assert!(dirty.get_new_value().is_some());
}

// Java: DirtinessCheckerUtilsTest.skipsSyscallCacheForRepoFile_andDoesntReturnNewValue
#[test]
fn skips_syscall_cache_for_repo_file_and_doesnt_return_new_value() {
    let f = Fixture::new();
    let external = f
        .setup
        .config
        .directories
        .get_output_base()
        .get_relative(&PathFragment::create("external/repo"));
    f.setup.scratch.file(
        external
            .get_relative(&PathFragment::create("file"))
            .get_path_string(),
        "a",
    );
    let rooted_path = RootedPath::to_rooted_path(
        Root::from_path(f.setup.config.directories.get_output_base().clone()),
        &PathFragment::create("external/repo/file"),
    );
    let key = SkyKey::new(FileStateKey::key(rooted_path));
    let old_value: blaze_skyframe::SkyValue = Arc::new(
        crate::actions::FileStateValue::create(key.downcast::<FileStateKey>().unwrap().argument())
            .unwrap(),
    );
    wait_for_timestamp_granularity();
    f.setup.scratch.file(
        external
            .get_relative(&PathFragment::create("file"))
            .get_path_string(),
        "bb",
    );
    let helper = Arc::new(ExternalFilesHelper::create(
        f.setup.config.pkg_locator.clone(),
        ExternalFileAction::DependOnExternalPkgForExternalRepoPaths,
        f.setup.config.directories.clone(),
        None,
    ));
    let checker = ExternalDirtinessChecker::new(helper, vec![FileType::ExternalRepo]);
    assert!(checker.applies(&key));
    let dirty = checker.check(&key, Some(&old_value), None).unwrap();
    assert!(dirty.is_dirty());
    assert!(dirty.get_new_value().is_none());
    // The repository is recorded as modified externally.
    assert_eq!(checker.get_dirty_external_repos().len(), 1);
}

// Java: DirtinessCheckerUtilsTest.externalDiffChecker_doesntMatchType
#[test]
fn external_diff_checker_doesnt_match_type() {
    let f = Fixture::new();
    let helper = Arc::new(ExternalFilesHelper::create(
        f.setup.config.pkg_locator.clone(),
        ExternalFileAction::DependOnExternalPkgForExternalRepoPaths,
        f.setup.config.directories.clone(),
        None,
    ));
    let checker = ExternalDirtinessChecker::new(helper, vec![FileType::ExternalRepo]);
    // A file under the package root is INTERNAL.
    assert!(!checker.applies(&SkyKey::new(FileStateKey::key(f.rooted_path("foo")))));
}

// Java: DirtinessCheckerUtilsTest.missingDiffDirtinessCheckers_nullMaxTransitiveSourceVersionForNewValue
#[test]
#[ignore = "max transitive source versions are not ported (docs/skyframe-incremental.md \
            section 6)"]
fn missing_diff_dirtiness_checkers_null_max_transitive_source_version_for_new_value() {}

// Java: DirtinessCheckerUtilsTest.externalDirtinessCheckers_nullMaxTransitiveSourceVersionForNewValue
#[test]
#[ignore = "max transitive source versions are not ported (docs/skyframe-incremental.md \
            section 6)"]
fn external_dirtiness_checkers_null_max_transitive_source_version_for_new_value() {}

// Java: DirtinessCheckerUtilsTest.unionDirtinessChecker_nullMaxTransitiveSourceVersionForNewValue
#[test]
#[ignore = "max transitive source versions are not ported (docs/skyframe-incremental.md \
            section 6)"]
fn union_dirtiness_checker_null_max_transitive_source_version_for_new_value() {}

/// The FILE dependency of a rewrite that keeps the size: the contents proxy changes (ctime),
/// so the checker reports the key (root's I2 review item, TimestampGranularityMonitor).
#[test]
fn same_size_rewrite_is_dirty() {
    let f = Fixture::new();
    let checker = checker();
    f.file("foo", "aaaa");
    let key = SkyKey::new(FileStateKey::key(f.rooted_path("foo")));
    assert!(!f.evaluate(vec![key.clone()]).has_error());
    wait_for_timestamp_granularity();
    f.file("foo", "bbbb");
    assert_diff_with_new_values(&f.get_dirty_filesystem_keys(&checker), &[key]);
}
