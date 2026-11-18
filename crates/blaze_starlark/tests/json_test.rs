//! `json` (port of net/starlark/java/lib/json/Json.java). The full Bazel json.star script runs
//! in crates/starlark_conformance; these are spot checks of exact texts.

mod common;

use common::eval_expr;
use common::exec_err;

#[test]
fn encode() {
    assert_eq!(eval_expr("json.encode(None)"), Ok(r#""null""#.into()));
    assert_eq!(
        eval_expr("json.encode([1, 'a', True])"),
        Ok(r#""[1,\"a\",true]""#.into())
    );
    assert_eq!(
        eval_expr("json.encode(dict(y = 'two', x = 1))"),
        Ok(r#""{\"x\":1,\"y\":\"two\"}""#.into())
    );
    assert_eq!(
        eval_expr("json.encode(struct(b = depset([1]).to_list(), a = (1, 2)))"),
        Ok(r#""{\"a\":[1,2],\"b\":[1]}""#.into())
    );
    assert_eq!(
        eval_expr("json.encode('\\x01\"')"),
        Ok(r#""\"\\u0001\\\"\"""#.into())
    );
    assert_eq!(
        eval_expr("json.encode(range(3))"),
        Ok(r#""[0,1,2]""#.into())
    );
}

#[test]
fn encode_errors() {
    assert_eq!(
        exec_err("json.encode({1: 'two'})"),
        Err("dict has int key, want string".into())
    );
    assert_eq!(
        exec_err("json.encode(float('nan'))"),
        Err("cannot encode non-finite float nan".into())
    );
    assert_eq!(
        exec_err("json.encode(struct(x = [1, len]))"),
        Err(
            "in struct field .x: at list index 1: cannot encode builtin_function_or_method as JSON"
                .into()
        )
    );
    assert_eq!(
        exec_err(
            "def f():\n    d = None\n    for i in range(100000):\n        d = [d]\n    return json.encode(d)\nx = f()"
        ),
        Err("nesting depth limit exceeded".into())
    );
}

#[test]
fn decode() {
    assert_eq!(
        eval_expr("json.decode('{\"a\": [1, 2.5, null, true, \"x\"]}')"),
        Ok("{\"a\": [1, 2.5, None, True, \"x\"]}".into())
    );
    assert_eq!(
        eval_expr(
            "json.decode('\"\\\\u00e9\\\\ud83d\\\\ude00\\\\ud800\"') == '\u{e9}\u{1f600}\u{fffd}'"
        ),
        Ok("True".into())
    );
    assert_eq!(eval_expr("json.decode('[1', default = 7)"), Ok("7".into()));
    assert_eq!(
        exec_err("json.decode('[1')"),
        Err("at offset 2, unexpected end of file".into())
    );
    assert_eq!(
        exec_err("json.decode('01')"),
        Err("at offset 0, invalid number: 01".into())
    );
    assert_eq!(
        exec_err("json.decode('[1] x')"),
        Err("at offset 4, unexpected character \"x\" after value".into())
    );
    assert_eq!(
        exec_err("json.decode('[' * 100000)"),
        Err("nesting depth limit exceeded".into())
    );
}

#[test]
fn indent() {
    assert_eq!(
        eval_expr("json.indent('{\"a\":[1,2],\"b\":{}}')"),
        Ok(r#""{\n\t\"a\": [\n\t\t1,\n\t\t2\n\t],\n\t\"b\": {}\n}""#.into())
    );
    assert_eq!(
        eval_expr("json.encode_indent([1], prefix = '>', indent = '  ')"),
        Ok(r#""[\n>  1\n>]""#.into())
    );
    assert_eq!(
        exec_err("json.indent('[\"abc')"),
        Err("input is not valid JSON".into())
    );
}
