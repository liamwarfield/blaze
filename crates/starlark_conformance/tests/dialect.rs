//! Dialect differences between Bazel 9.3.2 and starlark-rust 0.14.2, pinned as tests.
//!
//! "bazel_*" tests check Bazel behaviour Blaze already gets (or gets via `build_check`).
//! "deviation_*" tests pin current starlark-rust behaviour that differs from Bazel (verified on
//! the oracle where noted); each one fails when an upgrade or patch fixes it, so the fix plan in
//! docs/research/05-starlark-rust.md can be updated.

use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark_conformance::dialect;
use starlark_conformance::harness::bare_message;

fn parse(src: &str) -> Result<AstModule, String> {
    AstModule::parse("t.bzl", src.to_owned(), &dialect::bzl_dialect()).map_err(|e| bare_message(&e))
}

/// Evaluates `src` with the Bazel universe; returns repr of `r` or the error message.
fn eval(src: &str) -> Result<String, String> {
    let globals: Globals = dialect::bazel_universe();
    let ast = parse(src)?;
    Module::with_temp_heap(|module| {
        let mut e = Evaluator::new(&module);
        e.set_forbid_recursion(true);
        e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
        Ok(module.get("r").map(|v| v.to_repr()).unwrap_or_default())
    })
}

#[test]
fn bazel_universe_names() {
    let (_, missing, extra) = dialect::bazel_universe_builder();
    // Everything in Bazel's Starlark.UNIVERSE exists in starlark-rust (standard + set + print).
    assert_eq!(missing, Vec::<String>::new());
    // starlark-rust names Bazel does not have; Blaze must not expose them.
    let mut extra = extra;
    extra.sort();
    assert_eq!(extra, ["bytes", "chr", "ord"]);
}

#[test]
fn bazel_rejects_top_level_for_and_if() {
    assert!(parse("for x in []:\n    pass\n").is_err());
    assert!(parse("if True:\n    pass\n").is_err());
    assert!(parse("def f():\n    for x in []:\n        pass\n").is_ok());
}

#[test]
fn bazel_keyword_only_params_allowed_positional_only_rejected() {
    assert_eq!(
        eval("def f(a, *, b): return a + b\nr = f(1, b = 2)"),
        Ok("3".into())
    );
    assert!(parse("def f(a, /, b): pass\n").is_err());
}

#[test]
fn bazel_rejects_f_strings_and_type_annotations() {
    assert!(parse("x = f'{1}'\n").is_err());
    assert!(parse("def f(a: int) -> int: return a\n").is_err());
}

#[test]
fn bazel_lambda_and_nested_def_allowed_in_bzl() {
    assert_eq!(
        eval("def f():\n    g = lambda x: x * 2\n    return g(3)\nr = f()"),
        Ok("6".into())
    );
}

#[test]
fn bazel_build_file_restrictions() {
    // The checker is blaze_starlark::syntax (tested there); here: BUILD parses top-level if/for so
    // that the checker, not the parser, reports them.
    let e = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Build,
        "BUILD",
        "for i in []:\n    pass\n".to_owned(),
    )
    .unwrap_err();
    assert!(
        e.message
            .starts_with("`for` statements are not allowed in BUILD files."),
        "{}",
        e.message
    );
}

#[test]
fn bazel_top_level_rebinding_rejected_in_bzl() {
    // starlark-rust itself allows it (see the eval below); blaze_starlark::syntax rejects it.
    assert_eq!(eval("x = 1\nx = 2\nr = x"), Ok("2".into()));
    let e = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Bzl,
        "a.bzl",
        "x = 1\nx = 2\n".to_owned(),
    )
    .unwrap_err();
    assert_eq!(e.message, "'x' redeclared at top level (+ 1 more)");
}

#[test]
fn bazel_recursion_forbidden() {
    // Fixed by Blaze patch 0004 (Evaluator::set_forbid_recursion).
    assert_eq!(
        eval("def f(n):\n    return f(n - 1) if n > 0 else 0\nr = f(5)"),
        Err("function 'f' called recursively".into())
    );
}

