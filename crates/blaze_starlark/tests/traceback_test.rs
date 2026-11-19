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

/// A function called from native code (Java `Starlark.call` of a rule implementation) is the
/// outermost frame: no `File "<builtin>", in <toplevel>` entry (oracle: scripts/w4/scenarios/
/// ctx-errors, every "in err rule" traceback starts at `_err_impl`).
#[test]
fn native_call_has_no_builtin_toplevel_frame() {
    let m =
        exec("def impl(x):\n    return helper(x)\n\ndef helper(x):\n    return x + 'a'\n").unwrap();
    let f = m.get("impl").unwrap();
    let e = blaze_starlark::eval::run_on_evaluation_thread(move || {
        let events = std::sync::Arc::new(blaze_starlark::thread::CollectingEvents::default());
        let ctx = common::context(
            blaze_starlark::Phase::Analysis,
            StarlarkSemantics::default(),
            events,
        );
        blaze_starlark::eval::call_frozen(
            &f,
            &ctx,
            &Default::default(),
            |m| (vec![m.heap().alloc(1)], vec![]),
            |_, _, _| Ok(()),
        )
        .unwrap_err()
    });
    let s = e.message_with_stack();
    let (head, last) = s.rsplit_once('\n').unwrap();
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 2, column 18, in impl\n\
\t\treturn helper(x)\n\
\tFile \"test.bzl\", line 5, column 14, in helper\n\
\t\treturn x + 'a'"
    );
    assert!(last.starts_with("Error: "), "{last}");
}

/// Java: an argument error of a Starlark function is reported in the callee's frame, located at
/// its declaration (oracle: tests/data/locations/starlark_call_arity.oracle.txt).
#[test]
fn argument_error_is_in_the_callee_frame() {
    let (head, last) = stack("def g(a):\n    return a\n\ng(1, 2)\n");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 4, column 2, in <toplevel>\n\
\t\tg(1, 2)\n\
\tFile \"test.bzl\", line 1, column 5, in g\n\
\t\tdef g(a):"
    );
    assert!(last.starts_with("Error: "), "{last}");
}

/// Java's Resolver reports every undefined name of a file, in source order, each at its
/// identifier (net/starlark/java/syntax/Resolver.java:941).
#[test]
fn resolve_reports_every_undefined_name() {
    let semantics = StarlarkSemantics::default();
    let globals = blaze_starlark::env::NativeRegistry::with_blaze_starlark_builtins()
        .build(blaze_starlark::FileKind::Bzl, &semantics);
    let src = "x = y\ndef f():\n    return zz + w\nq = len(x)\n";
    let compiled =
        blaze_starlark::eval::compile(blaze_starlark::FileKind::Bzl, "test.bzl", src.to_owned())
            .unwrap();
    let e = blaze_starlark::eval::resolve(&compiled, &globals).unwrap_err();
    let EvalErrorKind::Syntax(errors) = &e.kind else {
        panic!("{e:?}");
    };
    let got: Vec<String> = errors.iter().map(|e| format!("{}", e.location)).collect();
    assert_eq!(got, ["test.bzl:1:5", "test.bzl:3:12", "test.bzl:3:17"]);
    assert!(
        errors[0].message.starts_with("name 'y' is not defined"),
        "{}",
        errors[0].message
    );
    assert!(
        errors[2].message.starts_with("name 'w' is not defined"),
        "{}",
        errors[2].message
    );
    // A file whose names all resolve.
    let ok = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Bzl,
        "test.bzl",
        "x = len([])\n".to_owned(),
    )
    .unwrap();
    assert!(blaze_starlark::eval::resolve(&ok, &globals).is_ok());
}

/// Columns count chars of the Latin-1 decoded file, i.e. bytes (Java `Location.column`), and the
/// source line is trimmed with `String.trim` (chars <= U+0020): a line ending in `à` (C3 A0)
/// keeps its U+00A0. Oracle: Bazel 9.3.2 on ~/.cache/blaze/work/bytes-interp/ws (`bazel query
/// //a:all`, `//b:all`, `//c:all`), 2026-11-18.
#[test]
fn non_ascii_columns_and_source_lines() {
    // a/defs.bzl: `x = "é" + 1` -> "line 1, column 10" (the `+` is the 10th byte).
    let (head, _) = stack("x = \"é\" + 1\n");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 1, column 10, in <toplevel>\n\
\t\tx = \"\u{c3}\u{a9}\" + 1"
    );
    // b/defs.bzl: `z = "é\q"` -> "1:9: invalid escape sequence: \q. Use '\\' to insert '\'."
    let e = exec("z = \"é\\q\"\n").unwrap_err();
    assert_eq!(
        e.message_with_stack(),
        "test.bzl:1:9: invalid escape sequence: \\q. Use '\\\\' to insert '\\'."
    );
    // c/defs.bzl: the source line `    fail("boom")  # voilà` keeps its last byte A0.
    let (head, last) = stack("def f():\n    fail(\"boom\")  # voilà\n\nf()\n");
    assert_eq!(
        head,
        "Traceback (most recent call last):\n\
\tFile \"test.bzl\", line 4, column 2, in <toplevel>\n\
\t\tf()\n\
\tFile \"test.bzl\", line 2, column 9, in f\n\
\t\tfail(\"boom\")  # voil\u{c3}\u{a0}"
    );
    assert_eq!(last, "Error in fail: boom");
}
