//! Port of PackageFactoryTest on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/PackageFactoryTest.java.
//!
//! Differences from the Java test environment that the ports below carry:
//! - Java adds a test-only native rule `fake_cc_library` (`srcs`, `deps`, `hdrs` label lists,
//!   `linkstatic`, `alwayslink`, `generator_name`) to `TestRuleClassProvider`. The rule class
//!   provider here is Bazel's, so `fake_cc_library` is a Starlark rule of the same name and
//!   attributes in `//fake_rules:defs.bzl` (without `generator_name`, which a Starlark rule cannot
//!   declare and no ported test sets), and BUILD files that use it start with
//!   [`FAKE_CC_LIBRARY_LOAD`]. The rest of each BUILD file and every expected text is Java's.
//! - Java's workspace is `/workspace`; expected texts that name a workspace path are built with
//!   `PackageLoadingTestCase::workspace_path`.
//! - `reporter.removeHandler(failFastHandler)`: a Java test that keeps the fail-fast handler
//!   fails on any ERROR event; those ports assert `assert_no_error_events()` (directly or through
//!   `get_package_without_errors`).
//!
//! Tests that stay ignored are at the end of this file with their reasons; the symbolic macro
//! tests and three tests on Java-only seams stay as stubs in
//! `crates/blaze_packages/src/build/loading_tests.rs`.

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_packages::package::{FeatureSet, RuleVisibility};
use blaze_packages::target::InputFileKind;
use blaze_packages::types::{AttrValue, LicenseType};

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase, sorted};

/// The first line of a BUILD file that uses Java's `fake_cc_library`.
const FAKE_CC_LIBRARY_LOAD: &str = "load('//fake_rules:defs.bzl', 'fake_cc_library')";

/// A test case with `//fake_rules:defs.bzl` and Java's `--default_visibility=public`
/// (`PackageLoadingTestCase.parsePackageOptions`).
fn new_test_case() -> PackageLoadingTestCase {
    let mut t = PackageLoadingTestCase::new();
    t.use_public_default_visibility();
    t.scratch_file("fake_rules/BUILD", &[]);
    t.scratch_file(
        "fake_rules/defs.bzl",
        &[
            "def _impl(ctx):",
            "    pass",
            "fake_cc_library = rule(",
            "    implementation = _impl,",
            "    attrs = {",
            "        'srcs': attr.label_list(allow_files = True),",
            "        'deps': attr.label_list(allow_files = True),",
            "        'hdrs': attr.label_list(allow_files = True),",
            "        'linkstatic': attr.bool(),",
            "        'alwayslink': attr.bool(),",
            "    },",
            ")",
        ],
    );
    t
}

// Java: PackageFactoryTest.expectEvalSuccess
fn expect_eval_success(t: &PackageLoadingTestCase, content: &[&str]) -> LoadedPackage {
    t.scratch_file("pkg/BUILD", content);
    let pkg = t.get_package("pkg");
    pkg.assert_no_error_events();
    assert!(!pkg.package.contains_errors());
    pkg
}

// Java: PackageFactoryTest.expectEvalError
fn expect_eval_error(t: &PackageLoadingTestCase, expected_error: &str, content: &[&str]) {
    t.scratch_file("pkg/BUILD", content);
    let pkg = t.get_package("pkg");
    assert!(
        pkg.package.contains_errors(),
        "Expected evaluation error, but none was not reported"
    );
    pkg.assert_contains_event(expected_error);
}

// Java: PackageFactoryTest.checkBuildDialectError
fn check_build_dialect_error(t: &PackageLoadingTestCase, content: &str, expected_error: &str) {
    t.scratch_file("p/BUILD", &[content]);
    let pkg = t.get_package("p");
    pkg.assert_contains_event(expected_error);
    assert!(pkg.package.contains_errors());
}

/// Java `Starlark.repr` of a list of strings.
fn starlark_repr(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

// Java: PackageFactoryTest.assertGlobFails / buildPackageWithGlob
fn assert_glob_fails(glob_call_expression: &str, expected_error: &str) {
    let t = new_test_case();
    t.scratch_file(
        "dummypackage/BUILD",
        &[format!("x = {glob_call_expression}").as_str()],
    );
    let pkg = t.get_package("dummypackage");
    pkg.assert_contains_event(expected_error);
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.evaluateGlob
fn evaluate_glob(
    t: &PackageLoadingTestCase,
    includes: &[&str],
    excludes: &[&str],
    exclude_dirs: bool,
    result_assertion: &str,
) -> LoadedPackage {
    t.scratch_dir("globs/subdir");
    for file in ["Wombat1.java", "Wombat2.java", "subdir/Wombat3.java"] {
        t.scratch_file(&format!("globs/{file}"), &[]);
    }
    let glob_line = format!(
        "result = glob({}, exclude={}, exclude_directories={}, allow_empty = True)",
        starlark_repr(includes),
        starlark_repr(excludes),
        if exclude_dirs { 1 } else { 0 }
    );
    t.scratch_file("globs/BUILD", &[glob_line.as_str(), result_assertion]);
    t.get_package("globs")
}

/// The package of Java `assertGlobMatches`: BUILD execution calls `fail()` if the glob does not
/// match `result`.
fn evaluate_glob_against(
    t: &PackageLoadingTestCase,
    result: &[&str],
    includes: &[&str],
    excludes: &[&str],
    exclude_dirs: bool,
) -> LoadedPackage {
    let result_assertion = format!(
        "(result == sorted({})) or fail('incorrect glob result: got %s, want %s' % (result, \
         sorted({})))",
        starlark_repr(result),
        starlark_repr(result)
    );
    evaluate_glob(t, includes, excludes, exclude_dirs, &result_assertion)
}

// Java: PackageFactoryTest.assertGlobMatches
fn assert_glob_matches(result: &[&str], includes: &[&str], excludes: &[&str], exclude_dirs: bool) {
    let t = new_test_case();
    let pkg = evaluate_glob_against(&t, result, includes, excludes, exclude_dirs);
    // If the glob doesn't match the expected result, BUILD execution calls fail() which
    // posts an ERROR to the fail-fast handler, throwing AssertionError.
    pkg.assert_no_error_events();
    // Execution succeeded. Assert that there were no other errors in the package.
    assert!(!pkg.package.contains_errors());
}

// Java: PackageFactoryTest.assertGlobProducesError
fn assert_glob_produces_error(pattern: &str, error_expected: bool) {
    let t = new_test_case();
    let pkg = evaluate_glob(&t, &[pattern], &[], false, "");
    assert_eq!(pkg.package.contains_errors(), error_expected);
    let mut found_error = false;
    for message in pkg.collected_events() {
        if message.contains("glob") {
            if !error_expected {
                panic!("error not expected for glob pattern {pattern}, but got: {message}");
            }
            found_error = error_expected;
            break;
        }
    }
    assert_eq!(found_error, error_expected);
}

/// Java `attributes(rule).get(attr, Type.STRING)` (`attributes` is `RawAttributeMapper.of`,
/// whose `get` evaluates computed defaults).
fn string_attr(pkg: &LoadedPackage, rule: &str, attr: &str) -> String {
    use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
    match RawAttributeMapper::of(&**pkg.rule(rule)).get(attr, blaze_packages::types::Type::String) {
        Some(AttrValue::String(s)) => s,
        other => panic!("attribute '{attr}' of rule '{rule}' is not a string: {other:?}"),
    }
}

/// Java `attributes(rule).get(attr, Type.BOOLEAN)`.
fn bool_attr(pkg: &LoadedPackage, rule: &str, attr: &str) -> bool {
    use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
    match RawAttributeMapper::of(&**pkg.rule(rule)).get(attr, blaze_packages::types::Type::Boolean)
    {
        Some(AttrValue::Bool(b)) => b,
        other => panic!("attribute '{attr}' of rule '{rule}' is not a boolean: {other:?}"),
    }
}

// Java: PackageFactoryTest.testCreatePackage
#[test]
fn test_create_package() {
    let t = new_test_case();
    t.scratch_file("pkgname/BUILD", &["# empty build file "]);
    let pkg = t.get_package_without_errors("pkgname");
    assert_eq!(pkg.package.get_name(), "pkgname");
    assert_eq!(pkg.package.get_rules().count(), 0);
}

// Java: PackageFactoryTest.testBadRuleName
#[test]
fn test_bad_rule_name() {
    let t = new_test_case();
    t.scratch_file(
        "badrulename/BUILD",
        &[FAKE_CC_LIBRARY_LOAD, "fake_cc_library(name = 3)"],
    );
    let pkg = t.get_package("badrulename");
    pkg.assert_contains_event("cc_library 'name' attribute must be a string");
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.testNoRuleName
#[test]
fn test_no_rule_name() {
    let t = new_test_case();
    t.scratch_file(
        "badrulename/BUILD",
        &[FAKE_CC_LIBRARY_LOAD, "fake_cc_library()"],
    );
    let pkg = t.get_package("badrulename");
    pkg.assert_contains_event("cc_library rule has no 'name' attribute");
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.testBadPackageName
#[test]
fn test_bad_package_name() {
    // This is a "shallow" syntactic error: failure to form the
    // PackageIdentifier that is the real argument to loadPackage.
    let e =
        PackageIdentifier::parse("not even a legal/.../label").expect_err("a LabelSyntaxException");
    assert!(
        e.get_message()
            .contains("invalid package name 'not even a legal/.../label'"),
        "{}",
        e.get_message()
    );
}

// Java: PackageFactoryTest.testColonInExportsFilesTargetName
#[test]
fn test_colon_in_exports_files_target_name() {
    let t = new_test_case();
    t.scratch_file(
        "googledata/cafe/BUILD",
        &["exports_files(['houseads/house_ads:ca-aol_parenting_html'])"],
    );
    let pkg = t.get_package("googledata/cafe");
    pkg.assert_contains_event("target names may not contain ':'");
    let file_targets: Vec<String> = pkg
        .package
        .get_targets()
        .values()
        .filter(|target| target.is_file())
        .map(|target| target.to_string())
        .collect();
    assert!(
        !format!("{file_targets:?}").contains("houseads/house_ads:ca-aol_parenting_html"),
        "{file_targets:?}"
    );
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.testExportsFilesVisibilityMustBeSequence
#[test]
fn test_exports_files_visibility_must_be_sequence() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "in call to exports_files(), parameter 'visibility' got value of type 'depset', want \
         'sequence or NoneType'",
        &["exports_files(srcs=[], visibility=depset(['notice']))"],
    );
}

// Java: PackageFactoryTest.testExportsFilesLicensesMustBeSequence
#[test]
fn test_exports_files_licenses_must_be_sequence() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "in call to exports_files(), parameter 'licenses' got value of type 'depset', want \
         'sequence or NoneType'",
        &["exports_files(srcs=[], licenses=depset(['notice']))"],
    );
}

// Java: PackageFactoryTest.isValidPackageName
fn is_valid_package_name(t: &PackageLoadingTestCase, package_name: &str) -> bool {
    // Write a license decl just in case it's a third_party package:
    t.scratch_file(&format!("{package_name}/BUILD"), &["licenses(['notice'])"]);
    let pkg = t.get_package(package_name);
    !pkg.package.contains_errors()
}

// Java: PackageFactoryTest.testPackageNameWithPROTECTEDIsOk
#[test]
fn test_package_name_with_protected_is_ok() {
    let t = new_test_case();
    // One "PROTECTED":
    assert!(is_valid_package_name(&t, "foo/PROTECTED/bar"));
    // Multiple "PROTECTED"s:
    assert!(is_valid_package_name(&t, "foo/PROTECTED/bar/PROTECTED/wiz"));
}

// Java: PackageFactoryTest.testDuplicatedDependencies
#[test]
fn test_duplicated_dependencies() {
    let t = new_test_case();
    t.scratch_file(
        "has_dupe/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "fake_cc_library(name='dep')",
            "fake_cc_library(name='has_dupe', deps=[':dep', ':dep'])",
        ],
    );

    let pkg = t.get_package("has_dupe");
    pkg.assert_contains_event(
        "Label '//has_dupe:dep' is duplicated in the 'deps' attribute of rule 'has_dupe'",
    );
    assert!(pkg.package.contains_errors());
    assert!(pkg.package.get_rule("has_dupe").is_some());
    assert!(pkg.package.get_rule("dep").is_some());
    assert!(pkg.rule("has_dupe").contains_errors());
    // All rules in an errant package are themselves errant.
    assert!(pkg.rule("dep").contains_errors());
}

