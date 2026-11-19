//! Port of PackageFunctionTest (the monolithic-package `ComputationMode`).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/PackageFunctionTest.java. Builtins
//! injection is disabled (`--experimental_builtins_bzl_path=`), so BUILD files see the native
//! rules directly.
//!
//! Differences from the Java test environment that the ports below carry:
//! - `PreludeTest` sets the prelude to `//tools/test_build_rules:test_prelude`; the rule class
//!   provider here is Bazel's, whose prelude is `//tools/build_rules:prelude_bazel`. Only that
//!   path is substituted.
//! - Java's workspace is `/workspace`; here it is a temp directory, so expected texts that name
//!   a workspace path are built with `PackageLoadingTestCase::workspace_path`.
//! - Every evaluation uses a fresh evaluator. A Java test that changes files, invalidates them
//!   and loads again is ported as loads from scratch: its assertions on each load are kept, the
//!   Skyframe invalidation itself is not exercised (tests whose assertion is about invalidation,
//!   for example "same instance after an irrelevant change", stay ignored).
//! - `assertDetailedExitCode(ex, code, exitCode)`: the `PackageLoading.Code` is asserted, the
//!   process `ExitCode` is not (it is not derived in this crate).

use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_events::{Event, StoredEventHandler};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchPackageKind, PackageLoadingCode};
use blaze_packages::package::Package;
use blaze_skyframe::{EvaluationResult, SkyKey};

use crate::package_function::PackageKey;
use crate::test_util::{FakeRepos, TestSetup, evaluate};
use crate::tests::package_loading_test_case::{
    PackageLoadingTestCase, assert_contains_event, assert_does_not_contain_event,
};

fn setup() -> TestSetup {
    TestSetup::with_bazel_rules(FakeRepos::default())
}

fn file(setup: &TestSetup, rel: &str, contents: &str) {
    setup.scratch.file(&setup.ws(rel), contents);
}

fn key(pkg: &str) -> PackageKey {
    PackageKey::key(PackageIdentifier::create_in_main_repo_str(pkg))
}

fn eval(setup: &TestSetup, pkg: &str) -> (EvaluationResult, Arc<StoredEventHandler>) {
    let evaluator = setup.evaluator();
    let events = Arc::new(StoredEventHandler::new());
    let result = evaluate(
        &evaluator,
        vec![SkyKey::new(key(pkg))],
        false,
        events.clone(),
    );
    (result, events)
}

// Java: PackageFunctionTest.validPackageoid
fn valid_package(setup: &TestSetup, pkg: &str) -> (Arc<Package>, Arc<StoredEventHandler>) {
    let (result, events) = eval(setup, pkg);
    assert!(!result.has_error(), "{:?}", result);
    (result.get(&key(pkg)).unwrap().get_package().clone(), events)
}

// Java: PackageFunctionTest.validPackageoidWithoutErrors
fn valid_package_without_errors(setup: &TestSetup, pkg: &str) -> Arc<Package> {
    let (pkg, events) = valid_package(setup, pkg);
    assert!(!pkg.contains_errors(), "{:?}", events.get_events());
    pkg
}

// Java: PackageFunctionTest.evaluatePackageoidToException
fn evaluate_package_to_exception(setup: &TestSetup, pkg: &str) -> NoSuchPackageException {
    let (result, _) = eval(setup, pkg);
    let error = result.get_error(&SkyKey::new(key(pkg))).expect("an error");
    error
        .exception_as::<NoSuchPackageException>()
        .expect("a NoSuchPackageException")
        .clone()
}

fn messages(events: &StoredEventHandler) -> Vec<String> {
    events
        .get_events()
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect()
}

#[test]
fn test_valid_package() {
    let s = setup();
    file(&s, "pkg/BUILD", "filegroup(name = 'foo')");
    let pkg = valid_package_without_errors(&s, "pkg");
    assert!(pkg.get_targets().contains_key("foo"));
}

#[test]
fn test_load_ok() {
    let s = setup();
    file(&s, "p/a.bzl", "a = 1; b = 1; d = 1");
    file(&s, "p/subdir/a.bzl", "c = 1; e = 1");
    file(
        &s,
        "p/BUILD",
        "load(\"//p:a.bzl\", \"d\")\n\
         load(\"//p:subdir/a.bzl\", \"e\")\n\
         load(\":a.bzl\", \"a\")\n\
         load(\"a.bzl\", \"b\")\n\
         load(\"subdir/a.bzl\", \"c\")\n",
    );
    valid_package_without_errors(&s, "p");
}

#[test]
fn test_load_bad_label() {
    let s = setup();
    file(&s, "p/BUILD", "load('this\tis not a label', 'a')");
    let (_, events) = eval(&s, "p");
    let messages = messages(&events);
    assert!(
        messages.iter().any(|m| m.contains(
            "in load statement: invalid target name 'this<?>is not a label': target names may \
             not contain non-printable characters"
        )),
        "{:?}",
        messages
    );
}

#[test]
fn test_recursive_glob_never_matches_package_directory() {
    let s = setup();
    file(
        &s,
        "foo/BUILD",
        "[filegroup(name = x + '-matched') for x in glob(['**'], exclude_directories = 0)]",
    );
    file(&s, "foo/bar", "");
    let pkg = valid_package_without_errors(&s, "foo");
    assert_eq!(
        pkg.get_target("bar-matched").unwrap().get_name(),
        "bar-matched"
    );
    assert!(pkg.get_target("-matched").is_err());
}

