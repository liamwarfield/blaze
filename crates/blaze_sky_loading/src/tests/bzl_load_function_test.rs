//! Port of BzlLoadFunctionTest.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/BzlLoadFunctionTest.java. Builtins
//! injection is disabled (`--experimental_builtins_bzl_path=`) unless a test says otherwise.
//!
//! Differences from the Java test environment (`BuildViewTestCase`) that the ports carry:
//! - Two fixtures. [`setup`] is blaze_starlark's minimal Starlark environment (the tests that
//!   need no Bazel symbol); `PackageLoadingTestCase` is Bazel's environment (`visibility()`,
//!   `struct` in .scl, ...), used by the load-visibility and .scl tests.
//! - Every lookup uses a fresh evaluator and its own event collector, so `assertNoEvents()` /
//!   `assertContainsEvent()` after a lookup are asserted on the events of that lookup, and
//!   Java's `eventCollector.clear()` between two lookups has nothing to do.
//! - `reporter.removeHandler(failFastHandler)`: [`check_successful_lookup`] always fails on an
//!   ERROR event (every Java call happens while the fail-fast handler is installed);
//!   [`check_failing_lookup`] never does.
//! - Java's workspace is `/workspace`; here it is a temp directory, so expected texts that name
//!   a workspace path are built with `PackageLoadingTestCase::workspace_path`.
//! - External repositories: Java declares them in MODULE.bazel (`bazel_dep` +
//!   `local_path_override`, or the test registry) and Bzlmod computes the repository mappings.
//!   Bzlmod is not in this crate, so [`set_external_repos`] supplies the mappings and the
//!   repository directories Bzlmod would compute (through `OracleRepos`, which symlinks
//!   `<output_base>/external/<name>` to the directory as Bazel does for a local override).

use std::sync::Arc;

use blaze_cmdline::{Label, RepositoryName};
use blaze_events::{Event, EventKind, StoredEventHandler};
use blaze_skyframe::{EvaluationResult, SkyKey};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, Root};

use crate::bzl_load::{BzlLoadFailedException, BzlLoadKey};
use crate::repos::OracleRepos;
use crate::starlark_builtins::{StarlarkBuiltinsKey, exports_entrypoint_key};
use crate::test_util::{FakeRepos, TestSetup, evaluate};
use crate::tests::package_loading_test_case::{
    PackageLoadingTestCase, assert_contains_event, event_strings,
};

fn setup() -> TestSetup {
    let mut setup = TestSetup::new(FakeRepos::default());
    setup.semantics.experimental_builtins_bzl_path = String::new();
    setup
}

// Java: BzlLoadFunctionTest.key
fn key(label: &str) -> BzlLoadKey {
    BzlLoadKey::key_for_build(Label::parse_canonical_unchecked(label))
}

fn file(setup: &TestSetup, rel: &str, contents: &str) {
    setup.scratch.file(&setup.ws(rel), contents);
}

/// Java `SkyframeExecutorTestUtils.evaluate(getSkyframeExecutor(), skyKey, /* keepGoing= */
/// false, reporter)`, with the events the reporter would have received.
fn eval_key(setup: &TestSetup, k: &BzlLoadKey) -> (EvaluationResult, Vec<Event>) {
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(k.clone())],
        false,
        events.clone(),
    );
    (result, events.get_events())
}

/// Loads a .bzl with the given label and asserts success; returns the events of the lookup.
// Java: BzlLoadFunctionTest.checkSuccessfulLookup (and BzlLoadFunctionTest.get)
fn check_successful_lookup(setup: &TestSetup, label: &str) -> Vec<Event> {
    let k = key(label);
    let (result, events) = eval_key(setup, &k);
    assert!(
        !result.has_error(),
        "{:?}\nevents: {:#?}",
        result,
        event_strings(&events)
    );
    // Ensure that the file has been processed by checking its Module for the label field.
    assert_eq!(
        result.get(&k).unwrap().get_module_context().label,
        Label::parse_canonical_unchecked(label)
    );
    // Java: FoundationTestCase.failFastHandler
    let errors: Vec<String> = events
        .iter()
        .filter(|e| EventKind::ERRORS.contains(e.get_kind()))
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "unexpected errors: {errors:#?}");
    events
}

/// Loads a .bzl with the given label and asserts BzlLoadFailedException with the given message;
/// returns the events of the lookup.
// Java: BzlLoadFunctionTest.checkFailingLookup
fn check_failing_lookup(setup: &TestSetup, label: &str, expected_message: &str) -> Vec<Event> {
    let k = key(label);
    let (result, events) = eval_key(setup, &k);
    assert!(
        result.has_error(),
        "the lookup of {label} succeeded; events: {:#?}",
        event_strings(&events)
    );
    let error = result.get_error(&SkyKey::new(k)).expect("error entry");
    let e = error
        .exception_as::<BzlLoadFailedException>()
        .expect("a BzlLoadFailedException");
    assert!(
        e.get_message().contains(expected_message),
        "{:?} does not contain {:?}",
        e.get_message(),
        expected_message
    );
    events
}

/// Java `assertNoEvents()`: the test's `eventCollector` keeps
/// `EventKind.ERRORS_WARNINGS_AND_INFO` (FoundationTestCase).
fn assert_no_events(events: &[Event]) {
    let collected: Vec<String> = events
        .iter()
        .filter(|e| EventKind::ERRORS_WARNINGS_AND_INFO.contains(e.get_kind()))
        .map(|e| e.to_string())
        .collect();
    assert!(collected.is_empty(), "unexpected events: {collected:#?}");
}

/// Java `assertDoesNotContainEvent(unexpected)` (`MoreAsserts.assertDoesNotContainEvents`): no
/// event's message contains `unexpected`.
fn assert_does_not_contain_event(events: &[Event], unexpected: &str) {
    let messages: Vec<String> = events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect();
    assert!(
        !messages.iter().any(|m| m.contains(unexpected)),
        "an event contains {unexpected:?}; events: {messages:#?}"
    );
}

/// One repository of [`set_external_repos`]: its canonical name, the module it belongs to, its
/// repository mapping (apparent name, canonical name) and its directory.
struct Repo<'a> {
    name: &'a str,
    module_name: &'a str,
    mapping: &'a [(&'a str, &'a str)],
    directory: Option<String>,
}