// Java: PackageFactoryTest.testPrefixWithinSameRule1
#[test]
fn test_prefix_within_same_rule1() {
    let t = new_test_case();
    t.scratch_file(
        "fruit/orange/BUILD",
        &["genrule(name='orange', srcs=[], outs=['a', 'a/b'], cmd='')"],
    );
    let pkg = t.get_package("fruit/orange");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event("rule 'orange' has conflicting output files 'a/b' and 'a");
}

// Java: PackageFactoryTest.testPrefixWithinSameRule2
#[test]
fn test_prefix_within_same_rule2() {
    let t = new_test_case();
    t.scratch_file(
        "fruit/orange/BUILD",
        &["genrule(name='orange', srcs=[], outs=['a/b', 'a'], cmd='')"],
    );
    let pkg = t.get_package("fruit/orange");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event("rule 'orange' has conflicting output files 'a' and 'a/b");
}

// Java: PackageFactoryTest.testPrefixBetweenRules1
#[test]
fn test_prefix_between_rules1() {
    let t = new_test_case();
    t.scratch_file(
        "fruit/kiwi/BUILD",
        &[
            "genrule(name='kiwi1', srcs=[], outs=['a'], cmd='')",
            "genrule(name='kiwi2', srcs=[], outs=['a/b'], cmd='')",
        ],
    );
    let pkg = t.get_package("fruit/kiwi");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(
        "output file 'a/b' of rule 'kiwi2' conflicts with output file 'a' of rule 'kiwi1'",
    );
}

// Java: PackageFactoryTest.testPrefixBetweenRules2
#[test]
fn test_prefix_between_rules2() {
    let t = new_test_case();
    t.scratch_file(
        "fruit/kiwi/BUILD",
        &[
            "genrule(name='kiwi1', srcs=[], outs=['a/b'], cmd='')",
            "genrule(name='kiwi2', srcs=[], outs=['a'], cmd='')",
        ],
    );
    let pkg = t.get_package("fruit/kiwi");
    assert!(pkg.package.contains_errors());
    pkg.assert_contains_event(
        "output file 'a' of rule 'kiwi2' conflicts with output file 'a/b' of rule 'kiwi1'",
    );
}

// Java: PackageFactoryTest.testPackageNameFunction
#[test]
fn test_package_name_function() {
    let t = new_test_case();
    t.scratch_file(
        "pina/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "fake_cc_library(name=package_name() + '-colada')",
        ],
    );
    let pkg = t.get_package("pina");
    pkg.assert_no_events();
    assert!(!pkg.package.contains_errors());
    assert!(pkg.package.get_rule("pina-colada").is_some());
    assert!(!pkg.rule("pina-colada").contains_errors());
    assert_eq!(pkg.package.get_rules().count(), 1);
}