#[test]
fn test_package_loading_error_on_missing_build_file_single_package_path() {
    let s = setup();
    file(&s, "foo/bar", "");
    // There is no foo/BUILD file, but we enforce loading package 'foo'.
    let ex = evaluate_package_to_exception(&s, "foo");
    assert!(
        ex.get_message().contains(
            "BUILD file not found in any of the following directories. Add a BUILD file to a \
             directory to mark it as a package.\n - foo"
        ),
        "{}",
        ex.get_message()
    );
    assert_eq!(ex.kind(), NoSuchPackageKind::BuildFileNotFound);
    assert_eq!(
        ex.get_detailed_exit_code().code,
        blaze_packages::exceptions::FailureCode::PackageLoading(
            PackageLoadingCode::BuildFileMissing
        )
    );
}

#[test]
fn test_labels_crosses_subpackage_boundaries_single_subpackage_crossing() {
    let s = setup();
    file(&s, "pkg/foo/BUILD", "exports_files(['sub/bar/blah'])");
    file(&s, "pkg/foo/sub/BUILD", "");
    let (pkg, events) = valid_package(&s, "pkg/foo");
    assert!(pkg.contains_errors());
    let messages = messages(&events);
    assert!(
        messages.iter().any(|m| m.contains(
            "Label '//pkg/foo:sub/bar/blah' is invalid because 'pkg/foo/sub' is a subpackage; \
             perhaps you meant to put the colon here: '//pkg/foo/sub:bar/blah'?"
        )),
        "{:?}",
        messages
    );
}

#[test]
fn test_glob_allow_empty_function_param() {
    let s = setup();
    file(&s, "pkg/BUILD", "x = glob(['*.foo'], allow_empty=True)");
    let (pkg, events) = valid_package(&s, "pkg");
    assert!(!pkg.contains_errors());
    assert!(events.get_events().is_empty(), "{:?}", messages(&events));
}

#[test]
fn test_non_existing_starlark_extension() {
    let s = setup();
    file(
        &s,
        "test/starlark/BUILD",
        "load('//test/starlark:bad_extension.bzl', 'some_symbol')",
    );
    let ex = evaluate_package_to_exception(&s, "test/starlark");
    let expected_msg = "error loading package 'test/starlark': cannot load \
                        '//test/starlark:bad_extension.bzl': no such file";
    assert_eq!(ex.get_message(), expected_msg);
    assert_eq!(
        ex.get_detailed_exit_code().code,
        blaze_packages::exceptions::FailureCode::PackageLoading(
            PackageLoadingCode::ImportStarlarkFileError
        )
    );
}

// Not a Java test: `//external` is rejected before anything else.
#[test]
fn external_package_is_not_available() {
    let s = setup();
    let ex = evaluate_package_to_exception(&s, "external");
    assert_eq!(
        ex.get_message(),
        "no such package 'external': //external package is not available since the \
         WORKSPACE file is deprecated, please migrate to Bzlmod. See \
         https://bazel.build/external/migration#bind-targets."
    );
}

// Not a Java test: a syntax error yields a package in error with one event per error.
#[test]
fn syntax_error_gives_package_in_error() {
    let s = setup();
    file(&s, "pkg/BUILD", "filegroup(name = 'a'\n");
    let (pkg, events) = valid_package(&s, "pkg");
    assert!(pkg.contains_errors());
    assert!(!events.get_events().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Tests on `PackageLoadingTestCase`.
// ---------------------------------------------------------------------------------------------

// Java: PackageFunctionTest.evaluatePackageoidToException, with the events it reported.
fn exception_and_events(
    t: &PackageLoadingTestCase,
    pkg: &str,
) -> (NoSuchPackageException, Vec<Event>) {
    let (result, events) = t.evaluate_package(pkg);
    assert!(result.has_error(), "{:?}", result);
    let ex = result
        .get_error(&SkyKey::new(key(pkg)))
        .expect("an error")
        .exception_as::<NoSuchPackageException>()
        .expect("a NoSuchPackageException")
        .clone();
    (ex, events)
}

fn assert_package_loading_code(ex: &NoSuchPackageException, code: PackageLoadingCode) {
    assert_eq!(
        ex.get_detailed_exit_code().code,
        blaze_packages::exceptions::FailureCode::PackageLoading(code)
    );
}

// Java: PackageFunctionTest.assertSrcs (`containsExactlyElementsIn(expected).inOrder()`).
fn assert_srcs(
    pkg: &crate::tests::package_loading_test_case::LoadedPackage,
    target_name: &str,
    expected: &[&str],
) {
    assert_eq!(pkg.label_list_attr(target_name, "srcs"), expected);
}

const DISALLOW_EMPTY_GLOB_MESSAGE: &str = "glob pattern '*.foo' didn't match anything, but \
     allow_empty is set to False (the default value of allow_empty can be set with \
     --incompatible_disallow_empty_glob).";

const ALL_EXCLUDED_GLOB_MESSAGE: &str = "all files in the glob have been excluded, but \
     allow_empty is set to False (the default value of allow_empty can be set with \
     --incompatible_disallow_empty_glob).";

// Java: PackageFunctionTest.testGlobOrderStable
#[test]
fn test_glob_order_stable() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['**/*.txt']))"],
    );
    t.scratch_file("foo/b.txt", &[]);
    t.scratch_file("foo/c/c.txt", &[]);
    let pkg = t.get_package_without_errors("foo");
    assert_srcs(&pkg, "foo", &["//foo:b.txt", "//foo:c/c.txt"]);
    t.scratch_file("foo/d.txt", &[]);
    let pkg = t.get_package_without_errors("foo");
    assert_srcs(
        &pkg,
        "foo",
        &["//foo:b.txt", "//foo:c/c.txt", "//foo:d.txt"],
    );
}

