//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java`:
//! `LocalDiffAwareness` (the inotify watch service) on a scratch directory.
//!
//! PORT: Java's test extends `BuildIntegrationTestCase` only for its test root; each test here
//! has its own scratch directory.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use blaze_optdefs::command::local_diff_awareness::LocalDiffAwarenessOptions;
use blaze_vfs::{ModifiedFileSet, PathFragment, Root};
use indexmap::IndexSet;

use crate::diff_awareness::{
    DiffAwareness, DiffAwarenessFactory, DiffError, LocalDiffAwarenessFactory, SequentialView,
    View, WatchServiceDiffAwareness, are_in_sequence, everything_modified_view,
};
use crate::ignored_subdirectories::IgnoredSubdirectories;
use crate::test_util::Scratch;

/// Try this many times to pick up file changes. Inotify needs some nanoseconds of patience.
const MAX_RETRY_COUNT: usize = 20;

fn options(watch_fs: bool) -> LocalDiffAwarenessOptions {
    let mut o = blaze_options::Options::<LocalDiffAwarenessOptions>::get_defaults();
    o.watch_fs = watch_fs;
    o
}

struct Fixture {
    scratch: Scratch,
    test_case_root: PathBuf,
    test_case_ignored_dir: PathBuf,
    local_diff: Box<dyn DiffAwareness>,
    old_view: Option<Arc<dyn View>>,
}

impl Fixture {
    // Java: LocalDiffAwarenessTest.initializeSettings
    fn new() -> Fixture {
        let scratch = Scratch::new();
        let root = scratch.dir("testCaseRoot");
        let ignored = scratch.dir("testCaseRoot/ignored-dir");
        let factory = LocalDiffAwarenessFactory::new(vec![]);
        let mut prefixes = IndexSet::new();
        prefixes.insert(ignored.as_fragment().to_relative());
        let local_diff = factory
            .maybe_create(
                &Root::from_path(root.clone()),
                &IgnoredSubdirectories::of(prefixes, vec![]),
                &options(true),
            )
            .expect("LocalDiffAwareness");
        Fixture {
            test_case_root: crate::test_util::platform(&root),
            test_case_ignored_dir: crate::test_util::platform(&ignored),
            scratch,
            local_diff,
            old_view: None,
        }
    }

    fn capture_first_view(&mut self, options: &LocalDiffAwarenessOptions) {
        self.old_view = Some(self.local_diff.get_current_view(options).unwrap());
    }

    fn p(&self, rel: &str) -> PathBuf {
        self.test_case_root.join(rel)
    }

    // Java: LocalDiffAwarenessTest.touch
    fn touch(&self, rel: &str) {
        std::fs::write(self.p(rel), "Sunshine, sunshine, ladybugs awake!").unwrap();
    }

    fn touch_abs(&self, path: &std::path::Path) {
        std::fs::write(path, "Sunshine, sunshine, ladybugs awake!").unwrap();
    }

    // Java: LocalDiffAwarenessTest.mkdir
    fn mkdir(&self, rel: &str) {
        std::fs::create_dir_all(self.p(rel)).unwrap();
    }

    // Java: LocalDiffAwarenessTest.rm (Path.deleteTree)
    fn rm(&self, rel: &str) {
        let p = self.p(rel);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
            std::fs::remove_dir_all(p).unwrap();
        } else {
            std::fs::remove_file(p).unwrap();
        }
    }

    // Java: LocalDiffAwarenessTest.symlink (FileSystemUtils.ensureSymbolicLink(from, to))
    fn symlink(&self, from: &str, to: &str) {
        std::os::unix::fs::symlink(self.p(to), self.p(from)).unwrap();
    }

    /// Takes views until the accumulated diffs contain every path of `expected` (Blaze-only
    /// helper of the rename tests: the exact sets of the intermediate steps do not matter there).
    fn wait_for(&mut self, expected: &[&str]) {
        let expected: BTreeSet<PathFragment> =
            expected.iter().map(|p| PathFragment::create(p)).collect();
        let mut seen: BTreeSet<PathFragment> = BTreeSet::new();
        for _ in 0..MAX_RETRY_COUNT {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let new_view = self.local_diff.get_current_view(&options(true)).unwrap();
            let modified_file_set = self
                .local_diff
                .get_diff(self.old_view.as_ref(), &new_view)
                .unwrap_or_else(|e| panic!("{e:?}"));
            self.old_view = Some(new_view);
            assert!(!modified_file_set.treat_everything_as_modified());
            seen.extend(modified_file_set.modified_source_files().iter().cloned());
            if expected.is_subset(&seen) {
                return;
            }
        }
        panic!("never saw {expected:?}; saw {seen:?}");
    }

    /// Takes one view (draining what the watcher has) after a short wait.
    fn drain(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let new_view = self.local_diff.get_current_view(&options(true)).unwrap();
        let modified_file_set = self
            .local_diff
            .get_diff(self.old_view.as_ref(), &new_view)
            .unwrap_or_else(|e| panic!("{e:?}"));
        self.old_view = Some(new_view);
        assert!(!modified_file_set.treat_everything_as_modified());
    }

    // Java: LocalDiffAwarenessTest.ModifiedFileSetChecker.check
    fn check(&mut self, modified: &[&str]) {
        let modified: BTreeSet<PathFragment> =
            modified.iter().map(|p| PathFragment::create(p)).collect();
        // Unfortunately, inotify needs a few milliseconds (more than a few in the worst case)
        // after a change to pick up a list of changed files. Trying a few times to make sure.
        for _ in 0..MAX_RETRY_COUNT {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let new_view = self.local_diff.get_current_view(&options(true)).unwrap();
            let modified_file_set = self
                .local_diff
                .get_diff(self.old_view.as_ref(), &new_view)
                .unwrap_or_else(|e| panic!("{e:?}"));
            self.old_view = Some(new_view);
            assert!(!modified_file_set.treat_everything_as_modified());
            if modified_file_set.modified_source_files().is_empty() {
                continue;
            }
            let actual: BTreeSet<PathFragment> = modified_file_set
                .modified_source_files()
                .iter()
                .cloned()
                .collect();
            assert_eq!(actual, modified);
            return;
        }
        // If we never received any changes, make sure this is what we actually expect.
        assert!(
            modified.is_empty(),
            "no changes seen, expected {modified:?}"
        );
    }

    // Java: LocalDiffAwarenessTest.ModifiedFileSetChecker.checkEverythingModified
    fn check_everything_modified(&mut self, options: &LocalDiffAwarenessOptions) {
        let new_view = self.local_diff.get_current_view(options).unwrap();
        let modified_file_set = self
            .local_diff
            .get_diff(self.old_view.as_ref(), &new_view)
            .unwrap_or_else(|e| panic!("{e:?}"));
        self.old_view = Some(new_view);
        assert!(modified_file_set.treat_everything_as_modified());
    }
}