// Java: PackageFactoryTest.testDuplicateRuleName
#[test]
fn test_duplicate_rule_name() {
    let t = new_test_case();
    t.scratch_file(
        "duplicaterulename/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "filegroup(name = 'spellcheck_proto',",
            "         srcs = ['spellcheck.proto'])",
            "fake_cc_library(name = 'spellcheck_proto')  # conflict error stops execution",
            "x = 1//0  # not reached",
        ],
    );
    let pkg = t.get_package("duplicaterulename");
    pkg.assert_contains_event(
        "cc_library rule 'spellcheck_proto' conflicts with existing filegroup rule",
    );
    pkg.assert_does_not_contain_event("division by zero");
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.testBuildFileTargetExists. Not ported: "Test that it's memoized"
// (`getPackage(...).getTarget("BUILD")` is the same instance as `getTarget("//foo:BUILD")`):
// every load here builds a new package, so two loads never share a target.
#[test]
fn test_build_file_target_exists() {
    let t = new_test_case();
    t.scratch_file("foo/BUILD", &[]);
    let pkg = t.get_package_without_errors("foo");
    let target = pkg.target("BUILD");
    assert_eq!(target.get_name(), "BUILD");
}

// Java: PackageFactoryTest.testCreationOfInputFiles
#[test]
fn test_creation_of_input_files() {
    let mut t = new_test_case();
    t.set_build_language_options(&["--incompatible_no_implicit_file_export"]);
    t.scratch_file(
        "foo/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "exports_files(['Z'], visibility=[\"//visibility:public\"], licenses=[\"restricted\"])",
            "fake_cc_library(name='W', deps=['X', 'Y', 'A'])",
            "fake_cc_library(name='X', srcs=['X'])",
            "fake_cc_library(name='Y')",
        ],
    );
    let pkg = t.get_package_without_errors("foo");
    assert!(!pkg.package.contains_errors());

    // X is a rule with a circular self-dependency.
    assert!(pkg.target("X").is_rule());

    // Y is a rule
    assert!(pkg.target("Y").is_rule());

    // Z is an export file with specified visibility and license specified
    let export_file_target = pkg.target("Z");
    let export_file = export_file_target
        .as_input_file()
        .expect("Z is an input file");
    assert!(matches!(
        export_file.kind(),
        InputFileKind::VisibilityLicenseSpecified { .. }
    ));
    assert!(export_file.is_visibility_specified());
    assert_eq!(
        export_file_target.get_visibility().get_declared_labels(),
        vec![RuleVisibility::public_label()]
    );
    assert!(export_file.is_license_specified());
    let license = export_file_target.get_license();
    assert_eq!(
        license
            .get_license_types()
            .iter()
            .copied()
            .collect::<Vec<LicenseType>>(),
        vec![LicenseType::Restricted]
    );

    // A is an input file with private visibility
    let input_file_target = pkg.target("A");
    let input_file = input_file_target
        .as_input_file()
        .expect("A is an input file");
    assert!(matches!(
        input_file.kind(),
        InputFileKind::PrivateVisibility
    ));
    assert!(input_file.is_visibility_specified());
    assert_eq!(
        input_file_target.get_visibility().get_declared_labels(),
        vec![RuleVisibility::private_label()]
    );

    // B is nothing
    let e = pkg
        .package
        .get_target("B")
        .expect_err("a NoSuchTargetException");
    assert!(
        e.get_message()
            .contains("no such target '//foo:B': target 'B' not declared in package 'foo'"),
        "{}",
        e.get_message()
    );

    // These are the only input files: BUILD, Z
    let input_files: Vec<&str> = pkg
        .package
        .get_targets()
        .values()
        .filter(|target| target.is_input_file())
        .map(|target| target.get_name())
        .collect();
    assert_eq!(sorted_strs(input_files), ["A", "BUILD", "Z"]);
}

fn sorted_strs(mut values: Vec<&str>) -> Vec<&str> {
    values.sort();
    values
}

// Java: PackageFactoryTest.testDuplicateRuleIsNotAddedToPackage
#[test]
fn test_duplicate_rule_is_not_added_to_package() {
    let t = new_test_case();
    t.scratch_file(
        "dup/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "filegroup(name = 'dup_proto',",
            "              srcs  = ['dup.proto'])",
            "",
            "fake_cc_library(name = 'dup_proto',",
            "           srcs = ['dup.pb.cc', 'dup.pb.h'])",
        ],
    );
    let pkg = t.get_package("dup");
    pkg.assert_contains_event("cc_library rule 'dup_proto' conflicts with existing filegroup rule");
    assert!(pkg.package.contains_errors());

    let dup_proto = pkg.rule("dup_proto");
    // Check that the first rule of the given name "wins", and that each of the
    // "winning" rule's outputs is a member of the package.
    assert_eq!(dup_proto.get_rule_class(), "filegroup");
    for out in dup_proto.get_output_files() {
        let member = pkg
            .target(out.get_name())
            .as_output_file()
            .expect("an output file");
        assert!(Arc::ptr_eq(member.get_generating_rule(), dup_proto));
    }
}

// Java: PackageFactoryTest.testConflictingRuleDoesNotUpdatePackage
#[test]
fn test_conflicting_rule_does_not_update_package() {
    let t = new_test_case();

    // In this test, rule2's outputs conflict with rule1, so rule2 is rejected.
    // However, we must check that neither rule2, nor any of its inputs or
    // outputs is a member of the package, and that the conflicting output file
    // "out2" still has rule1 as its getGeneratingRule().
    t.scratch_file(
        "conflict/BUILD",
        &[
            "genrule(name = 'rule1',",
            "        cmd = '',",
            "        srcs = ['in1', 'in2'],",
            "        outs = ['out1', 'out2'])",
            "genrule(name = 'rule2',",
            "        cmd = '',",
            "        srcs = ['in3', 'in4'],",
            "        outs = ['out3', 'out2'])",
        ],
    );
    let pkg = t.get_package("conflict");
    pkg.assert_contains_event(
        "generated file 'out2' in rule 'rule2' conflicts with existing generated file from rule \
         'rule1'",
    );
    assert!(pkg.package.contains_errors());

    assert!(pkg.package.get_rule("rule2").is_none());

    // Ensure that rule2's "out2" didn't overwrite rule1's:
    let out2 = pkg
        .target("out2")
        .as_output_file()
        .expect("out2 is an output file");
    assert!(Arc::ptr_eq(out2.get_generating_rule(), pkg.rule("rule1")));

    // None of rule2, its inputs, or its outputs should belong to pkg:
    let mut found: Vec<&str> = Vec::new();
    for target_name in ["rule2", "in3", "in4", "out3"] {
        if pkg.package.get_target(target_name).is_ok() {
            found.push(target_name);
        }
    }
    assert!(found.is_empty(), "{found:?}");
}

// Was: Regression test for bug "Rules declared after an error in
// a package should be considered 'in error'".
// Then: Regression test for bug "Why aren't ERRORS considered
// fatal?*"
// Now: Regression test for: execution should stop at the first EvalException;
// all rules created prior to the exception error are marked in error.
// Java: PackageFactoryTest.testAllRulesInErrantPackageAreInError
#[test]
fn test_all_rules_in_errant_package_are_in_error() {
    let t = new_test_case();
    t.scratch_file(
        "error/BUILD",
        &[
            "genrule(name = 'rule1',",
            "        cmd = ':',",
            "        outs = ['out.1'])",
            "list = ['bad']",
            "x = 1//0  # dynamic error",
            "genrule(name = 'rule2',",
            "        cmd = ':',",
            "        outs = list)",
        ],
    );
    let pkg = t.get_package("error");
    pkg.assert_contains_event("division by zero");

    assert!(pkg.package.contains_errors());

    // rule1 would be fine but is still marked as in error:
    assert!(pkg.rule("rule1").contains_errors());

    // rule2's genrule is never executed.
    assert!(pkg.package.get_rule("rule2").is_none());
}

// Java: PackageFactoryTest.testHelpfulErrorForMissingExportsFiles
#[test]
fn test_helpful_error_for_missing_exports_files() {
    let t = new_test_case();
    t.scratch_file(
        "x/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "fake_cc_library(name='x', srcs=['x.cc'])",
        ],
    );
    t.scratch_file("x/x.cc", &[]);
    t.scratch_file("x/y.cc", &[]);
    t.scratch_file("x/dir/dummy", &[]);

    let pkg = t.get_package_without_errors("x");
    let build_file = t.workspace_path("x/BUILD");

    assert!(pkg.package.get_target("x.cc").is_ok()); // existing and mentioned.

    let e = pkg
        .package
        .get_target("y.cc")
        .expect_err("a NoSuchTargetException");
    assert_eq!(
        e.get_message(),
        format!(
            "no such target '//x:y.cc': target 'y.cc' not declared in package 'x' defined by \
             {build_file}; however, a source file of this name exists.  (Perhaps add \
             'exports_files([\"y.cc\"])' to x/BUILD?)"
        )
    );

    let e = pkg
        .package
        .get_target("z.cc")
        .expect_err("a NoSuchTargetException");
    assert_eq!(
        e.get_message(),
        format!(
            "no such target '//x:z.cc': target 'z.cc' not declared in package 'x' defined by \
             {build_file} (did you mean x.cc?)"
        )
    );

    let e = pkg
        .package
        .get_target("dir")
        .expect_err("a NoSuchTargetException");
    assert_eq!(
        e.get_message(),
        format!(
            "no such target '//x:dir': target 'dir' not declared in package 'x' defined by \
             {build_file}; however, a source directory of this name exists.  (Perhaps add \
             'exports_files([\"dir\"])' to x/BUILD, or define a filegroup?)"
        )
    );
}

// Java: PackageFactoryTest.testTestSuitesImplicitlyDependOnAllRulesInPackage
#[test]
fn test_test_suites_implicitly_depend_on_all_rules_in_package() {
    let t = new_test_case();
    t.scratch_file(
        "x/foo_test.bzl",
        &[
            "def _impl(ctx):",
            "  pass",
            "foo_test = rule(implementation = _impl, test = True,",
            "  attrs = {\"srcs\": attr.label_list(allow_files=True)})",
        ],
    );
    t.scratch_file(
        "x/BUILD",
        &[
            "load(':foo_test.bzl', 'foo_test')",
            "foo_test(name='s', srcs = ['foo.sh'])",
            "test_suite(name='t1')",
            "test_suite(name='t2', tests=[])",
            "test_suite(name='t3', tests=['//foo'])",
            "test_suite(name='t4', tests=['//foo'])",
            "foo_test(name='c')",
        ],
    );
    let pkg = t.get_package_without_errors("x");

    // Things to note:
    // - The '$implicit_tests' attribute is unset unless the 'tests' attribute is unset or empty.
    // - The '$implicit_tests' attribute's value for t1 and t2 is magically able to contain both s
    //    and c, even though c is instantiated after t1 and t2 are.

    assert_eq!(
        sorted(pkg.label_list_attr("t1", "$implicit_tests")),
        ["//x:c", "//x:s"]
    );
    assert_eq!(
        sorted(pkg.label_list_attr("t2", "$implicit_tests")),
        ["//x:c", "//x:s"]
    );
    assert!(pkg.label_list_attr("t3", "$implicit_tests").is_empty());
    assert!(pkg.label_list_attr("t4", "$implicit_tests").is_empty());
}

// Java: PackageFactoryTest.testGlobDirectoryExclusion
#[test]
fn test_glob_directory_exclusion() {
    let t = new_test_case();
    t.scratch_file("fruit/data/apple", &[]);
    t.scratch_file("fruit/data/pear", &[]);
    t.scratch_file("fruit/data/berry/black", &[]);
    t.scratch_file("fruit/data/berry/blue", &[]);
    t.scratch_file(
        "fruit/BUILD",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "fake_cc_library(name = 'yes', srcs = glob(['data/*']))",
            "fake_cc_library(name = 'no',  srcs = glob(['data/*'], exclude_directories=0))",
        ],
    );
    let pkg = t.get_package("fruit");
    pkg.assert_no_events();
    let yes_files = pkg.label_list_attr("yes", "srcs");
    let no_files = pkg.label_list_attr("no", "srcs");

    assert_eq!(
        sorted(yes_files),
        ["//fruit:data/apple", "//fruit:data/pear"]
    );

    assert_eq!(
        sorted(no_files),
        [
            "//fruit:data/apple",
            "//fruit:data/berry",
            "//fruit:data/pear"
        ]
    );
}

