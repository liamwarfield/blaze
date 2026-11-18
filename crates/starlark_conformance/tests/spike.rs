//! Integration spike for Blaze on starlark-rust 0.14.2 (see docs/research/05-starlark-rust.md).
//!
//! (a) custom Bazel-like values survive freezing;
//! (b) a .bzl loads another through a FileLoader;
//! (c) a frozen function is called from a fresh Module on another thread with a ctx value and
//!     per-call state in Evaluator::extra;
//! (d) many packages evaluated in parallel (rayon) share frozen .bzl modules;
//! (e) fail() renders as Bazel's traceback.

use std::time::Instant;

use rayon::prelude::*;
use starlark::environment::FrozenModule;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark_conformance::spike::Ctx;
use starlark_conformance::spike::EvalContext;
use starlark_conformance::spike::FrozenDepset;
use starlark_conformance::spike::Label;
use starlark_conformance::spike::MapLoader;
use starlark_conformance::spike::Phase;
use starlark_conformance::spike::eval_file;
use starlark_conformance::spike::frozen_into;
use starlark_conformance::spike::instance_provider;
use starlark_conformance::spike::provider_key;
use starlark_conformance::spike::spike_globals;
use starlark_conformance::traceback::render_bazel;

const A_BZL: &str = r#"
MyInfo = provider(fields = ["files", "label"])
LBL = Label("//pkg/a:a")
FILES = depset(["a.h", "a.cc"])

def impl(ctx):
    print("analyzing", ctx.label.name, sep = "|")
    files = depset(
        [ctx.label.name + ".o"],
        transitive = [d.files for d in ctx.deps] + [FILES],
    )
    ctx.run(mnemonic = "CppCompile")
    return [MyInfo(files = files, label = ctx.label)]
"#;

const B_BZL: &str = r#"
load("//a:a.bzl", "FILES", "MyInfo", "impl")

ALL = depset(["b.h"], transitive = [FILES])
ALL_LIST = ALL.to_list()

def b_impl(ctx):
    return impl(ctx)

def my_macro(name, extra = []):
    rule_target(name = name, deps = [f for f in ALL_LIST] + extra)
    rule_target(name = name + "_lib")

def failing_macro(name):
    _check(name)

def _check(name):
    fail("bad thing:", name)
"#;

fn globals() -> Globals {
    spike_globals()
}

/// Evaluates a.bzl and b.bzl; returns (a, b).
fn load_ab(g: &Globals) -> (FrozenModule, FrozenModule) {
    let cx_a = EvalContext::new(Phase::BzlLoad, "//a:a.bzl");
    let a = eval_file("a/a.bzl", A_BZL, g, &MapLoader::default(), &cx_a).unwrap();
    let mut loader = MapLoader::default();
    loader.modules.insert("//a:a.bzl".to_owned(), a.clone());
    let cx_b = EvalContext::new(Phase::BzlLoad, "//b:b.bzl");
    let b = match eval_file("b/b.bzl", B_BZL, g, &loader, &cx_b) {
        Ok(b) => b,
        Err(e) => panic!("{e}"),
    };
    (a, b)
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn frozen_types_are_send_and_sync() {
    assert_send_sync::<FrozenModule>();
    assert_send_sync::<OwnedFrozenValue>();
    assert_send_sync::<Globals>();
    // Module<'v> and Evaluator are confined to the thread (and heap brand) that created them.
}

#[test]
fn a_custom_values_survive_freezing() {
    let g = globals();
    let (a, _) = load_ab(&g);

    let lbl = a.get("LBL").unwrap();
    let Ok(lbl) = lbl.downcast::<Label>() else {
        panic!("LBL is not a Label")
    };
    assert_eq!(lbl.as_ref().package, "pkg/a");
    assert_eq!(lbl.as_ref().name, "a");

    let files = a.get("FILES").unwrap();
    assert!(files.value().downcast_ref::<FrozenDepset>().is_some());

    let info = a.get("MyInfo").unwrap();
    assert_eq!(
        provider_key(info.value()),
        Some(("//a:a.bzl".to_owned(), "MyInfo".to_owned())),
        "export_as named the provider at assignment and the name survived freezing"
    );
}

#[test]
fn b_loads_a_through_file_loader() {
    let g = globals();
    let (_, b) = load_ab(&g);
    let all = b.get("ALL_LIST").unwrap();
    let items: Vec<String> = ListRef::from_value(all.value())
        .unwrap()
        .iter()
        .map(|v| v.unpack_str().unwrap().to_owned())
        .collect();
    assert_eq!(items, ["a.h", "a.cc", "b.h"]);

    // Bazel: loaded names are file-local, not re-exported (enable_load_reexport = false).
    assert!(b.get("impl").is_err());
    let mut loader = MapLoader::default();
    loader.modules.insert("//b:b.bzl".to_owned(), b.clone());
    let cx = EvalContext::new(Phase::BzlLoad, "//c:c.bzl");
    let err = eval_file("c/c.bzl", "load('//b:b.bzl', 'impl')\n", &g, &loader, &cx).unwrap_err();
    assert!(format!("{err}").contains("impl"), "{err}");
    // Private names cannot be loaded either.
    let err = eval_file("c/c.bzl", "load('//b:b.bzl', '_check')\n", &g, &loader, &cx).unwrap_err();
    assert!(format!("{err}").contains("_check"), "{err}");
}

/// Runs b_impl on the current thread in a fresh Module, the way analysis of one configured target
/// would. Returns (actions, prints, file list of the returned MyInfo, provider key).
fn analyze(b: &FrozenModule, target: &str) -> (Vec<String>, Vec<String>, Vec<String>, String) {
    let cx = EvalContext::new(Phase::Analysis, "//pkg:BUILD");
    let b_impl = b.get("b_impl").unwrap();
    let (files, key) = Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(&cx);
        let heap = module.heap();
        // OwnedFrozenValue -> Value on this heap; also keeps b's heap alive for this module.
        let f = frozen_into(&module, &b_impl);
        let label = heap.alloc(Label::parse(target).unwrap());
        let ctx = heap.alloc(Ctx {
            label,
            deps: heap.alloc(AllocList(Vec::<Value>::new())),
        });
        let res = eval.eval_function(f, &[ctx], &[]).unwrap();
        let first = ListRef::from_value(res).unwrap()[0];
        let provider = instance_provider(first).unwrap();
        let key = provider_key(provider).unwrap();
        let files = first.get_attr("files", heap).unwrap().unwrap();
        let to_list = files.get_attr("to_list", heap).unwrap().unwrap();
        let list = eval.eval_function(to_list, &[], &[]).unwrap();
        let files: Vec<String> = ListRef::from_value(list)
            .unwrap()
            .iter()
            .map(|v| v.unpack_str().unwrap().to_owned())
            .collect();
        (files, format!("{}%{}", key.0, key.1))
    });
    (cx.actions.into_inner(), cx.prints.into_inner(), files, key)
}