/// Replaces the external repositories of `setup` (see the module comment).
fn set_external_repos(setup: &mut TestSetup, repos: &[Repo<'_>]) {
    let mut repos_json = serde_json::Map::new();
    for repo in repos {
        let mapping: Vec<Vec<&str>> = repo
            .mapping
            .iter()
            .map(|(apparent, canonical)| vec![*apparent, *canonical])
            .collect();
        repos_json.insert(
            repo.name.to_owned(),
            serde_json::json!({
                "found": true,
                "module_name": repo.module_name,
                "module_version": "",
                "mapping": mapping,
                "directory": repo.directory,
            }),
        );
    }
    let json = serde_json::json!({"format": 1, "repos": repos_json}).to_string();
    let repos =
        OracleRepos::parse(&json, setup.config.directories.clone()).expect("the repos JSON");
    setup.config.external_repos = Arc::new(repos);
}

/// The `@bazel_tools` entry every repository set carries (nothing fetches it).
const BAZEL_TOOLS_REPO: Repo<'static> = Repo {
    name: "bazel_tools",
    module_name: "bazel_tools",
    mapping: &[("bazel_tools", "bazel_tools")],
    directory: None,
};

/// Makes reading `path` fail (Java `CustomInMemoryFs.badPathForRead`: `getInputStream` throws
/// an IOException; `stat` still works).
fn make_unreadable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        crate::test_util::platform(path),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert!(
        std::fs::read(crate::test_util::platform(path)).is_err(),
        "{} is still readable (is the test running as root?)",
        path.get_path_string()
    );
}

// Java: BzlLoadFunctionTest.testBzlLoadLabels
#[test]
fn test_bzl_load_labels() {
    let s = setup();
    file(&s, "pkg1/BUILD", "");
    file(&s, "pkg1/ext.bzl", "");
    check_successful_lookup(&s, "//pkg1:ext.bzl");

    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg2/dir/ext.bzl", "");
    check_successful_lookup(&s, "//pkg2:dir/ext.bzl");

    file(&s, "dir/pkg3/BUILD", "");
    file(&s, "dir/pkg3/dir/ext.bzl", "");
    check_successful_lookup(&s, "//dir/pkg3:dir/ext.bzl");
}

// Java: BzlLoadFunctionTest.testBzlLoadLabelsAlternativeRoot (with
// BzlLoadFunctionTest.preparePackageLoading: the package path is the workspace and `/root_2`;
// here `root_2` is a sibling of the workspace in the scratch directory).
#[test]
fn test_bzl_load_labels_alternative_root() {
    let s = setup();
    let alternative_root = s.scratch.dir("root_2");
    s.set_package_path(vec![
        Root::from_path(s.workspace().clone()),
        Root::from_path(alternative_root),
    ]);
    s.scratch.file("root_2/pkg4/BUILD", "");
    s.scratch.file("root_2/pkg4/ext.bzl", "");
    check_successful_lookup(&s, "//pkg4:ext.bzl");
}

// Java: BzlLoadFunctionTest.testBzlLoadLabelsMultipleBuildFiles
#[test]
fn test_bzl_load_labels_multiple_build_files() {
    let s = setup();
    file(&s, "dir1/BUILD", "");
    file(&s, "dir1/dir2/BUILD", "");
    file(&s, "dir1/dir2/ext.bzl", "");
    check_successful_lookup(&s, "//dir1/dir2:ext.bzl");
}

// Java: BzlLoadFunctionTest.testLoadFromStarlarkFileInRemoteRepo
// Java's MODULE.bazel (`bazel_dep(name = 'a_remote_repo')` + `local_path_override(module_name =
// 'a_remote_repo', path = '/a_remote_repo')`) becomes the repository set below; `/a_remote_repo`
// is a sibling of the workspace in the scratch directory.
#[test]
fn test_load_from_starlark_file_in_remote_repo() {
    let mut s = setup();
    let remote_repo = s.scratch.dir("a_remote_repo");
    s.scratch.file(
        "a_remote_repo/MODULE.bazel",
        "module(name = 'a_remote_repo')",
    );
    s.scratch.file("a_remote_repo/remote_pkg/BUILD", "");
    s.scratch.file(
        "a_remote_repo/remote_pkg/ext1.bzl",
        "load(':ext2.bzl', 'CONST')",
    );
    s.scratch
        .file("a_remote_repo/remote_pkg/ext2.bzl", "CONST = 17");
    set_external_repos(
        &mut s,
        &[
            Repo {
                name: "",
                module_name: "",
                mapping: &[
                    ("", ""),
                    ("a_remote_repo", "a_remote_repo+"),
                    ("bazel_tools", "bazel_tools"),
                ],
                directory: None,
            },
            Repo {
                name: "a_remote_repo+",
                module_name: "a_remote_repo",
                mapping: &[
                    ("a_remote_repo", "a_remote_repo+"),
                    ("bazel_tools", "bazel_tools"),
                ],
                directory: Some(remote_repo.get_path_string().to_string()),
            },
            BAZEL_TOOLS_REPO,
        ],
    );
    check_successful_lookup(&s, "@@a_remote_repo+//remote_pkg:ext1.bzl");
}

// Java: BzlLoadFunctionTest.testLoadRelativeLabel
#[test]
fn test_load_relative_label() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext1.bzl", "a = 1");
    file(&s, "pkg/ext2.bzl", "load(':ext1.bzl', 'a')");
    check_successful_lookup(&s, "//pkg:ext2.bzl");
}

// Java: BzlLoadFunctionTest.testLoadAbsoluteLabel
#[test]
fn test_load_absolute_label() {
    let s = setup();
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg3/BUILD", "");
    file(&s, "pkg2/ext.bzl", "b = 1");
    file(&s, "pkg3/ext.bzl", "load('//pkg2:ext.bzl', 'b')");
    check_successful_lookup(&s, "//pkg3:ext.bzl");
}

