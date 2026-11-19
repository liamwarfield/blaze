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

/// Java StringModule.split/rsplit (StringModule.java:387, 426): `sep` required and nameable,
/// `maxsplit` nameable (negative: no limit), "Empty separator"; replace (StringModule.java:338):
/// a negative count means no limit.
#[test]
fn patch_0005_split_and_replace() {
    assert_eq!(
        eval(
            "r = ('a.b.c'.split(sep = '.', maxsplit = 1), 'a.b.c'.rsplit(sep = '.', maxsplit = 1), 'a-b-c'.split('-', -1), 'a-b-c'.rsplit('-', 0))"
        ),
        Ok(r#"(["a", "b.c"], ["a.b", "c"], ["a", "b", "c"], ["a-b-c"])"#.into())
    );
    assert_eq!(eval("r = 'abc'.rsplit('')"), Err("Empty separator".into()));
    assert_eq!(eval("r = 'abc'.split('')"), Err("Empty separator".into()));
    // Java has no whitespace split: `sep` is mandatory.
    assert!(eval("r = 'a b'.split()").is_err());
    assert_eq!(
        eval(
            "r = ('banana'.replace('a', 'o', -2), 'banana'.replace('a', 'o', 2), 'abc'.replace('', '-', 2))"
        ),
        Ok(r#"("bonono", "bonona", "-a-bc")"#.into())
    );
}

/// Java StarlarkSet (StarlarkSet.java:216 repr, 389-785 methods) and Eval.inplaceBinaryOp
/// (Eval.java:498-536).
#[test]
fn patch_0007_set_semantics() {
    assert_eq!(
        eval("r = (repr(set()), str(set([1])))"),
        Ok(r#"("set()", "set([1])")"#.into())
    );
    // Augmented assignment mutates the left set in place (aliases see it).
    assert_eq!(
        eval(
            "s = set([1, 2])\nt = s\ns |= set([3])\ns &= set([1, 3])\ns ^= set([3, 4])\ns -= set([4])\nt.add(9)\nr = (t, s)"
        ),
        Ok("(set([1, 9]), set([1, 9]))".into())
    );
    assert_eq!(
        eval("s = set([1])\nt = s\ns |= [2]\nr = t"),
        Err("Operation `|` not supported for types `set` and `list`".into())
    );
    // Variadic methods; dicts contribute keys.
    assert_eq!(
        eval(
            "s = set([1, 2, 3])\nr = (s.union([4], {5: 0}), s.intersection([1, 2], [2, 3]), s.difference([1], [3]), s.union())"
        ),
        Ok("(set([1, 2, 3, 4, 5]), set([2]), set([2]), set([1, 2, 3]))".into())
    );
    assert_eq!(
        eval(
            "s = set([1, 2, 3, 4])\ns.update([5], [6])\ns.intersection_update([1, 2, 3, 5, 6], [2, 3, 5, 6])\ns.difference_update([2], [6])\ns.symmetric_difference_update([3, 7])\nr = s"
        ),
        Ok("set([5, 7])".into())
    );
    assert_eq!(
        eval(
            "s = set([1, 2])\nr = (s.isdisjoint([3]), s.isdisjoint([2]), s.issubset(range(3)), s.issuperset((1,)))"
        ),
        Ok("(True, False, True, True)".into())
    );
    // pop removes the first element (insertion order).
    assert_eq!(
        eval("s = set([3, 1, 2])\nr = (s.pop(), s.pop(), s)"),
        Ok("(3, 1, set([2]))".into())
    );
    assert_eq!(eval("r = set().pop()"), Err("set is empty".into()));
    assert_eq!(
        eval("r = set().remove(1)"),
        Err("element 1 not found in set".into())
    );
    assert_eq!(
        eval("r = set().update(1)"),
        Err(
            "for update argument got value of type 'int', want a collection of hashable elements"
                .into()
        )
    );
    assert_eq!(
        eval("r = set().issubset('ab')"),
        Err("for issubset argument got value of type 'string', want a collection of hashable elements".into())
    );
}

/// Java StarlarkFloat.format (StarlarkFloat.java:120), Printer.formatWithList
/// (Printer.java:353-400), StarlarkInt.compareIntAndDouble (StarlarkInt.java:756),
/// StarlarkInt.toFiniteDouble (StarlarkInt.java:376), division texts (StarlarkFloat.java:213-229).
#[test]
fn patch_0008_floats() {
    assert_eq!(
        eval("r = [str(x) for x in (0.0, -0.0, 0.1, 1.0, 1e16, 1e17, 1.23e45, 1e-4, 1e-5, 1.5557538194652854e-61, 123456789.125)]"),
        Ok(r#"["0.0", "-0.0", "0.1", "1.0", "10000000000000000.0", "1e+17", "1.23e+45", "0.0001", "1e-05", "1.5557538194652854e-61", "123456789.125"]"#.into())
    );
    assert_eq!(
        eval("r = ('%e' % 123, '%e' % -1.23e-45, '%f' % 1.23e45, '%f' % -1.23e-45, '%g' % 1.110, '%E' % 0.000999999949, '%f' % 0.0000009)"),
        Ok(r#"("1.230000e+02", "-1.230000e-45", "1230000000000000000000000000000000000000000000.000000", "-0.000000", "1.11", "9.999999E-04", "0.000001")"#.into())
    );
    assert_eq!(
        eval("r = ('%d' % 1.23e45, '%x' % 123.0, '%o' % -8, '%x' % -1, '%X' % (-1 << 40), '%x' % (-1 << 128))"),
        Ok(r#"("1229999999999999973814869011019624571608236032", "7b", "37777777770", "ffffffff", "FFFFFF0000000000", "-100000000000000000000000000000000")"#.into())
    );
    assert_eq!(
        eval("r = '%d' % float('nan')"),
        Err("got nan, want a finite number".into())
    );
    assert_eq!(
        eval("r = '%x' % 'a'"),
        Err("got string for '%x' format, want int or float".into())
    );
    // Exact int/float comparison.
    assert_eq!(
        eval(
            "r = (1.23e45 == 1229999999999999973814869011019624571608236032, 1.23e45 != 1229999999999999973814869011019624571608236033, float((1 << 53) + 1) != (1 << 53) + 1, (1 << 53) + 1 > float((1 << 53) + 1))"
        ),
        Ok("(True, True, True, True)".into())
    );
    assert_eq!(
        eval("r = float(1 << 500 << 500 << 100)"),
        Err("int too large to convert to float".into())
    );
    assert_eq!(
        eval("r = (1 << 500 << 500 << 100) + 0.0"),
        Err("int too large to convert to float".into())
    );
    assert_eq!(
        eval("r = 1.0 / (1 << 500 << 500 << 100)"),
        Err("int too large to convert to float".into())
    );
    assert_eq!(
        eval("r = 1.0 / 0"),
        Err("floating-point division by zero".into())
    );
    assert_eq!(eval("r = 1.0 // 0"), Err("integer division by zero".into()));
    assert_eq!(
        eval("r = 1.0 % 0"),
        Err("floating-point modulo by zero".into())
    );
    assert_eq!(eval("r = 1 // 0"), Err("integer division by zero".into()));
    assert_eq!(eval("r = 1 % 0"), Err("integer modulo by zero".into()));
    assert_eq!(
        eval("r = int(float('-inf'))"),
        Err("can't convert float -inf to int".into())
    );
}

/// Java BuiltinFunction (type `builtin_function_or_method`, repr BuiltinFunction.java:192) and
/// StarlarkFunction.repr (StarlarkFunction.java:222).
#[test]
fn patch_0009_function_type_and_repr() {
    assert_eq!(
        eval("def f():\n    pass\nr = (type(len), str(len), type([].clear), str([].clear), str({}.get), type(f), str(f), str(lambda: 0))"),
        Ok(r#"("builtin_function_or_method", "<built-in function len>", "builtin_function_or_method", "<built-in method clear of list value>", "<built-in method get of dict value>", "function", "<function f>", "<function lambda>")"#.into())
    );
    // Builtins are unhashable (StarlarkValue.checkHashable: not immutable); defs are hashable.
    assert!(eval("r = {len: 1}").is_err());
    assert_eq!(eval("def f():\n    pass\nr = len({f: 1})"), Ok("1".into()));
}

/// Java Printer.repr (Printer.java:191): a value met again while printing is `...`.
#[test]
fn patch_0031_cycle_repr() {
    assert_eq!(
        eval("x = [1, 2]\nx.append(x)\nd = {1: 2}\nd[3] = d\nr = (repr(x), str(d))"),
        Ok(r#"("[1, 2, ...]", "{1: 2, 3: ...}")"#.into())
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

    /// Patches 0026 and 0027: attributes, `[]` and `in` that fail or read the evaluator.
    /// `ok` is 1, `bad` fails, `marker` is the executing module's global `marker`; `x[i]` is
    /// `(i, marker)`; `v in x` fails for "bad", else is `v == marker`.
    #[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
    pub struct Fallible;

    impl std::fmt::Display for Fallible {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "fallible")
        }
    }

    starlark_simple_value!(Fallible);

    fn marker<'v>(eval: &Evaluator<'v, '_, '_>) -> Value<'v> {
        eval.module().get("marker").unwrap_or_else(Value::new_none)
    }

    #[starlark_value(type = "fallible")]
    impl<'v> StarlarkValue<'v> for Fallible {
        fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
            (attribute == "ok").then(|| _heap.alloc(1))
        }

        fn get_attr_fallible(
            &self,
            attribute: starlark::collections::Hashed<&str>,
            eval: &mut Evaluator<'v, '_, '_>,
        ) -> starlark::Result<Option<Value<'v>>> {
            match *attribute.key() {
                "bad" => Err(starlark::Error::new_other(anyhow::anyhow!(
                    "computing 'bad' failed"
                ))),
                "marker" => Ok(Some(marker(eval))),
                other => Ok(self.get_attr(other, eval.heap())),
            }
        }

        fn has_attr_fallible(
            &self,
            attribute: &str,
            _eval: &mut Evaluator<'v, '_, '_>,
        ) -> starlark::Result<bool> {
            match attribute {
                "bad" => Err(starlark::Error::new_other(anyhow::anyhow!(
                    "computing 'bad' failed"
                ))),
                other => Ok(other == "ok" || other == "marker"),
            }
        }

        fn at_with_eval(
            &self,
            index: Value<'v>,
            eval: &mut Evaluator<'v, '_, '_>,
        ) -> starlark::Result<Value<'v>> {
            let m = marker(eval);
            Ok(eval.heap().alloc(AllocTuple([index, m])))
        }

        fn is_in_with_eval(
            &self,
            other: Value<'v>,
            eval: &mut Evaluator<'v, '_, '_>,
        ) -> starlark::Result<bool> {
            if other.unpack_str() == Some("bad") {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "'bad' in fallible"
                )));
            }
            other.equals(marker(eval))
        }
    }

    #[starlark_module]
    pub fn globals(builder: &mut GlobalsBuilder) {
        fn probe() -> anyhow::Result<Probe> {
            Ok(Probe)
        }

        fn fallible() -> anyhow::Result<Fallible> {
            Ok(Fallible)
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

        /// Patch 0025: Java `StarlarkSet.immutableCopyOf(x)`.
        fn immutable_set<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
            let elems: Vec<Value<'v>> = x.iterate(heap)?.collect();
            starlark::values::set::alloc_immutable_set(heap, elems)
        }

        /// Patch 0025: `StarlarkSet.isImmutable`, None for non-sets.
        fn set_is_immutable<'v>(x: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
            Ok(NoneOr::from_option(
                starlark::values::set::set_is_immutable(x),
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
    // A var statement (`x: T` without a value) binds `x` without a value (patch 0030).
    assert_eq!(eval_with_type_syntax("x: int\nr = 1"), Ok("1".into()));
}

/// Java Parser.java:1582 (VarStatement), Resolver.java:562 (bound in the enclosing block),
/// Eval.java:324 (executing it does nothing).
#[test]
fn patch_0030_var_statement() {
    // A global declared but not assigned reads as unassigned; a later assignment sets it.
    assert!(
        eval_with_type_syntax("x: int\nr = x")
            .unwrap_err()
            .contains("referenced before assignment")
    );
    assert_eq!(
        eval_with_type_syntax("x: list[Undefined]\ndef f():\n    return x\nx = [1]\nr = f()"),
        Ok("[1]".into())
    );
    // In a function it declares a local: the global of the same name is shadowed.
    assert!(
        eval_with_type_syntax("y = 1\ndef f():\n    y: int\n    return y\nr = f()")
            .unwrap_err()
            .contains("referenced before assignment")
    );
    assert_eq!(
        eval_with_type_syntax("def f():\n    y: int\n    y = 2\n    return y\nr = f()"),
        Ok("2".into())
    );
    // Only a single identifier may be annotated (Parser.java:1586).
    assert_eq!(
        eval_with_type_syntax("a, b: int\nr = 1"),
        Err("type annotations must have a single identifier on the left-hand side".into())
    );
    assert_eq!(
        eval_with_type_syntax("x = struct()\nx.f: int\nr = 1"),
        Err("type annotations must have a single identifier on the left-hand side".into())
    );
    // Without type syntax it stays a syntax error.
    assert!(eval("x: int\nr = 1").is_err());
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

/// Java StarlarkSet.immutableCopyOf (net/starlark/java/eval/StarlarkSet.java:374): reads like a
/// set; every mutator calls Starlark.checkMutable first ("trying to mutate a frozen set value",
/// before the element is looked at and before the for-loop check); isImmutable is true
/// (StarlarkSet.java:187: mutability frozen); still unhashable (StarlarkSet.java:206 "Even a
/// frozen set is unhashable"). A set frozen with its module fails with the same text (as 0023
/// for lists and dicts).
#[test]
fn patch_0025_immutable_set() {
    use starlark::eval::ReturnFileLoader;
    use starlark::values::set::SetRef;
    use starlark::values::set::alloc_immutable_set;
    use starlark::values::set::alloc_set;
    use starlark::values::set::set_is_immutable;
    // The native functions (`alloc_immutable_set` takes what 0015's `alloc_set` takes).
    Module::with_temp_heap(|module| {
        let heap = module.heap();
        let items = vec![heap.alloc("b"), heap.alloc("a"), heap.alloc("b")];
        let set = alloc_immutable_set(heap, items).unwrap();
        assert_eq!(set.get_type(), "set");
        assert_eq!(set.to_repr(), r#"set(["b", "a"])"#);
        assert_eq!(SetRef::from_value(set).unwrap().len(), 2);
        assert_eq!(set_is_immutable(set), Some(true));
        let empty = alloc_immutable_set(heap, Vec::<starlark::values::Value>::new()).unwrap();
        assert!(SetRef::from_value(empty).unwrap().is_empty());
        assert_eq!(set_is_immutable(empty), Some(true));
        let mutable = alloc_set(heap, vec![heap.alloc("a")]).unwrap();
        assert_eq!(set_is_immutable(mutable), Some(false));
        assert_eq!(set_is_immutable(heap.alloc("x")), None);
        assert!(
            alloc_immutable_set(
                heap,
                vec![heap.alloc(starlark::values::list::AllocList([1]))]
            )
            .is_err()
        );
    });

    let base = "x = immutable_set([3, \"a\", 3])\n";
    let ok = |body: &str, want: &str| {
        let src: &'static str = Box::leak(format!("{base}{body}").into_boxed_str());
        assert_eq!(eval_with_probe(src), Ok(want.to_owned()), "{body}");
    };
    let run = |body: &str| {
        let src: &'static str =
            Box::leak(format!("{base}def f():\n    {body}\nf()\n").into_boxed_str());
        eval_with_probe(src)
    };
    ok(
        "r = (type(x), repr(x), str(x), len(x), 3 in x, 4 in x, x == set([3, \"a\"]), set([\"a\", 3]) == x, list(x))",
        r#"("set", "set([3, \"a\"])", "set([3, \"a\"])", 2, True, False, True, True, [3, "a"])"#,
    );
    ok(
        "r = (x.union([1]), x | set([1]), x.intersection([3]), x - set([3]), [e for e in x], bool(x), set_is_immutable(x))",
        r#"(set([3, "a", 1]), set([3, "a", 1]), set([3]), set(["a"]), [3, "a"], True, True)"#,
    );
    // Copies are ordinary mutable sets.
    ok(
        "y = set(x)\ny.add(5)\nz = x | set()\nz.add(6)\nr = (x, y, z, set_is_immutable(y), set_is_immutable(z), set_is_immutable(set([1])), set_is_immutable(immutable_set([])), set_is_immutable([]))",
        r#"(set([3, "a"]), set([3, "a", 5]), set([3, "a", 6]), False, False, False, True, None)"#,
    );
    for m in [
        "x.add(1)",
        "x.add(3)",
        "x.update([1])",
        "x.update(x)",
        "x.remove(3)",
        "x.remove(99)",
        "x.discard(3)",
        "x.discard(99)",
        "x.pop()",
        "x.clear()",
        "for e in x:\n        x.add(1)",
    ] {
        assert_eq!(
            run(m),
            Err("trying to mutate a frozen set value".to_owned()),
            "{m}"
        );
    }
    assert!(
        eval_with_probe("r = {immutable_set([1]): 1}")
            .unwrap_err()
            .contains("not hashable")
    );

    // Frozen with its module: a set on a frozen heap (was "Immutable"), and a flagged set that
    // became an ordinary frozen set.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let (mut builder, _, _) = dialect::bazel_universe_builder();
            probe::globals(&mut builder);
            let globals = builder.build();
            let a = {
                let ast = AstModule::parse(
                    "a.bzl",
                    "S = set([1, 2])\nX = immutable_set([1, 2])\n".to_owned(),
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
            let run = |src: &str| -> Result<String, String> {
                let ast = AstModule::parse("b.bzl", src.to_owned(), &dialect::bzl_dialect())
                    .map_err(|e| bare_message(&e))?;
                Module::with_temp_heap(|module| {
                    let modules = [("a.bzl", &a)].into_iter().collect();
                    let loader = ReturnFileLoader { modules: &modules };
                    let mut e = Evaluator::new(&module);
                    e.set_loader(&loader);
                    e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
                    Ok(module.get("r").map(|v| v.to_repr()).unwrap_or_default())
                })
            };
            assert_eq!(
                run(
                    "load('a.bzl', 'S', 'X')\nr = (S, X, S == X, 1 in X, set_is_immutable(S), set_is_immutable(X), set_is_immutable(set(X)))\n"
                ),
                Ok("(set([1, 2]), set([1, 2]), True, True, True, True, False)".to_owned())
            );
            for src in [
                "load('a.bzl', 'S')\nS.add(3)\n",
                "load('a.bzl', 'S')\nS.update([3])\n",
                "load('a.bzl', 'S')\nS.remove(1)\n",
                "load('a.bzl', 'S')\nS.discard(1)\n",
                "load('a.bzl', 'S')\nS.pop()\n",
                "load('a.bzl', 'S')\nS.clear()\n",
                "load('a.bzl', 'X')\nX.add(3)\n",
                "load('a.bzl', 'X')\nX.clear()\n",
            ] {
                let err = run(src).unwrap_err();
                assert!(
                    err.contains("trying to mutate a frozen set value"),
                    "{src:?}: {err}"
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Java StarlarkFunction.getLocation (net/starlark/java/eval/StarlarkFunction.java:172): the name
/// of a `def`, the `lambda` keyword of a lambda. The source is the map_each probe of
/// blaze_actions, whose oracle recording (`tests/data/map_each_probe/expected.txt`) prints
/// "declared at <file>:9:9" for the nested def `inner` and "<file>:21:11" for the top-level
/// lambda.
#[test]
fn patch_0024_def_declaration_span() {
    use starlark::eval::DefModule;
    const CASES_BZL: &str =
        include_str!("../../blaze_actions/tests/data/map_each_probe/workspace/cases.bzl");
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let globals = dialect::bazel_universe();
            // The first 21 lines: the functions under test. (The rest calls `depset`, which this
            // crate's universe does not have.)
            let mut source: String = CASES_BZL
                .lines()
                .take(21)
                .map(|l| format!("{l}\n"))
                .collect();
            source.push_str("_not_a_def = len\nnested = _nested\nlam = _lambda\n");
            let ast = AstModule::parse("/workspace/cases.bzl", source, &dialect::bzl_dialect())
                .map_err(|e| bare_message(&e))
                .unwrap();
            // (file, 1-based line, 1-based column) of the start of the declaration span.
            let position = |f: starlark::values::Value| {
                DefModule::declaration_span(f).map(|span| {
                    let begin = span.resolve_span().begin;
                    (span.filename().to_owned(), begin.line + 1, begin.column + 1)
                })
            };
            let at = |line, column| Some(("/workspace/cases.bzl".to_owned(), line, column));
            let frozen = Module::with_temp_heap(|module| {
                let mut e = Evaluator::new(&module);
                e.eval_module(ast, &globals)
                    .map_err(|e| bare_message(&e))
                    .unwrap();
                drop(e);
                // While the module executes.
                assert_eq!(position(module.get("nested").unwrap()), at(9, 9));
                assert_eq!(position(module.get("lam").unwrap()), at(21, 11));
                module.freeze()
            })
            .unwrap();
            // Frozen: the nested def, a top-level def and its alias, the lambda.
            let get = |name: &str| position(frozen.get_any_visibility(name).unwrap().0.value());
            assert_eq!(get("_nested"), at(9, 9));
            assert_eq!(get("_mk"), at(8, 5));
            assert_eq!(get("_top"), at(16, 5));
            assert_eq!(get("_alias"), at(16, 5));
            assert_eq!(get("_lambda"), at(21, 11));
            // Not a def or lambda.
            assert_eq!(get("_not_a_def"), None);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Bazel: `Structure.getValue` may throw; `Starlark.getattr` (Starlark.java:978) returns the
/// default only for a missing field and propagates the error otherwise; `Starlark.hasattr`
/// (Starlark.java:967) propagates it too.
#[test]
fn patch_0026_fallible_attributes() {
    assert_eq!(eval_with_probe("r = fallible().ok"), Ok("1".into()));
    assert_eq!(
        eval_with_probe("marker = 'm'\nr = fallible().marker"),
        Ok(r#""m""#.into())
    );
    assert_eq!(
        eval_with_probe("r = fallible().bad"),
        Err("computing 'bad' failed".into())
    );
    assert_eq!(
        eval_with_probe("r = getattr(fallible(), 'bad', None)"),
        Err("computing 'bad' failed".into())
    );
    assert_eq!(
        eval_with_probe("r = hasattr(fallible(), 'bad')"),
        Err("computing 'bad' failed".into())
    );
    assert_eq!(
        eval_with_probe(
            "r = (getattr(fallible(), 'nosuch', 7), hasattr(fallible(), 'nosuch'), hasattr(fallible(), 'ok'))"
        ),
        Ok("(7, False, True)".into())
    );
    assert_eq!(
        eval_with_probe("def f(x):\n    return x.bad\nr = f(fallible())"),
        Err("computing 'bad' failed".into())
    );
    // A global's field is read when the function runs (fields are never folded at compile
    // time; only methods are).
    assert_eq!(
        eval_with_probe("F = fallible()\ndef f():\n    return F.marker\nmarker = 'late'\nr = f()"),
        Ok(r#""late""#.into())
    );
    // Methods are still found first.
    assert_eq!(
        eval_with_probe("r = getattr([], 'append', 1) != 1"),
        Ok("True".into())
    );
}

/// Bazel: `StarlarkIndexable.Threaded.getIndex` / `containsKey` receive the thread.
#[test]
fn patch_0027_index_and_in_with_evaluator() {
    assert_eq!(
        eval_with_probe("marker = 'm'\nr = fallible()[3]"),
        Ok(r#"(3, "m")"#.into())
    );
    assert_eq!(
        eval_with_probe(
            "marker = 'm'\nr = ('m' in fallible(), 'x' in fallible(), 'x' not in fallible())"
        ),
        Ok("(True, False, True)".into())
    );
    assert_eq!(
        eval_with_probe("r = 'bad' in fallible()"),
        Err("'bad' in fallible".into())
    );
    assert_eq!(
        eval_with_probe("r = (2 in [1, 2], [5, 6][1])"),
        Ok("(True, 6)".into())
    );
}

/// The error of `src` as (1-based line, 1-based column, source text of the error span, frame
/// names outermost first).
fn error_span(src: &'static str) -> (usize, usize, String, Vec<String>) {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let globals = dialect::bazel_universe();
            let ast = AstModule::parse("t.bzl", src.to_owned(), &dialect::bzl_dialect()).unwrap();
            Module::with_temp_heap(|module| {
                let mut e = Evaluator::new(&module);
                let err = e.eval_module(ast, &globals).unwrap_err();
                assert!(
                    !matches!(err.kind(), starlark::ErrorKind::Parser(_)),
                    "{src}: parser kind"
                );
                let span = err.span().expect("error has a span");
                let begin = span.resolve_span().begin;
                let frames = err
                    .call_stack()
                    .frames
                    .iter()
                    .map(|f| f.name.clone())
                    .collect();
                (
                    begin.line + 1,
                    begin.column + 1,
                    span.source_span().to_owned(),
                    frames,
                )
            })
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Java Eval's error locations (crates/blaze_starlark/tests/data/locations/*.oracle.txt): the
/// failing node keeps its own span, which blaze_starlark maps to Java's token.
#[test]
fn patch_0028_error_locations() {
    // `if` conditions keep their span (Java: the operator), not the statement's.
    assert_eq!(
        error_span("def f(a):\n    if a + 'x':\n        pass\nf(1)").2,
        "a + 'x'"
    );
    // Index assignment and unpacking: the statement (Java: the `=`).
    assert_eq!(error_span("x = (1, 2)\nx[0] = 3").2, "x[0] = 3");
    assert_eq!(error_span("a, b = 1").2, "a, b = 1");
    // `for` over a non-iterable: the collection's start (an empty span).
    let (line, col, text, _) = error_span("def f():\n    for x in 1:\n        pass\nf()");
    assert_eq!((line, col, text.as_str()), (2, 14, ""));
    // `x.f op= y`: reading or setting the field is located at `x.f` (Java: the `.`).
    assert_eq!(
        error_span("def f():\n    s = 'abc'\n    s.a += 1\nf()").2,
        "s.a"
    );
    // A failed method lookup: `object.method` (Java: the `.`), not the call.
    assert_eq!(error_span("x = 'abc'.nosuch()").2, "'abc'.nosuch");
    // ... while an error of the call itself keeps the call's span.
    assert_eq!(error_span("x = 'abc'.index(1)").2, "'abc'.index(1)");
    // Dict literal: the failing entry's `:`.
    let (line, col, text, _) = error_span("k = 1\nx = {k: 1, 1: 2}");
    assert_eq!((line, col, text.as_str()), (2, 13, ":"));
    // Argument and recursion errors: the callee's declaration, in the callee's frame.
    let (line, col, text, frames) = error_span("def g(a):\n    return a\n\ng(1, 2)");
    assert_eq!((line, col, text.as_str()), (1, 5, "g"));
    assert_eq!(frames.last().map(String::as_str), Some("g"));
    // `int("x")` fails as a value error, not a syntax error (asserted in `error_span`).
    error_span("x = int('x')");
}

/// Java's Resolver reports every undefined name (Resolver.java:941); `eval_module` stops at the
/// first, `resolve_errors` lists them all in source order without executing.
#[test]
fn patch_0029_resolve_errors() {
    let globals = dialect::bazel_universe();
    let ast = AstModule::parse(
        "t.bzl",
        "x = y\ndef f():\n    return zz + w\nq = len(x)\n".to_owned(),
        &dialect::bzl_dialect(),
    )
    .unwrap();
    let errors = starlark::eval::resolve_errors(&ast, &globals);
    let got: Vec<(usize, usize, String)> = errors
        .iter()
        .map(|e| {
            let span = e.span().unwrap();
            let begin = span.resolve_span().begin;
            (
                begin.line + 1,
                begin.column + 1,
                span.source_span().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (1, 5, "y".to_owned()),
            (3, 12, "zz".to_owned()),
            (3, 17, "w".to_owned())
        ]
    );
    let ok =
        AstModule::parse("t.bzl", "x = len([])\n".to_owned(), &dialect::bzl_dialect()).unwrap();
    assert!(starlark::eval::resolve_errors(&ok, &globals).is_empty());
}

/// Like `eval`, but the source is read as Bazel reads files (one char per byte of its UTF-8,
/// `ParserInput.fromLatin1`) and the byte-strings flag is `utf8_byte_strings`; returns the
/// Latin-1 bytes of `str(r)`, or the error message.
fn eval_bytes(src: &'static str, utf8_byte_strings: bool) -> Result<Vec<u8>, String> {
    std::thread::Builder::new()
        .stack_size(1 << 28)
        .spawn(move || {
            let globals = dialect::bazel_universe();
            let source = blaze_util::string_encoding::latin1_decode(src.as_bytes()).into_owned();
            let ast = AstModule::parse("t.bzl", source, &dialect::bzl_dialect())
                .map_err(|e| bare_message(&e))?;
            Module::with_temp_heap(|module| {
                let mut e = Evaluator::new(&module);
                e.set_forbid_recursion(true);
                e.set_utf8_byte_strings(utf8_byte_strings);
                e.eval_module(ast, &globals).map_err(|e| bare_message(&e))?;
                let r = module.get("r").map(|v| v.to_str()).unwrap_or_default();
                Ok(blaze_util::string_encoding::latin1_encode_lossy(&r).into_owned())
            })
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The error of parsing `src` (Latin-1 decoded), as Java's `SyntaxError.toString`.
fn syntax_error(src: &'static str) -> String {
    let source = blaze_util::string_encoding::latin1_decode(src.as_bytes()).into_owned();
    let e = AstModule::parse("t.bzl", source, &dialect::bzl_dialect()).unwrap_err();
    match blaze_starlark::EvalError::from_starlark(&e).kind {
        blaze_starlark::error::EvalErrorKind::Syntax(errors) => errors[0].to_string(),
        k => panic!("not a syntax error: {k:?}"),
    }
}

/// Java Lexer.java:322-404 (escapes) and :301 (unclosed literal). Expected values: Bazel's
/// interpreter (java/StarlarkProbe.java; tests/data/escape_*.latin1.txt).
#[test]
fn patch_0032_lexer_escapes() {
    // Octal escapes are one char 0..=0xFF; non-ASCII source bytes pass through as chars.
    assert_eq!(
        eval_bytes(
            r#"r = (len("\0"), len("\0000"), len("\377"), "\303\251" == "é", len("é"))"#,
            true
        ),
        Ok(b"(1, 2, 1, True, 2)".to_vec())
    );
    assert_eq!(
        eval_bytes(r#"r = repr("\0\1\12\123\1234\08\377")"#, true),
        Ok(b"\"\\x00\\x01\\nSS4\\x008\xff\"".to_vec())
    );
    assert_eq!(
        eval_bytes(r#"r = repr("\a\b\f\n\r\t\v\\\'\"")"#, true),
        Ok(br#""\x07\x08\x0c\n\r\t\x0b\\'\"""#.to_vec())
    );
    // Unknown escapes, including \x \u \U \N and a non-ASCII byte, are errors at the escape char.
    assert_eq!(
        syntax_error("x = \"a\\qb\"\n"),
        "t.bzl:1:8: invalid escape sequence: \\q. Use '\\\\' to insert '\\'."
    );
    assert_eq!(
        syntax_error("x = 1\ny = \"\\x41\"\n"),
        "t.bzl:2:7: invalid escape sequence: \\x. Use '\\\\' to insert '\\'."
    );
    assert_eq!(
        syntax_error("y = \"ab\\u0041\"\n"),
        "t.bzl:1:9: invalid escape sequence: \\u. Use '\\\\' to insert '\\'."
    );
    assert_eq!(
        syntax_error("y = '\\U00000041'\n"),
        "t.bzl:1:7: invalid escape sequence: \\U. Use '\\\\' to insert '\\'."
    );
    assert_eq!(
        syntax_error("y = \"\"\"\n\\N{DASH}\"\"\"\n"),
        "t.bzl:2:2: invalid escape sequence: \\N. Use '\\\\' to insert '\\'."
    );
    assert_eq!(
        syntax_error("x = \"\\é\"\n"),
        "t.bzl:1:7: invalid escape sequence: \\\u{c3}. Use '\\\\' to insert '\\'."
    );
    // Octal above \377: at the last digit.
    assert_eq!(
        syntax_error("x = \"ab\\400\"\n"),
        "t.bzl:1:11: octal escape sequence out of range (maximum is \\377)"
    );
    assert_eq!(
        syntax_error("x = \"\\7777\"\n"),
        "t.bzl:1:9: octal escape sequence out of range (maximum is \\377)"
    );
    assert_eq!(
        syntax_error("x = \"abc\\"),
        "t.bzl:1:5: unclosed string literal"
    );
    // Carriage returns: kept in literals; a backslash before CR or CRLF continues the line; in a
    // raw literal a backslash-CRLF is backslash-LF, a backslash-CR stays (Java's quick scan).
    assert_eq!(
        eval_bytes(
            "r = \"|\".join([str(len(\"a\\\rb\")), str(len(\"a\\\r\nb\")), str(len(\"\"\"a\r\nb\"\"\")), \
             str(len(r\"a\\\r\nb\")), repr(r\"a\\\rb\"), repr(r\"\"\"a\\\rb\"\"\"), repr(\"\"\"x\ry\"\"\")])",
            true
        ),
        Ok(br#"2|2|4|4|"a\\\rb"|"a\\\nb"|"x\ry""#.to_vec())
    );
}

/// Java StringModule.java:174-182 (Ascii.toUpperCase/toLowerCase), :526 capitalize, :542 title,
/// :757-878 predicates. Expected values: the oracle (07-byte-strings.md 2.3) and Bazel's
/// interpreter (tests/data/latin1_chars.latin1.txt, strings_misc.latin1.txt).
#[test]
fn patch_0033_case_mapping_and_predicates() {
    // ASCII-only upper/lower: "aé🐷".upper(), "AÉ".lower() (oracle).
    assert_eq!(
        eval_bytes(
            r#"r = "|".join(["aé🐷".upper(), "AÉ".lower(), "中".upper()])"#,
            true
        ),
        Ok(b"A\xc3\xa9\xf0\x9f\x90\xb7|a\xc3\x89|\xe4\xb8\xad".to_vec())
    );
    // Character.toUpperCase per char: "中".capitalize() turns the lead byte E4 into C4 (oracle);
    // "été".title() lowers Ã after a letter (oracle).
    assert_eq!(
        eval_bytes(
            r#"r = "|".join(["中".capitalize(), "été".title(), "AÉ".capitalize()])"#,
            true
        ),
        Ok(b"\xc4\xb8\xad|\xc3\xa9T\xe3\xa9|A\xc3\x89".to_vec())
    );
    // ÿ (U+00FF) and µ (U+00B5) leave Latin-1 as in Java; ß has no one-char upper case.
    assert_eq!(
        eval(r#"r = ["\377".capitalize(), "\265".title(), "\337".capitalize(), "\337a\377".title(), "\377".upper(), "\300\330".lower()]"#),
        Ok("[\"\u{178}\", \"\u{39c}\", \"\u{df}\", \"\u{df}a\u{ff}\", \"\u{ff}\", \"\u{c0}\u{d8}\"]".into())
    );
    // Predicates: ASCII matchers, javaDigit, Guava whitespace (oracle and Java).
    assert_eq!(
        eval_bytes(
            r#"r = ("é".isalpha(), "é".isalnum(), "é".isdigit(), "aé".islower(), "AÉ".isupper(), "Éa".istitle(), "\205".isspace(), "\240".isspace(), "\034".isspace(), "ÉCOLE".isupper(), "Été Été".istitle(), "aB".istitle(), "1é".isalnum())"#,
            true
        ),
        Ok(b"(False, False, False, True, True, False, True, True, False, True, False, False, False)".to_vec())
    );
}

/// Java StringModule.java:196-206, :326 matcher: without `chars`, strip removes ASCII whitespace
/// (with U+001C..U+001F) under the byte-strings flag, Latin-1 whitespace (plus U+0085, U+00A0)
/// without it. The flag is the evaluator's (default true, Bazel's default).
#[test]
fn patch_0034_strip_whitespace_flag() {
    let src = r#"r = "|".join([" é ".strip(), "Å".strip(), "\240a\240".strip(), "\205a\205".lstrip(), "\205a\205".rstrip(), "\034a\037".strip(), "\240a".strip("\240")])"#;
    // Oracle: [C3 A9], [C3 85], [A0 61 A0].
    assert_eq!(
        eval_bytes(src, true),
        Ok(b"\xc3\xa9|\xc3\x85|\xa0a\xa0|\x85a\x85|\x85a\x85|a|a".to_vec())
    );
    assert_eq!(
        eval_bytes(src, false),
        Ok(b"\xc3\xa9|\xc3|a|a\x85|\x85a|a|a".to_vec())
    );
    // The default evaluator has the flag on.
    assert_eq!(eval(r#"r = "\240a".strip()"#), Ok("\"\u{a0}a\"".into()));
}

/// Java Printer.java:239 escapeCharacter: only `"`, `\`, and chars below 0x20 are escaped.
#[test]
fn patch_0035_repr_escapes() {
    // Oracle: repr("é\001\177\200"), str(["é"]), "%s|%r" % (e, e).
    assert_eq!(
        eval_bytes(
            r#"r = "|".join([repr("é\001\177\200"), str(["é"]), "%s|%r" % ("é", "é"), repr({"\377": "\t"})])"#,
            true
        ),
        Ok(
            b"\"\xc3\xa9\\x01\x7f\x80\"|[\"\xc3\xa9\"]|\xc3\xa9|\"\xc3\xa9\"|{\"\xff\": \"\\t\"}"
                .to_vec()
        )
    );
    // Not even DEL or chars above U+00FF (Java appends them as they are).
    assert_eq!(
        eval("r = repr(\"\u{200b}\u{7f}\")"),
        Ok("\"\\\"\u{200b}\u{7f}\\\"\"".into())
    );
}

/// Java EvalUtils.java:433 repeatString: the limit and the message count chars (= bytes of an
/// internal string).
#[test]
fn patch_0036_repeat_limit_counts_chars() {
    assert_eq!(
        eval_bytes(r#"r = "é" * (1 << 30)"#, true),
        Err("excessive repeat (2 * 1073741824 characters)".to_owned())
    );
    assert_eq!(eval_bytes(r#"r = len("é" * 3)"#, true), Ok(b"6".to_vec()));
}

/// Java Lexer.tokenize (Lexer.java:824-828) ignores tabs and carriage returns between tokens and
/// inside brackets (newlineInsideExpression, :191); Lexer.computeIndentation (:217-220) rejects
/// a tab anywhere in indentation, also on blank and comment-only lines and at the end of the
/// file; a comment runs to the line feed (scanToNewline, :555). Expected values: Bazel's
/// interpreter (tests/data/interp/lex_tab_*.latin1.txt, lex_cr_*.latin1.txt).
#[test]
fn patch_0037_lexer_whitespace() {
    assert_eq!(eval("x =\t1\nr = x"), Ok("1".into()));
    assert_eq!(eval("x = 1\t# c\nr = x"), Ok("1".into()));
    assert_eq!(eval("r = [1,\n\t2]"), Ok("[1, 2]".into()));
    assert_eq!(eval("x = 1 + \\\n\t2\nr = x"), Ok("3".into()));
    assert_eq!(eval("x = 1 \r+ 2\nr = x"), Ok("3".into()));
    assert_eq!(eval("x = 1  # a\rb\nr = x"), Ok("1".into()));
    assert_eq!(eval("# a\rb\nr = 1"), Ok("1".into()));
    assert_eq!(eval("x = 1\r\r\nr = x"), Ok("1".into()));
    assert_eq!(eval("r = [1,\r2]"), Ok("[1, 2]".into()));
    let tab = "Tab characters are not allowed for indentation. Use spaces instead.";
    assert_eq!(
        syntax_error("x = 1\n\t\nr = x\n"),
        format!("t.bzl:2:2: {tab}")
    );
    assert_eq!(
        syntax_error("x = 1\n\t# c\nr = x\n"),
        format!("t.bzl:2:2: {tab}")
    );
    assert_eq!(syntax_error("r = 1\n\t"), format!("t.bzl:2:2: {tab}"));
    assert_eq!(syntax_error("\t\nr = 1\n"), format!("t.bzl:1:2: {tab}"));
    assert_eq!(
        syntax_error("def f():\n\treturn 1\n"),
        format!("t.bzl:2:2: {tab}")
    );
    // A lone CR is not a line end.
    assert!(eval("x = 1\rr = x").is_err());
}

/// Java IntLiteral.scan (IntLiteral.java:69-84): no binary literals and no leading 0 ("invalid
/// octal literal"); Lexer.scanNumberOrDotOrEllipsis (:1005-1008): "floating-point literal too
/// large". Expected values: tests/data/interp/lex_binary_literal*.latin1.txt,
/// lex_int_00.latin1.txt, lex_float_too_large.latin1.txt.
#[test]
fn patch_0038_number_literals() {
    assert_eq!(
        syntax_error("r = 0b101\n"),
        "t.bzl:1:5: invalid octal literal: 0b101 (use '0ob101')"
    );
    assert_eq!(
        syntax_error("r = 0B1\n"),
        "t.bzl:1:5: invalid octal literal: 0B1 (use '0oB1')"
    );
    assert_eq!(
        syntax_error("r = 00\n"),
        "t.bzl:1:5: invalid octal literal: 00 (use '0o0')"
    );
    assert_eq!(
        syntax_error("r = 1e999\n"),
        "t.bzl:1:5: floating-point literal too large"
    );
    assert_eq!(
        eval("r = (0o17, 0O17, 0x1F, 0XaB, 1., .5e1, 01.5, 0e0, 1e308)"),
        Ok("(15, 15, 31, 171, 1.0, 5.0, 1.5, 0.0, 1e+308)".into())
    );
}

/// Java's keywordMap (Lexer.java:569-599) has no `async` and `await`: they are identifiers.
#[test]
fn patch_0039_async_await_identifiers() {
    assert_eq!(
        eval("async = 1\nawait = 2\nr = (async, await)"),
        Ok("(1, 2)".into())
    );
}

/// Bazel has no bytes literals: `b"x"` is the identifier `b` and a string, a syntax error
/// (tests/data/interp/lex_bytes_*.latin1.txt). Upstream dialects keep them.
#[test]
fn patch_0040_no_bytes_literals() {
    for src in ["r = b\"abc\"\n", "r = rb\"abc\"\n", "r = br'abc'\n"] {
        let e = AstModule::parse("t.bzl", src.to_owned(), &dialect::bzl_dialect()).unwrap_err();
        assert!(
            bare_message(&e).contains("unexpected string literal"),
            "{src}: {}",
            bare_message(&e)
        );
    }
    assert!(
        AstModule::parse(
            "t.bzl",
            "r = b\"abc\"\n".to_owned(),
            &starlark::syntax::Dialect::Standard
        )
        .is_ok()
    );
}

/// Java StringModule.substringIndices (StringModule.java:111-126): indices clamped, an end
/// before the start is the start. Expected values: tests/data/interp/str_values.latin1.txt.
#[test]
fn patch_0041_string_index_clamping() {
    assert_eq!(
        eval(
            "s = 'abc'\nr = (s.find('', 5), s.rfind('', 5), s.index('', 5), s.rindex('', 5), \
             s.find('', 2, 1), s.rfind('', 2, 1), s.find('', -10), s.rfind('', -10, -20))"
        ),
        Ok("(3, 3, 3, 3, 2, 2, 0, 0)".into())
    );
    assert_eq!(
        eval(
            "s = 'abc'\nr = (s.count('', 2, 1), s.count('', 5), s.count(''), s.count('', -1), \
             s.startswith('', 5), s.endswith('', 5), s.startswith('', 3, 1), \
             s.endswith(('x', ''), 5), s.startswith('c', 2, 1), s.find('a', 1, 0))"
        ),
        Ok("(1, 1, 4, 2, True, True, True, True, False, -1)".into())
    );
}

/// Java Starlark.slice (Starlark.java:692, :698): a `None` bound is the default.
#[test]
fn patch_0042_string_slice_none_bounds() {
    assert_eq!(
        eval("s = 'abc'\nr = (s[None:2], s[1:None], s[None:None:None], s[None:None:-1])"),
        Ok("(\"ab\", \"bc\", \"abc\", \"cba\")".into())
    );
}

/// Java StringModule has no `codepoints`.
#[test]
fn patch_0043_no_codepoints() {
    assert!(eval("r = 'abc'.codepoints()").is_err());
    assert_eq!(eval("r = 'codepoints' in dir('')"), Ok("False".into()));
}

/// Java MethodLibrary.intForStarlark (MethodLibrary.java:613) and StarlarkInt.parse
/// (StarlarkInt.java:99-168). Expected values: tests/data/interp/str_int_*.latin1.txt,
/// more_i_*.latin1.txt.
#[test]
fn patch_0044_int_parse() {
    assert_eq!(
        eval(
            "r = (int('0x10', 0), int('0b11', 0), int('0o7', 0), int('10', 16), int('-0x10', 0), \
             int('0b1', 16), int('z', 36), int('+7'), int('-0'), int('0', 0), int('-0o7', 0))"
        ),
        Ok("(16, 3, 7, 16, -16, 177, 35, 7, 0, 0, -7)".into())
    );
    for (src, want) in [
        ("r = int('1_000')", "invalid base-10 literal: \"1_000\""),
        ("r = int('f_f', 16)", "invalid base-16 literal: \"f_f\""),
        ("r = int('0x10')", "invalid base-10 literal: \"0x10\""),
        (
            "r = int('010', 0)",
            "cannot infer base when string begins with a 0: \"010\"",
        ),
        (
            "r = int('00', 0)",
            "cannot infer base when string begins with a 0: \"00\"",
        ),
        ("r = int('1', 1)", "invalid base 1 (want 2 <= base <= 36)"),
        ("r = int('')", "empty string"),
        ("r = int('0x-1', 16)", "invalid base-16 literal: \"0x-1\""),
        (
            "r = int(10, 2)",
            "can't convert non-string with explicit base",
        ),
    ] {
        assert_eq!(eval(src), Err(want.to_owned()), "{src}");
    }
    assert!(eval("r = int()").is_err());
}

/// Java FormatParser (FormatParser.java:85-100, :193): no `!r`/`!s` conversions (the whole field
/// is the key); a field of digits and `-` is an index when Integer.parseInt accepts it.
/// Expected values: tests/data/interp/str_format_*.latin1.txt, more_f_*.latin1.txt.
#[test]
fn patch_0045_format_fields() {
    assert!(eval("r = '{!r}'.format(1)").is_err());
    assert!(eval("r = '{0!r}'.format('a')").is_err());
    assert!(eval("r = '{x!s}'.format(x = 1)").is_err());
    assert_eq!(eval("r = '{a!r}'.format(**{'a!r': 1})"), Ok("\"1\"".into()));
    assert_eq!(eval("r = '{-0}'.format('a')"), Ok("\"a\"".into()));
    assert!(eval("r = '{-1}'.format('a')").is_err());
    assert_eq!(
        eval("r = '{99999999999}'.format(**{'99999999999': 7})"),
        Ok("\"7\"".into())
    );
    assert_eq!(eval("r = '{-}'.format(**{'-': 8})"), Ok("\"8\"".into()));
}

/// Java StringModule.strip/lstrip/rstrip (StringModule.java:243-320): `chars` may be None.
#[test]
fn patch_0046_strip_none_chars() {
    assert_eq!(
        eval("r = (' a '.strip(None), ' a '.lstrip(None), ' a '.rstrip(None))"),
        Ok("(\"a\", \"a \", \" a\")".into())
    );
}