// Java: PackageFactoryTest.testTooManyArgumentsGlobErrors
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'glob() accepts no more than 4 positional arguments but got 5' (argument binding)"]
fn test_too_many_arguments_glob_errors() {
    assert_glob_fails(
        "glob(['incl'],['excl'],3,True,'extraarg')",
        "glob() accepts no more than 4 positional arguments but got 5",
    );
}

// Java: PackageFactoryTest.testGlobEnforcesListArgument
#[test]
fn test_glob_enforces_list_argument() {
    assert_glob_fails(
        "glob(1, exclude=2)",
        "in call to glob(), parameter 'include' got value of type 'int', want 'sequence'",
    );
}

// Java: PackageFactoryTest.testGlobEnforcesListOfStringsArguments
#[test]
fn test_glob_enforces_list_of_strings_arguments() {
    assert_glob_fails(
        "glob(['a', 'b'], exclude=['c', 42])",
        "expected value of type 'string' for element 1 of 'glob' argument, but got 42 (int)",
    );
}

// Java: PackageFactoryTest.testGlobNegativeTest
#[test]
fn test_glob_negative_test() {
    // Negative test that assertGlob does throw an error when asserting against the wrong values.
    // The AssertionError comes from FoundationTestCase.failFastHandler.
    let t = new_test_case();
    let pkg = evaluate_glob_against(
        &t,
        &["Wombat1.java", "This_file_doesn_t_exist.java"],
        &["W*", "subdir"],
        &[],
        true,
    );
    let errors = pkg.error_events();
    assert!(
        errors.iter().any(|e| e.contains("incorrect glob result")),
        "{errors:#?}"
    );
}

// Java: PackageFactoryTest.testGlobExcludeDirectories
#[test]
fn test_glob_exclude_directories() {
    assert_glob_matches(
        &["Wombat1.java", "Wombat2.java"],
        &["W*", "subdir"],
        &[],
        true,
    );
}

// Java: PackageFactoryTest.testGlobDoesNotExcludeDirectories
#[test]
fn test_glob_does_not_exclude_directories() {
    assert_glob_matches(
        &["Wombat1.java", "Wombat2.java", "subdir"],
        &["W*", "subdir"],
        &[],
        false,
    );
}

// Java: PackageFactoryTest.testGlobWithEmptyExcludedList
#[test]
fn test_glob_with_empty_excluded_list() {
    assert_glob_matches(&["Wombat1.java", "Wombat2.java"], &["W*"], &[], false);
}

// Java: PackageFactoryTest.testGlobWithQuestionMarkProducesError
#[test]
fn test_glob_with_question_mark_produces_error() {
    assert_glob_produces_error("Wombat?.java", true);
}

// Java: PackageFactoryTest.testGlobWithoutQuestionMarkDoesntProduceError
#[test]
fn test_glob_without_question_mark_doesnt_produce_error() {
    assert_glob_produces_error("Wombat*.java", false);
}

// Java: PackageFactoryTest.testGlobWithNonMatchingExcludedList
#[test]
fn test_glob_with_non_matching_excluded_list() {
    assert_glob_matches(&["Wombat1.java"], &["W*"], &["*2*"], false);
}

// Java: PackageFactoryTest.testGlobWithTwoMatchingGlobExpressionsAndNonmatchingExclusion
#[test]
fn test_glob_with_two_matching_glob_expressions_and_nonmatching_exclusion() {
    assert_glob_matches(
        &["Wombat1.java", "subdir/Wombat3.java"],
        &["W*", "subdir/W*"],
        &["*2*"],
        false,
    );
}

// Java: PackageFactoryTest.testGlobWithSubdirMatchAndExclusion
#[test]
fn test_glob_with_subdir_match_and_exclusion() {
    assert_glob_matches(
        &["subdir/Wombat3.java"],
        &["W*", "subdir/W*"],
        &["Wombat*.java"],
        false,
    );
}

// Java: PackageFactoryTest.testBadCharacterInGlob
#[test]
fn test_bad_character_in_glob() {
    assert_glob_fails(
        "glob(['?'])",
        "Error in glob: invalid glob pattern '?': wildcard ? forbidden",
    );
}

// Java: PackageFactoryTest.testBadExcludePattern
#[test]
fn test_bad_exclude_pattern() {
    // The 'exclude' check is currently only reached if the pattern is "complex".
    // This seems like a bug:
    //   assertGlobFails("glob(['BUILD'], ['/'])", "pattern cannot be absolute");
    assert_glob_fails("glob(['BUILD'], ['/*/*'])", "pattern cannot be absolute");
}

// Java: PackageFactoryTest.testGlobEscapesAt
#[test]
fn test_glob_escapes_at() {
    // See lib.skyframe.PackageFunctionTest.globEscapesAt and
    // https://github.com/bazelbuild/bazel/issues/10606.
    let t = new_test_case();
    t.scratch_file("p/@f.txt", &[]);
    t.scratch_file(
        "p/BUILD",
        &[
            "name = glob(['*.txt'])[0]",
            "# Note the prepended colon",
            "name == ':@f.txt' or fail('got %s' % name)",
        ],
    );
    let pkg = t.get_package_without_errors("p"); // no error
    assert!(!pkg.package.contains_errors());
}

/// Restores the permissions of a directory made unreadable, so that the scratch directory can
/// be removed.
struct RestoreReadable(String);

impl Drop for RestoreReadable {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(
            &*blaze_util::string_encoding::internal_to_platform(&self.0),
            std::fs::Permissions::from_mode(0o755),
        );
    }
}

/// Tests that a glob evaluation that encounters an I/O error throws instead of constructing a
/// package.
// Java: PackageFactoryTest.testGlobWithIOErrors. `dir.setReadable(false)` is a chmod on the real
// file system here, which does not bind a process running as root.
#[test]
fn test_glob_with_io_errors() {
    let t = new_test_case();
    t.scratch_file("pkg/BUILD", &["glob(['globs/**'])"]);
    t.scratch_dir("pkg/globs/unreadable");
    let dir = t.workspace_path("pkg/globs/unreadable");
    std::fs::set_permissions(
        &*blaze_util::string_encoding::internal_to_platform(&dir),
        std::fs::Permissions::from_mode(0o300),
    )
    .unwrap();
    let _restore = RestoreReadable(dir.clone());

    let ex = t.get_package_exception("pkg");
    assert!(
        ex.get_message().contains(&format!(
            "error globbing [globs/**] op=FILES: {dir} (Permission denied)"
        )),
        "{}",
        ex.get_message()
    );
}

// Java: PackageFactoryTest.testNativeModuleIsDisabled
#[test]
fn test_native_module_is_disabled() {
    let t = new_test_case();
    t.scratch_file("pkg/BUILD", &["native.fake_cc_library(name='bar')"]);
    let pkg = t.get_package("pkg");
    assert!(pkg.package.contains_errors());
}

// Java: PackageFactoryTest.testPackageGroupSpecMinimal
#[test]
fn test_package_group_spec_minimal() {
    let t = new_test_case();
    expect_eval_success(&t, &["package_group(name='skin', packages=[])"]);
}

// Java: PackageFactoryTest.testPackageGroupSpecSimple
#[test]
fn test_package_group_spec_simple() {
    let t = new_test_case();
    expect_eval_success(
        &t,
        &["package_group(name='skin', packages=['//group/abelian'])"],
    );
}

// Java: PackageFactoryTest.testPackageGroupSpecEmpty
#[test]
fn test_package_group_spec_empty() {
    let t = new_test_case();
    expect_eval_success(&t, &["package_group(name='seed')"]);
}

// Java: PackageFactoryTest.testPackageGroupSpecIncludes
#[test]
fn test_package_group_spec_includes() {
    let t = new_test_case();
    expect_eval_success(
        &t,
        &[
            "package_group(name='wine',",
            "              includes=['//wine:cabernet_sauvignon',",
            "                        '//wine:pinot_noir'])",
        ],
    );
}

// Java: PackageFactoryTest.testPackageGroupSpecBad
#[test]
fn test_package_group_spec_bad() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "invalid package name",
        &["package_group(name='skin', packages=['--25:17--'])"],
    );
}

// Java: PackageFactoryTest.testPackageGroupsWithSameName
#[test]
fn test_package_groups_with_same_name() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "conflicts with existing package group",
        &[
            "package_group(name='skin', packages=[])",
            "package_group(name='skin', packages=[])",
        ],
    );
}

// Java: PackageFactoryTest.testPackageGroupNamedArguments
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'package_group() got unexpected positional argument' (argument binding)"]
fn test_package_group_named_arguments() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "package_group() got unexpected positional argument",
        &["package_group('skin', name = 'x')"],
    );
}

// Java: PackageFactoryTest.testPackageSpecMinimal
#[test]
fn test_package_spec_minimal() {
    let t = new_test_case();
    let pkg = expect_eval_success(&t, &["package(default_visibility=[])"]);
    assert!(
        pkg.package
            .get_package_args()
            .default_visibility()
            .is_some()
    );
}