// Java: BzlLoadFunctionTest.testLoadFromSameAbsoluteLabelTwice
#[test]
fn test_load_from_same_absolute_label_twice() {
    let s = setup();
    file(&s, "pkg1/BUILD", "");
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg1/ext.bzl", "a = 1\nb = 2\n");
    file(
        &s,
        "pkg2/ext.bzl",
        "load(\"//pkg1:ext.bzl\", \"a\", \"b\")\n",
    );
    check_successful_lookup(&s, "//pkg2:ext.bzl");
}

// Java: BzlLoadFunctionTest.testLoadFromSameRelativeLabelTwice
#[test]
fn test_load_from_same_relative_label_twice() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext1.bzl", "a = 1\nb = 2\n");
    file(&s, "pkg/ext2.bzl", "load(\":ext1.bzl\", \"a\", \"b\")\n");
    check_successful_lookup(&s, "//pkg:ext2.bzl");
}

// Java: BzlLoadFunctionTest.testLoadFromRelativeLabelInSubdir
#[test]
fn test_load_from_relative_label_in_subdir() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/subdir/ext1.bzl", "a = 1");
    file(&s, "pkg/subdir/ext2.bzl", "load(':subdir/ext1.bzl', 'a')");
    check_successful_lookup(&s, "//pkg:subdir/ext2.bzl");
}

// Java: BzlLoadFunctionTest.testLoadBadExtension_sclDisabled
#[test]
fn test_load_bad_extension_scl_disabled() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = false;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.bzl", "load(':foo.garbage', 'a')");
    let events = check_failing_lookup(&s, "//pkg:ext.bzl", "has invalid load statements");
    assert_contains_event(
        &events,
        "The label must reference a file with extension \".bzl\"",
    );
    assert_does_not_contain_event(&events, ".scl");
}

// Java: BzlLoadFunctionTest.testLoadBadExtension_sclEnabled
#[test]
fn test_load_bad_extension_scl_enabled() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = true;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.bzl", "load(':foo.garbage', 'a')");
    let events = check_failing_lookup(&s, "//pkg:ext.bzl", "has invalid load statements");
    assert_contains_event(
        &events,
        "The label must reference a file with extension \".bzl\" or \".scl\"",
    );
}

// Java: BzlLoadFunctionTest.testLoadingSclRequiresExperimentalFlag
#[test]
fn test_loading_scl_requires_experimental_flag() {
    let mut s = setup();
    s.semantics.experimental_enable_scl_dialect = false;
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.scl", "");
    check_failing_lookup(
        &s,
        "//pkg:ext.scl",
        "loading .scl files requires setting --experimental_enable_scl_dialect",
    );
}

// Java: BzlLoadFunctionTest.testCanLoadScl
#[test]
fn test_can_load_scl() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext.scl", &[]);
    check_successful_lookup(&t.setup, "//pkg:ext.scl");
}

// Java: BzlLoadFunctionTest.testCanLoadSclFromBzlAndScl
#[test]
fn test_can_load_scl_from_bzl_and_scl() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext1.scl", &["a = 1"]);
    // Can use relative load label syntax from ext2a.bzl, but not from ext2b.scl.
    t.scratch_file("pkg/ext2a.bzl", &["load(':ext1.scl', 'a')"]);
    t.scratch_file("pkg/ext2b.scl", &["load('//pkg:ext1.scl', 'a')"]);

    check_successful_lookup(&t.setup, "//pkg:ext2a.bzl");
    check_successful_lookup(&t.setup, "//pkg:ext2b.scl");
}

// Java: BzlLoadFunctionTest.testSclCannotLoadNonSclFiles
#[test]
fn test_scl_cannot_load_non_scl_files() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext1a.bzl", &["a = 1"]);
    t.scratch_file("pkg/ext1a.garbage", &["a = 1"]);
    // Cannot use relative label.
    t.scratch_file("pkg/ext2a.scl", &["load('//pkg:ext1a.bzl', 'a')"]);
    t.scratch_file("pkg/ext2b.scl", &["load('//pkg:ext1b.garbage', 'a')"]);

    let events = check_failing_lookup(&t.setup, "//pkg:ext2a.scl", "has invalid load statements");
    assert_contains_event(
        &events,
        "The label must reference a file with extension \".scl\" (.scl files cannot load .bzl \
         files)",
    );
    let events = check_failing_lookup(&t.setup, "//pkg:ext2b.scl", "has invalid load statements");
    assert_contains_event(
        &events,
        "The label must reference a file with extension \".scl\"",
    );
    assert_does_not_contain_event(&events, ".bzl");
}

// Java: BzlLoadFunctionTest.testSclCanOnlyLoadLabelsRelativeToDefaultRepoRoot
#[test]
fn test_scl_can_only_load_labels_relative_to_default_repo_root() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext1.scl", &["load(':foo.scl', 'a')"]);
    t.scratch_file("pkg/ext2.scl", &["load('@repo//:foo.scl', 'a')"]);

    let events = check_failing_lookup(&t.setup, "//pkg:ext1.scl", "has invalid load statements");
    assert_contains_event(&events, "in .scl files, load labels must begin with \"//\"");
    let events = check_failing_lookup(&t.setup, "//pkg:ext2.scl", "has invalid load statements");
    assert_contains_event(&events, "in .scl files, load labels must begin with \"//\"");
}

// Java: BzlLoadFunctionTest.testSclSupportsStructAndVisibility
#[test]
fn test_scl_supports_struct_and_visibility() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext1.scl", &["visibility('private')", "a = struct()"]);
    t.scratch_file("pkg/ext2.scl", &["load('//pkg:ext1.scl', 'a')"]);
    t.scratch_file("pkg2/BUILD", &[]);
    t.scratch_file("pkg2/ext3.scl", &["load('//pkg:ext1.scl', 'a')"]);

    check_successful_lookup(&t.setup, "//pkg:ext2.scl");
    check_failing_lookup(
        &t.setup,
        "//pkg2:ext3.scl",
        "module //pkg2:ext3.scl contains .bzl load visibility violations",
    );
}

fn scl_does_not_support_other_bazel_symbols_events() -> Vec<Event> {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/ext.scl", &["a = depset([])"]);

    check_failing_lookup(
        &t.setup,
        "//pkg:ext.scl",
        "compilation of module 'pkg/ext.scl' failed",
    )
}

