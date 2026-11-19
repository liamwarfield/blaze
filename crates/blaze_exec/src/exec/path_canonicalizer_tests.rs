//! Port of `PathCanonicalizerTest`
//! (src/test/java/com/google/devtools/build/lib/remote/PathCanonicalizerTest.java): the 28
//! `@Test` methods, one Rust test each.
//!
//! PORT: Java drives the canonicalizer over a plain `InMemoryFileSystem`; Blaze's port of
//! `InMemoryFileSystem` is [`RemoteInMemoryFileSystem`] (same inode logic), whose
//! `getOutputStream` throws and whose file nodes must carry remote metadata. `createNonSymlink`
//! therefore injects an empty remote file (`injectFile`, size 0, the digest of no bytes) where
//! Java writes empty content: either way a regular-file node, which is all the canonicalizer's
//! resolver distinguishes (symlink or not).
//!
//! PORT: Java's canonicalizer holds its resolver; the Rust one takes it per call (see
//! [`PathCanonicalizer`]), so the fixture passes `self` as the resolver.
//!
//! The two Windows tests (`assumeTrue(OS.getCurrent() == OS.WINDOWS)`) are skipped by JUnit on
//! Linux; here they are `#[ignore]` with that reason.

use std::any::Any;
use std::io;
use std::panic::AssertUnwindSafe;

use blaze_vfs::{
    DigestHashFunction, FileSymlinkLoopException, FileSystem, MAX_SYMLINKS, NotASymlinkException,
    PathFragment,
};

use crate::actions::FileArtifactValue;

use super::{PathCanonicalizer, PathCanonicalizerResolver, RemoteInMemoryFileSystem};

// Java: PathCanonicalizerTest.java:49 PathCanonicalizerTest (fields fs, canonicalizer)
struct Fixture {
    fs: RemoteInMemoryFileSystem,
    canonicalizer: PathCanonicalizer,
}

// Java: PathCanonicalizerTest.java:53 PathCanonicalizerTest.resolve
impl PathCanonicalizerResolver for RemoteInMemoryFileSystem {
    fn resolve_one_link(&self, path_fragment: &PathFragment) -> io::Result<Option<PathFragment>> {
        match self.read_symbolic_link(path_fragment) {
            Ok(target) => Ok(Some(target)),
            Err(e) if NotASymlinkException::is(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Which exception class `assertFailure` expects.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
enum Expected {
    FileNotFoundException,
    FileSymlinkLoopException,
    IllegalArgumentException,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            fs: RemoteInMemoryFileSystem::new(DigestHashFunction::Sha256),
            canonicalizer: PathCanonicalizer::new(),
        }
    }

    // Java: PathCanonicalizerTest.java:278 PathCanonicalizerTest.createSymlink
    fn create_symlink(&self, link_path_str: &str, target_path_str: &str) {
        let link_path = path_fragment(link_path_str);
        self.fs
            .create_directory_and_parents(&link_path.get_parent_directory().unwrap())
            .unwrap();
        self.fs
            .create_symbolic_link(&link_path, &path_fragment(target_path_str))
            .unwrap();
    }

    // Java: PathCanonicalizerTest.java:284 PathCanonicalizerTest.createNonSymlink
    fn create_non_symlink(&self, path_str: &str) {
        let path = path_fragment(path_str);
        self.fs
            .create_directory_and_parents(&path.get_parent_directory().unwrap())
            .unwrap();
        // FileSystemUtils.writeContent(path, UTF_8, ""): see the PORT note of this module.
        let empty_digest = DigestHashFunction::Sha256
            .hash_reader(&mut io::empty())
            .unwrap();
        self.fs
            .inject_file(
                &path,
                FileArtifactValue::create_for_remote_file(empty_digest, 0, 1),
            )
            .unwrap();
    }

    // Java: PathCanonicalizerTest.java:290 PathCanonicalizerTest.deleteTree
    fn delete_tree(&self, path_str: &str) {
        self.canonicalizer.clear_prefix(&path_fragment(path_str));
        self.fs.delete_tree(&path_fragment(path_str)).unwrap();
    }

    // Java: PathCanonicalizerTest.java:295 PathCanonicalizerTest.assertSuccess
    fn assert_success(&self, input: &str, output: &str) {
        let result = self
            .canonicalizer
            .resolve_symbolic_links(&path_fragment(input), &self.fs)
            .unwrap_or_else(|e| panic!("resolveSymbolicLinks({input}) failed: {e}"));
        assert_eq!(result, path_fragment(output));
    }

    // Java: PathCanonicalizerTest.java:300 PathCanonicalizerTest.assertFailure
    fn assert_failure(&self, exception_class: Expected, input: &str) {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            self.canonicalizer
                .resolve_symbolic_links(&path_fragment(input), &self.fs)
        }));
        match (exception_class, result) {
            (Expected::FileNotFoundException, Ok(Err(e))) => {
                assert_eq!(e.kind(), io::ErrorKind::NotFound, "{e}")
            }
            (Expected::FileSymlinkLoopException, Ok(Err(e))) => {
                assert!(FileSymlinkLoopException::is(&e), "{e}")
            }
            (Expected::IllegalArgumentException, Err(payload)) => {
                let message = panic_message(&payload);
                assert!(message.starts_with("IllegalArgumentException"), "{message}");
            }
            (expected, Ok(result)) => panic!("expected {expected:?}, got {result:?}"),
            (expected, Err(payload)) => {
                panic!(
                    "expected {expected:?}, got panic {}",
                    panic_message(&payload)
                )
            }
        }
    }
}

