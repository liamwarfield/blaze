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
use starlark_conformance::build_check::Checker;
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
    let src = "\
load(\":a.bzl\", \"m\")
def f():
    pass
x = lambda: 1
for i in []:
    pass
if True:
    m(*[1])
m(**{\"a\": 1})
m(**d)
";
    let ast = AstModule::parse("BUILD", src.to_owned(), &dialect::build_dialect()).unwrap();
    let got: Vec<String> = Checker::build_files()
        .check(&ast)
        .into_iter()
        .map(|e| format!("{}:{}: {}", e.line, e.column, e.message))
        .collect();
    assert_eq!(
        got,
        [
            "2:1: functions may not be defined in BUILD files. You may move the function to a .bzl file and load it.",
            "4:5: functions may not be defined in BUILD files. You may move the function to a .bzl file and load it.",
            "5:1: `for` statements are not allowed in BUILD files. You may inline the loop, move it to a function definition (in a .bzl file), or as a last resort use a list comprehension.",
            "7:1: `if` statements are not allowed in BUILD files. You may move conditional logic to a function definition (in a .bzl file), or use an `if` expression for simple cases.",
            "8:7: *args arguments are not allowed in BUILD files. Pass the arguments in explicitly.",
            "9:3: **kwargs arguments are not allowed in BUILD files. Pass the arguments in explicitly.",
            "10:3: **kwargs arguments are not allowed in BUILD files. Pass the arguments in explicitly.",
            "10:3: **kwargs arguments must be a literal dict in BUILD files.",
        ]
    );
}

#[test]
fn deviation_top_level_rebinding_allowed() {
    // Bazel .bzl: "'x' redeclared at top level" (Resolver.toplevelRebinding). BUILD files allow
    // it, so Blaze needs a .bzl-only AST check.
    assert_eq!(eval("x = 1\nx = 2\nr = x"), Ok("2".into()));
}

#[test]
fn deviation_recursion_allowed() {
    // Bazel: "function 'f' called recursively".
    assert_eq!(
        eval("def f(n):\n    return f(n - 1) if n > 0 else 0\nr = f(5)"),
        Ok("0".into())
    );
}

#[test]
fn deviation_left_shift_overflows_i32() {
    // Oracle (Bazel 9.3.2): 1 << 31 == 2147483648, 2 << 31 == 4294967296.
    assert_eq!(
        eval("r = (1 << 31, 2 << 31)"),
        Ok("(-2147483648, 0)".into())
    );
}

#[test]
fn deviation_raw_string_backslash_quote() {
    // Oracle: r"\"" is two characters, backslash and quote.
    assert_eq!(eval("r = len(r\"\\\"\")"), Ok("1".into()));
}

#[test]
fn deviation_float_str_not_shortest_roundtrip() {
    // Oracle: str(1.23e-45 - 1e-61) == "1.2299999999999998e-45".
    assert_eq!(eval("r = str(1.23e-45 - 1e-61)"), Ok("\"1.23e-45\"".into()));
}

#[test]
fn deviation_builtin_type_and_str() {
    // Oracle: type(len) == "builtin_function_or_method", str(len) == "<built-in function len>".
    assert_eq!(
        eval("r = (type(len), str(len))"),
        Ok("(\"function\", \"len\")".into())
    );
}

#[test]
fn deviation_percent_format_width() {
    // Bazel also rejects width/precision; only the message differs.
    assert!(eval("r = '%5d' % 1").is_err());
}

#[test]
fn deviation_string_method_named_args() {
    // Bazel: "a.b.c".split(sep=".", maxsplit=1) == ["a", "b.c"].
    assert!(eval("r = 'a.b.c'.split(sep = '.', maxsplit = 1)").is_err());
}

#[test]
fn deviation_list_pop_negative_index() {
    // Bazel: [1, 2, 3].pop(-2) == 2.
    assert!(eval("x = [1, 2, 3]\nr = x.pop(-2)").is_err());
}

#[test]
fn deviation_set_repr() {
    // Bazel: repr(set()) == "set()".
    assert_eq!(eval("r = repr(set())"), Ok("\"set([])\"".into()));
}
