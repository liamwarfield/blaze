//! Immutable lists (Java `StarlarkList.immutableCopyOf`) and the module a function was defined
//! in (Java `StarlarkFunction.getModule`, `Module.ofInnermostEnclosingStarlarkFunction`).
//! Expected values follow net/starlark/java/eval/{StarlarkList, ImmutableStarlarkList,
//! Starlark, Module}.java and lib/collect/nestedset/Depset.java at the pin.

mod common;

use std::sync::Arc;

use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::defining_module;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::list::list_immutable_copy_of;
use blaze_starlark::list::list_is_immutable;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use starlark::environment::FrozenModule;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::none::NoneOr;

#[starlark_module]
fn test_natives(builder: &mut GlobalsBuilder) {
    /// `StarlarkList.immutableCopyOf(x)` (what `cc_internal.freeze` does for iterables).
    fn immutable_copy_of<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        list_immutable_copy_of(heap, x)
    }

    fn list_immutable<'v>(x: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        Ok(NoneOr::from_option(list_is_immutable(x)))
    }

    /// Module key of `fn.getModule()`.
    fn module_of<'v>(f: Value<'v>) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            defining_module::module_file_of_function(f),
        ))
    }

    /// Module key of `Module.ofInnermostEnclosingStarlarkFunction(thread, depth)`.
    fn innermost<'v>(
        depth: i32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(
            defining_module::module_file_of_innermost_enclosing_starlark_function(
                eval,
                depth as usize,
            ),
        ))
    }

    /// `fn.getModule().getGlobal(fn.getName()) == fn`.
    fn is_toplevel<'v>(f: Value<'v>, eval: &mut Evaluator<'v, '_, '_>) -> anyhow::Result<bool> {
        Ok(defining_module::is_global_of_defining_module(f, eval))
    }
}

/// Executes `src` as .bzl file `file` with blaze_starlark's builtins plus [`test_natives`].
fn exec_file(file: &str, src: &str, loads: LoadMap) -> Result<FrozenModule, String> {
    let (file, src) = (file.to_owned(), src.to_owned());
    eval::run_on_evaluation_thread(move || {
        let semantics = StarlarkSemantics::default();
        let ctx = common::context(
            Phase::BzlLoad,
            semantics.clone(),
            Arc::new(CollectingEvents::default()),
        );
        let mut registry = NativeRegistry::with_blaze_starlark_builtins();
        registry.register(&[FileKind::Bzl], test_natives);
        let globals = registry.build(FileKind::Bzl, &semantics);
        eval::compile(FileKind::Bzl, &file, src)
            .and_then(|c| eval::exec_module(c, &globals, &loads, &ctx, &Default::default()))
            .map_err(|e| e.message)
    })
}

fn eval_r(src: &str) -> Result<String, String> {
    exec_file("test.bzl", src, LoadMap::new()).map(|m| common::global_repr(&m, "r"))
}

