//! Port of `src/test/java/com/google/devtools/build/lib/query2/engine/QueryParserTest.java`.

mod support;

use std::sync::Arc;

use blaze_query_engine::{
    Argument, ArgumentType, Callback, QueryEnvironment, QueryExpression, QueryExpressionContext,
    QueryFunction, QueryFunctionBase, QueryTaskFuture, default_query_functions,
};
use support::MockEnvironment;

struct MockFunction {
    name: String,
    mandatory_arguments: i32,
    arguments: Vec<ArgumentType>,
}

impl MockFunction {
    fn new(name: &str, mandatory_arguments: i32, arguments: &[ArgumentType]) -> Self {
        MockFunction {
            name: name.to_owned(),
            mandatory_arguments,
            arguments: arguments.to_vec(),
        }
    }
}

impl QueryFunctionBase for MockFunction {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_mandatory_arguments(&self) -> i32 {
        self.mandatory_arguments
    }

    fn get_argument_types(&self) -> Vec<ArgumentType> {
        self.arguments.clone()
    }
}

impl QueryFunction<()> for MockFunction {
    fn eval(
        &self,
        _env: &dyn QueryEnvironment<()>,
        _context: &QueryExpressionContext<()>,
        _expression: &QueryExpression,
        _args: &[Argument],
        _callback: &dyn Callback<()>,
    ) -> QueryTaskFuture<()> {
        panic!("IllegalStateException");
    }
}

fn mock_environment() -> MockEnvironment {
    let mut functions = default_query_functions::<()>();
    functions.push(Arc::new(MockFunction::new(
        "opt",
        2,
        &[ArgumentType::Word, ArgumentType::Word, ArgumentType::Word],
    )));
    MockEnvironment { functions }
}

// Asserts that 'query' parses, and that when pretty-printed, yields 'expectedPrettyPrintOutput'.
fn check_pretty_print_as(expected_pretty_print_output: &str, query: &str) -> String {
    let env = mock_environment();
    assert_eq!(
        QueryExpression::parse::<()>(query, &env)
            .unwrap()
            .to_string(),
        expected_pretty_print_output
    );
    expected_pretty_print_output.to_owned()
}

// Asserts that 'query' parses, and that when pretty-printed, yields 'query'.
fn check_pretty_print(query: &str) -> String {
    check_pretty_print_as(query, query)
}

fn check_parse_fails(query: &str, expected_error: &str) {
    let env = mock_environment();
    let e = QueryExpression::parse::<()>(query, &env).unwrap_err();
    assert_eq!(e.get_message(), expected_error);
}

#[test]
fn test_optional_arguments() {
    check_pretty_print("opt('foo', 'bar')");
    check_pretty_print("opt('foo', 'bar', 'qux')");
    check_parse_fails(
        "opt('foo', 'bar', 'qux', 'zyc')",
        "too many arguments to function 'opt' at ', zyc )'",
    );
    check_parse_fails("opt('foo')", "too few arguments to function 'opt' at ')'");
    check_parse_fails("opt()", "too few arguments to function 'opt' at ')'");
}

#[test]
fn test_unknown_function() {
    let mut names: Vec<String> = default_query_functions::<()>()
        .iter()
        .map(|f| format!("'{}'", f.get_name()))
        .chain(std::iter::once("'opt'".to_owned()))
        .collect();
    names.sort();
    let known_functions = names.join(", ");
    check_parse_fails(
        "badfunc('foo', 'bar', 'qux', 'zyc')",
        &format!(
            "unknown function 'badfunc' at 'badfunc ( foo'; expected one of [{known_functions}]"
        ),
    );
}

#[test]
fn test_target_literals() {
    check_pretty_print("x");
    check_pretty_print("//x");
    check_pretty_print("//x:y");
    check_pretty_print("x/...:all-targets");
    check_pretty_print("\"set\""); // reserved word
    check_pretty_print("\"\"");
}

