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

/// Java StringModule.elems returns `StarlarkList.wrap(null, ...)`: an immutable list, so slicing,
/// indexing and `len` work (rules_python's parse_requirements_txt does `content.elems()[:]`).
#[test]
fn patch_0005_elems_list() {
    let ok = |src: &'static str, want: &str| {
        assert_eq!(eval_with_probe(src), Ok(want.to_owned()), "{src}");
    };
    ok(
        "x = 'abc'.elems()\nr = (type(x), x, len(x), x[1], x[1:], list_is_immutable(x))",
        r#"("list", ["a", "b", "c"], 3, "b", ["b", "c"], True)"#,
    );
    ok(
        "y = 'ab'.elems()[:]\ny.append('c')\nr = (y, ''.elems(), list_is_immutable(y))",
        r#"(["a", "b", "c"], [], False)"#,
    );
    assert_eq!(
        eval_with_probe("x = 'ab'.elems()\nx.append('c')\nr = x"),
        Err("trying to mutate a frozen list value".to_owned())
    );
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
    use starlark::eval::DefModule;
    use starlark::eval::Evaluator;
    use starlark::starlark_module;
    use starlark::starlark_simple_value;
    use starlark::values::Heap;
    use starlark::values::NoSerialize;
    use starlark::values::StarlarkValue;
    use starlark::values::Value;
    use starlark::values::none::NoneOr;
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

        /// Patch 0016: Java `StarlarkList.immutableCopyOf(x)`.
        fn immutable_list<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
            let elems: Vec<Value<'v>> = x.iterate(heap)?.collect();
            Ok(starlark::values::list::alloc_immutable_list(heap, &elems))
        }

        /// Patch 0016: `StarlarkList.isImmutable`, None for non-lists.
        fn list_is_immutable<'v>(x: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
            Ok(NoneOr::from_option(
                starlark::values::list::list_is_immutable(x),
            ))
        }

        /// Patch 0018: Java `Dict.immutableCopyOf(x)`.
        fn immutable_dict<'v>(x: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
            let d = starlark::values::dict::DictRef::from_value(x)
                .ok_or_else(|| anyhow::anyhow!("not a dict"))?;
            let content = d.iter_hashed().collect();
            Ok(starlark::values::dict::alloc_immutable_dict(
                heap,
                starlark::values::dict::Dict::new(content),
            ))
        }

        /// Patch 0018: `Dict.isImmutable`, None for non-dicts.
        fn dict_is_immutable<'v>(x: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
            Ok(NoneOr::from_option(
                starlark::values::dict::dict_is_immutable(x),
            ))
        }

        /// Patch 0017: file name of the module `f` was defined in.
        fn def_module_file<'v>(f: Value<'v>) -> anyhow::Result<NoneOr<String>> {
            Ok(NoneOr::from_option(
                DefModule::of_function(f).map(|m| m.file_name().to_owned()),
            ))
        }

        /// Patch 0017: `(file, frozen)` of the `depth`-th innermost Starlark function's module.
        fn innermost_module<'v>(
            depth: i32,
            eval: &mut Evaluator<'v, '_, '_>,
        ) -> anyhow::Result<NoneOr<(String, bool)>> {
            Ok(NoneOr::from_option(
                eval.innermost_enclosing_def_module(depth as usize)
                    .map(|m| (m.file_name().to_owned(), m.is_frozen())),
            ))
        }

        /// Patch 0017: Java `fn.getModule().getGlobal(name)`.
        fn module_global<'v>(
            f: Value<'v>,
            name: &str,
            eval: &mut Evaluator<'v, '_, '_>,
        ) -> anyhow::Result<NoneOr<Value<'v>>> {
            Ok(NoneOr::from_option(
                DefModule::of_function(f).and_then(|m| m.get_global(name, eval)),
            ))
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