// Java: PackageFunctionTest.testGlobOrderStableWithNonSkyframeAndSkyframeComponents
// (`resetEvaluator()` and the second `preparePackageLoading` are what every load does here).
#[test]
fn test_glob_order_stable_with_non_skyframe_and_skyframe_components() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['*.txt']))"],
    );
    t.scratch_file("foo/b.txt", &[]);
    t.scratch_file("foo/a.config", &[]);
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:b.txt"],
    );
    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['*.txt', '*.config']))"],
    );
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:a.config", "//foo:b.txt"],
    );
    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['*.txt', '*.config'])) # comment"],
    );
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:a.config", "//foo:b.txt"],
    );
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:a.config", "//foo:b.txt"],
    );
}

// Java: PackageFunctionTest.globEscapesAt
#[test]
fn glob_escapes_at() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['*.txt']))"],
    );
    t.scratch_file("foo/@f.txt", &[]);
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:@f.txt"],
    );

    t.scratch_file(
        "foo/BUILD",
        &["filegroup(name = 'foo', srcs = glob(['*.txt'])) # comment"],
    );
    assert_srcs(
        &t.get_package_without_errors("foo"),
        "foo",
        &["//foo:@f.txt"],
    );
}

// Java: PackageFunctionTest.testGlobWithExternalSymlink. Not ported: `assertThat(fooPkg2)
// .isNotEqualTo(fooPkg)` (every load here builds a new package, so it would hold trivially).
#[test]
fn test_glob_with_external_symlink() {
    let t = PackageLoadingTestCase::new();
    let build_file = [
        "filegroup(",
        "    name = \"foo\",",
        "    srcs = glob([\"*.sh\"]),",
        ")",
        "",
        "filegroup(",
        "    name = \"bar\",",
        "    srcs = glob([\"link.sh\"]),",
        ")",
        "",
        "filegroup(",
        "    name = \"baz\",",
        "    srcs = glob([\"subdir_link/*.txt\"]),",
        ")",
    ];
    t.scratch_file("foo/BUILD", &build_file);
    t.scratch_file("foo/ordinary.sh", &[]);
    // Java `scratch.file("../ops/target.txt")`: next to the workspace, outside of it.
    let external_target = t.setup.scratch.file("ops/target.txt", "");
    t.setup.scratch.symlink(
        &t.setup.ws("foo/link.sh"),
        external_target.get_path_string(),
    );
    t.setup.scratch.symlink(
        &t.setup.ws("foo/subdir_link"),
        external_target
            .get_parent_directory()
            .unwrap()
            .get_path_string(),
    );
    let foo_pkg = t.get_package_without_errors("foo");
    assert_srcs(&foo_pkg, "foo", &["//foo:link.sh", "//foo:ordinary.sh"]);
    assert_srcs(&foo_pkg, "bar", &["//foo:link.sh"]);
    assert_srcs(&foo_pkg, "baz", &["//foo:subdir_link/target.txt"]);

    let mut build_file_2 = build_file.to_vec();
    build_file_2[3] = ")  #comment";
    t.scratch_file("foo/BUILD", &build_file_2);
    let foo_pkg_2 = t.get_package_without_errors("foo");
    assert_srcs(&foo_pkg_2, "foo", &["//foo:link.sh", "//foo:ordinary.sh"]);
    assert_srcs(&foo_pkg_2, "bar", &["//foo:link.sh"]);
    assert_srcs(&foo_pkg_2, "baz", &["//foo:subdir_link/target.txt"]);
}

// Java: PackageFunctionTest.testTransitiveStarlarkDepsStoredInPackage
#[test]
fn test_transitive_starlark_deps_stored_in_package() {
    let mut t = PackageLoadingTestCase::new();
    t.scratch_file("foo/BUILD", &["load('//bar:ext.bzl', 'a')"]);
    t.scratch_file("bar/BUILD", &[]);
    t.scratch_file(
        "bar/ext.bzl",
        &["load(\"//baz:ext.scl\", \"b\")", "", "a = b"],
    );
    t.scratch_file("baz/BUILD", &[]);
    t.scratch_file("baz/ext.scl", &["b = 1"]);
    t.scratch_file("qux/BUILD", &[]);
    t.scratch_file("qux/ext.bzl", &["c = 1"]);
    t.set_build_language_options(&["--experimental_enable_scl_dialect=true"]);

    let pkg = t.get_package_without_errors("foo");
    assert_eq!(
        loaded_starlark_files(&pkg.package),
        ["//bar:ext.bzl", "//baz:ext.scl"]
    );

    t.scratch_file(
        "bar/ext.bzl",
        &["load(\"//qux:ext.bzl\", \"c\")", "", "a = c"],
    );
    let pkg = t.get_package_without_errors("foo");
    assert_eq!(
        loaded_starlark_files(&pkg.package),
        ["//bar:ext.bzl", "//qux:ext.bzl"]
    );
}

// Java `getOrComputeTransitivelyLoadedStarlarkFiles()` under `containsExactly` (no order).
fn loaded_starlark_files(pkg: &Package) -> Vec<String> {
    let mut files: Vec<String> = pkg
        .get_declarations()
        .get_or_compute_transitively_loaded_starlark_files()
        .iter()
        .map(|l| l.to_string())
        .collect();
    files.sort();
    files
}

