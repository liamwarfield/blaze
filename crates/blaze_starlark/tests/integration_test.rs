//! The starlark_conformance spikes, on blaze_starlark's API: loads, frozen values, a frozen rule
//! implementation called on another thread with per-phase payload state, parallel package
//! loading over shared frozen .bzl modules, and Bazel tracebacks across files.

mod common;

use std::sync::Arc;
use std::time::Instant;

use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::depset::Depset;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::label::LabelValue;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::structs::StarlarkInfo;
use blaze_starlark::structs::StructProvider;
use blaze_starlark::thread::CollectingEvents;
use rayon::prelude::*;
use starlark::environment::FrozenModule;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;

/// Payload of a loading-phase evaluation (stands in for Package.Builder).
#[derive(Default)]
struct PackageState {
    targets: Vec<(String, Vec<String>)>,
}

/// Payload of an analysis-phase evaluation (stands in for RuleContext).
#[derive(Default)]
struct AnalysisState {
    actions: Vec<String>,
}

#[starlark_module]
fn test_natives(builder: &mut GlobalsBuilder) {
    fn rule_target<'v>(
        #[starlark(require = named)] name: &str,
        #[starlark(require = named, default = NoneOr::None)] deps: NoneOr<
            UnpackListOrTuple<String>,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let cx = ThreadContext::get(eval)?;
        let mut pkg = cx.payload::<PackageState>().ok_or_else(|| {
            anyhow::anyhow!("rule_target can only be used while evaluating a BUILD file")
        })?;
        pkg.targets.push((
            name.to_owned(),
            deps.into_option().map(|d| d.items).unwrap_or_default(),
        ));
        Ok(NoneType)
    }

    fn run_action<'v>(
        #[starlark(require = named)] mnemonic: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let cx = ThreadContext::require(eval, &[Phase::Analysis], || {
            "run_action can only be called during analysis".to_owned()
        })?;
        cx.payload::<AnalysisState>()
            .expect("analysis payload")
            .actions
            .push(mnemonic.to_owned());
        Ok(NoneType)
    }
}

fn registry() -> NativeRegistry {
    let mut r = NativeRegistry::with_blaze_starlark_builtins();
    r.register(&[FileKind::Bzl, FileKind::Build], test_natives);
    r
}

const A_BZL: &str = r#"
LBL = Label("//pkg/a:a")
FILES = depset(["a.h", "a.cc"])
INFO = struct(files = FILES, label = LBL)

def impl(ctx):
    print("analyzing", ctx.label.name, sep = "|")
    files = depset([ctx.label.name + ".o"], transitive = [d.files for d in ctx.deps] + [FILES])
    run_action(mnemonic = "CppCompile")
    return [struct(files = files, label = ctx.label)]
"#;

const B_BZL: &str = r#"
load("//a:a.bzl", "FILES", "impl")

ALL = depset(["b.h"], transitive = [FILES])
ALL_LIST = ALL.to_list()

def b_impl(ctx):
    return impl(ctx)

def my_macro(name, extra = []):
    rule_target(name = name, deps = ALL_LIST + extra)
    rule_target(name = name + "_lib")

def failing_macro(name):
    _check(name)

def _check(name):
    fail("bad thing:", name)
"#;

fn ctx(phase: Phase) -> ThreadContext {
    common::context(
        phase,
        StarlarkSemantics::default(),
        Arc::new(CollectingEvents::default()),
    )
}

fn exec(
    kind: FileKind,
    name: &str,
    src: &str,
    loads: &LoadMap,
    cx: &ThreadContext,
) -> Result<FrozenModule, blaze_starlark::EvalError> {
    let globals = registry().build(kind, &cx.semantics);
    let c = eval::compile(kind, name, src.to_owned())?;
    eval::exec_module(c, &globals, loads, cx, &Default::default())
}

fn load_ab() -> (FrozenModule, FrozenModule) {
    eval::run_on_evaluation_thread(|| {
        let a = exec(
            FileKind::Bzl,
            "a/a.bzl",
            A_BZL,
            &LoadMap::new(),
            &ctx(Phase::BzlLoad),
        )
        .unwrap();
        let mut loads = LoadMap::new();
        loads.insert("//a:a.bzl", a.clone());
        let b = exec(
            FileKind::Bzl,
            "b/b.bzl",
            B_BZL,
            &loads,
            &ctx(Phase::BzlLoad),
        )
        .unwrap();
        (a, b)
    })
}