impl Drop for Fixture {
    // Java: LocalDiffAwarenessTest.closeLocalDiff
    fn drop(&mut self) {
        self.local_diff.close();
        let _ = &self.scratch;
    }
}

fn sequential(view: &Arc<dyn View>) -> &SequentialView {
    view.as_any().downcast_ref::<SequentialView>().unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:104 LocalDiffAwarenessTest.areInSequenceWithEverythingModifiedShouldAlwaysReturnFalse
#[test]
fn are_in_sequence_with_everything_modified_should_always_return_false() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    let old = f.old_view.clone().unwrap();
    let everything_mod = everything_modified_view();
    assert!(!are_in_sequence(
        sequential(&old),
        sequential(&everything_mod)
    ));
    assert!(!are_in_sequence(
        sequential(&everything_mod),
        sequential(&old)
    ));
    assert!(!are_in_sequence(
        sequential(&everything_mod),
        sequential(&everything_mod)
    ));
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:115 LocalDiffAwarenessTest.testFileAdded
#[test]
fn test_file_added() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.touch("foo.txt");
    f.check(&["foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:122 LocalDiffAwarenessTest.testSymlink
#[test]
fn test_symlink() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.touch("a");
    f.symlink("b", "a");
    f.check(&["a", "b"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:130 LocalDiffAwarenessTest.testSymlinkBroken
#[test]
fn test_symlink_broken() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.symlink("b", "a");
    f.check(&["b"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:137 LocalDiffAwarenessTest.testFileModified
#[test]
fn test_file_modified() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.touch("foo.txt");
    f.check(&["foo.txt"]);
    f.check(&[]);
    f.touch("foo.txt");
    f.check(&["foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:147 LocalDiffAwarenessTest.testIgnoredFileModified
#[test]
fn test_ignored_file_modified() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    let ignored = f.test_case_ignored_dir.join("foo");
    f.touch_abs(&ignored);
    f.check(&[]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:154 LocalDiffAwarenessTest.testFileRemoved
#[test]
fn test_file_removed() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.touch("foo.txt");
    f.check(&["foo.txt"]);
    f.rm("foo.txt");
    f.check(&["foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:163 LocalDiffAwarenessTest.testFileAddAndRemove
#[test]
fn test_file_add_and_remove() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.touch("foo.txt");
    f.touch("bar.txt");
    f.check(&["foo.txt", "bar.txt"]);
    f.rm("foo.txt");
    f.touch("foo.txt");
    f.check(&["foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:174 LocalDiffAwarenessTest.testAddDirectory
#[test]
fn test_add_directory() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.mkdir("equestria");
    f.touch("equestria/foo.txt");
    f.check(&["equestria", "equestria/foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:182 LocalDiffAwarenessTest.testRemoveDirectory
#[test]
fn test_remove_directory() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.mkdir("equestria");
    f.touch("equestria/foo.txt");
    f.check(&["equestria", "equestria/foo.txt"]);
    f.rm("equestria");
    f.check(&["equestria", "equestria/foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:192 LocalDiffAwarenessTest.testMoveDirectory
#[test]
fn test_move_directory() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.mkdir("equestria");
    f.touch("equestria/foo.txt");
    f.check(&["equestria", "equestria/foo.txt"]);
    std::fs::rename(f.p("equestria"), f.p("equestria2")).unwrap();
    // The contents of a moved directory are *not* reported as modified.
    f.check(&["equestria", "equestria2", "equestria2/foo.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:207 LocalDiffAwarenessTest.testLotsOfChanges
#[test]
fn test_lots_of_changes() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.mkdir("pre");
    f.touch("pre/pre.txt");
    f.check(&["pre", "pre/pre.txt"]);
    f.mkdir("a");
    f.touch("a/a1.txt");
    f.touch("a/a2.txt");
    f.rm("a");
    f.mkdir("a");
    f.touch("a/a1.txt");
    f.touch("a/a2.txt");
    f.mkdir("a/b");
    f.touch("a/b/b1.txt");
    f.check(&["a", "a/a1.txt", "a/a2.txt", "a/b", "a/b/b1.txt"]);
    f.rm("a/b/b1.txt");
    f.touch("a/b/b2.txt");
    f.rm("a/b");
    f.mkdir("a/b");
    f.rm("a/b");
    f.mkdir("a/b");
    f.touch("a/b/b3.txt");
    f.check(&["a/b", "a/b/b1.txt", "a/b/b2.txt", "a/b/b3.txt"]);
    f.rm("a/b/b3.txt");
    f.check(&["a/b/b3.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:255 LocalDiffAwarenessTest.testEnableWatchFs
#[test]
fn test_enable_watch_fs() {
    let mut f = Fixture::new();
    let watch_fs_disabled = options(false);
    f.capture_first_view(&watch_fs_disabled);
    f.check_everything_modified(&watch_fs_disabled);
    f.touch("a.txt");
    f.check_everything_modified(&watch_fs_disabled);
    f.touch("b.txt");
    f.check_everything_modified(&options(true));
    f.touch("c.txt");
    f.check(&["c.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:269 LocalDiffAwarenessTest.testDisableWatchFs
#[test]
fn test_disable_watch_fs() {
    let mut f = Fixture::new();
    f.capture_first_view(&options(true));
    f.check(&[]);
    assert!(f.local_diff.get_current_view(&options(false)).is_err());
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/LocalDiffAwarenessTest.java:279 LocalDiffAwarenessTest.modifiedPathIsntUnderWatchRoot
#[test]
fn modified_path_isnt_under_watch_root() {
    let scratch = Scratch::new();
    let root = crate::test_util::platform(&scratch.dir("testCaseRoot"));
    let other_root_directory = PathBuf::from("/notundertestroot");
    assert!(!other_root_directory.starts_with(&root));
    let mut local_diff =
        WatchServiceDiffAwareness::new(root.clone(), IgnoredSubdirectories::empty());
    let old_view = local_diff.sequential_view_for_testing(0, HashSet::new());
    let new_view = local_diff
        .sequential_view_for_testing(1, HashSet::from([other_root_directory.join("foo.txt")]));
    match local_diff.get_diff(Some(&old_view), &new_view) {
        Err(DiffError::Broken(e)) => assert!(
            e.get_message().contains(&format!(
                "/notundertestroot/foo.txt is not under {}",
                root.display()
            )),
            "{}",
            e.get_message()
        ),
        other => panic!(
            "expected BrokenDiffAwarenessException: {:?}",
            other.map(|m: ModifiedFileSet| m.to_string())
        ),
    }
    local_diff.close();
}

/// Not a Java test (review of package I3, round 2, S1-B): after `mv a b` the watch service hands
/// the key of `a/sub` to `b/sub`; `watchKeyToDirBiMap.put` must drop `a/sub` from the inverse
/// view, or the deletion of a new `a/sub` cancels the watch of `b/sub` and its edits are lost.
#[test]
fn renamed_directory_keeps_watching_its_subdirectory() {
    let mut f = Fixture::new();
    f.mkdir("a/sub");
    f.touch("a/sub/x.txt");
    f.capture_first_view(&options(true));
    f.wait_for(&[]);
    std::fs::rename(f.p("a"), f.p("b")).unwrap();
    f.wait_for(&["a", "b", "b/sub", "b/sub/x.txt"]);
    f.mkdir("a");
    f.wait_for(&["a"]);
    f.mkdir("a/sub");
    std::fs::remove_dir(f.p("a/sub")).unwrap();
    f.drain();
    f.touch("b/sub/x.txt");
    f.wait_for(&["b/sub/x.txt"]);
}

/// Not a Java test (round 2, S1-B, the crash variant): `mkdir -p a/sub` after `mv a b`, then an
/// edit in `b/sub`: the event of `b/sub` must find its directory (Java's
/// `checkArgument(dir != null)`).
#[test]
fn recreated_path_of_renamed_directory_does_not_lose_the_moved_one() {
    let mut f = Fixture::new();
    f.mkdir("a/sub");
    f.touch("a/sub/x.txt");
    f.capture_first_view(&options(true));
    f.wait_for(&[]);
    std::fs::rename(f.p("a"), f.p("b")).unwrap();
    f.wait_for(&["b/sub/x.txt"]);
    f.mkdir("a/sub");
    f.wait_for(&["a", "a/sub"]);
    f.touch("b/sub/x.txt");
    f.wait_for(&["b/sub/x.txt"]);
}