// Java: PackageFactoryTest.testPackageSpecSimple
#[test]
fn test_package_spec_simple() {
    let t = new_test_case();
    expect_eval_success(&t, &["package(default_visibility=['//group:lie'])"]);
}

// Java: PackageFactoryTest.testPackageSpecBad
#[test]
fn test_package_spec_bad() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "invalid target name",
        &["package(default_visibility=[':::'])"],
    );
}

// Java: PackageFactoryTest.testDoublePackageSpecification
#[test]
fn test_double_package_specification() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "can only be used once",
        &[
            "package(default_visibility=[])",
            "package(default_visibility=[])",
        ],
    );
}

// Java: PackageFactoryTest.testEmptyPackageSpecification
#[test]
fn test_empty_package_specification() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "at least one argument must be given to the 'package' function",
        &["package()"],
    );
}

// Java: PackageFactoryTest.testDefaultTestonly
#[test]
fn test_default_testonly() {
    let t = new_test_case();
    let pkg = expect_eval_success(&t, &["package(default_testonly = 1)"]);
    assert_eq!(
        pkg.package.get_package_args().default_test_only(),
        Some(true)
    );
}

// Java: PackageFactoryTest.testDefaultDeprecation
#[test]
fn test_default_deprecation() {
    let t = new_test_case();
    let test_message = "OMG PONIES!";
    let pkg = expect_eval_success(
        &t,
        &[format!("package(default_deprecation = \"{test_message}\")").as_str()],
    );
    assert_eq!(
        pkg.package.get_package_args().default_deprecation(),
        Some(test_message)
    );
}

// Java: PackageFactoryTest.testExportsBuildFile
#[test]
fn test_exports_build_file() {
    let t = new_test_case();
    let pkg = expect_eval_success(
        &t,
        &["exports_files(['BUILD'], visibility=['//visibility:private'])"],
    );
    let build = pkg
        .target("BUILD")
        .as_input_file()
        .expect("BUILD is an input file");
    assert!(Arc::ptr_eq(build, pkg.package.get_build_file()));
}

// Java: PackageFactoryTest.testDefaultDeprecationPropagation
#[test]
fn test_default_deprecation_propagation() {
    let t = new_test_case();
    let msg = "I am completely operational, and all my circuits are functioning perfectly.";
    t.scratch_file(
        "foo/BUILD",
        &[
            format!("package(default_deprecation = \"{msg}\")").as_str(),
            "filegroup(name = 'bar', srcs=['b'])",
        ],
    );
    let pkg = t.get_package_without_errors("foo");
    assert_eq!(string_attr(&pkg, "bar", "deprecation"), msg);
}

// Java: PackageFactoryTest.testDefaultTestonlyPropagation
#[test]
fn test_default_testonly_propagation() {
    let t = new_test_case();
    t.scratch_file(
        "foo/BUILD",
        &[
            "package(default_testonly = 1)",
            "filegroup(name = 'foo', srcs=['b'])",
            "filegroup(name = 'bar', srcs=['b'], testonly = 0)",
        ],
    );
    let pkg = t.get_package_without_errors("foo");

    assert!(bool_attr(&pkg, "foo", "testonly"));

    assert!(!bool_attr(&pkg, "bar", "testonly"));
}

// Java: PackageFactoryTest.testDefaultDeprecationOverriding
#[test]
fn test_default_deprecation_overriding() {
    let t = new_test_case();
    let msg = "I am completely operational, and all my circuits are functioning perfectly.";
    let deceive = "OMG PONIES!";
    t.scratch_file(
        "foo/BUILD",
        &[
            format!("package(default_deprecation = \"{deceive}\")").as_str(),
            format!("filegroup(name = 'bar', srcs=['b'], deprecation = \"{msg}\")").as_str(),
        ],
    );
    let pkg = t.get_package_without_errors("foo");
    assert_eq!(string_attr(&pkg, "bar", "deprecation"), msg);
}

// Java: PackageFactoryTest.testPackageFeatures
#[test]
fn test_package_features() {
    let t = new_test_case();
    t.scratch_file(
        "a/BUILD",
        &[
            "filegroup(name='before')",
            "package(features=['b', 'c'])",
            "filegroup(name='after')",
        ],
    );
    let pkg = t.get_package_without_errors("a");
    assert_eq!(
        pkg.package.get_package_args().features(),
        &FeatureSet::parse(&["b".to_owned(), "c".to_owned()])
    );
}

// Java: PackageFactoryTest.testExportTwicePublicOK
#[test]
fn test_export_twice_public_ok() {
    // In theory, this could be an error, but too many existing files rely on it
    // and it is okay.
    let t = new_test_case();
    expect_eval_success(
        &t,
        &[
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
        ],
    );
}

// Java: PackageFactoryTest.testExportTwicePublicOK2
#[test]
fn test_export_twice_public_ok2() {
    let t = new_test_case();
    expect_eval_success(
        &t,
        &[
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:private\" ])",
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:private\" ])",
        ],
    );
}

// Java: PackageFactoryTest.testExportTwiceFail
#[test]
fn test_export_twice_fail() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "visibility for exported file 'a.cc' declared twice",
        &[
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:private\" ])",
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
        ],
    );
}

// Java: PackageFactoryTest.testExportTwiceFail2
#[test]
fn test_export_twice_fail2() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "visibility for exported file 'a.cc' declared twice",
        &[
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:private\" ])",
        ],
    );
}

// Java: PackageFactoryTest.testExportLicenseTwice
#[test]
fn test_export_license_twice() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "licenses for exported file 'a.cc' declared twice",
        &[
            "exports_files([\"a.cc\"], licenses = [\"notice\"])",
            "exports_files([\"a.cc\"], licenses = [\"notice\"])",
        ],
    );
}

// Java: PackageFactoryTest.testExportGenruleConflict
#[test]
fn test_export_genrule_conflict() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "generated file 'a.cc' in rule 'foo' conflicts with existing source file",
        &[
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
            "genrule(name = 'foo',",
            "    outs = ['a.cc'],",
            "    cmd = '')",
        ],
    );
}

// Java: PackageFactoryTest.testGenruleExportConflict
#[test]
fn test_genrule_export_conflict() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "source file 'a.cc' conflicts with existing generated file from rule 'foo'",
        &[
            "genrule(name = 'foo',",
            "    outs = ['a.cc'],",
            "    cmd = '')",
            "exports_files([\"a.cc\"],",
            "    visibility = [ \"//visibility:public\" ])",
        ],
    );
}

// Java: PackageFactoryTest.testValidEnvironmentGroup
#[test]
fn test_valid_environment_group() {
    let t = new_test_case();
    expect_eval_success(
        &t,
        &[
            "environment(name = 'foo')",
            "environment_group(name='group', environments = [':foo'], defaults = [':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testIncompleteEnvironmentGroup
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'environment_group() missing 1 required named argument: defaults' (argument binding)"]
fn test_incomplete_environment_group() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "environment_group() missing 1 required named argument: defaults",
        &[
            "environment(name = 'foo')",
            "environment_group(name='group', environments = [':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupMissingTarget
#[test]
fn test_environment_group_missing_target() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "environment //pkg:foo does not exist",
        &["environment_group(name='group', environments = [':foo'], defaults = [':foo'])"],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupWrongTargetType
#[test]
fn test_environment_group_wrong_target_type() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "//pkg:foo is not a valid environment",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "fake_cc_library(name = 'foo')",
            "environment_group(name='group', environments = [':foo'], defaults = [':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupWrongPackage
#[test]
fn test_environment_group_wrong_package() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "//foo:foo is not in the same package as group //pkg:group",
        &["environment_group(name='group', environments = ['//foo'], defaults = ['//foo'])"],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupInvalidDefault
#[test]
fn test_environment_group_invalid_default() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "default //pkg:bar is not a declared environment for group //pkg:group",
        &[
            "environment(name = 'foo')",
            "environment(name = 'bar')",
            "environment_group(name='group', environments = [':foo'], defaults = [':bar'])",
        ],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupDuplicateEnvironments
#[test]
fn test_environment_group_duplicate_environments() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "label '//pkg:foo' is duplicated in the 'environments' list of 'group'",
        &[
            "environment(name = 'foo')",
            "environment_group(name='group', environments = [':foo', ':foo'], defaults = [':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testEnvironmentGroupDuplicateDefaults
#[test]
fn test_environment_group_duplicate_defaults() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "label '//pkg:foo' is duplicated in the 'defaults' list of 'group'",
        &[
            "environment(name = 'foo')",
            "environment_group(name='group', environments = [':foo'], defaults = [':foo', ':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testMultipleEnvironmentGroupsValidMembership
#[test]
fn test_multiple_environment_groups_valid_membership() {
    let t = new_test_case();
    expect_eval_success(
        &t,
        &[
            "environment(name = 'foo')",
            "environment(name = 'bar')",
            "environment_group(name='foo_group', environments = [':foo'], defaults = [':foo'])",
            "environment_group(name='bar_group', environments = [':bar'], defaults = [':bar'])",
        ],
    );
}

// Java: PackageFactoryTest.testMultipleEnvironmentGroupsConflictingMembership
#[test]
fn test_multiple_environment_groups_conflicting_membership() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "environment //pkg:foo belongs to both //pkg:bar_group and //pkg:foo_group",
        &[
            "environment(name = 'foo')",
            "environment(name = 'bar')",
            "environment_group(name='foo_group', environments = [':foo'], defaults = [':foo'])",
            "environment_group(name='bar_group', environments = [':foo'], defaults = [':foo'])",
        ],
    );
}

// Java: PackageFactoryTest.testFulfillsReferencesWrongTargetType
#[test]
fn test_fulfills_references_wrong_target_type() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "in \"fulfills\" attribute of //pkg:foo: //pkg:bar is not a valid environment",
        &[
            FAKE_CC_LIBRARY_LOAD,
            "environment(name = 'foo', fulfills = [':bar'])",
            "fake_cc_library(name = 'bar')",
            "environment_group(name='foo_group', environments = [':foo'], defaults = [])",
        ],
    );
}

// Java: PackageFactoryTest.testFulfillsNotInEnvironmentGroup
#[test]
fn test_fulfills_not_in_environment_group() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "in \"fulfills\" attribute of //pkg:foo: //pkg:bar is not a member of this group",
        &[
            "environment(name = 'foo', fulfills = [':bar'])",
            "environment(name = 'bar')",
            "environment_group(name='foo_group', environments = [':foo'], defaults = [])",
        ],
    );
}

