//! Port of
//! `src/test/java/com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitorTest.java`.

mod support;

use std::sync::Arc;

use blaze_query_engine::{
    AllRdepsFunction, QueryExpression, TotalWeightQueryExpressionVisitor, default_query_functions,
};
use support::MockEnvironment;

/// PORT: Java parses with a bare-minimum `SkyQueryEnvironment`; parsing only needs its function
/// list, which adds `allrdeps` (and SkyQuery-only functions these queries do not use) to the
/// default functions.
fn make_env() -> MockEnvironment {
    let mut functions = default_query_functions::<()>();
    functions.push(Arc::new(AllRdepsFunction::new()));
    MockEnvironment { functions }
}

fn calculate_query_weight(expr: &str) -> i64 {
    let env = make_env();
    let mut under_test = TotalWeightQueryExpressionVisitor::new();
    QueryExpression::parse::<()>(expr, &env)
        .unwrap()
        .accept(&mut under_test, &())
}

#[test]
fn basic_body_count() {
    assert_eq!(calculate_query_weight("$x - attr('tags', 'foo', $x)"), 11);
}

#[test]
fn nested_query_functions() {
    assert_eq!(
        calculate_query_weight("kind('pat', allrdeps(//a + //b + //c + //d + //e + //f))"),
        21
    );
}

#[test]
fn multiple_let_statements_same_variable_rebound() {
    assert_eq!(
        calculate_query_weight("let x = (let x = //... in $x - deps($x)) in $x - allrdeps($x)"),
        15
    );
}

#[test]
fn multiple_let_statements_same_variable_rebound_also_in_variable_expr() {
    assert_eq!(calculate_query_weight("let x = $x in $x - allrdeps($x)"), 7);
}

#[test]
fn multiple_let_statements_different_variable() {
    assert_eq!(
        calculate_query_weight("let x = (let y = //... in $x - deps($y)) in $x - allrdeps($y)"),
        15
    );
}

#[test]
fn variable_inside_set_expression() {
    assert_eq!(calculate_query_weight("$x + set($x)"), 4);
}

// Not in the Java test: Java `String.length()` of an internal string counts its bytes, so
// `//:fg_é` (the internal form of a UTF-8 argument) weighs 8, and `'é'` in a word weighs 2.
#[test]
fn internal_string_weight_counts_chars() {
    assert_eq!(calculate_query_weight("//:fg_\u{c3}\u{a9}"), 8);
    assert_eq!(calculate_query_weight("attr(cmd, '\u{c3}\u{a9}', //:x)"), 9);
    assert_eq!(calculate_query_weight("//:\u{ff}"), 4);
}