#[test]
fn a_values_survive_freezing() {
    let (a, _) = load_ab();
    let lbl = a.get("LBL").unwrap();
    let l = lbl
        .value()
        .downcast_ref::<LabelValue>()
        .unwrap()
        .label()
        .clone();
    assert_eq!(l.get_package_name(), "pkg/a");
    let files = a.get("FILES").unwrap();
    assert_eq!(
        Depset::from_value(files.value())
            .unwrap()
            .to_list()
            .unwrap()
            .len(),
        2
    );
    let info = a.get("INFO").unwrap();
    assert_eq!(
        StarlarkInfo::from_value(info.value())
            .unwrap()
            .field_names()
            .collect::<Vec<_>>(),
        ["files", "label"]
    );
}

#[test]
fn b_loads_a() {
    let (_, b) = load_ab();
    assert_eq!(
        b.get("ALL_LIST").unwrap().value().to_repr(),
        r#"["a.h", "a.cc", "b.h"]"#
    );
    // Loaded names are not re-exported.
    assert!(b.get("impl").is_err());
}

/// Runs b_impl in a fresh module (analysis of one target) and returns
/// (actions, debug events, to_list of the returned files).
fn analyze(b: &FrozenModule, target: &str) -> (Vec<String>, Vec<String>, String) {
    let events = Arc::new(CollectingEvents::default());
    let cx = common::context(
        Phase::Analysis,
        StarlarkSemantics::default(),
        events.clone(),
    )
    .with_payload(AnalysisState::default());
    let b_impl = b.get("b_impl").unwrap();
    let label = blaze_cmdline::Label::parse_canonical(target).unwrap();
    let files = eval::call_frozen(
        &b_impl,
        &cx,
        &Default::default(),
        |m| {
            let heap = m.heap();
            let provider = heap.alloc(StructProvider);
            let ctx = heap.alloc(StarlarkInfo::new(
                provider,
                vec![
                    ("label".to_owned(), heap.alloc(LabelValue(label))),
                    (
                        "deps".to_owned(),
                        heap.alloc(AllocList(Vec::<Value>::new())),
                    ),
                ],
            ));
            (vec![ctx], vec![])
        },
        |res, m, _| {
            let first = ListRef::from_value(res).unwrap()[0];
            let files = first.get_attr("files", m.heap()).unwrap().unwrap();
            Ok(Depset::from_value(files)
                .unwrap()
                .to_list()
                .unwrap()
                .iter()
                .map(|v| v.to_str())
                .collect::<Vec<_>>()
                .join(","))
        },
    )
    .unwrap();
    let actions = cx.take_payload::<AnalysisState>().unwrap().actions;
    let prints = events.take().iter().map(|e| e.to_console()).collect();
    (actions, prints, files)
}

#[test]
fn c_frozen_function_on_another_thread_with_payload() {
    let (_, b) = load_ab();
    let t0 = Instant::now();
    let (actions, prints, files) = eval::run_on_evaluation_thread(move || analyze(&b, "//pkg:t"));
    eprintln!("analysis on another thread: {:?}", t0.elapsed());
    assert_eq!(actions, ["CppCompile"]);
    assert_eq!(prints, ["DEBUG: a/a.bzl:7:10: analyzing|t"]);
    assert_eq!(files, "a.h,a.cc,t.o");
}