fn panic_message(payload: &Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        String::new()
    }
}

// Java: PathCanonicalizerTest.java:305 PathCanonicalizerTest.pathFragment
// PORT: the Windows drive prefix is skipped (Blaze is Linux-only).
fn path_fragment(path_str: &str) -> PathFragment {
    PathFragment::create(path_str)
}

// Java: PathCanonicalizerTest.java:63 PathCanonicalizerTest.testRoot
#[test]
fn test_root() {
    let t = Fixture::new();
    t.assert_success("/", "/");
}

// Java: PathCanonicalizerTest.java:68 PathCanonicalizerTest.testAlreadyCanonical
#[test]
fn test_already_canonical() {
    let t = Fixture::new();
    t.create_non_symlink("/a/b");
    t.assert_success("/a/b", "/a/b");
}

// Java: PathCanonicalizerTest.java:74 PathCanonicalizerTest.testAbsoluteSymlinkToFile
#[test]
fn test_absolute_symlink_to_file() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/c/d");
    t.create_non_symlink("/c/d");
    t.assert_success("/a/b", "/c/d");
}

// Java: PathCanonicalizerTest.java:81 PathCanonicalizerTest.testAbsoluteSymlinkToDirectory
#[test]
fn test_absolute_symlink_to_directory() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d/e");
    t.create_non_symlink("/d/e/c");
    t.assert_success("/a/b/c", "/d/e/c");
}

// Java: PathCanonicalizerTest.java:88 PathCanonicalizerTest.testAbsoluteSymlinkToDifferentDrive
#[test]
#[ignore = "Windows only: assumeTrue(OS.getCurrent() == OS.WINDOWS) at PathCanonicalizerTest.java:89; JUnit skips it on Linux"]
fn test_absolute_symlink_to_different_drive() {
    let t = Fixture::new();
    t.create_symlink("C:/a/b", "D:/e/f");
    t.create_non_symlink("D:/e/f/c");
    t.assert_success("C:/a/b/c", "D:/e/f/c");
}

// Java: PathCanonicalizerTest.java:96 PathCanonicalizerTest.testRelativeSymlinkToFileInSameDirectory
#[test]
fn test_relative_symlink_to_file_in_same_directory() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "c");
    t.create_non_symlink("/a/c");
    t.assert_success("/a/b", "/a/c");
}

