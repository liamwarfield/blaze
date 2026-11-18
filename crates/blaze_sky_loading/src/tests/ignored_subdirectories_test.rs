//! Port of IgnoredSubdirectoriesTest, plus IGNORED_SUBDIRECTORIES function checks.
//!
//! Java: src/test/java/com/google/devtools/build/lib/cmdline/IgnoredSubdirectoriesTest.java.

use blaze_cmdline::repository_name::MAIN;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use crate::test_util::{FakeRepos, TestSetup, value_of};

fn prefixes(prefixes: &[&str]) -> IndexSet<PathFragment> {
    prefixes.iter().map(|p| PathFragment::create(p)).collect()
}

fn patterns(patterns: &[&str]) -> Vec<String> {
    patterns.iter().map(|p| p.to_string()).collect()
}

#[test]
fn test_simple_union() {
    let one = IgnoredSubdirectories::of(prefixes(&["prefix1"]), patterns(&["pattern1"]));
    let two = IgnoredSubdirectories::of(prefixes(&["prefix2"]), patterns(&["pattern2"]));
    let union = one.union(&two);
    assert_eq!(
        union,
        IgnoredSubdirectories::of(
            prefixes(&["prefix1", "prefix2"]),
            patterns(&["pattern1", "pattern2"])
        )
    );
}

#[test]
fn test_self_union_noop() {
    let ignored = IgnoredSubdirectories::of(prefixes(&["pre"]), patterns(&["pat"]));
    assert_eq!(ignored.union(&ignored), ignored);
}

#[test]
fn filter_prefixes() {
    let original =
        IgnoredSubdirectories::of(prefixes(&["foo", "bar", "barbaz", "bar/qux"]), vec![]);
    let filtered = original.filter_for_directory(&PathFragment::create("bar"));
    assert_eq!(
        filtered,
        IgnoredSubdirectories::of(prefixes(&["bar", "bar/qux"]), vec![])
    );
}

#[test]
fn filter_patterns() {
    let original = IgnoredSubdirectories::of(
        prefixes(&[]),
        patterns(&[
            "**/sub",
            "foo",
            "bar/*/onesub",
            "bar/qux/**",
            "bar/ba*",
            "bar/not/*/twosub",
            "bar/**/barsub",
            "bar/sub/subsub",
        ]),
    );
    let filtered = original.filter_for_directory(&PathFragment::create("bar/sub"));
    assert_eq!(
        filtered,
        IgnoredSubdirectories::of(
            prefixes(&[]),
            patterns(&["**/sub", "bar/*/onesub", "bar/**/barsub", "bar/sub/subsub"])
        )
    );
}

#[test]
fn filter_patterns_for_root_directory() {
    // Regression test for https://github.com/bazelbuild/bazel/issues/30526: filtering for the root
    // directory (which is what happens for the "//..." target pattern) must not drop any pattern.
    let original = IgnoredSubdirectories::of(
        prefixes(&["prefix"]),
        patterns(&[".claude", "foo/bar", "**/sub", "*/qux"]),
    );
    let filtered = original.filter_for_directory(&PathFragment::empty_fragment());
    assert_eq!(filtered, original);
}

#[test]
fn filter_patterns_for_hidden_files() {
    let original = IgnoredSubdirectories::of(
        prefixes(&[]),
        patterns(&[
            "not/sub",
            "*dden/sub",
            ".hidden/**/sub",
            ".hi*/*/sub",
            "*/sub",
            "**/sub",
        ]),
    );
    let filtered = original.filter_for_directory(&PathFragment::create(".hidden"));
    // Glob semantics say that "*dden" is not supposed to match ".hidden".
    // "**" and "*" and ".hi*" should, though.
    assert_eq!(
        filtered,
        IgnoredSubdirectories::of(
            prefixes(&[]),
            patterns(&[".hidden/**/sub", ".hi*/*/sub", "*/sub", "**/sub"])
        )
    );
}

// Not a Java test: the IGNORED_SUBDIRECTORIES function on a .bazelignore with comments and blank
// lines, and REPO.bazel's ignore_directories().
#[test]
fn ignored_subdirectories_function_reads_bazelignore_and_repo_file() {
    let setup = TestSetup::new(FakeRepos::default());
    setup.scratch.file(
        &setup.ws(".bazelignore"),
        "# Exclude Bazel roots (workspaces)\nc/fuzz\n\ngo\r\njava\n",
    );
    setup.scratch.file(
        &setup.ws("REPO.bazel"),
        "ignore_directories([\"**/node_modules\"])\n",
    );
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, IgnoredSubdirectoriesKey::key(MAIN.clone()));
    let ignored = value.as_ignored_subdirectories();
    assert_eq!(
        *ignored,
        IgnoredSubdirectories::of(
            prefixes(&["c/fuzz", "go", "java"]),
            patterns(&["**/node_modules"])
        )
    );
    assert_eq!(
        ignored.matching_entry(&PathFragment::create("c/fuzz/x")),
        Some("c/fuzz".to_owned())
    );
    assert_eq!(
        ignored.matching_entry(&PathFragment::create("a/node_modules/b")),
        Some("**/node_modules".to_owned())
    );
    assert_eq!(ignored.matching_entry(&PathFragment::create("c")), None);
}