#[test]
fn c_results_outlive_the_call_when_frozen() {
    let (_, b) = load_ab();
    let b_impl = b.get("b_impl").unwrap();
    let providers = eval::run_on_evaluation_thread(|| {
        let cx = ctx(Phase::Analysis).with_payload(AnalysisState::default());
        eval::call_frozen_and_freeze(&b_impl, &cx, &Default::default(), |m| {
            let heap = m.heap();
            let ctx = heap.alloc(StarlarkInfo::new(
                heap.alloc(StructProvider),
                vec![
                    (
                        "label".to_owned(),
                        heap.alloc(LabelValue(
                            blaze_cmdline::Label::parse_canonical("//pkg:u").unwrap(),
                        )),
                    ),
                    (
                        "deps".to_owned(),
                        heap.alloc(AllocList(Vec::<Value>::new())),
                    ),
                ],
            ));
            (vec![ctx], vec![])
        })
        .unwrap()
    });
    // Use the frozen providers as deps of another target, on another thread.
    let files = eval::run_on_evaluation_thread(move || {
        let cx = ctx(Phase::Analysis).with_payload(AnalysisState::default());
        let b_impl = b.get("b_impl").unwrap();
        eval::call_frozen(
            &b_impl,
            &cx,
            &Default::default(),
            |m| {
                let heap = m.heap();
                let deps = eval::frozen_into(m, &providers);
                let ctx = heap.alloc(StarlarkInfo::new(
                    heap.alloc(StructProvider),
                    vec![
                        (
                            "label".to_owned(),
                            heap.alloc(LabelValue(
                                blaze_cmdline::Label::parse_canonical("//pkg:v").unwrap(),
                            )),
                        ),
                        ("deps".to_owned(), deps),
                    ],
                ));
                (vec![ctx], vec![])
            },
            |res, m, _| {
                let first = ListRef::from_value(res).unwrap()[0];
                let files = first.get_attr("files", m.heap()).unwrap().unwrap();
                Ok(files.to_repr())
            },
        )
        .unwrap()
    });
    assert_eq!(files, r#"depset(["a.h", "a.cc", "u.o", "v.o"])"#);
}

#[test]
fn d_parallel_packages_share_frozen_bzl() {
    let (_, b) = load_ab();
    let mut loads = LoadMap::new();
    loads.insert("//b:b.bzl", b);
    let semantics = StarlarkSemantics::default();
    let globals = registry().build(FileKind::Build, &semantics);
    let n = 2000;
    let srcs: Vec<(String, String)> = (0..n)
        .map(|i| {
            (
                format!("pkg{i}/BUILD"),
                format!("load(\"//b:b.bzl\", \"my_macro\")\nmy_macro(name = \"t{i}\", extra = [\"x{i}\"])\n"),
            )
        })
        .collect();
    let run = |(name, src): &(String, String)| {
        let cx = ctx(Phase::Loading).with_payload(PackageState::default());
        let c = eval::compile(FileKind::Build, name, src.clone()).unwrap();
        eval::exec_module(c, &globals, &loads, &cx, &Default::default()).unwrap();
        cx.take_payload::<PackageState>().unwrap().targets
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(eval::EVAL_STACK_SIZE)
        .build()
        .unwrap();
    let t0 = Instant::now();
    let seq: Vec<_> = pool.install(|| srcs.iter().map(run).collect());
    let t_seq = t0.elapsed();
    let t0 = Instant::now();
    let par: Vec<_> = pool.install(|| srcs.par_iter().map(run).collect());
    eprintln!(
        "{n} packages: sequential {t_seq:?}, rayon ({} threads) {:?}",
        pool.current_num_threads(),
        t0.elapsed()
    );
    assert_eq!(seq, par);
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
            ("t7_lib".to_owned(), vec![]),
        ]
    );
}

#[test]
fn d_phase_checks() {
    let (_, b) = load_ab();
    let mut loads = LoadMap::new();
    loads.insert("//b:b.bzl", b);
    let e = eval::run_on_evaluation_thread(|| {
        exec(
            FileKind::Bzl,
            "x/x.bzl",
            "load('//b:b.bzl', 'my_macro')\nmy_macro(name = 'a')\n",
            &loads,
            &ctx(Phase::BzlLoad),
        )
        .unwrap_err()
    });
    assert!(
        e.message_with_stack().ends_with(
            "Error in rule_target: rule_target can only be used while evaluating a BUILD file"
        ),
        "{}",
        e.message_with_stack()
    );
}

#[test]
fn e_fail_traceback_across_files_matches_oracle() {
    let (_, b) = load_ab();
    let mut loads = LoadMap::new();
    loads.insert("//b:b.bzl", b);
    let e = eval::run_on_evaluation_thread(|| {
        let cx = ctx(Phase::Loading).with_payload(PackageState::default());
        exec(
            FileKind::Build,
            "pkg/BUILD",
            "load(\"//b:b.bzl\", \"failing_macro\")\n\nfailing_macro(name = \"x\")\n",
            &loads,
            &cx,
        )
        .unwrap_err()
    });
    // Byte-identical to Bazel 9.3.2's output for the same files (docs/research/05).
    assert_eq!(
        e.message_with_stack(),
        "Traceback (most recent call last):\n\
\tFile \"pkg/BUILD\", line 3, column 14, in <toplevel>\n\
\t\tfailing_macro(name = \"x\")\n\
\tFile \"b/b.bzl\", line 15, column 11, in failing_macro\n\
\t\t_check(name)\n\
\tFile \"b/b.bzl\", line 18, column 9, in _check\n\
\t\tfail(\"bad thing:\", name)\n\
Error in fail: bad thing: x"
    );
}

#[test]
fn cancellation_hook_interrupts() {
    use std::sync::atomic::AtomicBool;
    let flag = Arc::new(AtomicBool::new(true));
    let opts = eval::EvalOptions {
        cancelled: Some(flag),
        ..Default::default()
    };
    let e = eval::run_on_evaluation_thread(|| {
        let cx = ctx(Phase::BzlLoad);
        let globals = registry().build(FileKind::Bzl, &cx.semantics);
        let c = eval::compile(
            FileKind::Bzl,
            "x.bzl",
            "def f():\n    n = 0\n    for i in range(1000000):\n        n += i\n    return n\nx = f()\n".to_owned(),
        )
        .unwrap();
        eval::exec_module(c, &globals, &LoadMap::new(), &cx, &opts).unwrap_err()
    });
    assert_eq!(e.kind, blaze_starlark::error::EvalErrorKind::Cancelled);
}