// Java: PathCanonicalizerTest.java:103 PathCanonicalizerTest.testRelativeSymlinkToFileInDirectoryBelow
#[test]
fn test_relative_symlink_to_file_in_directory_below() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "c/d");
    t.create_non_symlink("/a/c/d");
    t.assert_success("/a/b", "/a/c/d");
}

// Java: PathCanonicalizerTest.java:110 PathCanonicalizerTest.testRelativeSymlinkToFileInDirectoryAbove
#[test]
fn test_relative_symlink_to_file_in_directory_above() {
    let t = Fixture::new();
    t.create_symlink("/a/b/c", "../d/e");
    t.create_non_symlink("/a/d/e");
    t.assert_success("/a/b/c", "/a/d/e");
}

// Java: PathCanonicalizerTest.java:117 PathCanonicalizerTest.testRelativeSymlinkToRoot
#[test]
fn test_relative_symlink_to_root() {
    let t = Fixture::new();
    t.create_symlink("/a/b/c", "../../d");
    t.create_non_symlink("/d");
    t.assert_success("/a/b/c", "/d");
}

// Java: PathCanonicalizerTest.java:124 PathCanonicalizerTest.testRelativeSymlinkWithTooManyUplevelReferences
#[test]
fn test_relative_symlink_with_too_many_uplevel_references() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "../../d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");
}

// Java: PathCanonicalizerTest.java:131 PathCanonicalizerTest.testMultipleSymlinks
#[test]
fn test_multiple_symlinks() {
    let t = Fixture::new();
    t.create_symlink("/a", "/b");
    t.create_symlink("/b/c", "/d");
    t.create_symlink("/d/e", "/f");
    t.create_non_symlink("/f");
    t.assert_success("/a/c/e", "/f");
}

// Java: PathCanonicalizerTest.java:140 PathCanonicalizerTest.testReplayCanonical
#[test]
fn test_replay_canonical() {
    let t = Fixture::new();
    t.create_non_symlink("/a/b/c");
    t.assert_success("/a/b/c", "/a/b/c");
    t.assert_success("/a/b/c", "/a/b/c");
}

// Java: PathCanonicalizerTest.java:147 PathCanonicalizerTest.testReplaySymlink
#[test]
fn test_replay_symlink() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");
    t.assert_success("/a/b/c", "/d/c");
}

// Java: PathCanonicalizerTest.java:155 PathCanonicalizerTest.testDistinguishPathsWithCommonPrefix
#[test]
fn test_distinguish_paths_with_common_prefix() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.create_non_symlink("/a/e");
    t.assert_success("/a/b/c", "/d/c");
    t.assert_success("/a/e", "/a/e");
}

// Java: PathCanonicalizerTest.java:164 PathCanonicalizerTest.testDistinguishPathsWithDifferentDriveLetter
#[test]
#[ignore = "Windows only: assumeTrue(OS.getCurrent() == OS.WINDOWS) at PathCanonicalizerTest.java:165; JUnit skips it on Linux"]
fn test_distinguish_paths_with_different_drive_letter() {
    let t = Fixture::new();
    t.create_symlink("C:/a/b", "D:/d");
    t.create_non_symlink("D:/d/c");
    t.create_non_symlink("D:/a/b/c");
    t.assert_success("C:/a/b/c", "D:/d/c");
    t.assert_success("D:/a/b/c", "D:/a/b/c");
}

// Java: PathCanonicalizerTest.java:174 PathCanonicalizerTest.testClearAndReplaceWithSymlink
#[test]
fn test_clear_and_replace_with_symlink() {
    let t = Fixture::new();
    t.create_non_symlink("/a/b/c");
    t.assert_success("/a/b/c", "/a/b/c");
    t.delete_tree("/a/b");
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");
}

// Java: PathCanonicalizerTest.java:184 PathCanonicalizerTest.testClearAndReplaceWithNonSymlink
#[test]
fn test_clear_and_replace_with_non_symlink() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");
    t.delete_tree("/a/b");
    t.create_non_symlink("/a/b/c");
    t.assert_success("/a/b/c", "/a/b/c");
}