// Java: PackageFunctionTest.testNonExistingStarlarkExtensionFromExtension
#[test]
fn test_non_existing_starlark_extension_from_extension() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "test/starlark/extension.bzl",
        &[
            "load(\"//test/starlark:bad_extension.bzl\", \"some_symbol\")",
            "",
            "a = \"a\"",
        ],
    );
    t.scratch_file(
        "test/starlark/BUILD",
        &["load('//test/starlark:extension.bzl', 'a')"],
    );
    let ex = t.get_package_exception("test/starlark");
    assert_eq!(
        ex.get_message(),
        format!(
            "error loading package 'test/starlark': at {}:1:6: cannot load \
             '//test/starlark:bad_extension.bzl': no such file",
            t.workspace_path("test/starlark/extension.bzl")
        )
    );
    assert_package_loading_code(&ex, PackageLoadingCode::ImportStarlarkFileError);
}

// Java: PackageFunctionTest.testBuiltinsInjectionFailure
#[test]
fn test_builtins_injection_failure() {
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

    let ex = t.get_package_exception("pkg");
    assert_eq!(
        ex.get_message(),
        "error loading package 'pkg': Internal error while loading Starlark builtins: Failed to \
         load builtins sources: initialization of module 'exports.bzl' (internal) failed"
    );
    assert_package_loading_code(&ex, PackageLoadingCode::BuiltinsInjectionFailure);
}

// Java: PackageFunctionTest.testLoadFromExternalPackage
#[test]
fn test_load_from_external_package() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("p/BUILD", &["load('//external:file.bzl', 'a')"]);
    let (_, events) = t.evaluate_package("p");
    assert_contains_event(
        &events,
        "Starlark files may not be loaded from the //external package",
    );
}

// Java: PackageFunctionTest.testLoadWithoutBzlSuffix
#[test]
fn test_load_without_bzl_suffix() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("p/BUILD", &["load('//p:file.starlark', 'a')"]);
    let (_, events) = t.evaluate_package("p");
    assert_contains_event(
        &events,
        "The label must reference a file with extension \".bzl\"",
    );
}

// Java: PackageFunctionTest.testBzlVisibilityViolation
#[test]
fn test_bzl_visibility_violation() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);
    t.scratch_file("a/BUILD", &["load(\"//b:foo.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/foo.bzl", &["visibility(\"private\")", "x = 1"]);

    let (ex, events) = exception_and_events(&t, "a");
    assert!(
        ex.get_message().contains(
            "error loading package 'a': file //a:BUILD contains .bzl load visibility violations"
        ),
        "{}",
        ex.get_message()
    );
    assert_package_loading_code(&ex, PackageLoadingCode::ImportStarlarkFileError);
    assert_contains_event(
        &events,
        "Starlark file //b:foo.bzl is not visible for loading from package //a.",
    );
}

// Java: PackageFunctionTest.testBzlVisibilityViolationDemotedToWarningWhenBreakGlassFlagIsSet
#[test]
fn test_bzl_visibility_violation_demoted_to_warning_when_break_glass_flag_is_set() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&[
        "--experimental_bzl_visibility=true",
        "--check_bzl_visibility=false",
    ]);
    t.scratch_file("a/BUILD", &["load(\"//b:foo.bzl\", \"x\")"]);
    t.scratch_file("b/BUILD", &[]);
    t.scratch_file("b/foo.bzl", &["visibility(\"private\")", "x = 1"]);

    let pkg = t.get_package_without_errors("a");
    pkg.assert_contains_event(
        "Starlark file //b:foo.bzl is not visible for loading from package //a.",
    );
    pkg.assert_contains_event("Continuing because --nocheck_bzl_visibility is active");
}

// Java: PackageFunctionTest.testVisibilityCallableNotAvailableInBUILD
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java \"name 'visibility' is not defined\""]
fn test_visibility_callable_not_available_in_build() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);
    t.scratch_file("a/BUILD", &["visibility(\"public\")"]);
    let (_, events) = t.evaluate_package("a");
    assert_contains_event(&events, "name 'visibility' is not defined");
}

// Java: PackageFunctionTest.testVisibilityCallableErroneouslyInvokedInBUILD
#[test]
fn test_visibility_callable_erroneously_invoked_in_build() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_bzl_visibility=true"]);
    t.scratch_file(
        "a/BUILD",
        &["load(\":helper.bzl\", \"helper\")", "", "helper()"],
    );
    t.scratch_file(
        "a/helper.bzl",
        &["def helper():", "    visibility(\"public\")"],
    );
    let (_, events) = t.evaluate_package("a");
    assert_contains_event(
        &events,
        "visibility() can only be used during .bzl initialization (top-level evaluation)",
    );
}

// Regression test for the two ugly consequences of a bug where GlobFunction incorrectly matched
// dangling symlinks.
// Java: PackageFunctionTest.testIncrementalSkyframeHybridGlobbingOnDanglingSymlink. Each load is
// from scratch here, so the Skyframe cache hits and change pruning the Java test is about are not
// exercised; the assertions on each load are kept. Not ported: `assertThat(newPkg)
// .isNotSameInstanceAs(pkg)` (every load here builds a new package, so it would hold trivially).
#[test]
fn test_incremental_skyframe_hybrid_globbing_on_dangling_symlink() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("foo/BUILD", &["exports_files(glob(['*.txt']))"]);
    t.scratch_file("foo/existing.txt", &[]);
    t.setup
        .scratch
        .symlink(&t.setup.ws("foo/dangling.txt"), "nope");

    let pkg = t.get_package_without_errors("foo");
    assert!(!pkg.package.contains_errors());
    assert_eq!(pkg.target("existing.txt").get_name(), "existing.txt");
    assert!(pkg.package.get_target("dangling.txt").is_err());

    t.scratch_file(
        "foo/BUILD",
        &[
            "exports_files(glob([\"*.txt\"]))",
            "#some-irrelevant-comment",
        ],
    );

    let pkg2 = t.get_package_without_errors("foo");
    assert!(!pkg2.package.contains_errors());
    assert_eq!(pkg2.target("existing.txt").get_name(), "existing.txt");
    assert!(pkg2.package.get_target("dangling.txt").is_err());
    // One consequence of the bug was that dangling symlinks were matched by globs evaluated by
    // Skyframe globbing, meaning there would incorrectly be corresponding targets in packages
    // that had skyframe cache hits during skyframe hybrid globbing.

    t.scratch_file("foo/nope", &[]);

    let new_pkg = t.get_package_without_errors("foo");
    assert!(!new_pkg.package.contains_errors());
    assert_eq!(new_pkg.target("existing.txt").get_name(), "existing.txt");
    // Another consequence of the bug is that change pruning would incorrectly cut off changes that
    // caused a dangling symlink potentially matched by a glob to come into existence.
    assert_eq!(new_pkg.target("dangling.txt").get_name(), "dangling.txt");
}

