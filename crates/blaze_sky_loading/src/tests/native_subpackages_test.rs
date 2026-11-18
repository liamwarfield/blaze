//! Port of NativeSubpackagesTest (tests for the `native.subpackages` function) on the PACKAGE
//! harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/NativeSubpackagesTest.java.
//!
//! - Java reads the attribute through `ConfiguredAttributeMapper` of the configured target; the
//!   attribute has no `select()` here, so its raw value in the loaded package is the same list.
//! - Java's `AssertionError`s are the fail-fast handler's, raised for the ERROR event of loading;
//!   the message asserted is the event's.
//! - The `subpackages_incremental_*` tests load a second time from scratch here (a fresh
//!   evaluator), so Java's `invalidateSkyFrameFiles` has no counterpart: the assertions on both
//!   loads are kept, Skyframe invalidation is not exercised.

use crate::tests::package_loading_test_case::{PackageLoadingTestCase, sorted};

const ALL_SUBDIRS: &str = "**";

/// Constructs a BUILD file with a single filegroup target whose srcs attribute is the list of all
/// //p:files, where //p is a subpackage returned by a call to native.subpackages.
// Java: NativeSubpackagesTest.makeSubpackageFileGroup
fn make_subpackage_file_group(
    t: &PackageLoadingTestCase,
    build_path: &str,
    include: &[&str],
    exclude: Option<&str>,
    allow_empty: Option<bool>,
) {
    let mut subpackages = String::new();
    subpackages.push_str("subpackages(include = [");
    let quoted: Vec<String> = include.iter().map(|i| format!("'{i}'")).collect();
    subpackages.push_str(&quoted.join(", "));
    subpackages.push(']');

    if let Some(exclude) = exclude {
        subpackages.push_str(", exclude = ");
        subpackages.push_str(exclude);
    }

    if let Some(allow_empty) = allow_empty {
        subpackages.push_str(", allow_empty = ");
        subpackages.push_str(if allow_empty { "True" } else { "False" });
    }
    subpackages.push(')');

    t.scratch_file(
        build_path,
        &[
            "filegroup(",
            "   name = 'files',",
            "   srcs = [",
            format!("     '//%s/%s:files' % (package_name(), s) for s in {subpackages}").as_str(),
            "   ],",
            ")",
        ],
    );
}

/// Creates a BUILD file and single file at the given packagePath, the BUILD file will contain a
/// single filegroup called 'files' which contains the created file.
// Java: NativeSubpackagesTest.makeFilesSubPackage
fn make_files_sub_package(t: &PackageLoadingTestCase, package_path: &str) {
    t.scratch_file(&format!("{package_path}/file.txt"), &[]);
    t.scratch_file(
        &format!("{package_path}/BUILD"),
        &[
            "filegroup(",
            "   name = 'files',",
            "   srcs = glob(['*']),",
            ")",
        ],
    );
}

// Java: NativeSubpackagesTest.assertAttrLabelList, for `//test/starlark:files`
// (`containsExactlyElementsIn`, no order).
fn assert_attr_label_list(t: &PackageLoadingTestCase, attr_name: &str, expected_labels: &[&str]) {
    let loaded = t.get_package_without_errors("test/starlark");
    let expected: Vec<String> = expected_labels.iter().map(|l| (*l).to_owned()).collect();
    assert_eq!(
        sorted(loaded.label_list_attr("files", attr_name)),
        sorted(expected)
    );
}

/// An ERROR event of loading `pkg` contains `expected` (Java: the fail-fast handler's
/// AssertionError has the message).
fn assert_loading_error(t: &PackageLoadingTestCase, pkg: &str, expected: &str) {
    let loaded = t.get_package(pkg);
    let errors = loaded.error_events();
    assert!(
        errors.iter().any(|e| e.contains(expected)),
        "no error contains {expected:?}: {errors:#?}"
    );
}

// Java: NativeSubpackagesTest.subpackages_simple_subDir
#[test]
fn subpackages_simple_sub_dir() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, None);
    make_files_sub_package(&t, "test/starlark/sub");

    assert_attr_label_list(&t, "srcs", &["//test/starlark/sub:files"]);
}