// Java: BzlLoadFunctionTest.testSclDoesNotSupportOtherBazelSymbols
// Holds the Java test without its last line, `assertContainsEvent("name 'depset' is not
// defined")` (the interpreter's text), which is in
// `test_scl_does_not_support_other_bazel_symbols_interpreter_text`.
#[test]
fn test_scl_does_not_support_other_bazel_symbols() {
    scl_does_not_support_other_bazel_symbols_events();
}

// Java: BzlLoadFunctionTest.testSclDoesNotSupportOtherBazelSymbols (its last line,
// BzlLoadFunctionTest.java:333 `assertContainsEvent("name 'depset' is not defined")`).
#[test]
fn test_scl_does_not_support_other_bazel_symbols_interpreter_text() {
    let events = scl_does_not_support_other_bazel_symbols_events();
    assert_contains_event(&events, "name 'depset' is not defined");
}

// Java: BzlLoadFunctionTest.testSclDisallowsNonAsciiStringLiterals
// Java's `scratch.file` writes ISO-8859-1 (Scratch.DEFAULT_CHARSET), so `'x\377z'` is the bytes
// `'x`, 0xFF, `z'`.
#[test]
fn test_scl_disallows_non_ascii_string_literals() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    t.scratch_file("pkg/BUILD", &[]);
    t.setup
        .scratch
        .file_bytes(&t.setup.ws("pkg/ext1.bzl"), b"'x\xffz'\n"); // xÿz
    t.setup
        .scratch
        .file_bytes(&t.setup.ws("pkg/ext2.scl"), b"'x\xffz'\n");

    check_successful_lookup(&t.setup, "//pkg:ext1.bzl");
    let events = check_failing_lookup(
        &t.setup,
        "//pkg:ext2.scl",
        "compilation of module 'pkg/ext2.scl' failed",
    );
    assert_contains_event(&events, "string literal contains non-ASCII character");
}

// Java: BzlLoadFunctionTest.testBzlLoadNoBuildFile
#[test]
fn test_bzl_load_no_build_file() {
    let s = setup();
    file(&s, "pkg/ext.bzl", "");
    check_failing_lookup(
        &s,
        "//pkg:ext.bzl",
        "Every .bzl file must have a corresponding package, but '//pkg:ext.bzl' does not",
    );
}

// Java: BzlLoadFunctionTest.testBzlLoadNoBuildFileForLoad
#[test]
fn test_bzl_load_no_build_file_for_load() {
    let s = setup();
    file(&s, "pkg2/BUILD", "");
    file(&s, "pkg1/ext.bzl", "a = 1");
    file(&s, "pkg2/ext.bzl", "load('//pkg1:ext.bzl', 'a')");
    check_failing_lookup(
        &s,
        "//pkg:ext.bzl",
        "Every .bzl file must have a corresponding package",
    );
}

// Java: BzlLoadFunctionTest.testBzlLoadFilenameWithControlChars
// Java expects `evaluate` to throw the AssertionError of FoundationTestCase.failFastHandler
// (`fail(event.toString())` on the first ERROR event) and asserts on its message; here the
// ERROR event is asserted.
#[test]
fn test_bzl_load_filename_with_control_chars() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    file(&s, "pkg/ext.bzl", "load('//pkg:oops\u{0}.bzl', 'a')");
    let (_, events) = eval_key(&s, &key("//pkg:ext.bzl"));
    let errors: Vec<String> = events
        .iter()
        .filter(|e| EventKind::ERRORS.contains(e.get_kind()))
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.iter().any(|e| e.contains(
            "invalid target name 'oops<?>.bzl': target names may not contain non-printable \
             characters: '\\x00'"
        )),
        "{errors:#?}"
    );
}

// Java: BzlLoadFunctionTest.testLoadFromSubdirInSamePackageIsOk
#[test]
fn test_load_from_subdir_in_same_package_is_ok() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('//a:b/b.bzl', 'b')");
    file(&s, "a/b/b.bzl", "b = 42");
    check_successful_lookup(&s, "//a:a.bzl");
}

// Java: BzlLoadFunctionTest.testLoadMustRespectPackageBoundary_ofSubpkg
#[test]
fn test_load_must_respect_package_boundary_of_subpkg() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('//a:b/b.bzl', 'b')");
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/b.bzl", "b = 42");
    check_failing_lookup(
        &s,
        "//a:a.bzl",
        "Label '//a:b/b.bzl' is invalid because 'a/b' is a subpackage; perhaps you meant to put \
         the colon here: '//a/b:b.bzl'?",
    );
}

// Java: BzlLoadFunctionTest.testLoadMustRespectPackageBoundary_ofSubpkg_relative
#[test]
fn test_load_must_respect_package_boundary_of_subpkg_relative() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('b/b.bzl', 'b')");
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/b.bzl", "b = 42");
    check_failing_lookup(
        &s,
        "//a:a.bzl",
        "Label '//a:b/b.bzl' is invalid because 'a/b' is a subpackage; perhaps you meant to put \
         the colon here: '//a/b:b.bzl'?",
    );
}

// Java: BzlLoadFunctionTest.testLoadMustRespectPackageBoundary_ofIndirectSubpkg
#[test]
fn test_load_must_respect_package_boundary_of_indirect_subpkg() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a.bzl", "load('//a/b:c/c.bzl', 'c')");
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/c/BUILD", "");
    file(&s, "a/b/c/c.bzl", "c = 42");
    check_failing_lookup(
        &s,
        "//a:a.bzl",
        "Label '//a/b:c/c.bzl' is invalid because 'a/b/c' is a subpackage; perhaps you meant to \
         put the colon here: '//a/b/c:c.bzl'?",
    );
}

