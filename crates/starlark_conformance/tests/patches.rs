//! One test per entry of third_party/starlark-rust/PATCHES.md. Expected values come from Bazel
//! 9.3.2 (oracle runs or the Java source cited in each patch).

use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark_conformance::dialect;
use starlark_conformance::harness::bare_message;

/// Evaluates `src` (Bazel universe, recursion forbidden) on a big-stack thread; returns repr of
/// `r` or the error message.
fn eval(src: &'static str) -> Result<String, String> {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let globals = dialect::bazel_universe();
            let ast = AstModule::parse("t.bzl", src.to_owned(), &dialect::bzl_dialect())
                .map_err(|e| bare_message(&e))?;
            Module::with_temp_heap(|module| {
                let mut e = Evaluator::new(&module);
                e.set_forbid_recursion(true);
                e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
                Ok(module.get("r").map(|v| v.to_repr()).unwrap_or_default())
            })
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn patch_0001_left_shift_overflow() {
    assert_eq!(
        eval("r = (1 << 31, 2 << 31, -1 << 31, (1 << 62) - (2 << 31) + 1 == 0x3fffffff00000001)"),
        Ok("(2147483648, 4294967296, -2147483648, True)".into())
    );
    assert_eq!(eval("r = 1 << 511 > 0"), Ok("True".into()));
    assert_eq!(
        eval("r = 1 << 512"),
        Err("shift count too large: 512".into())
    );
}

#[test]
fn patch_0002_allocation_limits() {
    assert_eq!(
        eval("r = [1, 2, 3] * (1 << 30)"),
        Err("excessive repeat (3 * 1073741824 elements)".into())
    );
    assert_eq!(
        eval("r = (1, 2) * (1 << 30)"),
        Err("excessive repeat (2 * 1073741824 elements)".into())
    );
    assert_eq!(
        eval("r = 'abc' * (1 << 30)"),
        Err("excessive repeat (3 * 1073741824 characters)".into())
    );
    assert_eq!(
        eval("x = []\nx.extend(range((1 << 31) - 1))\nr = x"),
        Err("excessive capacity requested (0 + 2147483647 elements)".into())
    );
    assert_eq!(eval("r = len([1] * 1000)"), Ok("1000".into()));
}

#[test]
fn patch_0002_nesting_depth_guards() {
    // repr of a very deep value no longer overflows the native stack.
    let src = "def deep():\n    d = None\n    for i in range(100000):\n        d = [d]\n    return d\nr = len(str(deep())) > 0";
    assert_eq!(eval(src), Ok("True".into()));
}

#[test]
fn patch_0003_raw_string_keeps_backslash() {
    assert_eq!(
        eval(r#"r = (len(r"\""), r"a\'b", r'\'')"#),
        Ok(r#"(2, "a\\'b", "\\'")"#.into())
    );
}

#[test]
fn patch_0004_recursion_check() {
    assert_eq!(
        eval("def f(n):\n    return 0 if n == 0 else f(n - 1)\nr = f(3)"),
        Err("function 'f' called recursively".into())
    );
    // Mutual recursion and recursion through a lambda closure.
    assert_eq!(
        eval("def a(n):\n    return b(n)\ndef b(n):\n    return a(n)\nr = a(1)"),
        Err("function 'a' called recursively".into())
    );
    // Calling the same function twice in sequence is not recursion.
    assert_eq!(
        eval("def g(x):\n    return x + 1\ndef h():\n    return g(g(1))\nr = h()"),
        Ok("3".into())
    );
    // Without the flag (starlark-rust default) recursion is allowed.
    let globals = dialect::bazel_universe();
    let ast = AstModule::parse(
        "t.bzl",
        "def f(n):\n    return 0 if n == 0 else f(n - 1)\nr = f(3)".to_owned(),
        &dialect::bzl_dialect(),
    )
    .unwrap();
    Module::with_temp_heap(|module| {
        let mut e = Evaluator::new(&module);
        e.eval_module(ast, &globals).unwrap();
    });
}

#[test]
fn patch_0006_list_pop_negative_index() {
    assert_eq!(
        eval("x = [1, 2, 3, 4]\nr = (x.pop(-1), x.pop(-3), x)"),
        Ok("(4, 1, [2, 3])".into())
    );
    assert!(eval("x = [1]\nr = x.pop(-2)").is_err());
}

/// A test value for patches 0013 and 0014: `probe()` has no attributes, its own unknown-field
/// message, and a reflected `|` that returns `("ror", lhs)`.
mod probe {
    use allocative::Allocative;
    use starlark::any::ProvidesStaticType;
    use starlark::environment::GlobalsBuilder;
    use starlark::starlark_module;
    use starlark::starlark_simple_value;
    use starlark::values::Heap;
    use starlark::values::NoSerialize;
    use starlark::values::StarlarkValue;
    use starlark::values::Value;
    use starlark::values::starlark_value;
    use starlark::values::tuple::AllocTuple;

    #[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
    pub struct Probe;

    impl std::fmt::Display for Probe {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "probe")
        }
    }

    starlark_simple_value!(Probe);

    #[starlark_value(type = "probe")]
    impl<'v> StarlarkValue<'v> for Probe {
        fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
            Some(format!(
                "'probe' value has no field or method '{attribute}'"
            ))
        }

        fn rbit_or(&self, lhs: Value<'v>, heap: Heap<'v>) -> Option<starlark::Result<Value<'v>>> {
            Some(Ok(heap.alloc(AllocTuple([heap.alloc("ror"), lhs]))))
        }
    }

    #[starlark_module]
    pub fn globals(builder: &mut GlobalsBuilder) {
        fn probe() -> anyhow::Result<Probe> {
            Ok(Probe)
        }
    }
}