// Java: NativeSubpackagesTest.subpackages_simple_include
#[test]
fn subpackages_simple_include() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &["sub1"], None, None);

    make_files_sub_package(&t, "test/starlark/sub");
    make_files_sub_package(&t, "test/starlark/sub1");
    make_files_sub_package(&t, "test/starlark/sub2");

    assert_attr_label_list(&t, "srcs", &["//test/starlark/sub1:files"]);
}

// Java: NativeSubpackagesTest.subpackages_simple_exclude
#[test]
fn subpackages_simple_exclude() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(
        &t,
        "test/starlark/BUILD",
        &[ALL_SUBDIRS],
        Some("['sub2/**']"),
        None,
    );

    make_files_sub_package(&t, "test/starlark/sub");
    make_files_sub_package(&t, "test/starlark/sub1");
    make_files_sub_package(&t, "test/starlark/sub2");
    make_files_sub_package(&t, "test/starlark/sub3");

    assert_attr_label_list(
        &t,
        "srcs",
        &[
            "//test/starlark/sub:files",
            "//test/starlark/sub1:files",
            "//test/starlark/sub3:files",
        ],
    );
}

// Java: NativeSubpackagesTest.subpackages_simple_empty_allow
#[test]
fn subpackages_simple_empty_allow() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, Some(true));
    assert_attr_label_list(&t, "srcs", &[]);
}

// Java: NativeSubpackagesTest.subpackages_simple_empty_disallow
#[test]
fn subpackages_simple_empty_disallow() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, None);

    // force evaluation
    assert_loading_error(
        &t,
        "test/starlark",
        "subpackages pattern '**' didn't match anything",
    );
}

// Java: NativeSubpackagesTest.subpackages_deeplyNested_withSubdirs
#[test]
fn subpackages_deeply_nested_with_subdirs() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, Some(true));

    // Setup a dir with 2 subdirs, 1 a package one not
    make_files_sub_package(&t, "test/starlark/sub");
    // Should be blocked by 'sub'
    make_files_sub_package(&t, "test/starlark/sub/sub2");

    make_files_sub_package(&t, "test/starlark/sub3");
    make_files_sub_package(&t, "test/starlark/not_sub/sub_is_pkg/eventually");

    t.scratch_file("test/starlark/not_sub/file1.txt", &[]);
    t.scratch_file("test/starlark/not_sub/double_not_sub/file.txt", &[]);

    assert_attr_label_list(
        &t,
        "srcs",
        &[
            "//test/starlark/sub:files",
            "//test/starlark/sub3:files",
            "//test/starlark/not_sub/sub_is_pkg/eventually:files",
        ],
    );
}

// Java: NativeSubpackagesTest.subpackages_incremental_addSubPkg
#[test]
fn subpackages_incremental_add_sub_pkg() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, None);

    // Setup a two subdirs one shallow and one deep
    make_files_sub_package(&t, "test/starlark/sub");
    make_files_sub_package(&t, "test/starlark/deep/1/2/3");

    assert_attr_label_list(
        &t,
        "srcs",
        &[
            "//test/starlark/sub:files",
            "//test/starlark/deep/1/2/3:files",
        ],
    );

    // Add a 2nd shallow and 2nd deep mid
    make_files_sub_package(&t, "test/starlark/sub2");

    // We should now be aware of the new one via Skyframe invalidation.
    assert_attr_label_list(
        &t,
        "srcs",
        &[
            "//test/starlark/sub:files",
            "//test/starlark/sub2:files",
            "//test/starlark/deep/1/2/3:files",
        ],
    );
}

// Java: NativeSubpackagesTest.subpackages_incremental_delSubPkg
#[test]
fn subpackages_incremental_del_sub_pkg() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, None);

    // Setup a single subdir
    make_files_sub_package(&t, "test/starlark/sub");
    make_files_sub_package(&t, "test/starlark/sub2");

    assert_attr_label_list(
        &t,
        "srcs",
        &["//test/starlark/sub:files", "//test/starlark/sub2:files"],
    );

    t.delete_file("test/starlark/sub2/BUILD");
    t.delete_file("test/starlark/sub2/file.txt");

    // We should now be aware of the new one.
    assert_attr_label_list(&t, "srcs", &["//test/starlark/sub:files"]);
}

