//! Bazel traceback rendering. Ports of the stack tests in
//! src/test/java/net/starlark/java/eval/MethodLibraryTest.java (call stack and columns compared
//! exactly; the final message is starlark-rust's where Bazel's text differs), plus print/fail.

mod common;

use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::loader::LoadMap;
use common::exec;
use common::exec_with;

/// Renders the error of `src` (file "test.bzl"); returns (traceback without the last line, last
/// line).
fn stack(src: &str) -> (String, String) {
    let e = exec(src).unwrap_err();
    let s = e.message_with_stack();
    let (head, last) = s.rsplit_once('\n').unwrap_or(("", &s));
    (head.to_owned(), last.to_owned())
}

#[test]
fn test_stack_trace() {
    let (head, last) = stack(
        "def foo():\n  bar(1)\ndef bar(x):\n  if x == 1:\n    a = x\n    b = 2\n    'test'.index(x) # hello\nfoo()",
    );
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 8, column 4, in <toplevel>\n\
\t\tfoo()\n\
\tFile \"test.bzl\", line 2, column 6, in foo\n\
\t\tbar(1)\n\
\tFile \"test.bzl\", line 7, column 17, in bar\n\
\t\t'test'.index(x) # hello"
    );
    assert!(last.starts_with("Error in index: "), "{last}");
}

#[test]
fn test_stack_trace_with_if() {
    let (head, last) = stack("def foo():\n  s = []\n  if s[0] == 1:\n    x = 1\nfoo()");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 5, column 4, in <toplevel>\n\
\t\tfoo()\n\
\tFile \"test.bzl\", line 3, column 7, in foo\n\
\t\tif s[0] == 1:"
    );
    assert!(last.starts_with("Error: "), "{last}");
}

#[test]
fn test_stack_trace_with_augmented_assignment() {
    let (head, last) = stack("def foo():\n  s = 1\n  s += '2'\nfoo()");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 4, column 4, in <toplevel>\n\
\t\tfoo()\n\
\tFile \"test.bzl\", line 3, column 5, in foo\n\
\t\ts += '2'"
    );
    assert!(last.starts_with("Error: "), "{last}");
}

#[test]
fn test_stack_error_in_builtin_function() {
    let (head, last) = stack("len(1)");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\tFile \"test.bzl\", line 1, column 4, in <toplevel>\n\t\tlen(1)"
    );
    assert!(last.starts_with("Error in len: "), "{last}");

    let (head, last) = stack("def f():\n  len(1)\nf()");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 3, column 2, in <toplevel>\n\
\t\tf()\n\
\tFile \"test.bzl\", line 2, column 6, in f\n\
\t\tlen(1)"
    );
    assert!(last.starts_with("Error in len: "), "{last}");
}

#[test]
fn test_stack_error_in_operator() {
    let (head, last) = stack("x = 1//0");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\tFile \"test.bzl\", line 1, column 6, in <toplevel>\n\t\tx = 1//0"
    );
    assert!(last.starts_with("Error: "), "{last}");
}

#[test]
fn fail_matches_oracle() {
    // Same shape as the oracle-verified traceback in docs/research/05-starlark-rust.md.
    let e = exec("def g(name):\n    _check(name)\n\ndef _check(name):\n    fail(\"bad thing:\", name)\n\ng(\"x\")\n")
        .unwrap_err();
    assert_eq!(
        e.message_with_stack(),
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 7, column 2, in <toplevel>\n\
\t\tg(\"x\")\n\
\tFile \"test.bzl\", line 2, column 11, in g\n\
\t\t_check(name)\n\
\tFile \"test.bzl\", line 5, column 9, in _check\n\
\t\tfail(\"bad thing:\", name)\n\
Error in fail: bad thing: x"
    );
}

#[test]
fn fail_keyword_arguments() {
    let e = exec("fail('a', 1, msg = 'm', attr = 'srcs', sep = '|')").unwrap_err();
    assert_eq!(e.message, "attribute srcs:|m|a|1");
    let e = exec("fail('quiet', stack_trace = False)").unwrap_err();
    assert_eq!(e.message_with_stack(), "quiet");
    let sem = StarlarkSemantics {
        force_starlark_stack_trace: true,
        ..Default::default()
    };
    let (r, _) = exec_with("fail('loud', stack_trace = False)", sem, LoadMap::new());
    assert!(
        r.unwrap_err()
            .message_with_stack()
            .ends_with("Error in fail: loud")
    );
}

#[test]
fn print_event_format() {
    let (r, events) = exec_with(
        "def g():\n    print(\"analyzing\", \"t\", sep = \"|\")\n\ng()\nprint(Label('@foo//x:y'), [1])\n",
        StarlarkSemantics::default(),
        LoadMap::new(),
    );
    r.unwrap();
    let lines: Vec<String> = events.iter().map(|e| e.to_console()).collect();
    // Oracle: "DEBUG: <path>/BUILD.bzl:2:10: analyzing|t".
    assert_eq!(
        lines,
        [
            "DEBUG: test.bzl:2:10: analyzing|t",
            "DEBUG: test.bzl:5:6: @foo//x:y [1]",
        ]
    );
}

#[test]
fn recursion_is_rejected() {
    let e = exec("def f(n):\n    return f(n - 1) if n else 0\nx = f(2)").unwrap_err();
    assert_eq!(e.message, "function 'f' called recursively");
}

#[test]
fn call_depth_limit_is_not_starlark_rusts_50() {
    let mut src = String::new();
    for i in 0..120 {
        src.push_str(&format!("def f{i}():\n    return f{}() + 1\n", i + 1));
    }
    src.push_str("def f120():\n    return 0\nx = f0()\n");
    let m = exec(&src).unwrap();
    assert_eq!(m.get("x").unwrap().value().to_repr(), "120");
}

#[test]
fn syntax_errors_render_as_locations() {
    let e = exec("x = 1\nx = 2\n").unwrap_err();
    assert!(matches!(e.kind, EvalErrorKind::Syntax(_)));
    assert_eq!(
        e.message_with_stack(),
        "test.bzl:2:1: 'x' redeclared at top level\ntest.bzl:1:1: 'x' previously declared here"
    );
    assert_eq!(e.message, "'x' redeclared at top level (+ 1 more)");
}
