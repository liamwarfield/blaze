//! Immutable dicts (Java `Dict.immutableCopyOf`, what `cc_internal.freeze` does for dicts).
//! Expected values follow net/starlark/java/eval/{Dict, Starlark}.java and
//! lib/collect/nestedset/Depset.java at the pin.

mod common;

use std::sync::Arc;

use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::dict::dict_immutable_copy_of;
use blaze_starlark::dict::dict_is_immutable;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::none::NoneOr;

#[starlark_module]
fn test_natives(builder: &mut GlobalsBuilder) {
    fn immutable_copy_of<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        dict_immutable_copy_of(heap, x)
    }

    fn dict_immutable<'v>(x: Value<'v>) -> anyhow::Result<NoneOr<bool>> {
        Ok(NoneOr::from_option(dict_is_immutable(x)))
    }
}

/// Executes `src` as test.bzl with blaze_starlark's builtins plus [`test_natives`]; repr of `r`.
fn eval_r(src: &str) -> Result<String, String> {
    let src = src.to_owned();
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
        eval::compile(FileKind::Bzl, "test.bzl", src)
            .and_then(|c| {
                eval::exec_module(c, &globals, &LoadMap::new(), &ctx, &Default::default())
            })
            .map(|m| common::global_repr(&m, "r"))
            .map_err(|e| e.message)
    })
}

#[test]
fn immutable_copy_of_reads_like_a_dict() {
    assert_eq!(
        eval_r(
            "d = immutable_copy_of({'a': [1], 'b': 2})\n\
             r = (type(d), d, len(d), d['a'], d.get('b'), 'a' in d, d.keys(), d.values(), d.items(), \
             [k for k in d], d == {'b': 2, 'a': [1]}, dict_immutable(d))"
        ),
        Ok(r#"("dict", {"a": [1], "b": 2}, 2, [1], 2, True, ["a", "b"], [[1], 2], [("a", [1]), ("b", 2)], ["a", "b"], True, True)"#.into())
    );
    // Copies are mutable; values keep their own mutability (Java copies references).
    assert_eq!(
        eval_r(
            "d = immutable_copy_of({'a': [1]})\n\
             e = dict(d)\ne['c'] = 3\nd['a'].append(2)\n\
             r = (d, e, d | {'z': 0}, dict_immutable(e), dict_immutable(immutable_copy_of({})), dict_immutable(()))"
        ),
        Ok(
            r#"({"a": [1, 2]}, {"a": [1, 2], "c": 3}, {"a": [1, 2], "z": 0}, False, True, None)"#
                .into()
        )
    );
    assert_eq!(
        eval_r("r = immutable_copy_of([1])"),
        Err("got value of type 'list', want 'dict'".into())
    );
}

/// Java Starlark.checkMutable on an immutable Dict.
#[test]
fn immutable_copy_of_rejects_mutation() {
    for m in [
        "d['c'] = 1",
        "d.update(c = 1)",
        "d.pop('a')",
        "d.pop('zz', None)",
        "d.popitem()",
        "d.setdefault('c', 1)",
        "d.clear()",
        "x = d\n    x |= {'c': 1}",
    ] {
        assert_eq!(
            eval_r(&format!(
                "d = immutable_copy_of({{'a': 1}})\ndef f():\n    {m}\nf()\n"
            )),
            Err("trying to mutate a frozen dict value".into()),
            "{m}"
        );
    }
}

/// Java copyOf(null, m): an immutable dict is returned as is.
#[test]
fn immutable_copy_of_an_immutable_dict_is_identity() {
    eval::run_on_evaluation_thread(|| {
        starlark::environment::Module::with_temp_heap(|module| {
            let heap = module.heap();
            let src = heap.alloc(starlark::values::dict::AllocDict([("a", 1)]));
            let a = dict_immutable_copy_of(heap, src).unwrap();
            assert!(!a.ptr_eq(src));
            assert!(dict_immutable_copy_of(heap, a).unwrap().ptr_eq(a));
            assert_eq!(dict_is_immutable(src), Some(false));
            assert_eq!(dict_is_immutable(a), Some(true));
        });
    });
}

/// Java Dict.isImmutable is `true` for an immutable dict whatever it holds; Depset.checkElement
/// then still rejects a top-level dict by type.
#[test]
fn depset_immutability_of_dicts_follows_the_dict_flag() {
    assert_eq!(
        eval_r("r = depset([struct(a = immutable_copy_of({'k': [1]}))]).to_list()"),
        Ok(r#"[struct(a = {"k": [1]})]"#.into())
    );
    assert_eq!(
        eval_r("r = depset([immutable_copy_of({'k': 1})])"),
        Err("depsets cannot contain items of type 'dict'".into())
    );
    assert_eq!(
        eval_r("r = depset([struct(a = {'k': 1})])"),
        Err("depset elements must not be mutable values".into())
    );
}
