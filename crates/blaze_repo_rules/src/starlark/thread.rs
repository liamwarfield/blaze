//! Running a repository rule or module extension implementation function.
//!
//! Java: the `StarlarkThread` set up in `RepositoryFetchFunction.fetch` and
//! `RegularRunnableExtension.runInternal` (`Mutability.create`, `StarlarkThread.create`,
//! `setPrintHandler(Event.makeDebugPrintHandler(...))`, `storeRepoMappingRecorderInThread`,
//! the `StarlarkThreadContext` with the main repo mapping), then `Starlark.positionalOnlyCall`.

use std::cell::RefCell;
use std::sync::Arc;

use blaze_cmdline::RepositoryMapping;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_sky_loading::starlark_util::event_location;
use blaze_starlark::EvalError;
use blaze_starlark::Location;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::eval::call_frozen;
use blaze_starlark::thread::Event;
use blaze_starlark::thread::StarlarkEvents;
use starlark::eval::Evaluator;
use starlark::values::Heap;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;

use super::context::ExternalContext;
use crate::env_handle::SkyEnv;

/// The outcome of [`call_implementation`].
pub struct ImplementationOutcome<R> {
    /// The extracted return value, or the Starlark error (Java `EvalException`). When
    /// `context.abort` is set, the error only unwinds the evaluation and must not be reported.
    pub result: Result<R, EvalError>,
    /// The payload, taken back from the thread.
    pub context: ExternalContext,
    /// Always empty: `print()` output and Starlark warnings are reported to the environment's
    /// listener as they happen (see `DebugPrintHandler`). The field is kept so that code written
    /// against the buffering version still compiles.
    pub events: Vec<Event>,
}

thread_local! {
    /// The Skyframe environments of the repository rule and module extension implementations
    /// running on this thread, innermost last (a native of one implementation can request a
    /// dependency whose computation runs another implementation on the same thread).
    ///
    /// PORT: Java's print handler closes over `env.getListener()`. A [`StarlarkEvents`] sink
    /// must be `Send + Sync` and so cannot hold the [`SkyEnv`]; [`DebugPrintHandler`] finds the
    /// environment of the evaluation it runs in here.
    static LISTENER_ENVS: RefCell<Vec<SkyEnv>> = const { RefCell::new(Vec::new()) };
}

/// Removes the innermost environment from [`LISTENER_ENVS`] when the implementation call ends,
/// also when it unwinds: the handle must not outlive the call.
struct ListenerEnvGuard;

impl ListenerEnvGuard {
    fn push(env: SkyEnv) -> ListenerEnvGuard {
        LISTENER_ENVS.with(|envs| envs.borrow_mut().push(env));
        ListenerEnvGuard
    }
}

impl Drop for ListenerEnvGuard {
    fn drop(&mut self) {
        LISTENER_ENVS.with(|envs| {
            envs.borrow_mut().pop();
        });
    }
}

/// Reports `event` to the listener of the environment of the implementation running on this
/// thread.
///
/// Java: `SkyFunctionEnvironment.handle` (through `WorkerSkyFunctionEnvironment.handle`, which
/// forwards to the current delegate's listener). DEBUG and WARNING events are not stored for
/// replay (`Event.storeForReplay`), so the environment hands them to the reporter at once; they
/// are not kept with the Skyframe node and not replayed.
// Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:816 SkyFunctionEnvironment.handle
// Java: com/google/devtools/build/skyframe/WorkerSkyFunctionEnvironment.java:143 WorkerSkyFunctionEnvironment.handle
fn report(event: blaze_events::Event) {
    LISTENER_ENVS.with(|envs| {
        let envs = envs.borrow();
        let env = envs
            .last()
            .expect("a Starlark event outside of a repository rule or module extension call");
        env.get().listener().handle(&event);
    })
}

/// The print handler of a repository rule or module extension thread: sends DEBUG events to the
/// environment's listener as `print()` runs, so that they keep their order relative to what the
/// implementation reports directly (`execute(quiet = False)` output, progress, warnings).
// Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:656 RepositoryFetchFunction.fetch
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegularRunnableExtension.java:290 RegularRunnableExtension.runInternal
struct DebugPrintHandler;

impl StarlarkEvents for DebugPrintHandler {
    fn debug(&self, location: Option<&Location>, message: &str) {
        report(blaze_events::Event::debug_at(
            location.map(event_location),
            message,
        ));
    }

    fn warn(&self, location: Option<&Location>, message: &str) {
        report(blaze_events::Event::warn_at(
            location.map(event_location),
            message,
        ));
    }
}

/// Calls `function(ctx)` where `ctx = make_ctx(heap)` with `payload` as the thread's
/// [`ExternalContext`], and converts the return value with `extract`.
///
/// `module_context` is the `BazelModuleContext` of the .bzl that exported the rule or extension:
/// `Label()` called by any function reachable through its loads resolves against the calling
/// function's .bzl (Java `BazelModuleContext.ofInnermostBzlOrFail`).
///
/// `print()` output is reported to `payload.base.env`'s listener while the function runs (Java
/// `thread.setPrintHandler(Event.makeDebugPrintHandler(env.getListener()))`).
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
    // A second handle to the payload's environment, for the print handler. The handle leaves the
    // closure of `SkyEnv::scope`, which is sound here only because it aliases `payload.base.env`:
    // that handle is valid for this whole call (the caller's `SkyEnv::scope` is still running),
    // and the guard drops this one before the call returns.
    let listener_env = SkyEnv::scope(payload.base.env.get(), |env| env);
    let _listener_env_guard = ListenerEnvGuard::push(listener_env);

    // Java: thread.setPrintHandler(Event.makeDebugPrintHandler(env.getListener()))
    let mut thread = ThreadContext::new(phase, semantics, Arc::new(DebugPrintHandler));
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
        events: Vec::new(),
    }
}
