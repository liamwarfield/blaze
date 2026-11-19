//! Shared helpers for blaze_starlark's integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::LabelRepoContextImpl;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_starlark::EvalError;
use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::thread::Event;
use starlark::environment::FrozenModule;

/// Main-repo mapping with `@foo` -> `@@foo+`.
pub fn main_mapping() -> RepositoryMapping {
    RepositoryMapping::create(
        [("foo", RepositoryName::create_unvalidated("foo+"))],
        blaze_cmdline::repository_name::MAIN.clone(),
    )
}

pub fn context(
    phase: Phase,
    semantics: StarlarkSemantics,
    events: Arc<CollectingEvents>,
) -> ThreadContext {
    let label = Label::parse_canonical("//pkg:test.bzl").unwrap();
    // `Label()` resolves against the module of the calling function (Java
    // BazelModuleContext.ofInnermostBzlOrFail). These tests execute modules without an extra
    // value, so the package context of each test file is registered on the thread.
    let package_context =
        blaze_cmdline::LabelPackageContext::of(label.get_package_identifier(), main_mapping());
    let mut ctx = ThreadContext::new(phase, Arc::new(semantics), events)
        .with_file_label(label)
        .with_label_repo_context(LabelRepoContextImpl::of(
            blaze_cmdline::repository_name::MAIN.clone(),
            main_mapping(),
        ))
        .with_main_repo_mapping(main_mapping());
    for file in [
        "test.bzl", "a.bzl", "a/a.bzl", "b/b.bzl", "x.bzl", "x/x.bzl",
    ] {
        ctx = ctx.with_file_package_context(file, package_context.clone());
    }
    ctx
}

/// Executes `src` as a .bzl file (with blaze_starlark's builtins) on an evaluation thread. The
/// file's bytes are the UTF-8 of `src`, decoded as Blaze decodes source files (Latin-1).
/// Returns the frozen module or error, plus recorded events.
pub fn exec_with(
    src: &str,
    semantics: StarlarkSemantics,
    loads: LoadMap,
) -> (Result<FrozenModule, EvalError>, Vec<Event>) {
    let src = src.to_owned();
    eval::run_on_evaluation_thread(move || {
        let events = Arc::new(CollectingEvents::default());
        let ctx = context(Phase::BzlLoad, semantics.clone(), events.clone());
        let globals =
            NativeRegistry::with_blaze_starlark_builtins().build(FileKind::Bzl, &semantics);
        // As Blaze reads a .bzl file: one char per byte (Java ParserInput.fromLatin1).
        let src = blaze_util::string_encoding::latin1_decode(src.as_bytes()).into_owned();
        let r = eval::compile(FileKind::Bzl, "test.bzl", src)
            .and_then(|c| eval::exec_module(c, &globals, &loads, &ctx, &Default::default()));
        (r, events.take())
    })
}

pub fn exec(src: &str) -> Result<FrozenModule, EvalError> {
    exec_with(src, StarlarkSemantics::default(), LoadMap::new()).0
}

/// `repr` of the module global `name`.
pub fn global_repr(m: &FrozenModule, name: &str) -> String {
    m.get(name).unwrap().value().to_repr()
}

/// Evaluates `r = <expr>` and returns repr(r) or the error message.
pub fn eval_expr(expr: &str) -> Result<String, String> {
    exec(&format!("r = {expr}\n"))
        .map(|m| global_repr(&m, "r"))
        .map_err(|e| e.message)
}

/// Executes statements; Ok(()) or the error message.
pub fn exec_err(src: &str) -> Result<(), String> {
    exec(src).map(|_| ()).map_err(|e| e.message)
}