#[test]
fn check_parse_errors() {
    check_parse_fails("rdeps(", "premature end of input");
    check_parse_fails("rdeps(,", "syntax error at ','");
    check_parse_fails("rdeps(a", "premature end of input");
    check_parse_fails("rdeps(a, ", "premature end of input");
    check_parse_fails("rdeps(a, )", "syntax error at ')'");
    check_parse_fails("rdeps(a, b", "premature end of input");
    check_parse_fails("rdeps(a, b, ", "premature end of input");
    check_parse_fails("rdeps(a, b, )", "syntax error at ')'");
    check_parse_fails("rdeps(a, b, 3", "premature end of input");
    check_parse_fails(
        "rdeps(a, b, 3, ",
        "too many arguments to function 'rdeps' at ','",
    );
    check_parse_fails("rdeps(a, b, c, d)", "expected an integer literal: 'c'");
    check_parse_fails("set(", "premature end of input");
    check_parse_fails("set(a", "premature end of input");
    check_parse_fails("set(a b", "premature end of input");
    check_parse_fails("set(a, ", "syntax error at ','");
    check_parse_fails("set(a, b)", "syntax error at ', b )'");
}

#[test]
fn test_binary_operators() {
    check_parse_fails("foo intersect", "premature end of input");

    check_pretty_print_as("(a - b)", "a - b");

    check_pretty_print_as("(a intersect b)", "a intersect b");
    check_pretty_print_as("(a intersect b intersect c)", "a intersect b intersect c");
    check_pretty_print_as("(a union b)", "a union b");
    check_pretty_print_as("(a union b union c)", "a union b union c");
    check_pretty_print_as("(a except b)", "a except b");
    check_pretty_print_as("(a except b except c)", "a except b except c");
    check_pretty_print_as("((a union b) except c)", "a union b except c");
    check_pretty_print_as("((a except b) union c)", "a except b union c");
}

#[test]
fn test_operators() {
    check_pretty_print("some(x)");
    check_pretty_print("somepath(x, y)");
    check_pretty_print("allpaths(x, y)");
    check_pretty_print("deps(x)");
    check_pretty_print("deps(x, 1)");
    check_pretty_print("rdeps(x, y)");
    check_pretty_print("rdeps(x, y, 1)");
    check_pretty_print_as("kind('rule', x)", "kind(rule, x)");
    check_pretty_print("kind('source file', x)");
    check_pretty_print("kind('.*', x)");
    check_pretty_print_as("attr('linkshared', '1', x)", "attr(linkshared,1,x)");
    check_pretty_print_as("filter('jar$', x)", "filter(jar$, x)");
    check_pretty_print("let x = e1 in e2");
    check_pretty_print("labels('srcs', x)");
    check_pretty_print("tests(x)");
    check_pretty_print("executables(x)");
    check_pretty_print("set()");
    check_pretty_print("set(//a)");
    check_pretty_print("set(//a //b)");
}

#[test]
fn test_multiple_operator_parsing() {
    check_pretty_print(&check_pretty_print_as("kind('rule', x)", "kind(rule, x)"));
    check_pretty_print(&check_pretty_print_as(
        "attr('linkshared', '1', x)",
        "attr(linkshared,1,x)",
    ));
    check_pretty_print(&check_pretty_print_as(
        "filter('jar$', x)",
        "filter(jar$, x)",
    ));
}

#[test]
fn test_multiple_binary_operator_parsing() {
    check_pretty_print(&check_pretty_print_as(
        "((a union b) except c)",
        "a union b except c",
    ));
    check_pretty_print(&check_pretty_print_as(
        "(a intersect b intersect c)",
        "a intersect b intersect c",
    ));
    check_pretty_print(&check_pretty_print_as(
        "(a union b union c)",
        "a union b union c",
    ));
    check_pretty_print(&check_pretty_print_as(
        "((((a union b) intersect c) except d) intersect e)",
        "a union b intersect c except d intersect e",
    ));
}