// Java: PackageFunctionTest.testPackageLoadingErrorOnMissingBuildFile_multiplePackagePath
#[test]
fn test_package_loading_error_on_missing_build_file_multiple_package_path() {
    let mut t = PackageLoadingTestCase::new();
    t.scratch_file("foo/bar", &[]);
    // Java `scratch.dir("/ws2")`: a second root next to the workspace.
    t.setup.scratch.file("ws2/foo/bar", "");
    let other_root_dir = t.setup.scratch.path("ws2").get_path_string().to_owned();
    let root_directory = t.setup.workspace().get_path_string().to_owned();
    t.set_package_options(|o| {
        o.package_path = vec![root_directory.clone(), other_root_dir.clone()];
    });

    // There is no foo/BUILD file under both `package_path`s foo directory, but we enforce loading
    // package 'foo'.
    let ex = t.get_package_exception("foo");
    let message = ex.get_message();
    assert!(
        message.contains(
            "BUILD file not found in any of the following directories. Add a BUILD file to a \
             directory to mark it as a package."
        ),
        "{message}"
    );
    // Print the absolute directory paths if multiple `package_path`s are provided.
    assert!(
        message.contains(&format!("- {root_directory}/foo")),
        "{message}"
    );
    assert!(
        message.contains(&format!("- {other_root_dir}/foo")),
        "{message}"
    );
    assert_eq!(ex.kind(), NoSuchPackageKind::BuildFileNotFound);
    assert_package_loading_code(&ex, PackageLoadingCode::BuildFileMissing);
}

// Java: PackageFunctionTest.testLabelsCrossesSubpackageBoundaries_complexSubpackageCrossing
#[test]
fn test_labels_crosses_subpackage_boundaries_complex_subpackage_crossing() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/foo/BUILD",
        &[
            "exports_files([\"sub11/sub12/blah1\"])",
            "",
            "exports_files([\"sub21/sub22/blah2\"])",
        ],
    );
    t.scratch_file("pkg/foo/sub11/BUILD", &[]);
    t.scratch_file("pkg/foo/sub11/sub12/BUILD", &[]);
    t.scratch_file("pkg/foo/sub21/BUILD", &[]);
    t.scratch_file("pkg/foo/sub21/sub22/BUILD", &[]);

    let pkg = t.get_package("pkg/foo");
    assert!(pkg.package.contains_errors());

    // Only the deepest package that crosses subpackage boundary should be displayed in the error
    // message.
    pkg.assert_contains_event(
        "Label '//pkg/foo:sub11/sub12/blah1' is invalid because 'pkg/foo/sub11/sub12' is a \
         subpackage; perhaps you meant to put the colon here: '//pkg/foo/sub11/sub12:blah1'?",
    );
    pkg.assert_contains_event(
        "Label '//pkg/foo:sub21/sub22/blah2' is invalid because 'pkg/foo/sub21/sub22' is a \
         subpackage; perhaps you meant to put the colon here: '//pkg/foo/sub21/sub22:blah2'?",
    );
    assert_eq!(pkg.error_events().len(), 2, "{:#?}", pkg.error_events());
}

// Java: PackageFunctionTest.testGlobAllowEmpty_paramValueMustBeBoolean
#[test]
fn test_glob_allow_empty_param_value_must_be_boolean() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'], allow_empty = 5)"]);
    let pkg = t.get_package("pkg");
    pkg.assert_contains_event("expected boolean for argument `allow_empty`, got `5`");
}

// Java: PackageFunctionTest.testGlobAllowEmpty_starlarkOption
#[test]
fn test_glob_allow_empty_starlark_option() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_disallow_empty_glob=false"]);
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'])"]);
    let pkg = t.get_package("pkg");
    assert!(!pkg.package.contains_errors());
    pkg.assert_no_events();
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_functionParam_wasNonEmptyAndBecomesEmpty
#[test]
fn test_glob_disallow_empty_function_param_was_non_empty_and_becomes_empty() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'], allow_empty=False)"]);
    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(!pkg.package.contains_errors());
    pkg.assert_no_events();

    t.delete_file("pkg/blah.foo");
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_starlarkOption_wasNonEmptyAndBecomesEmpty
#[test]
fn test_glob_disallow_empty_starlark_option_was_non_empty_and_becomes_empty() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_disallow_empty_glob=true"]);
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'])"]);
    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(!pkg.package.contains_errors());
    pkg.assert_no_events();

    t.delete_file("pkg/blah.foo");
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_functionParam_wasEmptyAndStaysEmpty
#[test]
fn test_glob_disallow_empty_function_param_was_empty_and_stays_empty() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'], allow_empty=False)"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);

    t.scratch_file(
        "pkg/BUILD",
        &["x = glob(['*.foo'], allow_empty=False) #comment"],
    );
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_starlarkOption_wasEmptyAndStaysEmpty
#[test]
fn test_glob_disallow_empty_starlark_option_was_empty_and_stays_empty() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_disallow_empty_glob=true"]);
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'])"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);

    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo']) #comment"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_functionParam_wasEmptyDueToExcludeAndStaysEmpty