// Java: BzlLoadFunctionTest.testLoadMustRespectPackageBoundary_ofParentPkg
#[test]
fn test_load_must_respect_package_boundary_of_parent_pkg() {
    let s = setup();
    file(&s, "a/b/BUILD", "");
    file(&s, "a/b/b.bzl", "load('//a/c:c/c.bzl', 'c')");
    file(&s, "a/BUILD", "");
    file(&s, "a/c/c/c.bzl", "c = 42");
    check_failing_lookup(
        &s,
        "//a/b:b.bzl",
        "Label '//a/c:c/c.bzl' is invalid because 'a/c' is not a package; perhaps you meant to \
         put the colon here: '//a:c/c/c.bzl'?",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_disabledWithoutFlag
#[test]
fn test_bzl_visibility_disabled_without_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=false"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility(\"private\")", "x = 1"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'b/bar.bzl' failed",
    );
    assert_contains_event(
        &events,
        "Use of `visibility()` requires --experimental_bzl_visibility",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_publicExplicit
#[test]
fn test_bzl_visibility_public_explicit() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility(\"public\")", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a:foo.bzl");
    assert_no_events(&events);
}

// Java: BzlLoadFunctionTest.testBzlVisibility_publicImplicit
#[test]
fn test_bzl_visibility_public_implicit() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file(
        "b/bar.bzl",
        // No visibility() declaration, defaults to public.
        &["x = 1"],
    );

    let events = check_successful_lookup(&t.setup, "//a:foo.bzl");
    assert_no_events(&events);
}

// Java: BzlLoadFunctionTest.testBzlVisibility_privateSamePackage
#[test]
fn test_bzl_visibility_private_same_package() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//a:bar.bzl\", \"x\")"]);
    t.scratch_file("a/bar.bzl", &["visibility(\"private\")", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a:foo.bzl");
    assert_no_events(&events);
}

// Java: BzlLoadFunctionTest.testBzlVisibility_privateDifferentPackage
#[test]
fn test_bzl_visibility_private_different_package() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility(\"private\")", "x = 1"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "module //a:foo.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_emptyListMeansPrivate
#[test]
fn test_bzl_visibility_empty_list_means_private() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility([])", "x = 1"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "module //a:foo.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_publicListElement
#[test]
fn test_bzl_visibility_public_list_element() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file(
        "b/bar.bzl",
        &[
            "# Tests \"public\" as a list item, and alongside other list items.",
            "visibility([\"public\", \"//c\"])",
            "x = 1",
        ],
    );

    let events = check_successful_lookup(&t.setup, "//a:foo.bzl");
    assert_no_events(&events);
}

// Java: BzlLoadFunctionTest.testBzlVisibility_privateListElement
#[test]
fn test_bzl_visibility_private_list_element() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a1/BUILD", &[]);
    t.scratch_file("a1/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("a2/BUILD", &[]);
    t.scratch_file("a2/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file(
        "b/bar.bzl",
        &[
            "# Tests \"private\" as a list item, and alongside other list items.",
            "visibility([\"private\", \"//a1\"])",
            "x = 1",
        ],
    );

    let events = check_successful_lookup(&t.setup, "//a1:foo.bzl");
    assert_no_events(&events);
    let events = check_failing_lookup(
        &t.setup,
        "//a2:foo.bzl",
        "module //a2:foo.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a2.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_failureInDependency
#[test]
fn test_bzl_visibility_failure_in_dependency() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file(
        "b/bar.bzl",
        &[
            "load(\"//c:baz.bzl\", \"y\")",
            "",
            "visibility(\"public\")",
            "x = y",
        ],
    );
    t.scratch_file("c/BUILD", &[]);
    t.scratch_file("c/baz.bzl", &["visibility(\"private\")", "y = 1"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        // Java: "at /workspace/a/foo.bzl:1:6: module //b:bar.bzl contains .bzl load visibility
        // violations"
        &format!(
            "at {}:1:6: module //b:bar.bzl contains .bzl load visibility violations",
            t.workspace_path("a/foo.bzl")
        ),
    );
    assert_contains_event(
        &events,
        "Starlark file //c:baz.bzl is not visible for loading from package //b.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_cannotBeSetInFunction
#[test]
fn test_bzl_visibility_cannot_be_set_in_function() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file(
        "a/foo.bzl",
        &[
            "def helper():",
            "    visibility(\"public\")",
            "",
            "helper()",
        ],
    );

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'a/foo.bzl' failed",
    );
    assert_contains_event(&events, "load visibility may only be set at the top level");
}

// Java: BzlLoadFunctionTest.testBzlVisibility_cannotBeSetTwice
#[test]
fn test_bzl_visibility_cannot_be_set_twice() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file(
        "a/foo.bzl",
        &["visibility(\"public\")", "visibility(\"public\")"],
    );

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'a/foo.bzl' failed",
    );
    assert_contains_event(&events, "load visibility may not be set more than once");
}

// Java: BzlLoadFunctionTest.testBzlVisibility_enumeratedPackages
#[test]
fn test_bzl_visibility_enumerated_packages() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a1/BUILD", &[]);
    t.scratch_file("a1/foo1.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("a2/BUILD", &[]);
    t.scratch_file("a2/foo2.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility([\"//a1\"])", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a1:foo1.bzl");
    assert_no_events(&events);

    let events = check_failing_lookup(
        &t.setup,
        "//a2:foo2.bzl",
        "module //a2:foo2.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a2.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_singleEnumeratedPackageAsString
#[test]
fn test_bzl_visibility_single_enumerated_package_as_string() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a1/BUILD", &[]);
    t.scratch_file("a1/foo1.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("a2/BUILD", &[]);
    t.scratch_file("a2/foo2.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file(
        "b/bar.bzl",
        &[
            "# Note: \"//a1\", not [\"//a1\"]",
            "visibility(\"//a1\")",
            "x = 1",
        ],
    );

    let events = check_successful_lookup(&t.setup, "//a1:foo1.bzl");
    assert_no_events(&events);

    let events = check_failing_lookup(
        &t.setup,
        "//a2:foo2.bzl",
        "module //a2:foo2.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a2.",
    );
}

/// The repository set of the two tests whose Java MODULE.bazel is `bazel_dep(name = 'repo')` +
/// `local_path_override(module_name = 'repo', path = 'repo')`: module `repo` is `@@repo+`, in
/// `<workspace>/repo`, visible from the main repository as `@repo`.
fn set_local_path_override_of_repo(t: &mut PackageLoadingTestCase) {
    let repo_directory = t.workspace_path("repo");
    set_external_repos(
        &mut t.setup,
        &[
            Repo {
                name: "",
                module_name: "",
                mapping: &[("", ""), ("repo", "repo+"), ("bazel_tools", "bazel_tools")],
                directory: None,
            },
            Repo {
                name: "repo+",
                module_name: "repo",
                mapping: &[("repo", "repo+"), ("bazel_tools", "bazel_tools")],
                directory: Some(repo_directory),
            },
            BAZEL_TOOLS_REPO,
        ],
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_enumeratedPackagesMultipleRepos
#[test]
fn test_bzl_visibility_enumerated_packages_multiple_repos() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    // @repo//pkg:foo1.bzl and @//pkg:foo2.bzl both try to access @repo//lib:bar.bzl. Test that
    // when bar.bzl declares a visibility allowing "//pkg", it means @repo//pkg and *not* @//pkg.
    t.scratch_file("repo/MODULE.bazel", &["module(name = 'repo')"]);
    t.scratch_file("repo/pkg/BUILD", &[]);
    t.scratch_file("repo/pkg/foo1.bzl", &["load(\"//lib:bar.bzl\", \"x\")"]);
    t.scratch_file("repo/lib/BUILD", &[]);
    t.scratch_file("repo/lib/bar.bzl", &["visibility([\"//pkg\"])", "x = 1"]);
    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/foo2.bzl", &["load(\"@repo//lib:bar.bzl\", \"x\")"]);
    set_local_path_override_of_repo(&mut t);

    let events = check_successful_lookup(&t.setup, "@@repo+//pkg:foo1.bzl");
    assert_no_events(&events);

    let events = check_failing_lookup(
        &t.setup,
        "//pkg:foo2.bzl",
        "module //pkg:foo2.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file @@repo+//lib:bar.bzl is not visible for loading from package //pkg.",
    );
}

// TODO(#16365): This test case can be deleted once --incompatible_package_group_has_public_syntax
// is deleted (not just flipped).
// Java: BzlLoadFunctionTest.testBzlVisibility_canUsePublicPrivate_regardlessOfFlag
#[test]
fn test_bzl_visibility_can_use_public_private_regardless_of_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--experimental_bzl_visibility=true",
        // Test that we can use "public" and "private" visibility for .bzl files even when the
        // incompatible flag is disabled.
        "--incompatible_package_group_has_public_syntax=false",
    ]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo1.bzl", &["visibility(\"public\")"]);
    t.scratch_file("a/foo2.bzl", &["visibility(\"private\")"]);

    let events1 = check_successful_lookup(&t.setup, "//a:foo1.bzl");
    let events2 = check_successful_lookup(&t.setup, "//a:foo2.bzl");
    assert_no_events(&events1);
    assert_no_events(&events2);
}

// TODO(#16324): Once --incompatible_fix_package_group_reporoot_syntax is deleted (not just
// flipped), this test case will be redundant with tests for //... in PackageGroupTest. At that
// point we'll just delete this test case.
// Java: BzlLoadFunctionTest.testBzlVisibility_repoRootSubpackagesIsNotPublic_regardlessOfFlag
#[test]
fn test_bzl_visibility_repo_root_subpackages_is_not_public_regardless_of_flag() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--experimental_bzl_visibility=true",
        // Test that we get the fixed behavior even when the incompatible flag is disabled.
        "--incompatible_fix_package_group_reporoot_syntax=false",
    ]);

    t.scratch_file("repo/MODULE.bazel", &["module(name = 'repo')"]);
    t.scratch_file("repo/a/BUILD", &[]);
    t.scratch_file("repo/a/foo.bzl", &["load(\"@@//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility([\"//...\"])", "x = 1"]);
    set_local_path_override_of_repo(&mut t);

    let events = check_failing_lookup(
        &t.setup,
        "@@repo+//a:foo.bzl",
        "module @@repo+//a:foo.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package @@repo+//a.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_disallowsSubpackagesWithoutWildcard
#[test]
fn test_bzl_visibility_disallows_subpackages_without_wildcard() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo1.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("a/subpkg/BUILD", &[]);
    t.scratch_file("a/subpkg/foo2.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility([\"//a\"])", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a:foo1.bzl");
    assert_no_events(&events);

    let events = check_failing_lookup(
        &t.setup,
        "//a/subpkg:foo2.bzl",
        "module //a/subpkg:foo2.bzl contains .bzl load visibility violations",
    );
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a/subpkg.",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_allowsSubpackagesWithWildcard
#[test]
fn test_bzl_visibility_allows_subpackages_with_wildcard() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo1.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("a/subpkg/BUILD", &[]);
    t.scratch_file("a/subpkg/foo2.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility([\"//a/...\"])", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a:foo1.bzl");
    assert_no_events(&events);

    let events = check_successful_lookup(&t.setup, "//a/subpkg:foo2.bzl");
    assert_no_events(&events);
}

// Java: BzlLoadFunctionTest.testBzlVisibility_invalid_badType
#[test]
fn test_bzl_visibility_invalid_bad_type() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["visibility(123)"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'a/foo.bzl' failed",
    );
    assert_contains_event(
        &events,
        "Invalid visibility: got 'int', want string or list of strings",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_invalid_badElementType
#[test]
fn test_bzl_visibility_invalid_bad_element_type() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["visibility([\"//a\", 123])"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'a/foo.bzl' failed",
    );
    assert_contains_event(
        &events,
        "at index 1 of visibility list, got element of type int, want string",
    );
}

// Java: BzlLoadFunctionTest.testBzlVisibility_packageWithRepoWorks
#[test]
fn test_bzl_visibility_package_with_repo_works() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["visibility([\"@repo//b\"])"]);

    check_successful_lookup(&t.setup, "//a:foo.bzl");
}