/// Java StarlarkList.immutableCopyOf (ImmutableStarlarkList): reads like a list, every mutation
/// fails with Starlark.checkMutable's "trying to mutate a frozen list value", isImmutable is true
/// without looking at the elements, and it stays unhashable ("Even a frozen list is unhashable").
#[test]
fn patch_0016_immutable_list() {
    let base = "x = immutable_list([3, [1], 2])\n";
    let ok = |body: &str, want: &str| {
        let src: &'static str = Box::leak(format!("{base}{body}").into_boxed_str());
        assert_eq!(eval_with_probe(src), Ok(want.to_owned()), "{body}");
    };
    let fails = |body: &str, want: &str| {
        let src: &'static str = Box::leak(format!("{base}{body}").into_boxed_str());
        assert_eq!(eval_with_probe(src), Err(want.to_owned()), "{body}");
    };
    ok(
        "r = (type(x), repr(x), str(x), len(x), x[1], x[-1], x[1:], 2 in x, x == [3, [1], 2], [3, [1], 2] == x)",
        r#"("list", "[3, [1], 2]", "[3, [1], 2]", 3, [1], 2, [[1], 2], True, True, True)"#,
    );
    ok("r = [e for e in x] + x", "[3, [1], 2, 3, [1], 2]");
    ok(
        "r = (sorted([3, 1, 2]), x * 2, list(x), list_is_immutable(x))",
        "([1, 2, 3], [3, [1], 2, 3, [1], 2], [3, [1], 2], True)",
    );
    // Copies are ordinary mutable lists.
    ok(
        "y = x + []\ny.append(1)\nr = (y, list_is_immutable(y), list_is_immutable(x[:]))",
        "([3, [1], 2, 1], False, False)",
    );
    // Elements keep their own mutability (Java copies references).
    ok("x[1].append(5)\nr = x", "[3, [1, 5], 2]");
    for m in [
        "x.append(1)",
        "x.extend([1])",
        "x.insert(0, 1)",
        "x.pop()",
        "x.remove(2)",
        "x.clear()",
        "x[0] = 1",
        "x += [1]",
    ] {
        fails(m, "trying to mutate a frozen list value");
    }
    fails("r = {x: 1}", "Value of type `list` is not hashable");
    ok(
        "r = (list_is_immutable([1]), list_is_immutable((1,)), list_is_immutable(immutable_list([])))",
        "(False, None, True)",
    );
}