#[test]
fn test_glob_disallow_empty_function_param_was_empty_due_to_exclude_and_stays_empty() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &["x = glob(include=['*.foo'], exclude=['blah.*'], allow_empty=False)"],
    );
    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(ALL_EXCLUDED_GLOB_MESSAGE);

    t.scratch_file(
        "pkg/BUILD",
        &["x = glob(include=['*.foo'], exclude=['blah.*'], allow_empty=False) # comment"],
    );
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(ALL_EXCLUDED_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_starlarkOption_wasEmptyDueToExcludeAndStaysEmpty
#[test]
fn test_glob_disallow_empty_starlark_option_was_empty_due_to_exclude_and_stays_empty() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_disallow_empty_glob=true"]);
    t.scratch_file(
        "pkg/BUILD",
        &["x = glob(include=['*.foo'], exclude=['blah.*'])"],
    );
    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(ALL_EXCLUDED_GLOB_MESSAGE);

    t.scratch_file(
        "pkg/BUILD",
        &["x = glob(include=['*.foo'], exclude=['blah.*']) # comment"],
    );
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(ALL_EXCLUDED_GLOB_MESSAGE);
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_functionParam_wasEmptyAndBecomesNonEmpty
#[test]
fn test_glob_disallow_empty_function_param_was_empty_and_becomes_non_empty() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'], allow_empty=False)"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);

    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(!pkg.package.contains_errors());
    pkg.assert_no_events();
}

// Java: PackageFunctionTest.testGlobDisallowEmpty_starlarkOption_wasEmptyAndBecomesNonEmpty
#[test]
fn test_glob_disallow_empty_starlark_option_was_empty_and_becomes_non_empty() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--incompatible_disallow_empty_glob=true"]);
    t.scratch_file("pkg/BUILD", &["x = glob(['*.foo'])"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(DISALLOW_EMPTY_GLOB_MESSAGE);

    t.scratch_file("pkg/blah.foo", &[]);
    let pkg = t.get_package("pkg");
    assert!(!pkg.package.contains_errors());
    pkg.assert_no_events();
}

// Java: PackageFunctionTest.testPackageRecordsLoadedModules
#[test]
#[ignore = "Package.Declarations stores the precomputed transitive loads only (PORT note at Declarations::visit_load_graph); Java's default stores the direct loads and a custom visitor sees a twice-loaded module twice"]
fn test_package_records_loaded_modules() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("p/BUILD", &["load('a.bzl', 'a'); load(':b.bzl', 'b')"]);
    t.scratch_file("p/a.bzl", &["load('c.bzl', 'c'); a = c"]);
    t.scratch_file("p/b.bzl", &["load(':c.bzl', 'c'); b = c"]);
    t.scratch_file("p/c.bzl", &["load(':d.bzl', 'd'); c = d"]);
    t.scratch_file("p/d.bzl", &["d = 0"]);

    // load p
    let p = t.get_package_without_errors("p");

    assert_eq!(
        loaded_starlark_files(&p.package),
        ["//p:a.bzl", "//p:b.bzl", "//p:c.bzl", "//p:d.bzl"]
    );
    assert_eq!(
        p.package
            .get_declarations()
            .count_transitively_loaded_starlark_files(),
        4
    );

    // Custom visitation: c.bzl is visited twice, but the second time we don't recurse, so d.bzl is
    // only visited once.
    let mut loads: Vec<String> = Vec::new();
    p.package
        .get_declarations()
        .visit_load_graph(&mut |load: &Label| -> Result<bool, ()> {
            let first_visit = !loads.contains(&load.to_string());
            loads.push(load.to_string());
            Ok(first_visit)
        })
        .unwrap();
    loads.sort();
    assert_eq!(
        loads,
        [
            "//p:a.bzl",
            "//p:b.bzl",
            "//p:c.bzl",
            "//p:c.bzl",
            "//p:d.bzl"
        ]
    );
}

// Java: PackageFunctionTest.testSymlinkCycleWithStarlarkExtension
#[test]
fn test_symlink_cycle_with_starlark_extension() {
    let t = PackageLoadingTestCase::new();
    t.setup
        .scratch
        .symlink(&t.setup.ws("test/starlark/extension.bzl"), "extension.bzl");
    t.scratch_file(
        "test/starlark/BUILD",
        &["load('//test/starlark:extension.bzl', 'a')"],
    );

    let ex = t.get_package_exception("test/starlark");
    assert_eq!(
        ex.get_message(),
        "error loading package 'test/starlark': Encountered error while reading extension file \
         'test/starlark/extension.bzl': Symlink cycle"
    );
    assert_package_loading_code(&ex, PackageLoadingCode::ImportStarlarkFileError);
}

