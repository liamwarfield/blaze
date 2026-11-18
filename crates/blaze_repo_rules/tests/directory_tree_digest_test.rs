//! Tests for DIRECTORY_TREE_DIGEST.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/DirectoryTreeDigestFunctionTest.java.
//! Each Java test builds a tree under a scratch file system, evaluates
//! `DirectoryTreeDigestValue.key(<absolute root>, <dir>)` with FILE_STATE, FILE,
//! DIRECTORY_LISTING_STATE, DIRECTORY_LISTING and DIRECTORY_TREE_DIGEST registered, changes the
//! tree and compares the digests. Not ported yet: the loading-function evaluator setup
//! (`blaze_sky_loading`'s `TestSetup` is private to that crate).

use blaze_repo_rules::repository::directory_tree_digest::DIRECTORY_TREE_DIGEST;
use blaze_repo_rules::repository::directory_tree_digest::DirectoryTreeDigestValue;

#[test]
fn function_name_and_value_to_string() {
    // Java: SkyFunctions.DIRECTORY_TREE_DIGEST and the DirectoryTreeDigestValue record toString.
    assert_eq!(DIRECTORY_TREE_DIGEST.get_name(), "DIRECTORY_TREE_DIGEST");
    assert_eq!(
        format!("{:?}", DirectoryTreeDigestValue::of("abc".to_owned())),
        "DirectoryTreeDigestValue[hexDigest=abc]"
    );
}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn basic() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn add_file() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn remove_file() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn rename_file() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn swap_dir_and_file() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn change_mtime() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn symlink() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn dangling_symlink() {}

#[test]
#[ignore = "needs a Skyframe evaluator with the file and directory listing functions over a scratch file system"]
fn symlink_pointing_to_same_contents() {}
