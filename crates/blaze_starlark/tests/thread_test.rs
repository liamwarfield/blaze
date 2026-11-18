//! ThreadContext: the repo mapping recorder thread-local (Java `Label.RepoMappingRecorder`),
//! payload replacement, the error event channel, and `exec_module_with_extra_value`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blaze_cmdline::LabelSimpleRepoMappingRecorder;
use blaze_cmdline::RepositoryName;
use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::thread::StarlarkEvents;

type Entries = Vec<(RepositoryName, String, RepositoryName)>;

fn name(s: &str) -> RepositoryName {
    RepositoryName::create_unvalidated(s)
}

/// Runs `src` as test.bzl with `configure` applied to the thread; returns the error message (if
/// any) and the thread's simple-recorder entries afterwards.
fn run(
    src: &str,
    configure: impl FnOnce(blaze_starlark::ThreadContext) -> blaze_starlark::ThreadContext + Send,
) -> (Result<(), String>, Option<Entries>) {
    let src = src.to_owned();
    eval::run_on_evaluation_thread(move || {
        let semantics = StarlarkSemantics::default();
        let events = Arc::new(CollectingEvents::default());
        let ctx = configure(common::context(Phase::BzlLoad, semantics.clone(), events));
        let globals =
            NativeRegistry::with_blaze_starlark_builtins().build(FileKind::Bzl, &semantics);
        let r = eval::compile(FileKind::Bzl, "test.bzl", src)
            .and_then(|c| {
                eval::exec_module(c, &globals, &LoadMap::new(), &ctx, &Default::default())
            })
            .map(|_| ())
            .map_err(|e| e.message);
        (
            r,
            ctx.take_simple_repo_mapping_recorder()
                .map(|r| r.recorded_entries()),
        )
    })
}

/// Java: BzlLoadFunction.executeBzlFile stores a SimpleRepoMappingRecorder in the thread;
/// `Label()` records apparent names it resolves (Label.parseWithPackageContext with a recorder),
/// but not canonical (`@@`) or repo-less labels.
#[test]
fn label_records_apparent_repo_names() {
    let (r, entries) = run(
        "a = Label('@foo//x:y')\nb = Label('@@bar//x')\nc = Label('//x')\nd = Label('@unknown//x')",
        |ctx| ctx.with_simple_repo_mapping_recorder(LabelSimpleRepoMappingRecorder::new()),
    );
    assert_eq!(r, Ok(()));
    let entries = entries.unwrap();
    let rendered: Vec<String> = entries
        .iter()
        .map(|(f, a, c)| format!("{}|{}|{}", f.get_name(), a, c.get_name()))
        .collect();
    // RepositoryMapping.get of an unknown apparent name returns a non-visible repo whose name
    // is the apparent name (Java RepositoryName.createUnvalidated(..).toNonVisible).
    assert_eq!(rendered.len(), 2, "{rendered:?}");
    assert_eq!(rendered[0], "|foo|foo+");
    assert!(rendered[1].starts_with("|unknown|"), "{rendered:?}");
}

/// Without a recorder `Label()` still resolves (Java passes a null recorder).
#[test]
fn label_without_recorder() {
    let (r, entries) = run("a = Label('@foo//x:y')", |ctx| ctx);
    assert_eq!(r, Ok(()));
    assert_eq!(entries, None);
}

/// Java StarlarkBaseExternalContext's recorder is a lambda writing into the context; here a
/// callback that sees the thread.
#[test]
fn label_records_through_callback() {
    let (r, seen) = eval::run_on_evaluation_thread(|| {
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let seen2 = seen.clone();
        let semantics = StarlarkSemantics::default();
        let events = Arc::new(CollectingEvents::default());
        let ctx = common::context(Phase::Repository, semantics.clone(), events)
            .with_repo_mapping_recorder_fn(move |thread, from, apparent, canonical| {
                assert!(thread.has_repo_mapping_recorder());
                assert_eq!(thread.recorded_repo_mapping_entries(), None);
                seen2.borrow_mut().push(format!(
                    "{}|{}|{}",
                    from.get_name(),
                    apparent,
                    canonical.get_name()
                ));
            });
        let globals =
            NativeRegistry::with_blaze_starlark_builtins().build(FileKind::Bzl, &semantics);
        let r = eval::compile(
            FileKind::Bzl,
            "test.bzl",
            "a = Label('@foo//:b')".to_owned(),
        )
        .and_then(|c| eval::exec_module(c, &globals, &LoadMap::new(), &ctx, &Default::default()))
        .map(|_| ())
        .map_err(|e| e.message);
        drop(ctx);
        let seen = seen.borrow().clone();
        (r, seen)
    });
    assert_eq!(r, Ok(()));
    assert_eq!(seen, vec!["|foo|foo+".to_owned()]);
}

#[test]
fn recorded_entries_include_prerecorded_and_are_sorted() {
    let mut rec = LabelSimpleRepoMappingRecorder::new();
    use blaze_cmdline::LabelRepoMappingRecorder;
    rec.record(&name("zz+"), "b", &name("b+"));
    let ctx = common::context(
        Phase::BzlLoad,
        StarlarkSemantics::default(),
        Arc::new(CollectingEvents::default()),
    )
    .with_simple_repo_mapping_recorder(rec);
    ctx.record_repo_mapping(&name(""), "a", &name("a+"));
    let entries = ctx.recorded_repo_mapping_entries().unwrap();
    let rendered: Vec<String> = entries
        .iter()
        .map(|(f, a, c)| format!("{}|{}|{}", f.get_name(), a, c.get_name()))
        .collect();
    assert_eq!(rendered, vec!["|a|a+", "zz+|b|b+"]);
}

#[test]
fn set_payload_replaces_payload() {
    let ctx = common::context(
        Phase::Other,
        StarlarkSemantics::default(),
        Arc::new(CollectingEvents::default()),
    )
    .with_payload(1u32);
    assert_eq!(*ctx.payload::<u32>().unwrap(), 1);
    ctx.set_payload(String::from("x"));
    assert!(ctx.payload::<u32>().is_none());
    assert_eq!(ctx.payload::<String>().unwrap().as_str(), "x");
    assert!(ctx.clear_payload().is_some());
    assert!(ctx.payload::<String>().is_none());
}

#[test]
fn collecting_events_error_channel() {
    let events = CollectingEvents::default();
    events.warn(None, "w");
    assert!(!events.saw_error());
    events.error(None, "boom");
    assert!(events.saw_error());
    let all = events.take();
    assert_eq!(all[1].kind, "ERROR");
    assert_eq!(all[1].to_console(), "ERROR: boom");
}

#[test]
fn exec_module_with_extra_value_sets_extra_before_freezing() {
    let extra = eval::run_on_evaluation_thread(|| {
        let semantics = StarlarkSemantics::default();
        let ctx = common::context(
            Phase::BzlLoad,
            semantics.clone(),
            Arc::new(CollectingEvents::default()),
        );
        let globals =
            NativeRegistry::with_blaze_starlark_builtins().build(FileKind::Bzl, &semantics);
        let compiled = eval::compile(FileKind::Bzl, "test.bzl", "x = 1".to_owned()).unwrap();
        let m = eval::exec_module_with_extra_value(
            compiled,
            &globals,
            &LoadMap::new(),
            &ctx,
            &Default::default(),
            |module| module.heap().alloc("extra"),
        )
        .unwrap();
        m.extra_value().map(|v| v.to_value().to_repr())
    });
    assert_eq!(extra, Some(r#""extra""#.to_owned()));
}
