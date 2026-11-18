//! Port of SelectTest (tests of the `select` function and data type) on the PACKAGE harness.
//!
//! Java: src/test/java/com/google/devtools/build/lib/packages/SelectTest.java.
//!
//! Java evaluates each expression directly (`Starlark.eval` with
//! `StarlarkGlobalsImpl.INSTANCE.getUtilToplevels()` and `StarlarkSemantics.DEFAULT`). Here the
//! expression is evaluated in a BUILD file under the default semantics
//! (`--incompatible_resolve_select_keys_eagerly` is off, so the string keys stay strings, as in
//! Java's context-free evaluation):
//! - `assertFails(expr, wantError)`: `x = <expr>` puts the package in error and an event contains
//!   `wantError`.
//! - `eval("repr(...)")`: `print(repr(...))`, and the DEBUG event's message is the value.
//!
//! `testSelect`, `testPlus` and `testKeyResolution` assert on the `SelectorList` object
//! (`getElements()`, `SelectorValue.getDictionary()`), which a package load cannot observe; their
//! stubs stay in `crates/blaze_packages/src/build/loading_tests.rs` (module `select_test`).

use crate::tests::package_loading_test_case::PackageLoadingTestCase;

// Java: SelectTest.assertFails
fn assert_fails(expr: &str, want_error: &str) {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &[format!("x = {expr}").as_str()]);
    let pkg = t.get_package("pkg");
    assert!(
        pkg.package.contains_errors(),
        "{expr} did not fail; events: {:#?}",
        pkg.messages()
    );
    pkg.assert_contains_event(want_error);
}

// Java: SelectTest.eval, for an expression that yields a string.
fn eval_string(expr: &str) -> String {
    let t = PackageLoadingTestCase::new();
    t.scratch_file("pkg/BUILD", &[format!("print({expr})").as_str()]);
    let pkg = t.get_package_without_errors("pkg");
    let mut printed = pkg.debug_events();
    assert_eq!(printed.len(), 1, "{printed:#?}");
    printed.remove(0)
}

// Java: SelectTest.testPlusIncompatibleType
// Holds the Java test without its last two assertions (SelectTest.java:111-114, the
// interpreter's "unsupported binary operation" text), which are in
// `test_plus_incompatible_type_interpreter_text`.
#[test]
fn test_plus_incompatible_type() {
    assert_fails(
        "select({'foo': ['FOO'], 'bar': ['BAR']}) + 1",
        "Cannot combine incompatible types (select of list, int)",
    );
    assert_fails(
        "select({'foo': ['FOO']}) + select({'bar': 2})",
        "Cannot combine incompatible types (select of list, select of int)",
    );

    assert_fails(
        "select({'foo': ['FOO']}) + select({'bar': {'a': 'a'}})",
        "Cannot combine incompatible types (select of list, select of dict)",
    );
    assert_fails(
        "select({'bar': {'a': 'a'}}) + select({'foo': ['FOO']})",
        "Cannot combine incompatible types (select of dict, select of list)",
    );
    assert_fails(
        "['FOO'] + select({'bar': {'a': 'a'}})",
        "Cannot combine incompatible types (list, select of dict)",
    );
    assert_fails(
        "select({'bar': {'a': 'a'}}) + ['FOO']",
        "Cannot combine incompatible types (select of dict, list)",
    );
}

// Java: SelectTest.testPlusIncompatibleType (its last two assertions, SelectTest.java:111-114:
// `select + dict` and `dict + select` fail with EvalUtils.java:417's text).
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'unsupported binary operation: select + dict'"]
fn test_plus_incompatible_type_interpreter_text() {
    assert_fails(
        "select({'foo': ['FOO']}) + {'a': 'a'}",
        "unsupported binary operation: select + dict",
    );
    assert_fails(
        "{'a': 'a'} + select({'foo': ['FOO']})",
        "unsupported binary operation: dict + select",
    );
}

// Java: SelectTest.testUnionIncompatibleType
// Holds the Java test without its third and fourth assertions (SelectTest.java:125-128, the
// interpreter's "unsupported binary operation" text), which are in
// `test_union_incompatible_type_interpreter_text`.
#[test]
fn test_union_incompatible_type() {
    assert_fails(
        "select({'foo': ['FOO']}) | select({'bar': {'a': 'a'}})",
        "Cannot combine incompatible types (select of list, select of dict)",
    );
    assert_fails(
        "select({'bar': {'a': 'a'}}) | select({'foo': ['FOO']})",
        "Cannot combine incompatible types (select of dict, select of list)",
    );
    assert_fails(
        "select({'foo': ['FOO']}) | {'a': 'a'}",
        "Cannot combine incompatible types (select of list, dict)",
    );
    assert_fails(
        "{'a': 'a'} | select({'foo': ['FOO']})",
        "Cannot combine incompatible types (dict, select of list)",
    );
}

// Java: SelectTest.testUnionIncompatibleType (its third and fourth assertions,
// SelectTest.java:125-128: `list | select` and `select | list` fail with EvalUtils.java:417's
// text).
#[test]
#[ignore = "starlark-rust core-language error text (decisions.md 2026-11-17; patch 0010 queued): Java 'unsupported binary operation: list | select'"]
fn test_union_incompatible_type_interpreter_text() {
    assert_fails(
        "['FOO'] | select({'bar': {'a': 'a'}})",
        "unsupported binary operation: list | select",
    );
    assert_fails(
        "select({'bar': {'a': 'a'}}) | ['FOO']",
        "unsupported binary operation: select | list",
    );
}

// Java: SelectTest.testRepr
#[test]
fn test_repr() {
    assert_eq!(
        eval_string("repr(select({'foo': ['FOO']})+['BAR'])"),
        "select({\"foo\": [\"FOO\"]}) + [\"BAR\"]"
    );

    assert_eq!(
        eval_string("repr(['FOO']+select({'bar': ['BAR']}))"),
        "[\"FOO\"] + select({\"bar\": [\"BAR\"]})"
    );

    assert_eq!(
        eval_string("repr(select({'foo': ['FOO']})+select({'bar': ['BAR']}))"),
        "select({\"foo\": [\"FOO\"]}) + select({\"bar\": [\"BAR\"]})"
    );

    assert_eq!(
        eval_string("repr(select({'foo': {'FOO': 123}})|{'BAR': 456})"),
        "select({\"foo\": {\"FOO\": 123}}) | {\"BAR\": 456}"
    );

    assert_eq!(
        eval_string("repr({'FOO': 123}|select({'bar': {'BAR': 456}}))"),
        "{\"FOO\": 123} | select({\"bar\": {\"BAR\": 456}})"
    );

    assert_eq!(
        eval_string("repr(select({'foo': {'FOO': 123}})|select({'bar': {'BAR': 456}}))"),
        "select({\"foo\": {\"FOO\": 123}}) | select({\"bar\": {\"BAR\": 456}})"
    );
}