// Java: PackageFactoryTest.testPackageDefaultEnvironments
#[test]
fn test_package_default_environments() {
    let t = new_test_case();
    let pkg = expect_eval_success(
        &t,
        &[
            "package(",
            "    default_compatible_with=['//foo'],",
            "    default_restricted_to=['//bar'],",
            ")",
        ],
    );
    let package_args = pkg.package.get_package_args();
    let default_compatible_with: Vec<String> = package_args
        .default_compatible_with()
        .expect("default_compatible_with is set")
        .iter()
        .map(|label| label.to_string())
        .collect();
    assert_eq!(default_compatible_with, ["//foo:foo"]);
    let default_restricted_to: Vec<String> = package_args
        .default_restricted_to()
        .expect("default_restricted_to is set")
        .iter()
        .map(|label| label.to_string())
        .collect();
    assert_eq!(default_restricted_to, ["//bar:bar"]);
}

// Java: PackageFactoryTest.testPackageDefaultCompatibilityDuplicates
#[test]
fn test_package_default_compatibility_duplicates() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "duplicate label(s) in default_compatible_with: //foo:foo",
        &["package(default_compatible_with=['//foo', '//bar', '//foo'])"],
    );
}

// Java: PackageFactoryTest.testPackageDefaultRestrictionDuplicates
#[test]
fn test_package_default_restriction_duplicates() {
    let t = new_test_case();
    expect_eval_error(
        &t,
        "duplicate label(s) in default_restricted_to: //foo:foo",
        &["package(default_restricted_to=['//foo', '//bar', '//foo'])"],
    );
}

// Tests of BUILD file dialect checks:

// Java: PackageFactoryTest.testDefInBuild
#[test]
fn test_def_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "def func(): pass",
        "functions may not be defined in BUILD files",
    );
}

// Java: PackageFactoryTest.testLambdaInBuild
#[test]
fn test_lambda_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "lambda: None",
        "functions may not be defined in BUILD files",
    );
}

// Java: PackageFactoryTest.testForStatementForbiddenInBuild
#[test]
fn test_for_statement_forbidden_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "for _ in []: pass",
        "`for` statements are not allowed in BUILD files",
    );
}

// Java: PackageFactoryTest.testIfStatementForbiddenInBuild
#[test]
fn test_if_statement_forbidden_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "if False: pass",
        "`if` statements are not allowed in BUILD files",
    );
}

// Java: PackageFactoryTest.testKwargsForbiddenInBuild
#[test]
fn test_kwargs_forbidden_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "print(**dict)",
        "**kwargs arguments are not allowed in BUILD files",
    );
    check_build_dialect_error(
        &t,
        "len(dict(**{'a': 1}))",
        "**kwargs arguments are not allowed in BUILD files",
    );
}

// Java: PackageFactoryTest.testArgsForbiddenInBuild
#[test]
fn test_args_forbidden_in_build() {
    let t = new_test_case();
    check_build_dialect_error(
        &t,
        "print(*['a'])",
        "*args arguments are not allowed in BUILD files",
    );
}

// Java: PackageFactoryTest.testPackageValidationFailureRegisteredAfterLoading
#[test]
#[ignore = "needs a PackageValidator seam: Java installs a validator that throws \
            InvalidPackageException for package 'x'; LoadingFunctionsConfig has no test hook for \
            the validator"]
fn test_package_validation_failure_registered_after_loading() {}

// Java: PackageFactoryTest.testTransientErrorsInGlobbing
#[test]
#[ignore = "needs a file system whose readdir fails for one directory and then recovers (Java \
            overrides InMemoryFileSystem.readdir); the harness runs on the real file system"]
fn test_transient_errors_in_globbing() {}

// ---- Symbolic macros (Java PackageFactoryTest, testSymbolicMacro_* and visibility unioning).
//
// PORT: Java's macros call the test-only native rule as `native.fake_cc_library`; here the
// macro .bzl files load `fake_cc_library` from //fake_rules:defs.bzl and call it directly.

const FAKE_CC_LIBRARY_BZL_LOAD: &str = "load('//fake_rules:defs.bzl', 'fake_cc_library')";

/// Java `PackageFactoryTest.defineEmptyMacroBzl`.
fn define_empty_macro_bzl(t: &PackageLoadingTestCase) {
    t.scratch_file(
        "pkg/my_macro.bzl",
        &[
            "def _impl(name, visibility):",
            "    pass",
            "my_macro = macro(implementation = _impl)",
        ],
    );
}

/// `expect_eval_error` with Java's text block (one line per element).
fn expect_eval_error_lines(t: &PackageLoadingTestCase, expected_error: &str, content: &str) {
    let lines: Vec<&str> = content.lines().collect();
    expect_eval_error(t, expected_error, &lines);
}

fn expect_eval_success_lines(t: &PackageLoadingTestCase, content: &str) -> LoadedPackage {
    let lines: Vec<&str> = content.lines().collect();
    expect_eval_success(t, &lines)
}

fn scratch_text(t: &PackageLoadingTestCase, path: &str, content: &str) {
    let lines: Vec<&str> = content.lines().collect();
    t.scratch_file(path, &lines);
}

/// The BUILD prefix of a test that uses `fake_cc_library` at top level.
fn with_fake_load(content: &str) -> String {
    format!("{FAKE_CC_LIBRARY_LOAD}\n{content}")
}