#[test]
fn c_frozen_function_called_on_another_thread_with_ctx() {
    let g = globals();
    let (_, b) = load_ab(&g);
    let t0 = Instant::now();
    let (actions, prints, files, key) = std::thread::spawn(move || analyze(&b, "//pkg:t"))
        .join()
        .unwrap();
    eprintln!("analysis on another thread: {:?}", t0.elapsed());
    assert_eq!(actions, ["CppCompile"]);
    // print's location is the `(` of the call inside a.bzl (Bazel: thread.getCallerLocation()).
    assert_eq!(prints, ["DEBUG: a/a.bzl:7:10: analyzing|t"]);
    assert_eq!(files, ["a.h", "a.cc", "t.o"]);
    assert_eq!(key, "//a:a.bzl%MyInfo");
}

#[test]
fn c_results_outlive_the_analysis_heap_by_freezing() {
    // Bazel keeps provider instances after analysis (they feed dependents). Here the result is
    // stored in the per-target module and the module is frozen: the value outlives the heap.
    let g = globals();
    let (_, b) = load_ab(&g);
    let b_impl = b.get("b_impl").unwrap();
    let cx = EvalContext::new(Phase::Analysis, "//pkg:BUILD");
    let frozen: FrozenModule = Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&cx);
            let heap = module.heap();
            let f = frozen_into(&module, &b_impl);
            let ctx = heap.alloc(Ctx {
                label: heap.alloc(Label::parse("//pkg:u").unwrap()),
                deps: heap.alloc(AllocList(Vec::<Value>::new())),
            });
            let res = eval.eval_function(f, &[ctx], &[]).unwrap();
            module.set("providers", res);
        }
        module.freeze().unwrap()
    });
    // Ctx was allocated with alloc_complex_no_freeze but is not reachable from "providers".
    let providers = frozen.get("providers").unwrap();
    // Use the frozen provider instance as a dep of another target, on another thread.
    let b2 = b.clone();
    let files = std::thread::spawn(move || {
        let cx = EvalContext::new(Phase::Analysis, "//pkg:BUILD");
        let b_impl = b2.get("b_impl").unwrap();
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(&cx);
            let heap = module.heap();
            let f = frozen_into(&module, &b_impl);
            let deps = frozen_into(&module, &providers);
            let ctx = heap.alloc(Ctx {
                label: heap.alloc(Label::parse("//pkg:v").unwrap()),
                deps,
            });
            let res = eval.eval_function(f, &[ctx], &[]).unwrap();
            let first = ListRef::from_value(res).unwrap()[0];
            let files = first.get_attr("files", heap).unwrap().unwrap();
            let to_list = files.get_attr("to_list", heap).unwrap().unwrap();
            let list = eval.eval_function(to_list, &[], &[]).unwrap();
            ListRef::from_value(list)
                .unwrap()
                .iter()
                .map(|v| v.unpack_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
    })
    .join()
    .unwrap();
    assert_eq!(files, ["a.h", "a.cc", "u.o", "v.o"]);
}

