//! Port of InputFileTest on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/InputFileTest.java.

use std::sync::Arc;

use crate::tests::package_loading_test_case::{LoadedPackage, PackageLoadingTestCase};

// Java: InputFileTest.writeFiles (@Before)
fn write_files() -> (PackageLoadingTestCase, LoadedPackage, String, String) {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &["genrule(name='dummy', cmd='', outs=[], srcs=['x', 'subdir/y'])"],
    );
    let pkg = t.get_package("pkg");
    pkg.assert_no_events();

    t.scratch_file("pkg/x", &["blah"]);
    t.scratch_file("pkg/subdir/y", &["blah blah"]);
    let path_x = t.workspace_path("pkg/x");
    let path_y = t.workspace_path("pkg/subdir/y");
    (t, pkg, path_x, path_y)
}

// Java: InputFileTest.testGetAssociatedRule
#[test]
fn test_get_associated_rule() {
    let (_t, pkg, _, _) = write_files();
    assert!(pkg.target("x").get_associated_rule().is_none());
}

// Java: InputFileTest.testInputFileInPackageDirectory
#[test]
fn test_input_file_in_package_directory() {
    let (_t, pkg, path_x, _) = write_files();
    let target_x = pkg.target("x");
    let input_file_x = target_x.as_input_file().expect("x is an input file");
    assert_eq!(input_file_x.get_path().get_path_string(), path_x);
    assert_eq!(input_file_x.get_name(), "x");
    assert_eq!(input_file_x.get_label().to_string(), "//pkg:x");
    assert_eq!(target_x.get_target_kind(), "source file");
}

// Java: InputFileTest.testInputFileInSubdirectory
#[test]
fn test_input_file_in_subdirectory() {
    let (_t, pkg, _, path_y) = write_files();
    let target_y = pkg.target("subdir/y");
    let input_file_y = target_y.as_input_file().expect("subdir/y is an input file");
    assert_eq!(input_file_y.get_path().get_path_string(), path_y);
    assert_eq!(input_file_y.get_name(), "subdir/y");
    assert_eq!(input_file_y.get_label().to_string(), "//pkg:subdir/y");
}

// Java: InputFileTest.testEquivalenceRelation
#[test]
fn test_equivalence_relation() {
    let (_t, pkg, _, _) = write_files();
    let input_file_x = pkg.target("x").as_input_file().expect("an input file");
    assert!(Arc::ptr_eq(
        input_file_x,
        pkg.target("x").as_input_file().expect("an input file")
    ));
    let input_file_y = pkg
        .target("subdir/y")
        .as_input_file()
        .expect("an input file");
    assert!(Arc::ptr_eq(
        input_file_y,
        pkg.target("subdir/y")
            .as_input_file()
            .expect("an input file")
    ));
    // Java: EqualsTester with one equality group per file.
    assert!(pkg.target("x") == pkg.target("x"));
    assert!(pkg.target("subdir/y") == pkg.target("subdir/y"));
    assert!(pkg.target("x") != pkg.target("subdir/y"));
    assert!(pkg.target("subdir/y") != pkg.target("x"));
}

// Java: InputFileTest.testReduceForSerialization
#[test]
#[ignore = "Target.reduceForSerialization (ReducedTargetData) is not ported in blaze_packages"]
fn test_reduce_for_serialization() {}