// Java: BzlLoadFunctionTest.testBzlVisibility_invalid_negationNotSupported
#[test]
fn test_bzl_visibility_invalid_negation_not_supported() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["visibility([\"-//a\"])"]);

    let events = check_failing_lookup(
        &t.setup,
        "//a:foo.bzl",
        "initialization of module 'a/foo.bzl' failed",
    );
    assert_contains_event(&events, "Cannot use negative package patterns here");
}

// Java: BzlLoadFunctionTest.testBzlVisibility_errorsDemotedToWarningWhenBreakGlassFlagIsSet
#[test]
fn test_bzl_visibility_errors_demoted_to_warning_when_break_glass_flag_is_set() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--experimental_bzl_visibility=true",
        "--check_bzl_visibility=false",
    ]);

    t.scratch_file("a/BUILD", &[]);
    t.scratch_file("a/foo.bzl", &["load(\"//b:bar.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/bar.bzl", &["visibility(\"private\")", "x = 1"]);

    let events = check_successful_lookup(&t.setup, "//a:foo.bzl");
    assert_contains_event(
        &events,
        "Starlark file //b:bar.bzl is not visible for loading from package //a.",
    );
    assert_contains_event(
        &events,
        "Continuing because --nocheck_bzl_visibility is active",
    );
}