#[test]
fn immutable_copy_of_reads_like_a_list() {
    assert_eq!(
        eval_r(
            "x = immutable_copy_of((3, [1], 2))\n\
             r = (type(x), x, len(x), x[0], x[-1], x[1:], 2 in x, x == [3, [1], 2], sorted(x[::2]), list_immutable(x))"
        ),
        Ok(r#"("list", [3, [1], 2], 3, 3, 2, [[1], 2], True, True, [2, 3], True)"#.into())
    );
    // Copies are mutable; the elements keep their own mutability (Java copies references).
    assert_eq!(
        eval_r(
            "x = immutable_copy_of([[1]])\n\
             y = x + [2]\ny.append(3)\nx[0].append(4)\n\
             r = (x, y, list_immutable(y), list_immutable([]), list_immutable(immutable_copy_of([])), list_immutable(()))"
        ),
        Ok("([[1, 4]], [[1, 4], 2, 3], False, False, True, None)".into())
    );
    assert_eq!(
        eval_r("r = immutable_copy_of(depset([2, 1]).to_list() + [3])"),
        Ok("[2, 1, 3]".into())
    );
}

/// Java Starlark.checkMutable on an ImmutableStarlarkList.
#[test]
fn immutable_copy_of_rejects_mutation() {
    for m in [
        "x.append(1)",
        "x.extend([1])",
        "x.insert(0, 1)",
        "x.pop()",
        "x.remove(1)",
        "x.clear()",
        "x[0] = 2",
        "y = x\n    y += [2]",
    ] {
        assert_eq!(
            eval_r(&format!(
                "x = immutable_copy_of([1])\ndef f():\n    {m}\nf()\n"
            )),
            Err("trying to mutate a frozen list value".into()),
            "{m}"
        );
    }
}

/// Java copyOf(null, elems): an immutable list is returned as is.
#[test]
fn immutable_copy_of_an_immutable_list_is_identity() {
    eval::run_on_evaluation_thread(|| {
        starlark::environment::Module::with_temp_heap(|module| {
            let heap = module.heap();
            let src = heap.alloc(starlark::values::list::AllocList([1, 2]));
            let a = list_immutable_copy_of(heap, src).unwrap();
            assert!(!a.ptr_eq(src));
            let b = list_immutable_copy_of(heap, a).unwrap();
            assert!(b.ptr_eq(a));
            assert_eq!(list_is_immutable(src), Some(false));
            assert_eq!(list_is_immutable(a), Some(true));
            assert!(list_immutable_copy_of(heap, heap.alloc(1)).is_err());
        });
    });
}

/// Java Depset.checkElement: an immutable list passes isImmutable but is still a list.
#[test]
fn depset_of_immutable_list_is_still_a_list() {
    assert_eq!(
        eval_r("r = depset([immutable_copy_of([1])])"),
        Err("depsets cannot contain items of type 'list'".into())
    );
}

#[test]
fn depset_of_struct_with_immutable_list() {
    assert_eq!(
        eval_r("r = depset([struct(a = immutable_copy_of([1, 2]))]).to_list()"),
        Ok("[struct(a = [1, 2])]".into())
    );
}

/// Java ImmutableStarlarkList.isImmutable is `true` without an element check, so a struct
/// holding an immutable copy of mutable lists is a valid depset element; a mutable list is not
/// immutable whatever it holds (MutableStarlarkList.isImmutable).
#[test]
fn depset_immutability_of_lists_follows_the_list_flag() {
    assert_eq!(
        eval_r("r = depset([struct(a = immutable_copy_of([[1]]))]).to_list()"),
        Ok("[struct(a = [[1]])]".into())
    );
    assert_eq!(
        eval_r("r = depset([struct(a = [1, 2], b = 1)])"),
        Err("depset elements must not be mutable values".into())
    );
    assert_eq!(
        eval_r("r = depset([[1]])"),
        Err("depset elements must not be mutable values".into())
    );
}

#[test]
fn module_of_function() {
    let a = exec_file(
        "//pkg:a.bzl",
        r#"
def impl():
    pass
def make():
    return lambda: None
def outer():
    def inner():
        pass
    return inner
def who():
    return (innermost(0), innermost(1), innermost(2))
def who2():
    x = innermost(0)
    return (x, innermost(1))
alias = impl
during = (module_of(impl), module_of(make()), innermost(0), innermost(1), who(),
          is_toplevel(impl), is_toplevel(alias), is_toplevel(make()), is_toplevel(outer()),
          is_toplevel(len), module_of(len), module_of(1))
"#,
        LoadMap::new(),
    )
    .unwrap();
    assert_eq!(
        common::global_repr(&a, "during"),
        concat!(
            r#"("//pkg:a.bzl", "//pkg:a.bzl", "//pkg:a.bzl", None, ("//pkg:a.bzl", "//pkg:a.bzl", None), "#,
            "True, True, False, False, False, None, None)"
        )
    );
    let mut loads = LoadMap::new();
    loads.insert(":a.bzl", a);
    let b = exec_file(
        "//pkg:b.bzl",
        r#"
load(":a.bzl", "alias", "impl", "make", "outer", "who", "who2")
def mine():
    pass
def via_mine():
    return who()
r = (module_of(impl), module_of(make()), module_of(mine), who(), who2(), via_mine(),
     is_toplevel(impl), is_toplevel(alias), is_toplevel(mine), is_toplevel(make()), is_toplevel(outer()))
"#,
        loads,
    )
    .unwrap();
    assert_eq!(
        common::global_repr(&b, "r"),
        concat!(
            r#"("//pkg:a.bzl", "//pkg:a.bzl", "//pkg:b.bzl", "#,
            r#"("//pkg:a.bzl", "//pkg:b.bzl", None), ("//pkg:a.bzl", "//pkg:b.bzl"), "#,
            r#"("//pkg:a.bzl", "//pkg:b.bzl", "//pkg:b.bzl"), "#,
            "True, True, True, False, False)"
        )
    );
}