// Java: NativeSubpackagesTest.subpackages_incremental_convertSubDirToPkg
#[test]
fn subpackages_incremental_convert_sub_dir_to_pkg() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &[ALL_SUBDIRS], None, None);

    // Setup both immediate and deeply nested sub-dirs with BUILD files.
    make_files_sub_package(&t, "test/starlark/sub");
    t.scratch_file("test/starlark/sub2/file2.txt", &[]);

    // Initially we have a subdir with 'sub/BUILD' and sub2/file2.txt"
    assert_attr_label_list(&t, "srcs", &["//test/starlark/sub:files"]);

    // Then we add a BUILD file to sub2 making it a package Skyframe should pick
    // that up once invalidated.
    make_files_sub_package(&t, "test/starlark/sub2");

    assert_attr_label_list(
        &t,
        "srcs",
        &["//test/starlark/sub:files", "//test/starlark/sub2:files"],
    );
}

// Java: NativeSubpackagesTest.invalidPositionalParams
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'got unexpected positional argument' (argument binding)"]
fn invalid_positional_params() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("foo/subdir/BUILD", &[]);
    t.scratch_file(
        "foo/BUILD",
        &["[filegroup(name = p) for p in subpackages(['subdir'])]"],
    );

    assert_loading_error(&t, "foo", "got unexpected positional argument");
}

// Java: NativeSubpackagesTest.invalidMissingInclude
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'missing 1 required named argument: include' (argument binding)"]
fn invalid_missing_include() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("foo/subdir/BUILD", &[]);
    t.scratch_file(
        "foo/BUILD",
        &["[filegroup(name = p) for p in subpackages()]"],
    );

    assert_loading_error(&t, "foo", "missing 1 required named argument: include");
}

// Java: NativeSubpackagesTest.validNoWildCardInclude
#[test]
fn valid_no_wild_card_include() {
    let t = PackageLoadingTestCase::new();
    make_subpackage_file_group(&t, "test/starlark/BUILD", &["sub", "sub2/deep"], None, None);
    make_files_sub_package(&t, "test/starlark/sub");
    make_files_sub_package(&t, "test/starlark/sub2/deep");

    assert_attr_label_list(
        &t,
        "srcs",
        &[
            "//test/starlark/sub:files",
            "//test/starlark/sub2/deep:files",
        ],
    );
}

// Java: NativeSubpackagesTest.includeValidMatchSubdir
#[test]
fn include_valid_match_subdir() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("foo/subdir/BUILD", &[]);
    t.scratch_file(
        "foo/BUILD",
        &["[filegroup(name = p) for p in subpackages(include = ['subdir'])]"],
    );
    t.get_package_without_errors("foo").target("subdir");
}

// Java: NativeSubpackagesTest.includeValidSubMatchSubdir (@TestParameter: the five expressions)
#[test]
fn include_valid_sub_match_subdir() {
    for expression in [
        "subdir/*/deeper",
        "subdir/sub*/deeper",
        "subdir/**",
        "subdir/*/deeper/**",
        "subdir/**/deeper/**",
    ] {
        let t = PackageLoadingTestCase::new();
        make_files_sub_package(&t, "test/starlark/subdir/sub/deeper");
        make_files_sub_package(&t, "test/starlark/subdir/sub2/deeper");
        make_files_sub_package(&t, "test/starlark/subdir/sub3/deeper");

        make_subpackage_file_group(&t, "test/starlark/BUILD", &[expression], None, None);

        assert_attr_label_list(
            &t,
            "srcs",
            &[
                "//test/starlark/subdir/sub/deeper:files",
                "//test/starlark/subdir/sub2/deeper:files",
                "//test/starlark/subdir/sub3/deeper:files",
            ],
        );
    }
}
