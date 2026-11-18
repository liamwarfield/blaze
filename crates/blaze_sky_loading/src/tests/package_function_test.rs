//! Port of PackageFunctionTest (the monolithic-package `ComputationMode`).
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/PackageFunctionTest.java. Builtins
//! injection is disabled (`--experimental_builtins_bzl_path=`), so BUILD files see the native
//! rules directly.

use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_events::StoredEventHandler;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchPackageKind, PackageLoadingCode};
use blaze_packages::package::Package;
use blaze_skyframe::{EvaluationResult, SkyKey};

use crate::package_function::PackageKey;
use crate::test_util::{FakeRepos, TestSetup, evaluate};

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
    test_invalid_package: "needs a mock file system that fails reads",
    test_package_overhead_passed_to_validation_logic: "PackageOverheadEstimator is not ported",
    test_skyframe_executor_cleared_packages_results_in_reload: "needs incremental invalidation",
    test_propagates_filesystem_inconsistencies: "needs a mock file system with inconsistent stats",
    test_propagates_filesystem_inconsistencies_globbing: "needs a mock file system with inconsistent stats",
    test_discrepancy_between_globbing_errors: "needs a mock file system that fails reads",
    test_glob_order_stable: "needs incremental invalidation",
    test_glob_order_stable_with_non_skyframe_and_skyframe_components: "needs incremental invalidation",
    test_glob_with_external_symlink: "needs incremental invalidation",
    test_one_new_element_in_multiple_glob: "needs incremental invalidation",
    test_no_new_element_in_multiple_glob: "needs incremental invalidation",
    test_transitive_starlark_deps_stored_in_package: "needs incremental invalidation",
    test_non_existing_starlark_extension_from_extension: "needs the exact BZL_LOAD no-file message chain",
    test_builtins_injection_failure: "needs @_builtins evaluation (agent B's builtins)",
    test_symlink_cycle_with_starlark_extension: "needs cycle reporting",
    test_io_error_looking_for_subpackage_for_label_is_handled: "needs a mock file system that fails stats",
    test_load_from_external_package: "needs the //external BZL_LOAD message",
    test_load_without_bzl_suffix: "needs the BUILD load label message",
    test_bzl_visibility_violation: "needs agent B's visibility()",
    test_bzl_visibility_violation_demoted_to_warning_when_break_glass_flag_is_set: "needs agent B's visibility()",
    test_visibility_callable_not_available_in_build: "needs the uninjected BUILD environment's error for visibility",
    test_visibility_callable_erroneously_invoked_in_build: "needs agent B's visibility()",
    test_incremental_skyframe_hybrid_globbing_on_dangling_symlink: "needs incremental invalidation",
    test_package_loading_error_on_io_exception_reading_build_file: "needs a mock file system that fails reads",
    test_package_loading_error_on_missing_build_file_multiple_package_path: "needs --package_path with several entries",
    test_package_loading_error_on_io_exception_reading_bzl_file: "needs a mock file system that fails reads",
    test_labels_crosses_subpackage_boundaries_complex_subpackage_crossing: "needs package_group targets",
    test_symlink_cycle_encountered_while_handling_label_crossing_subpackage_boundaries: "needs cycle reporting",
    test_glob_allow_empty_param_value_must_be_boolean: "needs C's glob() parameter conversion messages",
    test_glob_allow_empty_starlark_option: "needs incremental invalidation",
    test_glob_disallow_empty_function_param_was_non_empty_and_becomes_empty: "needs incremental invalidation",
    test_glob_disallow_empty_starlark_option_was_non_empty_and_becomes_empty: "needs incremental invalidation",
    test_glob_disallow_empty_function_param_was_empty_and_stays_empty: "needs incremental invalidation",
    test_glob_disallow_empty_starlark_option_was_empty_and_stays_empty: "needs incremental invalidation",
    test_glob_disallow_empty_function_param_was_empty_due_to_exclude_and_stays_empty: "needs incremental invalidation",
    test_glob_disallow_empty_starlark_option_was_empty_due_to_exclude_and_stays_empty: "needs incremental invalidation",
    test_glob_disallow_empty_function_param_was_empty_and_becomes_non_empty: "needs incremental invalidation",
    test_glob_disallow_empty_starlark_option_was_empty_and_becomes_non_empty: "needs incremental invalidation",
    test_package_records_loaded_modules: "needs PackageDeclarations load accessors",
    test_non_skyframe_globbing_encounters_symlink_cycle_and_throws_io_exception: "needs cycle reporting",
    test_globbing_skyframe_dependency_structure: "needs graph inspection of GLOBS deps",
    test_prelude_defined_symbol_is_usable: "needs a rule class provider with a prelude label",
    test_prelude_automatically_reexports_loaded_symbols: "needs a rule class provider with a prelude label",
    test_prelude_can_export_underscore_symbols: "needs a rule class provider with a prelude label",
    test_prelude_can_shadow_universal: "needs a rule class provider with a prelude label",
    test_prelude_can_shadow_predeclareds: "needs a rule class provider with a prelude label",
    test_prelude_can_shadow_injected_predeclareds: "needs a rule class provider with a prelude label",
    test_prelude_symbol_cannot_be_mutated: "needs a rule class provider with a prelude label",
    test_prelude_can_access_bzl_dialect_features: "needs a rule class provider with a prelude label",
    test_prelude_need_not_be_present: "needs a rule class provider with a prelude label",
    test_prelude_need_not_be_present_even_when_package_is: "needs a rule class provider with a prelude label",
    test_prelude_file_not_recognized_without_package: "needs a rule class provider with a prelude label",
    test_prelude_fails_when_error_in_prelude_file: "needs a rule class provider with a prelude label",
    test_prelude_works_even_when_prelude_package_in_error: "needs a rule class provider with a prelude label",
}