#[test]
fn bazel_left_shift_is_exact() {
    // Oracle (Bazel 9.3.2): 1 << 31 == 2147483648, 2 << 31 == 4294967296. Blaze patch 0001.
    assert_eq!(
        eval("r = (1 << 31, 2 << 31)"),
        Ok("(2147483648, 4294967296)".into())
    );
}

#[test]
fn bazel_raw_string_backslash_quote() {
    // Oracle: r"\"" is two characters, backslash and quote. Blaze patch 0003.
    assert_eq!(eval("r = len(r\"\\\"\")"), Ok("2".into()));
}

#[test]
fn bazel_float_str_shortest_roundtrip() {
    // Oracle: str(1.23e-45 - 1e-61) == "1.2299999999999998e-45". Blaze patch 0008.
    assert_eq!(
        eval("r = str(1.23e-45 - 1e-61)"),
        Ok("\"1.2299999999999998e-45\"".into())
    );
}

#[test]
fn bazel_builtin_type_and_str() {
    // Oracle: type(len) == "builtin_function_or_method", str(len) == "<built-in function len>".
    // Blaze patch 0009.
    assert_eq!(
        eval("r = (type(len), str(len))"),
        Ok("(\"builtin_function_or_method\", \"<built-in function len>\")".into())
    );
}

#[test]
fn deviation_percent_format_width() {
    // Bazel also rejects width/precision; only the message differs.
    assert!(eval("r = '%5d' % 1").is_err());
}

#[test]
fn bazel_string_method_named_args() {
    // Bazel: "a.b.c".split(sep=".", maxsplit=1) == ["a", "b.c"]. Blaze patch 0005.
    assert_eq!(
        eval("r = 'a.b.c'.split(sep = '.', maxsplit = 1)"),
        Ok("[\"a\", \"b.c\"]".into())
    );
}

#[test]
fn bazel_list_pop_negative_index() {
    // Bazel: [1, 2, 3].pop(-2) == 2. Blaze patch 0006.
    assert_eq!(
        eval("x = [1, 2, 3]\nr = (x.pop(-2), x)"),
        Ok("(2, [1, 3])".into())
    );
}

#[test]
fn bazel_set_repr() {
    // Bazel: repr(set()) == "set()". Blaze patch 0007.
    assert_eq!(eval("r = repr(set())"), Ok("\"set()\"".into()));
}

#[test]
fn bazel_string_escapes() {
    // Bazel 9.3.2 (net/starlark/java/syntax/Lexer.java:322-404): octal escapes are one char
    // 0..=0xFF; \x, \u, \U, \N and other unknown escapes are errors. Blaze patch 0032 (stock
    // starlark-rust accepted \x41, \u0041 and kept an unknown escape's backslash).
    assert_eq!(
        eval(r#"r = (len("\377"), "\101")"#),
        Ok("(1, \"A\")".into())
    );
    for bad in [
        r#""\x41""#,
        r#""\u0041""#,
        r#""\U00000041""#,
        r#""\N{x}""#,
        r#""\q""#,
    ] {
        let e = parse(&format!("r = {bad}\n")).unwrap_err();
        assert!(e.starts_with("invalid escape sequence: \\"), "{bad}: {e}");
    }
    assert_eq!(
        parse("r = \"\\400\"\n").unwrap_err(),
        "octal escape sequence out of range (maximum is \\377)"
    );
}

#[test]
fn bazel_string_repr_escapes_only_control_chars() {
    // Bazel: repr escapes `"`, `\` and chars below 0x20 only (Printer.java:239). Blaze patch 0035.
    assert_eq!(
        eval("r = repr(\"\\001\u{7f}\u{e9}\u{200b}\")"),
        Ok("\"\\\"\\\\x01\u{7f}\u{e9}\u{200b}\\\"\"".into())
    );
}