// Java: PackageFunctionTest.testSymlinkCycleEncounteredWhileHandlingLabelCrossingSubpackageBoundaries
#[test]
fn test_symlink_cycle_encountered_while_handling_label_crossing_subpackage_boundaries() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["exports_files(['sub/blah'])"]);
    let sub_build_file_path = t.workspace_path("pkg/sub/BUILD");
    t.setup
        .scratch
        .symlink(&t.setup.ws("pkg/sub/BUILD"), &sub_build_file_path);

    let (ex, events) = exception_and_events(&t, "pkg");
    assert_eq!(ex.kind(), NoSuchPackageKind::BuildFileNotFound);
    assert!(
        ex.get_message().contains(
            "no such package 'pkg/sub': Symlink cycle detected while trying to find BUILD file"
        ),
        "{}",
        ex.get_message()
    );
    assert_contains_event(&events, "circular symlinks detected");
}

// Regression test for b/206459361.
// Java: PackageFunctionTest.nonSkyframeGlobbingIOException_andLabelCrossingSubpackageBoundaries_withKeepGoing
#[test]
fn non_skyframe_globbing_io_exception_and_label_crossing_subpackage_boundaries_with_keep_going() {
    let t = PackageLoadingTestCase::new();
    // When a package's BUILD file and the relevant filesystem state is such that non-Skyframe
    // globbing will encounter an IOException due to a directory symlink cycle *and* the BUILD file
    // defines a target with a label that crosses subpackage boundaries,
    t.scratch_file(
        "pkg/BUILD",
        &[
            "exports_files([\"sub/blah\"])  # label crossing subpackage boundaries",
            "",
            "glob([\"globcycle/**/foo.txt\"])  # triggers non-Skyframe globbing error",
        ],
    );
    t.scratch_file("pkg/sub/BUILD", &[]);
    let pkg_globcycle_path = t.workspace_path("pkg/globcycle");
    t.setup
        .scratch
        .symlink(&t.setup.ws("pkg/globcycle"), &pkg_globcycle_path);
    // Java: `assertThrows(IOException.class, () -> pkgGlobcyclePath.statIfFound(FOLLOW))`.
    assert!(
        std::fs::metadata(&*blaze_util::string_encoding::internal_to_platform(
            &pkg_globcycle_path
        ))
        .is_err()
    );

    // ... and we evaluate the package with keepGoing == true, we expect the evaluation to fail with
    // the non-Skyframe globbing error, but for the label crossing event to *not* get added (because
    // the globbing IOException would put Package.Builder in a state on which we cannot run
    // handleLabelsCrossingSubpackagesAndPropagateInconsistentFilesystemExceptions).
    let (result, events) = t.evaluate_package_keep_going("pkg");
    let ex = result
        .get_error(&SkyKey::new(key("pkg")))
        .expect("an error entry for the package key")
        .exception_as::<NoSuchPackageException>()
        .expect("a NoSuchPackageException")
        .clone();
    assert!(
        ex.get_message()
            .contains(&format!("Symlink cycle: {pkg_globcycle_path}")),
        "{}",
        ex.get_message()
    );
    assert_does_not_contain_event(
        &events,
        "Label '//pkg:sub/blah' is invalid because 'pkg/sub' is a subpackage",
    );
}

// ---------------------------------------------------------------------------------------------
// PackageFunctionTest.PreludeTest. The prelude is `//tools/build_rules:prelude_bazel` (Java:
// `//tools/test_build_rules:test_prelude`).
// ---------------------------------------------------------------------------------------------

