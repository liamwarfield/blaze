//! Tests of the `select()` value (Java `SelectorList.select`, `SelectorList.binaryOp`,
//! `SelectorList.repr`); Bazel's coverage is in its Starlark integration tests.

use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::repository_mapping::EMPTY;
use starlark::values::Value;

use crate::types::LabelConverter;
use crate::types::SelectorList;
use crate::types::select_impl;
use crate::types::test_util::eval_expr;
use crate::types::test_util::with_eval;

fn select_err(code: &str, lc: Option<&LabelConverter>) -> String {
    with_eval(|eval| {
        let d = eval_expr(eval, code);
        let e = select_impl(d, "", lc, eval.heap()).unwrap_err();
        blaze_starlark::error::bare_message(&e)
    })
}

#[test]
fn select_empty_dict() {
    assert_eq!(
        select_err("{}", None),
        "select({}) with an empty dictionary can never resolve because it includes no \
         conditions to match"
    );
}

#[test]
fn select_bad_key() {
    assert_eq!(
        select_err("{1: 2}", None),
        "select: got int for dict key, want a Label or label string"
    );
}

#[test]
fn select_invalid_label() {
    let lc = LabelConverter::new(
        &PackageIdentifier::create_in_main_repo_str("quux"),
        EMPTY.clone(),
    );
    let e = select_err("{'//a:../b': 1}", Some(&lc));
    assert!(e.starts_with("invalid label in select(): "), "{e}");
}

#[test]
fn select_repr_and_concatenation() {
    with_eval(|eval| {
        let d = eval_expr(
            eval,
            "{'//conditions:a': ['x'], '//conditions:default': []}",
        );
        let s = select_impl(d, "", None, eval.heap()).unwrap();
        assert_eq!(s.get_type(), "select");
        assert_eq!(
            s.to_repr(),
            "select({\"//conditions:a\": [\"x\"], \"//conditions:default\": []})"
        );
        let l = eval_expr(eval, "['y']");
        let sum: Value = s.add(l, eval.heap()).unwrap();
        assert_eq!(
            sum.to_repr(),
            "select({\"//conditions:a\": [\"x\"], \"//conditions:default\": []}) + [\"y\"]"
        );
        let rsum: Value = l.add(s, eval.heap()).unwrap();
        assert_eq!(
            rsum.to_repr(),
            "[\"y\"] + select({\"//conditions:a\": [\"x\"], \"//conditions:default\": []})"
        );
        assert_eq!(SelectorList::from_value(rsum).unwrap().get_type(), "list");
        // A string cannot be added to a select of lists.
        let str_value = eval.heap().alloc("z");
        let e = s.add(str_value, eval.heap()).unwrap_err();
        assert_eq!(
            blaze_starlark::error::bare_message(&e),
            "Cannot combine incompatible types (select of list, string)"
        );
    });
}

#[test]
fn select_dict_union() {
    with_eval(|eval| {
        let d = eval_expr(eval, "{'//conditions:default': {'a': 1}}");
        let s = select_impl(d, "", None, eval.heap()).unwrap();
        let other = eval_expr(eval, "{'b': 2}");
        let union = s.bit_or(other, eval.heap()).unwrap();
        assert_eq!(
            union.to_repr(),
            "select({\"//conditions:default\": {\"a\": 1}}) | {\"b\": 2}"
        );
    });
}