#[test]
fn c_freezing_an_unfreezable_ctx_fails() {
    // Bazel forbids keeping ctx; here freezing a module that still references it errors.
    let cx = EvalContext::new(Phase::Analysis, "//pkg:BUILD");
    let r = Module::with_temp_heap(|module| {
        let heap = module.heap();
        let ctx = heap.alloc(Ctx {
            label: heap.alloc(Label::parse("//pkg:u").unwrap()),
            deps: heap.alloc(AllocList(Vec::<Value>::new())),
        });
        module.set("leak", ctx);
        let _ = &cx;
        module.freeze().map(|_| ())
    });
    assert!(r.is_err());
}

#[test]
fn d_parallel_packages_share_frozen_bzl() {
    let g = globals();
    let (_, b) = load_ab(&g);
    let mut loader = MapLoader::default();
    loader.modules.insert("//b:b.bzl".to_owned(), b);
    let n = 2000;
    let srcs: Vec<(String, String)> = (0..n)
        .map(|i| {
            (
                format!("pkg{i}/BUILD"),
                format!(
                    "load(\"//b:b.bzl\", \"my_macro\")\nmy_macro(name = \"t{i}\", extra = [\"x{i}\"])\n"
                ),
            )
        })
        .collect();
    let run = |(name, src): &(String, String)| {
        let cx = EvalContext::new(Phase::Loading, name);
        eval_file(name, src, &g, &loader, &cx).unwrap();
        cx.targets.into_inner()
    };
    let t0 = Instant::now();
    let seq: Vec<_> = srcs.iter().map(run).collect();
    let t_seq = t0.elapsed();
    let t0 = Instant::now();
    let par: Vec<_> = srcs.par_iter().map(run).collect();
    let t_par = t0.elapsed();
    eprintln!(
        "{n} packages: sequential {t_seq:?}, rayon ({} threads) {t_par:?}",
        rayon::current_num_threads()
    );
    assert_eq!(seq, par);
    assert_eq!(par.len(), n);
    assert_eq!(
        par[7],
        vec![
            (
                "t7".to_owned(),
                vec![
                    "a.h".to_owned(),
                    "a.cc".to_owned(),
                    "b.h".to_owned(),
                    "x7".to_owned()
                ]
            ),
            ("t7_lib".to_owned(), vec![])
        ]
    );
}

#[test]
fn d_loading_native_rejected_outside_loading_phase() {
    let g = globals();
    let cx = EvalContext::new(Phase::BzlLoad, "//x:x.bzl");
    let err = eval_file(
        "x/x.bzl",
        "rule_target(name = 'a')\n",
        &g,
        &MapLoader::default(),
        &cx,
    )
    .unwrap_err();
    assert!(
        render_bazel(&err).ends_with(
            "Error in rule_target: 'rule_target' can only be called during the loading phase"
        ),
        "{}",
        render_bazel(&err)
    );
}

#[test]
fn e_fail_renders_bazel_traceback() {
    let g = globals();
    let (_, b) = load_ab(&g);
    let mut loader = MapLoader::default();
    loader.modules.insert("//b:b.bzl".to_owned(), b);
    let cx = EvalContext::new(Phase::Loading, "//pkg:BUILD");
    let build = "load(\"//b:b.bzl\", \"failing_macro\")\n\nfailing_macro(name = \"x\")\n";
    let err = eval_file("pkg/BUILD", build, &g, &loader, &cx).unwrap_err();
    let want = "Traceback (most recent call last):\n\
\tFile \"pkg/BUILD\", line 3, column 14, in <toplevel>\n\
\t\tfailing_macro(name = \"x\")\n\
\tFile \"b/b.bzl\", line 15, column 11, in failing_macro\n\
\t\t_check(name)\n\
\tFile \"b/b.bzl\", line 18, column 9, in _check\n\
\t\tfail(\"bad thing:\", name)\n\
Error in fail: bad thing: x";
    assert_eq!(render_bazel(&err), want);
}

#[test]
fn e_non_builtin_error_renders_error_prefix() {
    let g = globals();
    let cx = EvalContext::new(Phase::BzlLoad, "//x:x.bzl");
    let src = "def f(x):\n    return x + 1\n\nf(\"a\")\n";
    let err = eval_file("x/x.bzl", src, &g, &MapLoader::default(), &cx).unwrap_err();
    let got = render_bazel(&err);
    // Bazel: column 14 (the `+` operator); starlark-rust spans start at the left operand (12).
    // Message text is starlark-rust's ("unsupported binary operation: string + int" in Bazel).
    let want = "Traceback (most recent call last):\n\
\tFile \"x/x.bzl\", line 4, column 2, in <toplevel>\n\
\t\tf(\"a\")\n\
\tFile \"x/x.bzl\", line 2, column 12, in f\n\
\t\treturn x + 1\n\
Error: Operation `+` not supported for types `string` and `int`";
    assert_eq!(got, want);
}