// Java: PackageFunctionTest.PreludeTest.testPreludeDefinedSymbolIsUsable
#[test]
fn test_prelude_defined_symbol_is_usable() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["foo = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(foo)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeAutomaticallyReexportsLoadedSymbols
#[test]
fn test_prelude_automatically_reexports_loaded_symbols() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file(
        "tools/build_rules/prelude_bazel",
        &["load('//util:common.bzl', 'foo')"],
    );
    t.scratch_file("util/BUILD", &[]);
    t.scratch_file("util/common.bzl", &["foo = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(foo)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// TODO(brandjon): Invert this test once the prelude is a module instead of a syntactic
// mutation on BUILD files.
// Java: PackageFunctionTest.PreludeTest.testPreludeCanExportUnderscoreSymbols
#[test]
fn test_prelude_can_export_underscore_symbols() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["_foo = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(_foo)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeCanShadowUniversal
#[test]
fn test_prelude_can_shadow_universal() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["len = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(len)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeCanShadowPredeclareds
#[test]
fn test_prelude_can_shadow_predeclareds() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["cc_library = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(cc_library)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeCanShadowInjectedPredeclareds
#[test]
fn test_prelude_can_shadow_injected_predeclareds() {
    let mut t = PackageLoadingTestCase::new();
    t.set_build_language_options(&["--experimental_builtins_bzl_path=tools/builtins_staging"]);
    t.scratch_file(
        "tools/builtins_staging/exports.bzl",
        &[
            "exported_toplevels = {}",
            "exported_rules = {\"cc_library\": \"BAR\"}",
            "exported_to_java = {}",
        ],
    );
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["cc_library = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(cc_library)"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeSymbolCannotBeMutated
#[test]
fn test_prelude_symbol_cannot_be_mutated() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("tools/build_rules/prelude_bazel", &["foo = ['FOO']"]);
    t.scratch_file("pkg/BUILD", &["foo.append('BAR')"]);

    let (_, events) = t.evaluate_package("pkg");
    assert_contains_event(&events, "trying to mutate a frozen list value");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeCanAccessBzlDialectFeatures
#[test]
#[ignore = "starlark-rust repr of builtins (patch 0009 queued): Java '<built-in method glob of native value>'"]
fn test_prelude_can_access_bzl_dialect_features() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    // Test both bzl symbols and syntax (e.g. function defs).
    t.scratch_file(
        "tools/build_rules/prelude_bazel",
        &["def foo():", "    return native.glob"],
    );
    t.scratch_file("pkg/BUILD", &["print(foo())"]);

    let pkg = t.get_package_without_errors("pkg");
    // Prelude can access native.glob (though only a BUILD thread can call it).
    pkg.assert_contains_event("<built-in method glob of native value>");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeNeedNotBePresent
#[test]
fn test_prelude_need_not_be_present() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &["print('FOO')"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeNeedNotBePresent_evenWhenPackageIs
#[test]
fn test_prelude_need_not_be_present_even_when_package_is() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file("pkg/BUILD", &["print('FOO')"]);

    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeFileNotRecognizedWithoutPackage
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java \"name 'foo' is not defined\""]
fn test_prelude_file_not_recognized_without_package() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/prelude_bazel", &["foo = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(foo)"]);

    // The prelude file is not found without a corresponding package to contain it. BUILD files
    // get processed as if no prelude file is present.
    let (_, events) = t.evaluate_package("pkg");
    assert_contains_event(&events, "name 'foo' is not defined");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeFailsWhenErrorInPreludeFile
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'Error: integer division by zero'"]
fn test_prelude_fails_when_error_in_prelude_file() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("tools/build_rules/BUILD", &[]);
    t.scratch_file(
        "tools/build_rules/prelude_bazel",
        &[
            "1//0", // <-- dynamic error
            "foo = 'FOO'",
        ],
    );
    t.scratch_file("pkg/BUILD", &["print(foo)"]);

    let (_, events) = t.evaluate_package("pkg");
    assert_contains_event(
        &events,
        &format!(
            "File \"{}\", line 1, column 2, in <toplevel>",
            t.workspace_path("tools/build_rules/prelude_bazel")
        ),
    );
    assert_contains_event(&events, "Error: integer division by zero");
}

// Java: PackageFunctionTest.PreludeTest.testPreludeWorksEvenWhenPreludePackageInError
#[test]
fn test_prelude_works_even_when_prelude_package_in_error() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "tools/build_rules/BUILD",
        &["1//0"], // <-- dynamic error
    );
    t.scratch_file("tools/build_rules/prelude_bazel", &["foo = 'FOO'"]);
    t.scratch_file("pkg/BUILD", &["print(foo)"]);

    // Succeeds because prelude loading is only dependent on the prelude package's existence, not
    // its evaluation.
    let pkg = t.get_package_without_errors("pkg");
    pkg.assert_contains_event("FOO");
}

// ---------------------------------------------------------------------------------------------
// Not ported yet.
// ---------------------------------------------------------------------------------------------

macro_rules! ignored {
    ($($name:ident: $reason:literal,)*) => {
        $(
            #[test]
            #[ignore = $reason]
            fn $name() {}
        )*
    };
}

ignored! {
    test_invalid_package: "needs a PackageValidator seam: Java mocks PackageValidator.validate to throw InvalidPackageException; LoadingFunctionsConfig has no test hook for the validator",
    test_package_overhead_passed_to_validation_logic: "PackageOverheadEstimator is not ported (Java mocks it and captures the overhead passed to PackageValidator)",
    test_skyframe_executor_cleared_packages_results_in_reload: "needs a PackageValidator seam (Java counts PackageValidator.validate calls as a proxy for loads) and SkyframeExecutor.clearLoadedPackages on a kept evaluator; the harness evaluates from scratch each time",
    symbolic_macro_expansion_only_in_full_packages: "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!)",
    package_with_bad_macro_implementation: "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!)",
    package_with_macro_name_conflict: "symbolic macros do not run (MacroClass.instantiateAndAddMacro is unported!)",
    test_propagates_filesystem_inconsistencies: "needs a file system that reports inconsistent stats (Java CustomInMemoryFs.stubStat); the harness runs on the real file system",
    test_propagates_filesystem_inconsistencies_globbing: "needs a file system whose stat of a directory disagrees with its parent's listing (Java CustomInMemoryFs.stubStat(bazDir, null)); the harness runs on the real file system",
    test_discrepancy_between_globbing_errors: "needs a file system that turns a directory unreadable after readdir (Java CustomInMemoryFs.scheduleMakeUnreadableAfterReaddir); the harness runs on the real file system",
    test_one_new_element_in_multiple_glob: "asserts the package is the same instance after an irrelevant file change (Skyframe change pruning on a kept evaluator); the harness evaluates from scratch each time",
    test_no_new_element_in_multiple_glob: "asserts the package is the same instance after an irrelevant file change (Skyframe change pruning on a kept evaluator); the harness evaluates from scratch each time",
    test_io_error_looking_for_subpackage_for_label_is_handled: "needs a file system whose stat fails with an IOException (Java CustomInMemoryFs.stubStatError); the harness runs on the real file system",
    test_package_loading_error_on_io_exception_reading_build_file: "needs a file system whose read fails with an IOException (Java CustomInMemoryFs.throwExceptionOnGetInputStream); the harness runs on the real file system",
    test_package_loading_error_on_io_exception_reading_bzl_file: "needs a file system whose read fails with an IOException (Java CustomInMemoryFs.throwExceptionOnGetInputStream); the harness runs on the real file system",
    very_broken_package_posts_done_to_progress_receiver: "asserts on SkyframeExecutor.getPackageProgressReceiver().progressState(), which the harness does not expose",
    test_non_skyframe_globbing_encounters_symlink_cycle_and_throws_io_exception: "needs the choice of non-Skyframe globbing (Java parameterizes globUnderSingleDep) and its symlink cycle IOException text",
    test_globbing_skyframe_dependency_structure: "needs graph inspection of the GLOB / GLOBS dependencies of the PACKAGE node",
}
