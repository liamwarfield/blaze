//! Running a repository rule or module extension implementation function.
//!
//! Java: the `StarlarkThread` set up in `RepositoryFetchFunction.fetch` and
//! `RegularRunnableExtension.runInternal` (`Mutability.create`, `StarlarkThread.create`,
//! `setPrintHandler(Event.makeDebugPrintHandler(...))`, `storeRepoMappingRecorderInThread`,
//! the `StarlarkThreadContext` with the main repo mapping), then `Starlark.positionalOnlyCall`.

use std::sync::Arc;

use blaze_cmdline::RepositoryMapping;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_starlark::EvalError;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::eval::call_frozen;
use blaze_starlark::thread::CollectingEvents;
use blaze_starlark::thread::Event;
use starlark::eval::Evaluator;
use starlark::values::Heap;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;

use super::context::ExternalContext;

/// The outcome of [`call_implementation`].
pub struct ImplementationOutcome<R> {
    /// The extracted return value, or the Starlark error (Java `EvalException`). When
    /// `context.abort` is set, the error only unwinds the evaluation and must not be reported.
    pub result: Result<R, EvalError>,
    /// The payload, taken back from the thread.
    pub context: ExternalContext,
    /// `print()` output and Starlark warnings, in order (Java's print handler posts DEBUG events
    /// to the environment's listener as they happen).
    pub events: Vec<Event>,
}

/// Calls `function(ctx)` where `ctx = make_ctx(heap)` with `payload` as the thread's
/// [`ExternalContext`], and converts the return value with `extract`.
///
/// `module_context` is the `BazelModuleContext` of the .bzl that exported the rule or extension:
/// `Label()` called by any function reachable through its loads resolves against the calling
/// function's .bzl (Java `BazelModuleContext.ofInnermostBzlOrFail`).
///
/// PORT: Java's print handler reports events immediately; they are returned here and the caller
/// reports them after the call.
pub fn call_implementation<R>(
    function: &OwnedFrozenValue,
    module_context: Option<&Arc<BazelModuleContext>>,
    phase: Phase,
    semantics: Arc<StarlarkSemantics>,
    main_repo_mapping: Option<RepositoryMapping>,
    payload: ExternalContext,
    make_ctx: impl for<'v> FnOnce(Heap<'v>) -> Value<'v>,
    extract: impl for<'v> FnOnce(Value<'v>, &mut Evaluator<'v, '_, '_>) -> Result<R, EvalError>,
) -> ImplementationOutcome<R> {
    let events = Arc::new(CollectingEvents::default());
    let mut thread = ThreadContext::new(phase, semantics, events.clone());
    if let Some(module_context) = module_context {
        thread = thread
            .with_file_label(module_context.label().clone())
            .with_file_package_context(
                module_context.filename.clone(),
                module_context.package_context(),
            );
        let mut contexts = Vec::new();
        BazelModuleContext::visit_load_graph_recursively(&module_context.loads, &mut |m| {
            contexts.push((m.filename.clone(), m.package_context()));
            true
        });
        for (file, ctx) in contexts {
            thread = thread.with_file_package_context(file, ctx);
        }
    }
    if let Some(mapping) = main_repo_mapping {
        thread = thread.with_main_repo_mapping(mapping);
    }
    // Java: `storeRepoMappingRecorderInThread(thread)`: `Label()` records apparent repo name
    // lookups as `RecordedRepoMapping` inputs of the context (see #20721).
    let thread = thread
        .with_repo_mapping_recorder_fn(|thread, from_repo, apparent_repo_name, canonical| {
            let mut context = thread
                .payload::<ExternalContext>()
                .expect("the ExternalContext payload");
            context
                .base
                .record_repo_mapping(from_repo, apparent_repo_name, canonical);
        })
        .with_payload(payload);
    let result = call_frozen(
        function,
        &thread,
        &EvalOptions::default(),
        |module| (vec![make_ctx(module.heap())], vec![]),
        |value, _module, eval| extract(value, eval),
    );
    let context = thread
        .take_payload::<ExternalContext>()
        .expect("the ExternalContext payload is only taken here");
    ImplementationOutcome {
        result,
        context,
        events: events.take(),
    }
}
