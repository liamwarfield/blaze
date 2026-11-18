//! Port of ExportsFilesTest on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/ExportsFilesTest.java.

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase};

// Java: ExportsFilesTest.pkg
fn pkg(t: &PackageLoadingTestCase) -> LoadedPackage {
    t.scratch_file("pkg/BUILD", &["exports_files(['foo.txt', 'bar.txt'])"]);
    t.get_package_without_errors("pkg")
}

/// Returns the names of the input files that are known to pkg.
// Java: ExportsFilesTest.getFileNamesOf
fn get_file_names_of(pkg: &LoadedPackage) -> Vec<String> {
    let mut names: Vec<String> = pkg
        .package
        .get_targets()
        .values()
        .filter(|target| target.is_file())
        .map(|target| target.get_label().to_string())
        .collect();
    names.sort();
    names
}

// Java: ExportsFilesTest.testExportsFilesRegistersFilesWithPackage
#[test]
fn test_exports_files_registers_files_with_package() {
    let t = PackageLoadingTestCase::new();
    let names = get_file_names_of(&pkg(&t));
    let expected = "//pkg:BUILD //pkg:bar.txt //pkg:foo.txt";
    assert_eq!(names.join(" "), expected);
}

// Java: ExportsFilesTest.testFileThatsNotRegisteredYieldsUnknownTargetException
#[test]
fn test_file_thats_not_registered_yields_unknown_target_exception() {
    let t = PackageLoadingTestCase::new();
    let pkg = pkg(&t);
    let e = pkg
        .package
        .get_target("baz.txt")
        .expect_err("a NoSuchTargetException");
    assert!(
        e.get_message().contains(
            "no such target '//pkg:baz.txt': target 'baz.txt' not declared in package 'pkg'"
        ),
        "{}",
        e.get_message()
    );
}

// Java: ExportsFilesTest.testRegisteredFilesAreRetrievable
#[test]
fn test_registered_files_are_retrievable() {
    let t = PackageLoadingTestCase::new();
    let pkg = pkg(&t);
    assert_eq!(pkg.target("foo.txt").get_name(), "foo.txt");
    assert_eq!(pkg.target("bar.txt").get_name(), "bar.txt");
}

// Java: ExportsFilesTest.testExportsFilesAndRuleNameConflict
#[test]
fn test_exports_files_and_rule_name_conflict() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg2/BUILD",
        &[
            "exports_files([\"foo\"])",
            "",
            "genrule(",
            "    name = \"foo\",",
            "    srcs = [\"bar\"],",
            "    outs = [],",
            "    cmd = \"/bin/true\",",
            ")",
        ],
    );
    let pkg = t.get_package("pkg2");
    assert!(pkg.target("foo").is_input_file());
    pkg.assert_contains_event("rule 'foo' conflicts with existing source file");
}