// Java: PackageFactoryTest.testSymbolicMacro_duplicateMacroNamesDisallowed
#[test]
fn test_symbolic_macro_duplicate_macro_names_disallowed() {
    // However, note that duplicates are allowed if one is a submacro of the other.
    // See SymbolicMacroTest#submacroMayHaveSameNameAsAncestorMacros for coverage of that.
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing macro (and was not created by it)",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\nmy_macro(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndRuleClash_macroDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_rule_clash_macro_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "target 'foo' conflicts with an existing macro (and was not created by it)",
        &with_fake_load(
            "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
             fake_cc_library(name = \"foo\")",
        ),
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndRuleClash_ruleDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_rule_clash_rule_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing target",
        &with_fake_load(
            "load(\":my_macro.bzl\", \"my_macro\")\nfake_cc_library(name = \"foo\")\n\
             my_macro(name = \"foo\")",
        ),
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndOutputClash_macroDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_output_clash_macro_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "target 'foo' conflicts with an existing macro (and was not created by it)",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
         genrule(name = \"gen\", outs = [\"foo\"], cmd = \"\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndOutputClash_outputDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_output_clash_output_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing target",
        "load(\":my_macro.bzl\", \"my_macro\")\n\
         genrule(name = \"gen\", outs = [\"foo\"], cmd = \"\")\nmy_macro(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroMayCollideWithPrefixOfOutput
#[test]
fn test_symbolic_macro_macro_may_collide_with_prefix_of_output() {
    // TODO(#19922): Currently we only prevent output file prefixes from colliding with other
    // output files, and don't check if they collide with other types of targets.
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_success_lines(
        &t,
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
         genrule(name = \"gen\", outs = [\"foo/bar\"], cmd = \"\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndEnvironmentGroupClash_macroDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_environment_group_clash_macro_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "target 'foo' conflicts with an existing macro (and was not created by it)",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
         environment(name = \"env\")\n\
         environment_group(name=\"foo\", environments = [\":env\"], defaults = [\":env\"])",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndEnvironmentGroupClash_environmentGroupDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_environment_group_clash_environment_group_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing target",
        "load(\":my_macro.bzl\", \"my_macro\")\nenvironment(name = \"env\")\n\
         environment_group(name=\"foo\", environments = [\":env\"], defaults = [\":env\"])\n\
         my_macro(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndPackageGroupClash_macroDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_package_group_clash_macro_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "target 'foo' conflicts with an existing macro (and was not created by it)",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
         package_group(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndPackageGroupClash_packageGroupDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_package_group_clash_package_group_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing target",
        "load(\":my_macro.bzl\", \"my_macro\")\npackage_group(name = \"foo\")\n\
         my_macro(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndInputClash_macroDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_input_clash_macro_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "target 'foo' conflicts with an existing macro (and was not created by it)",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
         exports_files([\"foo\"])",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_macroAndInputClash_inputDeclaredFirst
#[test]
fn test_symbolic_macro_macro_and_input_clash_input_declared_first() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    expect_eval_error_lines(
        &t,
        "macro 'foo' conflicts with an existing target",
        "load(\":my_macro.bzl\", \"my_macro\")\nexports_files([\"foo\"])\n\
         my_macro(name = \"foo\")",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_implicitlyCreatedInput_isCreatedEvenInsideMacroNamespace
#[test]
fn test_symbolic_macro_implicitly_created_input_is_created_even_inside_macro_namespace() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
             fake_cc_library(\n    name = \"toplevel_target\",\n    srcs = [\"foo_implicit\"],\n)",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(matches!(
        pkg.target("foo_implicit"),
        blaze_packages::target::Target::InputFile(_)
    ));
}

// Java: PackageFactoryTest.testSymbolicMacro_implicitlyCreatedInput_isNotCreatedIfMacroDeclaresTarget
#[test]
fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_declares_target() {
    let t = new_test_case();
    scratch_text(
        &t,
        "pkg/my_macro.bzl",
        &format!(
            "{FAKE_CC_LIBRARY_BZL_LOAD}\ndef _impl(name, visibility):\n\
             \x20   fake_cc_library(name = name + \"_declared_target\")\n\
             \x20   fake_cc_library(name = \"illegally_named_target\")\n\
             my_macro = macro(implementation = _impl)"
        ),
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
             fake_cc_library(\n    name = \"toplevel_target\",\n    srcs = [\n\
             \x20       \"foo_declared_target\",\n        \"illegally_named_target\",\n    ],\n)",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(matches!(
        pkg.target("foo_declared_target"),
        blaze_packages::target::Target::Rule(_)
    ));
    // This target doesn't lie within the macro's namespace and so can't be analyzed, but it
    // still exists and prevents input file creation.
    assert!(matches!(
        pkg.target("illegally_named_target"),
        blaze_packages::target::Target::Rule(_)
    ));
}

// Java: PackageFactoryTest.testSymbolicMacro_implicitlyCreatedInput_isNotCreatedIfMacroNameMatchesExactly
#[test]
fn test_symbolic_macro_implicitly_created_input_is_not_created_if_macro_name_matches_exactly() {
    let t = new_test_case();
    define_empty_macro_bzl(&t);
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")\n\
             fake_cc_library(\n    name = \"toplevel_target\",\n    srcs = [\"foo\"],\n)",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(!pkg.package.get_targets().contains_key("foo"));
}

// Java: PackageFactoryTest.testSymbolicMacro_implicitlyCreatedInput_isCreatedByUsageInMacroAttr
#[test]
fn test_symbolic_macro_implicitly_created_input_is_created_by_usage_in_macro_attr() {
    // A usage in a macro, provided it is top-level, is sufficient to cause an input file to be
    // implicitly created, even if that input file is not also referred to by any actual targets.
    let t = new_test_case();
    scratch_text(
        &t,
        "pkg/my_macro.bzl",
        "def _impl(name, visibility, src):\n    pass\nmy_macro = macro(\n\
         \x20   implementation = _impl,\n    attrs = {\"src\": attr.label()},\n)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(\n    name = \"foo\",\n\
         \x20   src = \"//pkg:input\",\n)",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(matches!(
        pkg.target("input"),
        blaze_packages::target::Target::InputFile(_)
    ));
}

// Java: PackageFactoryTest.testSymbolicMacro_implicitlyCreatedInput_isNotCreatedByUsageInMacroBody
#[test]
fn test_symbolic_macro_implicitly_created_input_is_not_created_by_usage_in_macro_body() {
    // A usage in the body of a macro (whether the declaration is for a target or submacro),
    // does not by itself cause an input file to be implicitly created.
    let t = new_test_case();
    scratch_text(
        &t,
        "pkg/my_macro.bzl",
        &format!(
            "{FAKE_CC_LIBRARY_BZL_LOAD}\ndef _sub_impl(name, visibility):\n\
             \x20   fake_cc_library(\n        name = name,\n        srcs = [\"//pkg:input\"],\n    )\n\
             my_submacro = macro(implementation = _sub_impl)\n\n\
             def _impl(name, visibility):\n\
             \x20   fake_cc_library(\n        name = name,\n        srcs = [\"//pkg:input\"],\n    )\n\
             \x20   my_submacro(name = name + \"_submacro\")\n\
             my_macro = macro(implementation = _impl)"
        ),
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\":my_macro.bzl\", \"my_macro\")\nmy_macro(name = \"foo\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(!pkg.package.get_targets().contains_key("input"));
}

// Java: PackageFactoryTest.testSymbolicMacro_deferredEvaluationExpandsTransitively
#[test]
fn test_symbolic_macro_deferred_evaluation_expands_transitively() {
    let t = new_test_case();
    scratch_text(
        &t,
        "pkg/my_macro.bzl",
        &format!(
            "{FAKE_CC_LIBRARY_BZL_LOAD}\ndef _inner_impl(name, visibility):\n\
             \x20   fake_cc_library(name = name)\n\
             inner_macro = macro(implementation=_inner_impl, finalizer = True)\n\n\
             def _middle_impl(name, visibility):\n    inner_macro(name = name)\n\
             middle_macro = macro(implementation=_middle_impl, finalizer = True)\n\n\
             def _outer_impl(name, visibility):\n    middle_macro(name = name)\n\
             outer_macro = macro(implementation=_outer_impl, finalizer = True)"
        ),
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\":my_macro.bzl\", \"outer_macro\")\nouter_macro(name = \"abc\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert!(pkg.package.get_targets().contains_key("abc"));
    let ids: Vec<&String> = pkg.package.get_macros_by_id().keys().collect();
    assert_eq!(ids, vec!["abc:1", "abc:2", "abc:3"]);
}

/// Java `PackageFactoryTest.defineRecursiveMacro`.
fn define_recursive_macro(t: &PackageLoadingTestCase, deferred_evaluation: bool) {
    scratch_text(
        t,
        "pkg/recursive_macro.bzl",
        &format!(
            "{FAKE_CC_LIBRARY_BZL_LOAD}\ndef _impl(name, visibility, height):\n\
             \x20   if height == 0:\n        fake_cc_library(name = name)\n    else:\n\
             \x20       recursive_macro(\n            name = name + \"_x\",\n\
             \x20           height = height - 1,\n        )\n\n\
             recursive_macro = macro(\n    implementation = _impl,\n    attrs = {{\n\
             \x20       \"height\": attr.int(configurable=False),\n    }},\n\
             \x20   finalizer = {},\n)",
            if deferred_evaluation { "True" } else { "False" }
        ),
    );
}

const RECURSIVE_MACRO_ERROR: &str = "macro 'abc_x' is a direct recursive call of 'abc'. Macro \
     instantiation traceback (most recent call last):\n\
     \tPackage //pkg, macro 'abc' of type //pkg:recursive_macro.bzl%recursive_macro\n\
     \tPackage //pkg, macro 'abc_x' of type //pkg:recursive_macro.bzl%recursive_macro";

// Java: PackageFactoryTest.testSymbolicMacro_recursionProhibitedWithEagerEvaluation
#[test]
fn test_symbolic_macro_recursion_prohibited_with_eager_evaluation() {
    let t = new_test_case();
    define_recursive_macro(&t, /* deferred_evaluation= */ false);
    expect_eval_error_lines(
        &t,
        RECURSIVE_MACRO_ERROR,
        "load(\":recursive_macro.bzl\", \"recursive_macro\")\nrecursive_macro(\n\
         \x20   name = \"abc\",\n    height = 3,\n)",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_recursionProhibitedWithDeferredEvaluation
#[test]
fn test_symbolic_macro_recursion_prohibited_with_deferred_evaluation() {
    let t = new_test_case();
    define_recursive_macro(&t, /* deferred_evaluation= */ true);
    expect_eval_error_lines(
        &t,
        RECURSIVE_MACRO_ERROR,
        "load(\":recursive_macro.bzl\", \"recursive_macro\")\nrecursive_macro(\n\
         \x20   name = \"abc\",\n    height = 3,\n)",
    );
}

// Java: PackageFactoryTest.testSymbolicMacro_indirectRecursionAlsoProhibited
#[test]
fn test_symbolic_macro_indirect_recursion_also_prohibited() {
    // Define a pair of macros where A calls B calls A (and then would stop, if allowed to get
    // that far). Wrap it in a different entry point to test that the non-cyclic part is included
    // in the traceback.
    let t = new_test_case();
    scratch_text(
        &t,
        "pkg/recursive_macro.bzl",
        &format!(
            "{FAKE_CC_LIBRARY_BZL_LOAD}\ndef _A_impl(name, visibility, stop):\n    if stop:\n\
             \x20       fake_cc_library(name = name)\n    else:\n\
             \x20       macro_B(name = name + \"_B\")\n\n\
             macro_A = macro(\n    implementation = _A_impl,\n    attrs = {{\n\
             \x20       \"stop\": attr.bool(default=False, configurable=False),\n    }},\n)\n\n\
             def _B_impl(name, visibility):\n    macro_A(\n        name = name + \"_A\",\n\
             \x20       stop = True,\n    )\n\n\
             macro_B = macro(implementation = _B_impl)\n\n\
             def _main_impl(name, visibility):\n    macro_A(name = name)\n\n\
             main_macro = macro(implementation = _main_impl)"
        ),
    );
    expect_eval_error_lines(
        &t,
        "macro 'abc_B_A' is an indirect recursive call of 'abc'. Macro instantiation traceback \
         (most recent call last):\n\
         \tPackage //pkg, macro 'abc' of type //pkg:recursive_macro.bzl%main_macro\n\
         \tPackage //pkg, macro 'abc' of type //pkg:recursive_macro.bzl%macro_A\n\
         \tPackage //pkg, macro 'abc_B' of type //pkg:recursive_macro.bzl%macro_B\n\
         \tPackage //pkg, macro 'abc_B_A' of type //pkg:recursive_macro.bzl%macro_A",
        "load(\":recursive_macro.bzl\", \"main_macro\")\nmain_macro(name = \"abc\")",
    );
}

/// Java `PackageFactoryTest.assertVisibilityIs`: the target's actual visibility contains exactly
/// the given labels.
fn assert_visibility_is(pkg: &LoadedPackage, target: &str, visibility_labels: &[&str]) {
    let mut actual: Vec<String> = pkg
        .target(target)
        .get_actual_visibility()
        .get_declared_labels()
        .iter()
        .map(|l| l.get_canonical_form())
        .collect();
    let mut expected: Vec<String> = visibility_labels
        .iter()
        .map(|l| {
            blaze_cmdline::Label::parse_canonical_unchecked(l).get_canonical_form()
        })
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "visibility of {target}");
}

/// Java `PackageFactoryTest.enableMacrosAndUsePrivateVisibility`: the test case with
/// `--default_visibility=private` (the harness default) and `//fake_rules:defs.bzl`.
fn new_private_visibility_test_case() -> PackageLoadingTestCase {
    let t = new_test_case();
    let mut t = t;
    t.set_package_options(|o| {
        o.default_visibility = blaze_optdefs::command::rule_visibility::RuleVisibility::PRIVATE;
    });
    t
}

fn lib_macro_bzl(t: &PackageLoadingTestCase, path: &str, body: &str) {
    scratch_text(t, path, &format!("{FAKE_CC_LIBRARY_BZL_LOAD}\n{body}"));
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_occursBothInsideAndOutsideMacros
#[test]
fn test_declaration_visibility_unioning_occurs_both_inside_and_outside_macros() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    fake_cc_library(\n        name = name,\n\
         \x20       visibility = [\"//other_pkg:__pkg__\"],\n    )\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\"//lib:macro.bzl\", \"my_macro\")\n\nfake_cc_library(\n    name = \"foo\",\n\
             \x20   visibility = [\"//other_pkg:__pkg__\"],\n)\nmy_macro(name = \"bar\")",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo", &["//other_pkg:__pkg__", "//pkg:__pkg__"]);
    assert_visibility_is(&pkg, "bar", &["//other_pkg:__pkg__", "//lib:__pkg__"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_usesInnermostMacroLocation
#[test]
fn test_declaration_visibility_unioning_uses_innermost_macro_location() {
    let t = new_private_visibility_test_case();
    t.scratch_file("inner/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "inner/macro.bzl",
        "def _impl(name, visibility):\n    fake_cc_library(\n        name = name,\n\
         \x20       visibility = [\"//other_pkg:__pkg__\"],\n    )\n\
         inner_macro = macro(implementation = _impl)",
    );
    t.scratch_file("outer/BUILD", &[]);
    scratch_text(
        &t,
        "outer/macro.bzl",
        "load(\"//inner:macro.bzl\", \"inner_macro\")\ndef _impl(name, visibility):\n\
         \x20   inner_macro(name = name)\nouter_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//outer:macro.bzl\", \"outer_macro\")\n\nouter_macro(name = \"foo\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo", &["//other_pkg:__pkg__", "//inner:__pkg__"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_doesNotApplyPackageDefaultVisibility
#[test]
fn test_declaration_visibility_unioning_does_not_apply_package_default_visibility() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    fake_cc_library(name = name)\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\"//lib:macro.bzl\", \"my_macro\")\n\
             package(default_visibility = [\"//other_pkg:__pkg__\"])\n\n\
             fake_cc_library(name = \"foo\")\nmy_macro(name = \"bar\")",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo", &["//other_pkg:__pkg__", "//pkg:__pkg__"]);
    // other_pkg doesn't propagate to bar, it only has its own instantiation location.
    assert_visibility_is(&pkg, "bar", &["//lib:__pkg__"]);
}

// Java: PackageFactoryTest.testImplicitVisibility_worksWithPackageDefaultVisibility
#[test]
fn test_implicit_visibility_works_with_package_default_visibility() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n\
         \x20   fake_cc_library(name = name, visibility = native.package_default_visibility())\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        &with_fake_load(
            "load(\"//lib:macro.bzl\", \"my_macro\")\n\
             package(default_visibility = [\"//other_pkg:__pkg__\"])\n\n\
             fake_cc_library(name = \"foo\")\nmy_macro(name = \"bar\")",
        ),
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo", &["//other_pkg:__pkg__", "//pkg:__pkg__"]);
    // Package default visibility is propagated to bar via native.package_default_visibility()
    // Visibility to the package where the macro is defined is propagated implicitly.
    assert_visibility_is(
        &pkg,
        "bar",
        &["//lib:__pkg__", "//other_pkg:__pkg__", "//pkg:__pkg__"],
    );
}

// Java: PackageFactoryTest.testPackageDefaultVisibility_playsWellWithPrivateVisibility
#[test]
fn test_package_default_visibility_plays_well_with_private_visibility() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n\
         \x20   fake_cc_library(name = name, visibility = native.package_default_visibility())\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\
         package(default_visibility = [\"//visibility:private\"])\n\nmy_macro(name = \"bar\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "bar", &["//lib:__pkg__", "//pkg:__pkg__"]);
}

// Java: PackageFactoryTest.testPackageDefaultVisibility_succeedsIfNoDefaultVisibilitySet
#[test]
fn test_package_default_visibility_succeeds_if_no_default_visibility_set() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n\
         \x20   fake_cc_library(name = name, visibility = native.package_default_visibility())\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\nmy_macro(name = \"bar\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "bar", &["//lib:__pkg__", "//pkg:__pkg__"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_worksWithPublicPrivateAndDuplicateVisibilities
#[test]
fn test_declaration_visibility_unioning_works_with_public_private_and_duplicate_visibilities() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    fake_cc_library(\n        name = name + \"_public\",\n\
         \x20       visibility = [\"//visibility:public\"],\n    )\n\
         \x20   fake_cc_library(\n        name = name + \"_private\",\n\
         \x20       visibility = [\"//visibility:private\"],\n    )\n\
         \x20   fake_cc_library(\n        name = name + \"_selfvisible\",\n\
         \x20       visibility = [\"//lib:__pkg__\"],\n    )\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\nmy_macro(name = \"foo\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo_public", &["//visibility:public"]);
    assert_visibility_is(&pkg, "foo_private", &["//lib:__pkg__"]);
    // The visibility concatenation operation does not add any label that would duplicate an
    // existing one.
    assert_visibility_is(&pkg, "foo_selfvisible", &["//lib:__pkg__"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_appliesToExportsFiles
#[test]
fn test_declaration_visibility_unioning_applies_to_exports_files() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    scratch_text(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    native.exports_files([name + \"_exported\"])\n\
         \x20   native.exports_files([name + \"_internal\"], visibility = [\"//visibility:private\"])\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\nmy_macro(name = \"foo\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo_exported", &["//visibility:public"]);
    assert_visibility_is(&pkg, "foo_internal", &["//lib:__pkg__"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_hasNoEffectOnPackageGroups
#[test]
fn test_declaration_visibility_unioning_has_no_effect_on_package_groups() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    scratch_text(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    native.package_group(name = name)\n\
         my_macro = macro(implementation = _impl)",
    );
    scratch_text(
        &t,
        "pkg/BUILD",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\nmy_macro(name = \"foo\")",
    );
    let pkg = t.get_package_without_errors("pkg");
    assert_visibility_is(&pkg, "foo", &["//visibility:public"]);
}

// Java: PackageFactoryTest.testDeclarationVisibilityUnioning_failsGracefullyOnInvalidVisibility
#[test]
fn test_declaration_visibility_unioning_fails_gracefully_on_invalid_visibility() {
    let t = new_private_visibility_test_case();
    t.scratch_file("lib/BUILD", &[]);
    lib_macro_bzl(
        &t,
        "lib/macro.bzl",
        "def _impl(name, visibility):\n    fake_cc_library(\n        name = name,\n\
         \x20       visibility = [\"//visibility:not_a_valid_specifier\"],\n    )\n\
         my_macro = macro(implementation = _impl)",
    );
    expect_eval_error_lines(
        &t,
        "//pkg:foo Invalid visibility label '//visibility:not_a_valid_specifier'",
        "load(\"//lib:macro.bzl\", \"my_macro\")\n\nmy_macro(name = \"foo\")",
    );
}