#[test]
fn test_multiple_target_literal_parsing() {
    check_pretty_print(&check_pretty_print_as("//foo:.*@4", "\"//foo:.*@4\""));
    check_pretty_print(&check_pretty_print_as("set(//foo)", "set(\"//foo\")"));
    check_pretty_print("\"set(//foo)\"");
    check_pretty_print("\"set('//foo')\"");
}

#[test]
fn test_quoted_and_unquoted_metacharacters() {
    check_pretty_print("\"//foo:xx+xx\"");
    check_pretty_print(&check_pretty_print_as("(//foo:xx + xx)", "//foo:xx+xx"));
    check_pretty_print("\"//foo:xx=xx\"");
    check_parse_fails(
        "//foo:xx=xx",
        "unexpected token '=' after query expression '//foo:xx'",
    );
}

#[test]
fn test_quoted_special_characters() {
    check_pretty_print_as(
        "\"foo[]^$asd.|asd?*+{})_asd()2\"",
        "'foo[]^$asd.|asd?*+{})_asd()2'",
    );
    check_pretty_print("\"foo[]^$asd.|asd?*+{})_asd()2\"");
    check_pretty_print("\" #&()+,;<=>?[]{|}\"");
}

#[test]
fn test_unquoted_special_characters() {
    // All special characters in the Lexer#scanWord ./@_:~-*$
    check_pretty_print("a.b");
    check_pretty_print("a/b");
    check_pretty_print("a@b");
    check_pretty_print("a_b");
    check_pretty_print("a:b");
    check_pretty_print("a~b");
    check_pretty_print("a-b");
    check_pretty_print("a*b");
    check_pretty_print("a$b");
}

#[test]
fn test_preserve_quoting() {
    check_pretty_print(&check_pretty_print("\"a+b\""));
    // this should preserve quoting without being quoted in TargetLiteral#toString
    check_pretty_print(&check_pretty_print_as("aaa", "\"aaa\""));
}

#[test]
fn test_quoted_illegal_characters() {
    check_parse_fails("\"-x\"", "target literal must not begin with (-): -x");
    check_parse_fails("\"*x\"", "target literal must not begin with (*): *x");
}

#[test]
fn test_illegal_quoting() {
    check_parse_fails("\"a", "unclosed quotation");
    check_parse_fails("'a", "unclosed quotation");
    check_parse_fails("a\"a", "unclosed quotation");
    check_parse_fails("a'a", "unclosed quotation");
    check_parse_fails("a'\"a", "unclosed quotation");
    check_parse_fails("a\"'a", "unclosed quotation");
    check_parse_fails("'a\"'a'", "unclosed quotation");
    check_parse_fails("\"a'\"a\"", "unclosed quotation");
    check_parse_fails(
        "'\"a\" + 'a''",
        "unexpected token 'a' after query expression ''\"a\" + ''",
    );
    check_parse_fails(
        "\"'a' + \"a\"\"",
        "unexpected token 'a' after query expression '\"'a' + \"'",
    );
    check_parse_fails(
        "\"set(\"//foo\" + \"bar\")\"",
        "unexpected token '//foo' after query expression '\"set(\"'",
    );
    check_parse_fails(
        "'set('//foo' + 'bar')'",
        "unexpected token '//foo' after query expression '\"set(\"'",
    );
}

#[test]
fn test_using_correct_quoting_in_target_literal_to_string() {
    // These tests all fall into the needsQuoting == true use case in TargetLiteral#toString
    check_pretty_print("'set(\"//foo\" + \"bar\")'");
    check_pretty_print("\"set('//foo' + 'bar')\"");
    check_pretty_print("\"a'a\"");
    check_pretty_print("'a\"a'");
}

#[test]
fn test_unicode_labels() {
    // PORT: StringEncoding.unicodeToInternal is the identity (Rust strings are UTF-8).
    check_pretty_print_as("//:äöüÄÖÜß🌱", "'//:äöüÄÖÜß🌱'");
    check_pretty_print_as("//:äöüÄÖÜß🌱", "//:äöüÄÖÜß🌱");
}