// Java: BzlLoadFunctionTest.testLoadFromNonExistentRepository_producesMeaningfulError
#[test]
fn test_load_from_non_existent_repository_produces_meaningful_error() {
    let s = setup();
    file(&s, "BUILD", "load(\"@repository//dir:file.bzl\", \"foo\")");
    check_failing_lookup(
        &s,
        "@repository//dir:file.bzl",
        "Unable to find package for @@repository//dir:file.bzl: The repository '@@repository' \
         could not be resolved: Repository '@@repository' is not defined.",
    );
}

// Java: BzlLoadFunctionTest.testLoadBzlFileFromBzlmod
// Java's registry (modules foo@1.0 and bar@2.0, foo depending on bar as `bar_alias`) and module
// root become the repository set below; the module directories are siblings of the workspace
// in the scratch directory.
#[test]
fn test_load_bzl_file_from_bzlmod() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_enable_scl_dialect"]);
    let foo_dir = t.setup.scratch.dir("modules/foo+1.0");
    t.setup.scratch.file("modules/foo+1.0/REPO.bazel", "");
    t.setup.scratch.file("modules/foo+1.0/BUILD", "");
    t.setup.scratch.file(
        "modules/foo+1.0/test.bzl",
        // Also test that bzlmod .bzl files can load .scl files.
        "load('@bar_alias//:test.scl', 'haha')\nl = Label('@foo//:whatever')\nhoho = haha\n",
    );
    let bar_dir = t.setup.scratch.dir("modules/bar+2.0");
    t.setup.scratch.file("modules/bar+2.0/REPO.bazel", "");
    t.setup.scratch.file("modules/bar+2.0/BUILD", "");
    t.setup
        .scratch
        .file("modules/bar+2.0/test.scl", "haha = 5\n");
    set_external_repos(
        &mut t.setup,
        &[
            Repo {
                name: "",
                module_name: "",
                mapping: &[("", ""), ("foo", "foo+"), ("bazel_tools", "bazel_tools")],
                directory: None,
            },
            Repo {
                name: "foo+",
                module_name: "foo",
                mapping: &[
                    ("foo", "foo+"),
                    ("bar_alias", "bar+"),
                    ("bazel_tools", "bazel_tools"),
                ],
                directory: Some(foo_dir.get_path_string().to_string()),
            },
            Repo {
                name: "bar+",
                module_name: "bar",
                mapping: &[("bar", "bar+"), ("bazel_tools", "bazel_tools")],
                directory: Some(bar_dir.get_path_string().to_string()),
            },
            BAZEL_TOOLS_REPO,
        ],
    );

    let sky_key = BzlLoadKey::key_for_bzlmod(Label::parse_canonical_unchecked("@@foo+//:test.bzl"));
    let (result, events) = eval_key(&t.setup, &sky_key);

    assert!(
        !result.has_error(),
        "{:?}\nevents: {:#?}",
        result,
        event_strings(&events)
    );
    let bzl_load_value = result.get(&sky_key).unwrap();
    let hoho = bzl_load_value
        .get_module()
        .get_option("hoho")
        .unwrap()
        .expect("the global 'hoho'");
    assert_eq!(hoho.unpack_i32(), Some(5));
    assert_eq!(
        bzl_load_value.get_recorded_repo_mappings().to_vec(),
        vec![
            (
                RepositoryName::create_unvalidated("foo+"),
                "bar_alias".to_owned(),
                RepositoryName::create_unvalidated("bar+")
            ),
            (
                RepositoryName::create_unvalidated("foo+"),
                "foo".to_owned(),
                RepositoryName::create_unvalidated("foo+")
            ),
        ]
    );
    // Note that we're not testing the case of a non-registry override using @bazel_tools here,
    // but that is incredibly hard to set up in a unit test. So we should just rely on
    // integration tests for that.
}

/// The files and the lookup of BzlLoadFunctionTest.testBuiltinsInjectionFailure: the harness,
/// the result of `//pkg:foo.bzl` and its events.
fn builtins_injection_failure() -> (PackageLoadingTestCase, EvaluationResult, Vec<Event>) {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_builtins_bzl_path=tools/builtins_staging"]);
    t.scratch_file(
        "tools/builtins_staging/exports.bzl",
        &[
            "1 // 0  # <-- dynamic error",
            "exported_toplevels = {}",
            "exported_rules = {}",
            "exported_to_java = {}",
        ],
    );
    t.scratch_file("pkg/BUILD", &[]);
    t.scratch_file("pkg/foo.bzl", &[]);

    let (result, events) = eval_key(&t.setup, &key("//pkg:foo.bzl"));
    (t, result, events)
}

// Java: BzlLoadFunctionTest.testBuiltinsInjectionFailure
// Holds the Java test without `assertContainsEvent("Error: integer division by zero")` (the
// interpreter's text), which is in `test_builtins_injection_failure_interpreter_text`.
#[test]
fn test_builtins_injection_failure() {
    let (t, result, events) = builtins_injection_failure();

    // Java: "File \"/workspace/tools/builtins_staging/exports.bzl\", line 1, column 3, in
    // <toplevel>"
    assert_contains_event(
        &events,
        &format!(
            "File \"{}\", line 1, column 3, in <toplevel>",
            t.workspace_path("tools/builtins_staging/exports.bzl")
        ),
    );
    let ex = result
        .get_error(&SkyKey::new(key("//pkg:foo.bzl")))
        .expect("error entry")
        .get_exception()
        .expect("an exception")
        .to_string();
    assert!(
        ex.contains(
            "Internal error while loading Starlark builtins for //pkg:foo.bzl: Failed to load \
             builtins sources: initialization of module 'exports.bzl' (internal) failed"
        ),
        "{ex}"
    );
}

