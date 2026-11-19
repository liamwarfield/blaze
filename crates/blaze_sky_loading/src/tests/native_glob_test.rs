//! Port of NativeGlobTest (tests for the `native.glob` function) on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/NativeGlobTest.java.
//!
//! Java reads the attribute through `ConfiguredAttributeMapper` of the configured target; the
//! attribute has no `select()` here, so its raw value in the loaded package is the same list.

use crate::tests::package_loading_test_case::{PackageLoadingTestCase, sorted};

// Java: NativeGlobTest.makeFile
fn make_file(t: &PackageLoadingTestCase, file_name: &str) {
    t.scratch_file(file_name, &[format!("Content: {file_name}").as_str()]);
}

/// Constructs a BUILD file containing a single rule with uses glob() to list files look for a
/// rule called :files in it.
// Java: NativeGlobTest.makeGlobFilegroup
fn make_glob_filegroup(t: &PackageLoadingTestCase, build_path: &str, glob: &str) {
    t.scratch_file(
        build_path,
        &[
            "filegroup(",
            "   name = 'files',",
            format!("   srcs = {glob}").as_str(),
            ")",
        ],
    );
}

// Java: NativeGlobTest.assertAttrLabelList, for target `//<pkg>:<rule>`
// (`containsExactlyElementsIn`, no order).
fn assert_attr_label_list(
    t: &PackageLoadingTestCase,
    pkg: &str,
    rule: &str,
    attr_name: &str,
    expected_labels: &[&str],
) {
    let loaded = t.get_package_without_errors(pkg);
    let expected: Vec<String> = expected_labels.iter().map(|l| (*l).to_owned()).collect();
    assert_eq!(
        sorted(loaded.label_list_attr(rule, attr_name)),
        sorted(expected)
    );
}

// Java: NativeGlobTest.glob_simple
#[test]
fn glob_simple() {
    let t = PackageLoadingTestCase::new();
    make_file(&t, "test/starlark/file1.txt");
    make_file(&t, "test/starlark/file2.txt");
    make_file(&t, "test/starlark/file3.txt");

    make_glob_filegroup(&t, "test/starlark/BUILD", "glob(['*'])");

    assert_attr_label_list(
        &t,
        "test/starlark",
        "files",
        "srcs",
        &[
            "//test/starlark:BUILD",
            "//test/starlark:file1.txt",
            "//test/starlark:file2.txt",
            "//test/starlark:file3.txt",
        ],
    );
}

// Java: NativeGlobTest.glob_not_empty. Java's AssertionError is the fail-fast handler's, raised
// for the ERROR event of loading; its message is the event.
#[test]
fn glob_not_empty() {
    let t = PackageLoadingTestCase::new();
    make_glob_filegroup(
        &t,
        "test/starlark/BUILD",
        "glob(['foo*'], allow_empty=False)",
    );

    let pkg = t.get_package("test/starlark");
    let errors = pkg.error_events();
    assert!(
        errors.iter().any(|e| e.contains("allow_empty")),
        "{errors:#?}"
    );
}

// Java: NativeGlobTest.glob_simple_subdirs
#[test]
fn glob_simple_subdirs() {
    let t = PackageLoadingTestCase::new();
    make_file(&t, "test/starlark/sub/file1.txt");
    make_file(&t, "test/starlark/sub2/file2.txt");
    make_file(&t, "test/starlark/sub3/file3.txt");

    make_glob_filegroup(&t, "test/starlark/BUILD", "glob(['**'])");

    assert_attr_label_list(
        &t,
        "test/starlark",
        "files",
        "srcs",
        &[
            "//test/starlark:BUILD",
            "//test/starlark:sub/file1.txt",
            "//test/starlark:sub2/file2.txt",
            "//test/starlark:sub3/file3.txt",
        ],
    );
}

// Java: NativeGlobTest.glob_incremental. The second load is from scratch here (a fresh
// evaluator), so Java's `invalidateSkyFrameFiles` has no counterpart: the assertions on both
// loads are kept, Skyframe invalidation is not exercised.
#[test]
fn glob_incremental() {
    let t = PackageLoadingTestCase::new();
    make_file(&t, "test/starlark/file1.txt");
    make_glob_filegroup(&t, "test/starlark/BUILD", "glob(['**'])");

    assert_attr_label_list(
        &t,
        "test/starlark",
        "files",
        "srcs",
        &["//test/starlark:BUILD", "//test/starlark:file1.txt"],
    );

    t.scratch_file("test/starlark/file2.txt", &[]);
    t.scratch_file("test/starlark/sub/subfile3.txt", &[]);

    assert_attr_label_list(
        &t,
        "test/starlark",
        "files",
        "srcs",
        &[
            "//test/starlark:BUILD",
            "//test/starlark:file1.txt",
            "//test/starlark:file2.txt",
            "//test/starlark:sub/subfile3.txt",
        ],
    );
}

// Not a Java test (oracle, ~/.cache/blaze/work/bytes-C1): file names are internal strings (one
// char per byte), BUILD text is decoded one char per byte, and wildcards match chars. The file
// names are `é.txt` (C3 A9), a raw byte FF, `xÅy.txt` (C3 85: U+0085 is a Java line terminator,
// which `.` does not match) and `xéy.txt`.
#[test]
fn glob_non_ascii_file_names() {
    let t = PackageLoadingTestCase::new();
    for name in [
        "p/a.txt",
        "p/\u{c3}\u{a9}.txt",
        "p/\u{ff}x.txt",
        "p/x\u{c3}\u{85}y.txt",
        "p/x\u{c3}\u{a9}y.txt",
        "p/d\u{c3}\u{a9}/in.txt",
    ] {
        make_file(&t, name);
    }
    // The BUILD file is UTF-8 text: `é` in it is the two bytes C3 A9.
    t.scratch_file(
        "p/BUILD",
        &[
            "filegroup(name = 'g_txt', srcs = glob(['*.txt']))",
            "filegroup(name = 'g_e', srcs = glob(['é*']))",
            "filegroup(name = 'g_lt', srcs = glob(['x*y.txt']))",
            "filegroup(name = 'g_rec', srcs = glob(['**/*.txt']))",
        ],
    );
    let top = [
        "//p:a.txt",
        "//p:x\u{c3}\u{85}y.txt",
        "//p:x\u{c3}\u{a9}y.txt",
        "//p:\u{c3}\u{a9}.txt",
        "//p:\u{ff}x.txt",
    ];
    assert_attr_label_list(&t, "p", "g_txt", "srcs", &top);
    assert_attr_label_list(&t, "p", "g_e", "srcs", &["//p:\u{c3}\u{a9}.txt"]);
    assert_attr_label_list(&t, "p", "g_lt", "srcs", &["//p:x\u{c3}\u{a9}y.txt"]);
    let mut rec = top.to_vec();
    rec.push("//p:d\u{c3}\u{a9}/in.txt");
    assert_attr_label_list(&t, "p", "g_rec", "srcs", &rec);
}