// Java: PathCanonicalizerTest.java:194 PathCanonicalizerTest.testClearSymlinkAndDoNotReplace
#[test]
fn test_clear_symlink_and_do_not_replace() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");
    t.delete_tree("/a/b");
    t.assert_failure(Expected::FileNotFoundException, "/a/b/c");
}

// Java: PathCanonicalizerTest.java:203 PathCanonicalizerTest.testClearNonSymlinkAndDoNotReplace
#[test]
fn test_clear_non_symlink_and_do_not_replace() {
    let t = Fixture::new();
    t.create_non_symlink("/a/b/c");
    t.assert_success("/a/b/c", "/a/b/c");
    t.delete_tree("/a/b");
    t.assert_failure(Expected::FileNotFoundException, "/a/b/c");
}

// Java: PathCanonicalizerTest.java:211 PathCanonicalizerTest.testClearUnknownPathDescendingFromSymlink
#[test]
fn test_clear_unknown_path_descending_from_symlink() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d");
    t.assert_success("/a/b", "/d");
    t.delete_tree("/a/b/c");
    t.assert_success("/a/b", "/d");
}

// Java: PathCanonicalizerTest.java:220 PathCanonicalizerTest.testClearUnknownPathDescendingFromNonSymlink
#[test]
fn test_clear_unknown_path_descending_from_non_symlink() {
    let t = Fixture::new();
    t.create_non_symlink("/a/b");
    t.assert_success("/a/b", "/a/b");
    t.delete_tree("/a/b/c");
    t.assert_success("/a/b", "/a/b");
}

// Java: PathCanonicalizerTest.java:228 PathCanonicalizerTest.testClearPathDescendingThroughSymlinkInvalidatesIt
#[test]
fn test_clear_path_descending_through_symlink_invalidates_it() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/d");
    t.create_non_symlink("/d/c");
    t.assert_success("/a/b/c", "/d/c");

    t.fs.delete(&path_fragment("/a/b")).unwrap();
    t.create_non_symlink("/a/b/c");
    t.canonicalizer.clear_prefix(&path_fragment("/a/b/c"));

    t.assert_success("/a/b/c", "/a/b/c");
}

// Java: PathCanonicalizerTest.java:241 PathCanonicalizerTest.testSymlinkSelfLoop
#[test]
fn test_symlink_self_loop() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/a/b");
    t.assert_failure(Expected::FileSymlinkLoopException, "/a/b");
}

// Java: PathCanonicalizerTest.java:247 PathCanonicalizerTest.testSymlinkMutualLoop
#[test]
fn test_symlink_mutual_loop() {
    let t = Fixture::new();
    t.create_symlink("/a/b", "/c/d");
    t.create_symlink("/c/d", "/a/b");
    t.assert_failure(Expected::FileSymlinkLoopException, "/a/b");
}

// Java: PathCanonicalizerTest.java:254 PathCanonicalizerTest.testSymlinkChainTooLong
#[test]
fn test_symlink_chain_too_long() {
    let t = Fixture::new();
    for i in 0..MAX_SYMLINKS + 1 {
        t.create_symlink(&format!("/{}", i), &format!("/{}", i + 1));
    }
    t.assert_failure(Expected::FileSymlinkLoopException, "/0");
}

// Java: PathCanonicalizerTest.java:262 PathCanonicalizerTest.testFileNotFound
#[test]
fn test_file_not_found() {
    let t = Fixture::new();
    t.assert_failure(Expected::FileNotFoundException, "/a/b");
    t.create_non_symlink("/a/b");
    t.assert_success("/a/b", "/a/b");
}

// Java: PathCanonicalizerTest.java:269 PathCanonicalizerTest.testEmpty
#[test]
fn test_empty() {
    let t = Fixture::new();
    t.assert_failure(Expected::IllegalArgumentException, "");
}

// Java: PathCanonicalizerTest.java:274 PathCanonicalizerTest.testNonAbsolute
#[test]
fn test_non_absolute() {
    let t = Fixture::new();
    t.assert_failure(Expected::IllegalArgumentException, "a/b");
}