/// Like [`eval`], with `probe()` in the universe.
fn eval_with_probe(src: &'static str) -> Result<String, String> {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let (mut builder, _, _) = dialect::bazel_universe_builder();
            probe::globals(&mut builder);
            let globals = builder.build();
            let ast = AstModule::parse("t.bzl", src.to_owned(), &dialect::bzl_dialect())
                .map_err(|e| bare_message(&e))?;
            Module::with_temp_heap(|module| {
                let mut e = Evaluator::new(&module);
                e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
                Ok(module.get("r").map(|v| v.to_repr()).unwrap_or_default())
            })
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Bazel: `Starlark.getattr` raises `Structure.getErrorMessageForUnknownField` (for `x.f` and
/// `getattr(x, "f")`).
#[test]
fn patch_0013_unknown_field_message() {
    assert_eq!(
        eval_with_probe("r = probe().foo"),
        Err("'probe' value has no field or method 'foo'".into())
    );
    assert_eq!(
        eval_with_probe("r = getattr(probe(), 'foo')"),
        Err("'probe' value has no field or method 'foo'".into())
    );
    assert_eq!(
        eval_with_probe("r = getattr(probe(), 'foo', 3)"),
        Ok("3".into())
    );
    // Values without the hook keep starlark-rust's text.
    assert!(
        eval_with_probe("r = [].foo")
            .unwrap_err()
            .contains("has no attribute")
    );
}

/// Bazel EvalUtils.binaryOp: `x | y` falls back to `y`'s HasBinary.binaryOp(thisLeft = false)
/// when `x` (here a dict, int, list) does not support `|` with `y`; `dict |= y` too
/// (Eval.inplaceBinaryOp only merges dict |= map).
#[test]
fn patch_0014_reflected_bit_or() {
    assert_eq!(
        eval_with_probe("r = {'a': 1} | probe()"),
        Ok(r#"("ror", {"a": 1})"#.into())
    );
    assert_eq!(
        eval_with_probe("r = 1 | probe()"),
        Ok(r#"("ror", 1)"#.into())
    );
    assert_eq!(
        eval_with_probe("r = {'a': 1}\nr |= probe()"),
        Ok(r#"("ror", {"a": 1})"#.into())
    );
    assert_eq!(
        eval_with_probe("r = [1]\nr |= probe()"),
        Ok(r#"("ror", [1])"#.into())
    );
    // Unchanged: dict | dict, dict |= dict, and unsupported operands without a reflected form.
    assert_eq!(
        eval_with_probe("r = {'a': 1} | {'b': 2}"),
        Ok(r#"{"a": 1, "b": 2}"#.into())
    );
    assert_eq!(
        eval_with_probe("d = {'a': 1}\nr = d\nd |= {'b': 2}"),
        Ok(r#"{"a": 1, "b": 2}"#.into())
    );
    assert!(eval_with_probe("r = {'a': 1} | 1").is_err());
    assert!(eval_with_probe("r = probe() | 1").is_err());
}

/// Bazel StarlarkSet.immutableCopyOf / iteration of a `Set` without a thread.
#[test]
fn patch_0015_alloc_set() {
    use starlark::values::set::SetRef;
    use starlark::values::set::alloc_set;
    Module::with_temp_heap(|module| {
        let heap = module.heap();
        let items = vec![heap.alloc("b"), heap.alloc("a"), heap.alloc("b")];
        let set = alloc_set(heap, items).unwrap();
        assert_eq!(set.to_repr(), r#"set(["b", "a"])"#);
        let r = SetRef::from_value(set).unwrap();
        assert_eq!(r.len(), 2);
        assert!(!r.is_empty());
        let elems: Vec<String> = r.iter().map(|v| v.to_repr()).collect();
        assert_eq!(elems, vec![r#""b""#, r#""a""#]);
        assert!(
            alloc_set(
                heap,
                vec![heap.alloc(starlark::values::list::AllocList([1]))]
            )
            .is_err()
        );
        assert!(SetRef::from_value(heap.alloc("x")).is_none());
    });
}
