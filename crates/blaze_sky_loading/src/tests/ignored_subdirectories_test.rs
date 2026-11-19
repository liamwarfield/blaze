//! Port of IgnoredSubdirectoriesTest, plus IGNORED_SUBDIRECTORIES function checks.
//!
//! Java: src/test/java/com/google/devtools/build/lib/cmdline/IgnoredSubdirectoriesTest.java.

use blaze_cmdline::repository_name::MAIN;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use std::sync::Arc;

use blaze_events::StoredEventHandler;
use blaze_skyframe::SkyKey;

use crate::ignored_subdirectories::{
    IgnoredSubdirectories, IgnoredSubdirectoriesKey, InvalidIgnorePathException,
};
use crate::test_util::{FakeRepos, TestSetup, evaluate, value_of};

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

// Not a Java test (oracle, ~/.cache/blaze/work/bytes-C1): IgnoredSubdirectoriesFunction reads
// .bazelignore through a UTF-8 InputStreamReader, so `p/dé` is the Unicode string `p/d\u{e9}`;
// directory names are internal strings (`p/d\u{c3}\u{a9}`), so Bazel does not ignore `p/dé`
// (its files are still globbed) while it ignores the ASCII `p/dx`.
#[test]
fn bazelignore_non_ascii_entry_is_decoded_as_utf8() {
    let setup = TestSetup::new(FakeRepos::default());
    setup
        .scratch
        .file_bytes(&setup.ws(".bazelignore"), b"# c\np/d\xc3\xa9\np/dx\n");
    let evaluator = setup.evaluator();
    let value = value_of(&evaluator, IgnoredSubdirectoriesKey::key(MAIN.clone()));
    let ignored = value.as_ignored_subdirectories();
    assert_eq!(
        *ignored,
        IgnoredSubdirectories::of(prefixes(&["p/d\u{e9}", "p/dx"]), vec![])
    );
    assert_eq!(
        ignored.matching_entry(&PathFragment::create("p/d\u{c3}\u{a9}/in.txt")),
        None
    );
    assert_eq!(
        ignored.matching_entry(&PathFragment::create("p/dx/in.txt")),
        Some("p/dx".to_owned())
    );
}

fn bazelignore_error(contents: &[u8]) -> (String, String) {
    let setup = TestSetup::new(FakeRepos::default());
    let file = setup
        .scratch
        .file_bytes(&setup.ws(".bazelignore"), contents);
    let evaluator = setup.evaluator();
    let key = IgnoredSubdirectoriesKey::key(MAIN.clone());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key.clone())],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    let error = result
        .get_error(&SkyKey::new(key))
        .expect("IGNORED_SUBDIRECTORIES should fail");
    let e = error
        .exception_as::<InvalidIgnorePathException>()
        .expect("an InvalidIgnorePathException");
    (e.to_string(), file.to_string())
}

// Not a Java test (oracle): `Path.of(line)` encodes the line with the server's ISO-8859-1
// `sun.jnu.encoding`, so a character outside Latin-1 (🐷, F0 9F 90 B7) is an invalid path. The
// oracle prints the message through the Latin-1 console: `...unmappable characters: pig?`.
#[test]
fn bazelignore_entry_outside_latin1_is_an_invalid_path() {
    let (message, file) = bazelignore_error(b"p/dx\npig\xf0\x9f\x90\xb7//\n");
    assert_eq!(
        message,
        format!(
            "Invalid path in {file}: Malformed input or input contains unmappable characters: \
             pig\u{1f437}"
        )
    );
}

// Not a Java test (oracle): a byte that is not UTF-8 (FF) decodes to U+FFFD, which is not
// Latin-1 either (`...unmappable characters: zz?`).
#[test]
fn bazelignore_entry_with_malformed_utf8_is_an_invalid_path() {
    let (message, file) = bazelignore_error(b"zz\xff\n");
    assert_eq!(
        message,
        format!(
            "Invalid path in {file}: Malformed input or input contains unmappable characters: \
             zz\u{fffd}"
        )
    );
}

// Not a Java test: java.nio rejects NUL before encoding, with the line as written.
#[test]
fn bazelignore_entry_with_nul_is_an_invalid_path() {
    let (message, file) = bazelignore_error(b"a//b\x00/\n");
    assert_eq!(
        message,
        format!("Invalid path in {file}: Nul character not allowed: a//b\u{0}/")
    );
}
