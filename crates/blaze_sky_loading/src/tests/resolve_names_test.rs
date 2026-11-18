//! Name resolution of BUILD and .bzl files (Java `Program.compileFile`: the Resolver reports
//! every undefined name of a file, and the file is not executed).
//!
//! No JUnit test of the pin has more than one undefined name in a file; the expected events
//! (count, order, locations, the failure message) come from the oracle:
//! scripts/w4/scenarios/load-unresolved-names/ORACLE.txt. The message of each event is
//! starlark-rust's ("Variable `x` not found"; Java "name 'x' is not defined",
//! decisions.md 2026-11-17), so only the name is checked.

use blaze_events::{Event, EventKind};

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

/// (file name, line, column, name) of every located error event, in order.
fn error_locations(events: &[Event]) -> Vec<(String, i32, i32, String)> {
    events
        .iter()
        .filter(|e| e.get_kind() == EventKind::Error)
        .filter_map(|e| {
            let location = e.get_location()?;
            let file = location.file();
            let file = file[file.rfind("/").map_or(0, |i| i + 1)..].to_owned();
            let message = e.get_message().into_owned();
            let name = blaze_packages::builtins::env::undefined_variable_name(&message)
                .unwrap_or_else(|| panic!("not an undefined-name error: {message:?}"))
                .to_owned();
            Some((file, location.line(), location.column(), name))
        })
        .collect()
}

fn loc(file: &str, line: i32, column: i32, name: &str) -> (String, i32, i32, String) {
    (file.to_owned(), line, column, name.to_owned())
}

/// Oracle step 1: three errors at BUILD.bazel:3:5, 3:21 and 5:1; the `print` before them does
/// not run; the package contains errors.
#[test]
fn every_unresolved_name_of_a_build_file_is_reported() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &[
            "print(\"not executed\")",
            "x = undefined_one + undefined_two",
            "",
            "undefined_rule(name = \"t\")",
        ],
    );
    let events = t.load_failure_events("pkg");
    assert_eq!(
        error_locations(&events),
        [
            loc("BUILD", 2, 5, "undefined_one"),
            loc("BUILD", 2, 21, "undefined_two"),
            loc("BUILD", 4, 1, "undefined_rule"),
        ]
    );
    assert!(
        !events
            .iter()
            .any(|e| e.get_message().contains("not executed")),
        "the BUILD file was executed"
    );
}

/// Oracle step 2: three errors at defs.bzl:1:5, 4:12 and 4:23 (one at the top level, two in a
/// function body), then "compilation of module 'bzl_names/defs.bzl' failed".
#[test]
fn every_unresolved_name_of_a_bzl_file_is_reported() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &["load(\":defs.bzl\", \"y\")", "", "filegroup(name = \"t\")"],
    );
    t.scratch_file(
        "pkg/defs.bzl",
        &[
            "x = nope_one",
            "",
            "def f():",
            "    return nope_two + nope_three",
            "",
            "y = len([])",
        ],
    );
    let events = t.load_failure_events("pkg");
    assert_eq!(
        error_locations(&events),
        [
            loc("defs.bzl", 1, 5, "nope_one"),
            loc("defs.bzl", 4, 12, "nope_two"),
            loc("defs.bzl", 4, 23, "nope_three"),
        ]
    );
    let exception = t.get_package_exception("pkg");
    assert!(
        exception
            .to_string()
            .contains("compilation of module 'pkg/defs.bzl' failed"),
        "{exception}"
    );
}

/// Oracle step 3: the undefined name is reported and the load fails with the compile error;
/// the missing file the .bzl loads is never requested (Java resolves in BzlCompileFunction,
/// before BzlLoadFunction looks at the loads).
#[test]
fn unresolved_names_are_reported_before_loads() {
    let t = PackageLoadingTestCase::new();
    t.scratch_file(
        "pkg/BUILD",
        &["load(\":defs.bzl\", \"x\")", "", "filegroup(name = \"t\")"],
    );
    t.scratch_file(
        "pkg/defs.bzl",
        &["load(\"//nosuch:file.bzl\", \"z\")", "", "x = nope_one"],
    );
    let events = t.load_failure_events("pkg");
    assert_eq!(
        error_locations(&events),
        [loc("defs.bzl", 3, 5, "nope_one")]
    );
    let exception = t.get_package_exception("pkg");
    assert!(
        exception
            .to_string()
            .contains("compilation of module 'pkg/defs.bzl' failed"),
        "{exception}"
    );
}
