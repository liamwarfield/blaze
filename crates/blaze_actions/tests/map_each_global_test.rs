//! `map_each` must be a global of its defining module unless `allow_closure` is set
//! (review 2026-11-18-actions S2-2 and S3 group 1).
//!
//! `data/map_each_probe/workspace/cases.bzl` has one function per case, and
//! `data/map_each_probe/expected.txt` is what the oracle did with each of them
//! (`scripts/actions-probe/run`): `OK`, or the `Error in ...` line of the analysis failure. The
//! tests evaluate the same cases.bzl.
//!
//! Java: `StarlarkFunction.isGlobal()` (`module.getGlobal(getName()) == this`) in
//! `Args.MutableArgs.validateMapEach` and `TemplateDict.addJoined`.

use std::sync::Arc;

const CASES_BZL: &str = include_str!("data/map_each_probe/workspace/cases.bzl");
const EXPECTED: &str = include_str!("data/map_each_probe/expected.txt");

/// The directory the test compiles cases.bzl "in" (`<WORKSPACE>` of expected.txt).
const WORKSPACE: &str = "/workspace";

/// Test globals standing in for `ctx.actions.args()` and `ctx.actions.template_dict()`.
#[starlark::starlark_module]
fn probe_natives(builder: &mut starlark::environment::GlobalsBuilder) {
    fn make_args<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        let semantics = blaze_starlark::thread::ThreadContext::get(eval)?
            .semantics
            .clone();
        Ok(blaze_actions::analysis::starlark::args::new_args(
            eval.heap(),
            semantics,
        ))
    }

    fn make_template_dict<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        Ok(blaze_actions::analysis::starlark::template_dict::new_template_dict(eval.heap()))
    }
}

/// Runs one case the way defs.bzl does: `CASES[name](ctx.actions.args(),
/// ctx.actions.template_dict())`.
fn run_case(name: &str) -> Result<(), blaze_starlark::EvalError> {
    let code = format!(
        "{CASES_BZL}\nargs__ = make_args()\ndict__ = make_template_dict()\n\
         CASES[\"{name}\"](args__, dict__)\n"
    );
    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics)
        .with(blaze_starlark::depset::register_depset)
        .with(probe_natives)
        .build();
    let compiled = blaze_starlark::eval::compile(
        blaze_starlark::FileKind::Bzl,
        &format!("{WORKSPACE}/cases.bzl"),
        code,
    )
    .unwrap();
    let ctx = blaze_starlark::thread::ThreadContext::new(
        blaze_starlark::thread::Phase::BzlLoad,
        semantics,
        Arc::new(blaze_starlark::thread::CollectingEvents::default()),
    );
    blaze_starlark::eval::exec_module(
        compiled,
        &globals,
        &blaze_starlark::loader::LoadMap::new(),
        &ctx,
        &blaze_starlark::eval::EvalOptions::default(),
    )
    .map(|_| ())
}

/// (case, `None` for OK or the oracle's `Error in ...` line).
fn expected() -> Vec<(&'static str, Option<String>)> {
    let cases: Vec<_> = EXPECTED
        .lines()
        .map(|line| {
            let (case, result) = line.split_once('\t').unwrap();
            let error = (result != "OK").then(|| result.replace("<WORKSPACE>", WORKSPACE));
            (case, error)
        })
        .collect();
    assert_eq!(cases.len(), 13);
    cases
}

/// Every case the oracle accepts is accepted and every case it rejects is rejected: nested
/// `def`s (also when a top-level name is bound to one, the review's `nested_top` probe) and
/// lambdas are not global; a top-level `def`, an alias of one and a builtin are fine; and
/// `allow_closure` accepts anything.
#[test]
fn map_each_is_accepted_exactly_when_the_oracle_accepts_it() {
    for (case, error) in expected() {
        let got = run_case(case);
        match error {
            None => assert!(got.is_ok(), "{case}: oracle accepts, got {got:?}"),
            Some(_) => assert!(got.is_err(), "{case}: oracle rejects, Blaze accepted"),
        }
    }
}

/// The rejected cases fail with Bazel's error text, which names where the function is declared.
#[test]
#[ignore = "needs StarlarkFunction.getLocation: starlark-rust has no accessor for the position \
            of a def or lambda (DefInfo::signature_span); until then a rejected map_each is \
            unported! instead of this error (docs/reviews/2026-11-18-actions-response.md)"]
fn map_each_not_global_error_text() {
    for (case, error) in expected() {
        let Some(error) = error else { continue };
        let got = run_case(case).unwrap_err();
        // Java `EvalException.getMessageWithStack`: the last line is `Error in <fn>: <message>`.
        let text = got.message_with_stack();
        assert_eq!(text.lines().last().unwrap(), error, "{case}");
    }
}