/// Java StarlarkFunction.getModule / Module.ofInnermostEnclosingStarlarkFunction: a function's
/// module is where its `def` ran (closures made by a loaded function belong to the loaded
/// module); the toplevel of the executing module counts as a Starlark function. `f`, `g` and `g2`
/// (`return` of a call) are inlined by starlark-rust's optimizer; `k` is not.
#[test]
fn patch_0017_def_module() {
    use starlark::environment::FrozenModule;
    use starlark::eval::ReturnFileLoader;
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let (mut builder, _, _) = dialect::bazel_universe_builder();
            probe::globals(&mut builder);
            let globals = builder.build();
            let run = |file: &str, src: &str, loads: &[(&str, &FrozenModule)]| {
                let ast = AstModule::parse(file, src.to_owned(), &dialect::bzl_dialect())
                    .map_err(|e| bare_message(&e))
                    .unwrap();
                Module::with_temp_heap(|module| {
                    let loader = ReturnFileLoader { modules: &loads.iter().copied().collect() };
                    let mut e = Evaluator::new(&module);
                    e.set_loader(&loader);
                    e.eval_module(ast, &globals).map_err(|e| bare_message(&e)).unwrap();
                    drop(e);
                    let r = module.get("r").map(|v| v.to_repr()).unwrap_or_default();
                    anyhow::Ok((r, module.freeze()?))
                })
                .unwrap()
            };
            let (r, a) = run(
                "a.bzl",
                r#"
def f():
    return innermost_module(0)
def make():
    return lambda: innermost_module(0)
def g():
    return (innermost_module(0), innermost_module(1), innermost_module(2))
def g2():
    return g()
def k():
    x = innermost_module(0)
    return (x, innermost_module(1))
def _private():
    pass
r = (def_module_file(f), f(), innermost_module(0), module_global(f, "f") == f, module_global(f, "_private") == _private, module_global(f, "nope"))
"#,
                &[],
            );
            assert_eq!(
                r,
                r#"("a.bzl", ("a.bzl", False), ("a.bzl", False), True, True, None)"#
            );
            let (r, _) = run(
                "b.bzl",
                r#"
load("a.bzl", "f", "g", "g2", "k", "make")
def h():
    return g()
c = make()
r = (def_module_file(f), def_module_file(c), def_module_file(h), def_module_file(len), def_module_file(1),
     f(), c(), h(), innermost_module(0), innermost_module(1),
     module_global(f, "f") == f, module_global(f, "make") == make, module_global(c, "f") == f,
     module_global(h, "h") == h, module_global(f, "h"),
     g2(), k())
"#,
                &[("a.bzl", &a)],
            );
            assert_eq!(
                r,
                concat!(
                    r#"("a.bzl", "a.bzl", "b.bzl", None, None, "#,
                    r#"("a.bzl", True), ("a.bzl", True), "#,
                    r#"(("a.bzl", True), ("b.bzl", False), ("b.bzl", False)), "#,
                    r#"("b.bzl", False), None, "#,
                    r#"True, True, True, True, None, "#,
                    r#"(("a.bzl", True), ("a.bzl", True), ("b.bzl", False)), "#,
                    r#"(("a.bzl", True), ("b.bzl", False)))"#
                )
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Java Dict.immutableCopyOf: reads like a dict; every mutation fails with Starlark.checkMutable's
/// "trying to mutate a frozen dict value" (checked before the key lookup, as in Dict.pop);
/// isImmutable is true without looking at keys or values (Dict.isImmutable: mutability frozen);
/// still unhashable ("Even a frozen dict is unhashable").
#[test]
fn patch_0018_immutable_dict() {
    let run = |body: &str| {
        let src: &'static str = Box::leak(
            format!("d = immutable_dict({{'a': [1], 2: 'b'}})\ndef f():\n    {body}\nf()\n")
                .into_boxed_str(),
        );
        eval_with_probe(src)
    };
    let ok = |body: &str, want: &str| {
        let src: &'static str =
            Box::leak(format!("d = immutable_dict({{'a': [1], 2: 'b'}})\n{body}").into_boxed_str());
        assert_eq!(eval_with_probe(src), Ok(want.to_owned()), "{body}");
    };
    ok(
        "r = (type(d), repr(d), len(d), d['a'], d.get(2), d.get(3, 0), 'a' in d, 3 in d, d == {'a': [1], 2: 'b'}, {2: 'b', 'a': [1]} == d)",
        r#"("dict", "{\"a\": [1], 2: \"b\"}", 2, [1], "b", 0, True, False, True, True)"#,
    );
    ok(
        "r = (d.keys(), d.values(), d.items(), [k for k in d], dict(d), d | {3: 4}, dict_is_immutable(d))",
        r#"(["a", 2], [[1], "b"], [("a", [1]), (2, "b")], ["a", 2], {"a": [1], 2: "b"}, {"a": [1], 2: "b", 3: 4}, True)"#,
    );
    // Copies are mutable; values keep their own mutability.
    ok(
        "e = dict(d)\ne[5] = 6\nd['a'].append(2)\nr = (d, e, dict_is_immutable(e), dict_is_immutable({}), dict_is_immutable(immutable_dict({})), dict_is_immutable([]))",
        r#"({"a": [1, 2], 2: "b"}, {"a": [1, 2], 2: "b", 5: 6}, False, False, True, None)"#,
    );
    for m in [
        "d['c'] = 1",
        "d['a'] = 1",
        "d.update({'c': 1})",
        "d.update()",
        "d.pop('a')",
        "d.pop('zz', 0)",
        "d.popitem()",
        "d.setdefault('c', 1)",
        "d.setdefault('a')",
        "d.clear()",
        "x = d\n    x |= {'c': 1}",
        "x = d\n    x |= x",
    ] {
        assert_eq!(
            run(m),
            Err("trying to mutate a frozen dict value".to_owned()),
            "{m}"
        );
    }
    assert!(
        eval_with_probe("r = {immutable_dict({1: 2}): 1}")
            .unwrap_err()
            .contains("not hashable")
    );
}

/// Java Dict.get / Dict.setdefault / Dict.pop: `default` is `named = true`; `key` is positional.
#[test]
fn patch_0019_dict_named_default() {
    assert_eq!(
        eval_with_probe(
            "d = {'a': 1}\nr = (d.get('b', default = 2), d.get('a', default = 2), d.setdefault('c', default = 3), d.pop('a', default = 4), d)"
        ),
        Ok(r#"(2, 1, 3, 1, {"c": 3})"#.to_owned())
    );
    assert!(eval_with_probe("r = {}.get(key = 'a')").is_err());
}

/// Parses `src` with Bazel 9.3.2's default type syntax mode (`DialectTypes::ParseOnly`:
/// FileOptions allowTypeSyntax, no resolveTypeSyntax, tolerateInvalidTypeExpressions) and
/// evaluates it; returns repr of `r` or the error message.
fn eval_with_type_syntax(src: &'static str) -> Result<String, String> {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let globals = dialect::bazel_universe();
            let mut d = dialect::bzl_dialect();
            d.enable_types = starlark::syntax::DialectTypes::ParseOnly;
            let ast =
                AstModule::parse("t.bzl", src.to_owned(), &d).map_err(|e| bare_message(&e))?;
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

/// Java Parser.java (type syntax; `--experimental_starlark_type_syntax` defaults to true at
/// 9.3.2, FlagConstants.java:36) with BzlCompileFunction's options: annotations are arbitrary
/// expressions (Parser.parseTypeExprWithFallback), not resolved (Resolver.java:874) and not
/// evaluated; `cast(T, e)` is `e` (Eval.java:568); `type X = T` does nothing (Eval.java:322).
#[test]
fn patch_0020_bazel_type_syntax() {
    // Annotations on parameters, the return type and assignments, with names that are not
    // defined anywhere and expressions that are not type expressions.
    assert_eq!(
        eval_with_type_syntax(
            "def f(x: int, y: Undefined[str] = 2, *args: list[NoSuchType], **kwargs: 1 + 2) -> Nope | None:\n    z: whatever = x + y\n    return z\nr = f(1)"
        ),
        Ok("3".into())
    );
    // The same file without type syntax is a syntax error.
    assert!(eval("def f(x: int):\n    return x\nr = f(1)").is_err());
    // cast() evaluates to its operand; the type is not evaluated.
    assert_eq!(
        eval_with_type_syntax("r = cast(list[Undefined], [1, 2],)"),
        Ok("[1, 2]".into())
    );
    // `cast` and `isinstance` are keywords when type syntax is allowed (Lexer.java:615) ...
    assert!(eval_with_type_syntax("cast = 1\nr = cast").is_err());
    assert!(eval_with_type_syntax("def isinstance(x):\n    return x\nr = 1").is_err());
    // ... and ordinary identifiers when it is not.
    assert_eq!(eval("cast = 1\nr = cast"), Ok("1".into()));
    // Resolver.java:897
    assert_eq!(
        eval_with_type_syntax("r = isinstance(1, int)"),
        Err("isinstance() is not yet supported".into())
    );
    // Type aliases bind nothing and are top-level only (Resolver.java:906).
    assert_eq!(
        eval_with_type_syntax("type X = list[Undefined]\ntype Y[T, U] = dict[T, U]\nr = 1"),
        Ok("1".into())
    );
    assert_eq!(
        eval_with_type_syntax("def f():\n    type X = int\nr = 1"),
        Err("type alias statement not at top level".into())
    );
    // `type` stays an ordinary identifier.
    assert_eq!(eval_with_type_syntax("r = type(1)"), Ok("\"int\"".into()));
    // Generic defs: `def f[T](...)`.
    assert_eq!(
        eval_with_type_syntax("def f[T, U](x: T) -> U:\n    return x\nr = f(7)"),
        Ok("7".into())
    );
    // A var statement (`x: T` without a value) is not ported: a loud, recognizable error.
    let err = eval_with_type_syntax("x: int\nr = 1").unwrap_err();
    assert!(
        err.contains("BLAZE not ported yet: Bazel type syntax: VarStatement"),
        "{err}"
    );
}

/// Java Module.getGlobals(): every global binding, `_private` names included (and, in a file
/// compiled with loadBindsGlobally, names bound by load()).
#[test]
fn patch_0021_frozen_module_names_any_visibility() {
    use starlark::eval::ReturnFileLoader;
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let globals = dialect::bazel_universe();
            let run =
                |file: &str, src: &str, loads: &[(&str, &starlark::environment::FrozenModule)]| {
                    let ast = AstModule::parse(file, src.to_owned(), &dialect::bzl_dialect())
                        .map_err(|e| bare_message(&e))
                        .unwrap();
                    Module::with_temp_heap(|module| {
                        let loader = ReturnFileLoader {
                            modules: &loads.iter().copied().collect(),
                        };
                        let mut e = Evaluator::new(&module);
                        e.set_loader(&loader);
                        e.eval_module(ast, &globals)
                            .map_err(|e| bare_message(&e))
                            .unwrap();
                        drop(e);
                        module.freeze()
                    })
                    .unwrap()
                };
            let a = run("a.bzl", "foo = 'FOO'\n", &[]);
            let b = run(
                "b.bzl",
                "load('a.bzl', 'foo')\n_hidden = 1\npublic = 2\n",
                &[("a.bzl", &a)],
            );
            let mut public: Vec<String> = b.names().map(|n| n.as_str().to_owned()).collect();
            public.sort();
            assert_eq!(public, ["public"]);
            let mut all: Vec<String> = b
                .names_any_visibility()
                .map(|n| n.as_str().to_owned())
                .collect();
            all.sort();
            assert_eq!(all, ["_hidden", "foo", "public"]);
            assert_eq!(
                b.get_any_visibility("foo").unwrap().0.value().to_repr(),
                "\"FOO\""
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Java `fn.getModule().getClientData()`: a frozen function reaches the extra value of the
/// module that defined it, also when two modules were executed from the same file name.
#[test]
fn patch_0022_def_module_frozen_extra_value() {
    use starlark::eval::DefModule;
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let globals = dialect::bazel_universe();
            let run = |extra: i32| {
                let ast = AstModule::parse(
                    "same.bzl",
                    "def f():\n    pass\n".to_owned(),
                    &dialect::bzl_dialect(),
                )
                .map_err(|e| bare_message(&e))
                .unwrap();
                Module::with_temp_heap(|module| {
                    module.set_extra_value(module.heap().alloc(extra));
                    let mut e = Evaluator::new(&module);
                    e.eval_module(ast, &globals)
                        .map_err(|e| bare_message(&e))
                        .unwrap();
                    // While the module executes, its defs are not frozen.
                    let f = module.get("f").unwrap();
                    let m = DefModule::of_function(f).unwrap();
                    assert!(!m.is_frozen());
                    assert!(m.frozen_extra_value().is_none());
                    drop(e);
                    module.freeze()
                })
                .unwrap()
            };
            let one = run(1);
            let two = run(2);
            let extra_of = |m: &starlark::environment::FrozenModule| {
                let f = m.get("f").unwrap();
                let d = DefModule::of_function(f.value()).unwrap();
                assert!(d.is_frozen());
                d.frozen_extra_value().map(|v| v.to_value().to_repr())
            };
            assert_eq!(extra_of(&one), Some("1".to_owned()));
            assert_eq!(extra_of(&two), Some("2".to_owned()));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Java Starlark.checkMutable: "trying to mutate a frozen %s value" for a value frozen with its
/// module (PackageFunctionTest.testPreludeSymbolCannotBeMutated).
#[test]
fn patch_0023_frozen_mutation_text() {
    use starlark::eval::ReturnFileLoader;
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let globals = dialect::bazel_universe();
            let a = {
                let ast = AstModule::parse(
                    "a.bzl",
                    "L = ['FOO']\nD = {'a': 1}\n".to_owned(),
                    &dialect::bzl_dialect(),
                )
                .unwrap();
                Module::with_temp_heap(|module| {
                    let mut e = Evaluator::new(&module);
                    e.eval_module(ast, &globals).unwrap();
                    drop(e);
                    module.freeze()
                })
                .unwrap()
            };
            let run = |src: &str| -> Result<(), String> {
                let ast = AstModule::parse("b.bzl", src.to_owned(), &dialect::bzl_dialect())
                    .map_err(|e| bare_message(&e))?;
                Module::with_temp_heap(|module| {
                    let modules = [("a.bzl", &a)].into_iter().collect();
                    let loader = ReturnFileLoader { modules: &modules };
                    let mut e = Evaluator::new(&module);
                    e.set_loader(&loader);
                    e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
                    Ok(())
                })
            };
            for (src, want) in [
                (
                    "load('a.bzl', 'L')\nL.append('BAR')\n",
                    "trying to mutate a frozen list value",
                ),
                (
                    "load('a.bzl', 'L')\nL[0] = 'BAR'\n",
                    "trying to mutate a frozen list value",
                ),
                (
                    "load('a.bzl', 'D')\nD['b'] = 2\n",
                    "trying to mutate a frozen dict value",
                ),
                (
                    "load('a.bzl', 'D')\nD.pop('a')\n",
                    "trying to mutate a frozen dict value",
                ),
            ] {
                let err = run(src).unwrap_err();
                assert!(err.contains(want), "{src:?}: {err}");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