// Java: BzlLoadFunctionTest.testBuiltinsInjectionFailure (BzlLoadFunctionTest.java:1116
// `assertContainsEvent("Error: integer division by zero")`).
#[test]
fn test_builtins_injection_failure_interpreter_text() {
    let (_t, _result, events) = builtins_injection_failure();
    assert_contains_event(&events, "Error: integer division by zero");
}

// Java: BzlLoadFunctionTest.testErrorReadingBzlFileIsTransientWhenUsingASTInlining
// Java's CustomInMemoryFs throws an IOException from `getInputStream` of the file; on the real
// file system of the harness the file has no read permission.
#[test]
fn test_error_reading_bzl_file_is_transient_when_using_ast_inlining() {
    let s = setup();
    file(&s, "a/BUILD", "");
    make_unreadable(&s.scratch.file(&s.ws("a/a1.bzl"), "doesntmatter"));

    let k = key("//a:a1.bzl");
    let (result, events) = eval_key(&s, &k);
    let error = result
        .get_error(&SkyKey::new(k))
        .unwrap_or_else(|| panic!("no error entry; events: {:#?}", event_strings(&events)));
    assert!(error.is_transitively_transient(), "{error:?}");
}

// Java: BzlLoadFunctionTest.testErrorReadingOtherBzlFileIsPersistentFromPerspectiveOfParent
// (the read failure as in the test above).
#[test]
fn test_error_reading_other_bzl_file_is_persistent_from_perspective_of_parent() {
    let s = setup();
    file(&s, "a/BUILD", "");
    file(&s, "a/a1.bzl", "load('//a:a2.bzl', 'a2')");
    make_unreadable(&s.scratch.file(&s.ws("a/a2.bzl"), "doesntmatter"));

    let k = key("//a:a1.bzl");
    let (result, events) = eval_key(&s, &k);
    let error = result
        .get_error(&SkyKey::new(k))
        .unwrap_or_else(|| panic!("no error entry; events: {:#?}", event_strings(&events)));
    assert!(!error.is_transitively_transient(), "{error:?}");
}

// Java: BzlLoadFunctionTest.testErrorStatingBzlFileInFileStateFunctionIsPersistent
//   CustomInMemoryFs fs = (CustomInMemoryFs) fileSystem;
//   scratch.file("a/BUILD");
//   fs.badPathForStat = scratch.file("a/a1.bzl", "doesntmatter");
//   SkyKey key = key("//a:a1.bzl");
//   ... evaluate(key) ...
//   assertThatEvaluationResult(result).hasErrorEntryForKeyThat(key).isNotTransient();
#[test]
#[ignore = "needs a file system whose statIfFound throws an IOException for one path (Java CustomInMemoryFs.badPathForStat); the harness runs on the real UnixFileSystem, where no permission makes the stat of a/a1.bzl fail without also failing the stat of a/BUILD"]
fn test_error_stating_bzl_file_in_file_state_function_is_persistent() {}

// Java: BzlLoadFunctionTest.bzlCompileCacheHitFromNodeWithOtherKeyKind_registersFileDep
//   Regression test for https://github.com/bazelbuild/bazel/issues/30900: evaluates
//   keyForBuild(//pkg:foo.bzl) on a thread that blocks in the stat of pkg/bar.bzl
//   (CustomInMemoryFs.pathToBlockOnStat), interrupts it, invalidates pkg/bar.bzl, evaluates
//   keyForBzlmod(//pkg:foo.bzl) on the same SkyframeExecutor (y == 1), overwrites foo.bzl,
//   invalidates it and evaluates again (y == 2).
#[test]
#[ignore = "needs a file system that blocks one stat (Java CustomInMemoryFs.pathToBlockOnStat), an interruptible evaluation, and invalidation with re-evaluation on the same evaluator (invalidateFilesUnderPathForTesting); the harness has the real file system and a fresh evaluator per lookup, and BzlLoadFunction's bzlCompileCache (the subject of the test) only matters across evaluations"]
fn bzl_compile_cache_hit_from_node_with_other_key_kind_registers_file_dep() {}

// Not a Java test: the transitive digest is Java's Fingerprint of the file's digest followed by
// the transitive digests of its loads, in load order (no builtins digest with injection off).
#[test]
fn transitive_digest_matches_java_formula() {
    let s = setup();
    file(&s, "pkg/BUILD", "");
    let b = "b = 1\n";
    let a = "load(':b.bzl', 'b')\na = b\n";
    file(&s, "pkg/b.bzl", b);
    file(&s, "pkg/a.bzl", a);
    let evaluator = s.evaluator();
    let k = key("//pkg:a.bzl");
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(k.clone())],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
    let sha = |c: &str| {
        DigestHashFunction::Sha256
            .hash_reader(&mut c.as_bytes())
            .unwrap()
    };
    let digest_b = Fingerprint::new().add_bytes(&sha(b)).digest_and_reset();
    let digest_a = Fingerprint::new()
        .add_bytes(&sha(a))
        .add_bytes(&digest_b)
        .digest_and_reset();
    assert_eq!(result.get(&k).unwrap().get_transitive_digest(), digest_a);
    assert_eq!(
        format!("{:?}", k),
        "KeyForBuild{label=//pkg:a.bzl, isBuildPrelude=false}"
    );
}

// Not a Java test.
#[test]
#[ignore = "TestSetup::new's minimal Starlark environment (blaze_starlark's builtins only) has no rule-definition builtins (provider(), rule(), ...), which the bundled @_builtins .bzl files call"]
fn builtins_exports_evaluate() {
    let s = TestSetup::new(FakeRepos::default());
    let evaluator = s.evaluator();
    let result = evaluate(
        &evaluator,
        vec![
            SkyKey::new(exports_entrypoint_key()),
            SkyKey::new(StarlarkBuiltinsKey::key(false)),
        ],
        false,
        Arc::new(StoredEventHandler::new()),
    );
    assert!(!result.has_error(), "{:?}", result);
}
