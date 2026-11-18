//! MODULE.bazel evaluation: ports of `CompiledModuleFile`, `ModuleThreadContext`,
//! `ModuleFileGlobals` (with its proxies and `TagCallable`), and the Starlark part of
//! `ModuleFileFunction` (`execModuleFile`, `injectRepos`).
//!
//! OWNER: agent A (module file evaluation). The public signatures below are the contract the
//! Skyframe functions (`crate::skyframe`) code against; keep them.
//!
//! How the Java thread model maps onto starlark-rust:
//! - The `StarlarkThread` is a `blaze_starlark::ThreadContext` (phase `ModuleFile`) whose payload
//!   is the [`ModuleThreadContext`]; natives reach it with `fromOrFail`.
//! - `include()` runs the included file on the same `ThreadContext` in a fresh starlark-rust
//!   `Module` (Java gives each compiled file its own `predeclaredEnv` module). Java's call stack
//!   spans files; here the frames of the including files are kept in
//!   [`ModuleThreadContext`]'s call-stack prefix and prepended to stacks captured or errors
//!   raised in the included file.
//! - The post-assign hook (`StarlarkExportable.export` of proxies) is starlark-rust's
//!   `StarlarkValue::export_as`, which the evaluator calls for every assignment to a top-level
//!   variable, as Java's hook does.

pub mod compiled_module_file;
mod java_hash;
pub mod module_file_globals;
pub mod module_thread_context;

use std::sync::Arc;

use blaze_bzlmod_model::{ExternalDepsCode, ExternalDepsException, ModuleKey, NonRegistryOverride};
use blaze_events::{Event, ExtendedEventHandler, Location};
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::StackEntry;
use blaze_starlark::eval::run_on_evaluation_thread;
use blaze_starlark::thread::StarlarkEvents;
use blaze_starlark::{EvalError, Phase, ThreadContext};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use parking_lot::Mutex;

pub use compiled_module_file::{CompiledModuleFile, IncludeStatement};
pub use module_file_globals::module_file_globals;
pub use module_thread_context::ModuleThreadContext;

/// Java `net.starlark.java.eval.EvalException` as raised by MODULE.bazel evaluation and by
/// `ModuleThreadContext.buildModule`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalException {
    /// `getMessage()`.
    pub message: String,
    /// `getMessageWithStack()` (Bazel's traceback form when a call stack is attached).
    pub message_with_stack: String,
    /// `getInnermostLocation()`.
    pub innermost_location: Option<Location>,
}

impl EvalException {
    /// `Starlark.errorf(...)` / `new EvalException(message)`: no call stack attached.
    // Java: net/starlark/java/eval/EvalException.java:54 EvalException.<init>
    pub fn new(message: impl Into<String>) -> EvalException {
        let message = message.into();
        EvalException {
            message_with_stack: message.clone(),
            message,
            innermost_location: None,
        }
    }

    /// `Starlark.errorf(...).withCallStack(stack)`.
    // Java: net/starlark/java/eval/EvalException.java:81 EvalException.withCallStack
    pub fn with_call_stack(
        message: impl Into<String>,
        callstack: Vec<StackEntry>,
    ) -> EvalException {
        blaze_util::check_argument!(!callstack.is_empty(), "Callstack cannot be empty");
        Self::from_eval_error(&EvalError {
            kind: blaze_starlark::error::EvalErrorKind::Eval,
            message: message.into(),
            callstack,
            include_stack: true,
        })
    }

    /// Converts a Starlark failure (with its Bazel call stack) to the Java exception view.
    pub fn from_eval_error(e: &EvalError) -> EvalException {
        EvalException {
            message: e.message.clone(),
            message_with_stack: e.message_with_stack(),
            innermost_location: innermost_location(e),
        }
    }
}

impl std::fmt::Display for EvalException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message_with_stack)
    }
}

impl std::error::Error for EvalException {}

/// Java `EvalException.getInnermostLocation`: the innermost call-stack entry that is not a
/// builtin. Syntax errors (which Java reports as `SyntaxError`s, not `EvalException`s) use the
/// first error's location.
// Java: net/starlark/java/eval/EvalException.java:122 EvalException.getInnermostLocation
fn innermost_location(e: &EvalError) -> Option<Location> {
    match &e.kind {
        blaze_starlark::error::EvalErrorKind::Syntax(_) => e.location().map(to_events_location),
        _ => e
            .callstack
            .iter()
            .rev()
            .find_map(|entry| entry.location.as_ref())
            .map(to_events_location),
    }
}

/// A blaze_starlark location as an event location.
pub(crate) fn to_events_location(l: &blaze_starlark::Location) -> Location {
    Location::new(l.file.clone(), l.line as i32, l.column as i32)
}

/// A call-stack entry's location; `None` is a builtin (Java `Location.BUILTIN`).
pub(crate) fn entry_location(l: Option<&blaze_starlark::Location>) -> Location {
    match l {
        Some(l) => to_events_location(l),
        None => Location::builtin().clone(),
    }
}

/// The `print` handler of a MODULE.bazel thread: `Event.makeDebugPrintHandler` (events are
/// buffered and replayed after evaluation, see [`exec_module_file`]) or a no-op.
struct PrintHandler {
    print_is_noop: bool,
    events: Mutex<Vec<Event>>,
}

