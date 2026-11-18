//! Port of OutputFileTest on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/OutputFileTest.java.

use std::sync::Arc;

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase};

// Java: OutputFileTest.createRule (@Before). The rule is `pkg.rule("foo")`.
fn create_rule() -> (PackageLoadingTestCase, LoadedPackage) {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &["genrule(name='foo', srcs=[], cmd='', outs=['x', 'subdir/y'])"],
    );
    let pkg = t.get_package("pkg");
    let rule = pkg.target("foo").as_rule().expect("foo is a rule");
    assert!(Arc::ptr_eq(pkg.rule("foo"), rule));
    pkg.assert_no_events();
    (t, pkg)
}

// Java: OutputFileTest.checkTargetRetainsGeneratingRule
fn check_target_retains_generating_rule(pkg: &LoadedPackage, output: &str) {
    let output_file = pkg.target(output).as_output_file().expect("an output file");
    assert!(Arc::ptr_eq(
        output_file.get_generating_rule(),
        pkg.rule("foo")
    ));
}

// Java: OutputFileTest.testGetAssociatedRule
#[test]
fn test_get_associated_rule() {
    let (_t, pkg) = create_rule();
    let associated_rule = pkg
        .target("x")
        .get_associated_rule()
        .expect("an associated rule");
    assert!(Arc::ptr_eq(associated_rule, pkg.rule("foo")));
}

// Java: OutputFileTest.testOutputFileInPackageDir
#[test]
fn test_output_file_in_package_dir() {
    let (_t, pkg) = create_rule();
    let target_x = pkg.target("x");
    let output_file_x = target_x.as_output_file().expect("x is an output file");
    check_target_retains_generating_rule(&pkg, "x");
    assert_eq!(output_file_x.get_name(), "x");
    assert_eq!(output_file_x.get_label().to_string(), "//pkg:x");
    assert_eq!(target_x.get_target_kind(), "generated file");
}

// Java: OutputFileTest.testOutputFileInSubdirectory
#[test]
fn test_output_file_in_subdirectory() {
    let (_t, pkg) = create_rule();
    let target_y = pkg.target("subdir/y");
    let output_file_y = target_y
        .as_output_file()
        .expect("subdir/y is an output file");
    check_target_retains_generating_rule(&pkg, "subdir/y");
    assert_eq!(output_file_y.get_name(), "subdir/y");
    assert_eq!(output_file_y.get_label().to_string(), "//pkg:subdir/y");
}

// Java: OutputFileTest.testEquivalenceRelation
#[test]
fn test_equivalence_relation() {
    let (_t, pkg) = create_rule();
    let output_file_x1 = pkg.target("x").as_output_file().expect("an output file");
    let output_file_x2 = pkg.target("x").as_output_file().expect("an output file");
    let output_file_y1 = pkg
        .target("subdir/y")
        .as_output_file()
        .expect("an output file");
    let output_file_y2 = pkg
        .target("subdir/y")
        .as_output_file()
        .expect("an output file");
    assert!(Arc::ptr_eq(output_file_x2, output_file_x1));
    assert!(Arc::ptr_eq(output_file_y2, output_file_y1));
    // Java: EqualsTester with the groups (x1, x2) and (y1, y2).
    assert!(pkg.target("x") == pkg.target("x"));
    assert!(pkg.target("subdir/y") == pkg.target("subdir/y"));
    assert!(pkg.target("x") != pkg.target("subdir/y"));
    assert!(pkg.target("subdir/y") != pkg.target("x"));
}

// Java: OutputFileTest.testDuplicateOutputFilesInDifferentRules
#[test]
fn test_duplicate_output_files_in_different_rules() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "two_outs/BUILD",
        &[
            "genrule(",
            "    name = \"a\",",
            "    outs = [\"out\"],",
            "    cmd = \"ls >$(location out)\",",
            ")",
            "",
            "genrule(",
            "    name = \"b\",",
            "    outs = [\"out\"],",
            "    cmd = \"ls >$(location out)\",",
            ")",
        ],
    );

    let pkg = t.get_package("two_outs");
    pkg.target("BUILD");
    pkg.assert_contains_event(
        "generated file 'out' in rule 'b' conflicts with existing generated file from rule 'a'",
    );
}

// Java: OutputFileTest.testOutputFileNameConflictsWithExistingRule
#[test]
fn test_output_file_name_conflicts_with_existing_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "out_is_rule/BUILD",
        &[
            "genrule(",
            "    name = \"a\",",
            "    outs = [\"out\"],",
            "    cmd = \"ls >$(location out)\",",
            ")",
            "",
            "genrule(",
            "    name = \"b\",",
            "    outs = [\"a\"],",
            "    cmd = \"ls >$(location out)\",",
            ")",
        ],
    );

    let pkg = t.get_package("out_is_rule");
    pkg.target("BUILD");
    pkg.assert_contains_event(
        "generated file 'a' in rule 'b' conflicts with existing genrule rule",
    );
}

// Java: OutputFileTest.testDuplicateOutputFilesInSameRule
#[test]
fn test_duplicate_output_files_in_same_rule() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "two_outs/BUILD",
        &[
            "genrule(",
            "    name = \"a\",",
            "    outs = [",
            "        \"out\",",
            "        \"out\",",
            "    ],",
            "    cmd = \"ls >$(location out)\",",
            ")",
        ],
    );
    let pkg = t.get_package("two_outs");
    pkg.target("BUILD");
    pkg.assert_contains_event("rule 'a' has more than one generated file named 'out'");
}

// Java: OutputFileTest.testOutputFileWithIllegalName
#[test]
fn test_output_file_with_illegal_name() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "bad_out_name/BUILD",
        &["genrule(name='a', cmd='ls', outs=['!@#:'])"],
    );
    let pkg = t.get_package("bad_out_name");
    pkg.target("BUILD");
    pkg.assert_contains_event("invalid label '!@#:'");
}

// Java: OutputFileTest.testOutputFileWithCrossPackageLabel
#[test]
fn test_output_file_with_cross_package_label() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "cross_package_out/BUILD",
        &["genrule(name='a', cmd='ls', outs=['//foo:bar'])"],
    );
    let pkg = t.get_package("cross_package_out");
    pkg.target("BUILD");
    pkg.assert_contains_event("label '//foo:bar' is not in the current package");
}

// Java: OutputFileTest.testOutputFileNamedBUILD
#[test]
fn test_output_file_named_build() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "output_called_build/BUILD",
        &["genrule(name='a', cmd='ls', outs=['BUILD'])"],
    );
    let pkg = t.get_package("output_called_build");
    pkg.target("BUILD");
    pkg.assert_contains_event(
        "generated file 'BUILD' in rule 'a' conflicts with existing source file",
    );
}

// Java: OutputFileTest.testReduceForSerialization
#[test]
#[ignore = "Target.reduceForSerialization (ReducedTargetData) is not ported in blaze_packages"]
fn test_reduce_for_serialization() {}