impl StarlarkEvents for PrintHandler {
    // Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
    fn debug(&self, location: Option<&blaze_starlark::Location>, message: &str) {
        if self.print_is_noop {
            return;
        }
        self.events
            .lock()
            .push(Event::debug_at(Some(entry_location(location)), message));
    }

    fn warn(&self, location: Option<&blaze_starlark::Location>, message: &str) {
        self.events
            .lock()
            .push(Event::warn_at(location.map(to_events_location), message));
    }
}

/// Port of `ModuleFileFunction.execModuleFile` (and `injectRepos`): runs `compiled_root_module_file`
/// (on a big-stack evaluation thread) with a fresh [`ModuleThreadContext`], replays its warnings
/// on `event_handler`, and returns the context. On an `EvalException` it reports
/// `Event.error(innermostLocation, messageWithStack)` and fails with
/// `BAD_MODULE "error executing MODULE.bazel file for <key>"`.
// PORT: `print()` events are buffered during evaluation (the evaluation thread cannot borrow
// `event_handler`) and replayed, in order, before the warnings or the error; the order of events
// on `event_handler` is Java's. The profiler task and `SymbolGenerator` are skipped.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:644 ModuleFileFunction.execModuleFile
pub fn exec_module_file(
    compiled_root_module_file: &CompiledModuleFile,
    include_label_to_parsed_module_file: Option<Arc<IndexMap<String, CompiledModuleFile>>>,
    module_key: &ModuleKey,
    ignore_dev_deps: bool,
    builtin_modules: &IndexMap<String, NonRegistryOverride>,
    injected_repositories: &IndexMap<String, PathFragment>,
    print_is_noop: bool,
    starlark_semantics: &Arc<StarlarkSemantics>,
    event_handler: &dyn ExtendedEventHandler,
) -> Result<ModuleThreadContext, ExternalDepsException> {
    let context = ModuleThreadContext::new(
        builtin_modules.clone(),
        module_key.clone(),
        ignore_dev_deps,
        include_label_to_parsed_module_file,
    );
    let print_handler = Arc::new(PrintHandler {
        print_is_noop,
        events: Mutex::new(Vec::new()),
    });
    let events: Arc<dyn StarlarkEvents> = print_handler.clone();
    let semantics = starlark_semantics.clone();
    let (context, result) = run_on_evaluation_thread(move || {
        let thread = ThreadContext::new(Phase::ModuleFile, semantics, events).with_payload(context);
        let mut result = compiled_root_module_file
            .run_on_thread(&thread)
            .map_err(|e| EvalException::from_eval_error(&e));
        if result.is_ok() {
            let mut context = thread
                .payload::<ModuleThreadContext>()
                .expect("ModuleThreadContext payload");
            result = inject_repos(injected_repositories, &mut context);
        }
        let context = thread
            .take_payload::<ModuleThreadContext>()
            .expect("ModuleThreadContext payload");
        (context, result)
    });
    for event in print_handler.events.lock().drain(..) {
        event_handler.handle(&event);
    }
    match result {
        Ok(()) => {
            for warning in context.get_warnings() {
                event_handler.handle(&warning);
            }
            Ok(context)
        }
        Err(e) => {
            event_handler.handle(&Event::error_at(
                e.innermost_location.clone(),
                e.message_with_stack.clone(),
            ));
            Err(ExternalDepsException::with_message(
                ExternalDepsCode::BadModule,
                format!("error executing MODULE.bazel file for {module_key}"),
            ))
        }
    }
}

/// Adds a local_repository for each repository injected via --injected_repositories.
// PORT: runs after the program on the idle thread, as in Java: the caller location is
// `Location.BUILTIN` and the call stack is empty.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:697 ModuleFileFunction.injectRepos
fn inject_repos(
    injected_repositories: &IndexMap<String, PathFragment>,
    context: &mut ModuleThreadContext,
) -> Result<(), EvalException> {
    if injected_repositories.is_empty() {
        return Ok(());
    }
    // Use the innate extension backing use_repo_rule.
    let usage_builder = context.get_or_create_extension_usage_builder(
        "//:MODULE.bazel",
        "@bazel_tools//tools/build_defs/repo:local.bzl local_repository",
        /* isolate= */ false,
    );
    let containing_module_file_path = context.get_current_module_file_path().clone();
    let extension_proxy = context.new_module_extension_proxy(
        usage_builder,
        module_thread_context::ProxyBuilder::new(
            /* dev_dependency= */ true,
            Location::builtin().clone(),
            containing_module_file_path,
        ),
    );
    for (name, path) in injected_repositories {
        context.tag_call(
            extension_proxy,
            "repo",
            blaze_bzlmod_model::AttributeValues::create([
                (
                    "name",
                    blaze_bzlmod_model::AttributeValue::string(name.clone()),
                ),
                (
                    "path",
                    blaze_bzlmod_model::AttributeValue::string(path.get_path_string()),
                ),
            ]),
            Location::builtin().clone(),
        );
        context.proxy_add_import(
            extension_proxy,
            name,
            name,
            "by --inject_repository",
            Vec::new(),
        )?;
    }
    Ok(())
}
