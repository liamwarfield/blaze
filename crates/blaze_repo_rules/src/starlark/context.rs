//! The state behind `repository_ctx` and `module_ctx`, and their Starlark value types.
//!
//! Java: `StarlarkBaseExternalContext` (abstract) with subclasses `StarlarkRepositoryContext` and
//! `bzlmod.ModuleExtensionContext`; the Starlark object *is* the Java context. During a module
//! extension evaluation the thread also carries a `ModuleExtensionEvalStarlarkThreadContext`.
//!
//! PORT: starlark-rust values must be `Send + Sync` and cannot hold the Skyframe environment, so
//! the Rust state lives in the [`ThreadContext`] payload ([`ExternalContext`]) and the Starlark
//! objects ([`RepositoryCtx`], [`ModuleCtx`]) are stateless handles whose methods fetch the state
//! with [`ExternalContext::from_eval`]. Base methods (download, execute, file, path, read, ...)
//! are registered on both types by `base_external_context::base_external_context_methods`.
//!
//! Natives must drop the `RefMut` returned by [`ExternalContext::from_eval`] before calling back
//! into Starlark. Holding it across `env.get_value` is fine (a nested evaluation on the same
//! thread uses its own `ThreadContext`).

use std::cell::RefMut;
use std::fmt;

use allocative::Allocative;
use blaze_skyframe::Interrupted;
use blaze_starlark::ThreadContext;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use crate::bzlmod::eval_thread_context::ModuleExtensionEvalStarlarkThreadContext;
use crate::bzlmod::module_extension_context::ModuleExtensionContext;
use crate::starlark::base_external_context::StarlarkBaseExternalContext;
use crate::starlark::repository_context::StarlarkRepositoryContext;

/// Why a native aborted the Starlark evaluation without a Starlark-level error (Java exceptions
/// that propagate through `Starlark.call` unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationAbort {
    /// Java `NeedsSkyframeRestartException`: a Skyframe dependency is unavailable (in Blaze,
    /// only because it finished in error). The SkyFunction returns `Ok(None)`.
    NeedsSkyframeRestart,
    /// Java `InterruptedException`.
    Interrupted,
}

/// The subclass-specific part of an [`ExternalContext`].
pub enum ExternalContextKind {
    /// `repository_ctx` (Java `StarlarkRepositoryContext`).
    Repository(StarlarkRepositoryContext),
    /// `module_ctx` (Java `ModuleExtensionContext`), plus the thread's
    /// `ModuleExtensionEvalStarlarkThreadContext` (repo rules called by the extension record
    /// their calls there).
    ModuleExtension {
        ctx: ModuleExtensionContext,
        thread_context: ModuleExtensionEvalStarlarkThreadContext,
    },
}

/// The [`ThreadContext`] payload of a repository rule or module extension evaluation.
pub struct ExternalContext {
    /// Java `StarlarkBaseExternalContext` fields.
    pub base: StarlarkBaseExternalContext,
    pub kind: ExternalContextKind,
    /// Set by a native right before it fails because of [`EvaluationAbort`]; the caller of the
    /// implementation function checks it before reporting a Starlark error.
    pub abort: Option<EvaluationAbort>,
}

impl ExternalContext {
    /// The context of the running evaluation, or an error if this is not a repository rule or
    /// module extension evaluation.
    pub fn from_eval<'a>(
        eval: &Evaluator<'_, 'a, '_>,
    ) -> starlark::Result<RefMut<'a, ExternalContext>> {
        Self::from_eval_or_none(eval).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "internal error: no repository_ctx or module_ctx on this thread"
            ))
        })
    }

    /// The context of the running evaluation, if any.
    pub fn from_eval_or_none<'a>(
        eval: &Evaluator<'_, 'a, '_>,
    ) -> Option<RefMut<'a, ExternalContext>> {
        ThreadContext::get(eval)
            .ok()
            .and_then(|thread| thread.payload::<ExternalContext>())
    }

    /// Records `abort` and returns the Starlark error that unwinds the evaluation.
    pub fn abort(&mut self, abort: EvaluationAbort) -> starlark::Error {
        self.abort = Some(abort);
        starlark::Error::new_other(anyhow::anyhow!(match abort {
            EvaluationAbort::NeedsSkyframeRestart => "internal: needs Skyframe restart",
            EvaluationAbort::Interrupted => "interrupted",
        }))
    }

    /// `env.valuesMissing()` after a dependency request: aborts with
    /// [`EvaluationAbort::NeedsSkyframeRestart`].
    pub fn needs_restart(&mut self) -> starlark::Error {
        self.abort(EvaluationAbort::NeedsSkyframeRestart)
    }

    /// An [`Interrupted`] from a dependency request.
    pub fn interrupted(&mut self, _: Interrupted) -> starlark::Error {
        self.abort(EvaluationAbort::Interrupted)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:322 StarlarkBaseExternalContext.shouldDeleteWorkingDirectoryOnClose
    pub fn should_delete_working_directory_on_close(&self, successful: bool) -> bool {
        match &self.kind {
            ExternalContextKind::Repository(r) => {
                r.should_delete_working_directory_on_close(successful)
            }
            ExternalContextKind::ModuleExtension { ctx, .. } => {
                ctx.should_delete_working_directory_on_close(successful)
            }
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1859 StarlarkBaseExternalContext.isRemotable
    pub fn is_remotable(&self) -> bool {
        match &self.kind {
            ExternalContextKind::Repository(r) => r.is_remotable(),
            ExternalContextKind::ModuleExtension { ctx, .. } => ctx.is_remotable(),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1868 StarlarkBaseExternalContext.getRemoteExecProperties
    pub fn get_remote_exec_properties(&self) -> anyhow::Result<IndexMap<String, String>> {
        match &self.kind {
            ExternalContextKind::Repository(r) => r.get_remote_exec_properties(),
            ExternalContextKind::ModuleExtension { ctx, .. } => ctx.get_remote_exec_properties(),
        }
    }

    /// The `StarlarkRepositoryContext` part. Panics for a module extension (natives registered
    /// only on `repository_ctx` call this).
    pub fn repository(&mut self) -> &mut StarlarkRepositoryContext {
        match &mut self.kind {
            ExternalContextKind::Repository(r) => r,
            ExternalContextKind::ModuleExtension { .. } => {
                panic!("internal error: repository_ctx method on module_ctx")
            }
        }
    }

    /// The `ModuleExtensionContext` part. Panics for a repository rule.
    pub fn module_extension(&mut self) -> &mut ModuleExtensionContext {
        match &mut self.kind {
            ExternalContextKind::ModuleExtension { ctx, .. } => ctx,
            ExternalContextKind::Repository(_) => {
                panic!("internal error: module_ctx method on repository_ctx")
            }
        }
    }

    /// The `ModuleExtensionEvalStarlarkThreadContext`, if this is a module extension evaluation
    /// (Java `ModuleExtensionEvalStarlarkThreadContext.fromOrNull(thread)`).
    pub fn extension_thread_context(
        &mut self,
    ) -> Option<&mut ModuleExtensionEvalStarlarkThreadContext> {
        match &mut self.kind {
            ExternalContextKind::ModuleExtension { thread_context, .. } => Some(thread_context),
            ExternalContextKind::Repository(_) => None,
        }
    }
}

/// The `repository_ctx` object passed to a repository rule implementation.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:63 StarlarkRepositoryContext
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RepositoryCtx {
    /// Java `toString`: `repository_ctx[<name>]`.
    pub name: String,
    /// The repo being fetched (`name`, `original_name`, `attr` are attributes, which cannot
    /// reach the thread payload).
    #[allocative(skip)]
    pub repo_definition: std::sync::Arc<crate::repository::repo_definition::RepoDefinition>,
    /// Java `directories.getWorkspace()` (`workspace_root`).
    #[allocative(skip)]
    pub workspace_root: blaze_vfs::Path,
    /// `os` (Java `StarlarkBaseExternalContext.osObject`).
    #[allocative(skip)]
    pub os: crate::starlark::os::StarlarkOS,
    /// Java `identifyingStringForLogging`, for the `os` field's `WorkspaceRuleEvent` (an
    /// attribute cannot reach the thread payload).
    pub identifying_string_for_logging: String,
}

starlark_simple_value!(RepositoryCtx);

impl fmt::Display for RepositoryCtx {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:658 StarlarkRepositoryContext.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "repository_ctx[{}]", self.name)
    }
}

fn repository_ctx_all_methods(builder: &mut MethodsBuilder) {
    crate::starlark::base_external_context::base_external_context_methods(builder);
    crate::starlark::repository_context::repository_ctx_methods(builder);
}

#[starlark_value(type = "repository_ctx")]
impl<'v> StarlarkValue<'v> for RepositoryCtx {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("repository_ctx", repository_ctx_all_methods);
        Some(RES.methods())
    }
}

/// `repository_ctx` with `load_wasm` and `execute_wasm`, for a thread whose semantics have
/// `--experimental_repository_ctx_execute_wasm` (Java: methods with `enableOnlyWithFlag` are
/// not members of the object otherwise: `getattr` fails at the dot, `hasattr` and `dir` do not
/// list them).
///
/// PORT: starlark-rust's method table is per type, so the flag picks one of two types.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RepositoryCtxWithWasm(pub RepositoryCtx);

starlark_simple_value!(RepositoryCtxWithWasm);

impl fmt::Display for RepositoryCtxWithWasm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

fn repository_ctx_with_wasm_methods(builder: &mut MethodsBuilder) {
    repository_ctx_all_methods(builder);
    crate::starlark::base_external_context::wasm_methods(builder);
}

#[starlark_value(type = "repository_ctx")]
impl<'v> StarlarkValue<'v> for RepositoryCtxWithWasm {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic =
            MethodsStatic::new("repository_ctx", repository_ctx_with_wasm_methods);
        Some(RES.methods())
    }
}

/// Allocates the `repository_ctx` handle, with the wasm methods when `execute_wasm` is set.
pub fn alloc_repository_ctx<'v>(
    heap: Heap<'v>,
    ctx: RepositoryCtx,
    execute_wasm: bool,
) -> Value<'v> {
    if execute_wasm {
        heap.alloc(RepositoryCtxWithWasm(ctx))
    } else {
        heap.alloc(ctx)
    }
}

/// The [`RepositoryCtx`] of a `repository_ctx` handle of either type.
pub fn repository_ctx_of<'v>(value: Value<'v>) -> Option<&'v RepositoryCtx> {
    value
        .downcast_ref::<RepositoryCtx>()
        .or_else(|| value.downcast_ref::<RepositoryCtxWithWasm>().map(|c| &c.0))
}

/// The `module_ctx` object passed to a module extension implementation.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtensionContext.java:52 ModuleExtensionContext
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ModuleCtx {
    /// Java `toString`: `module_ctx[<extension id>]`.
    pub extension_id: String,
    /// `os` (Java `StarlarkBaseExternalContext.osObject`).
    #[allocative(skip)]
    pub os: crate::starlark::os::StarlarkOS,
    /// The values behind `modules`, `facts`, ... (attributes cannot reach the thread payload).
    #[allocative(skip)]
    pub fields: std::sync::Arc<crate::bzlmod::module_extension_context::ModuleCtxFields>,
    /// Java `identifyingStringForLogging` (see [`RepositoryCtx`]).
    pub identifying_string_for_logging: String,
}

starlark_simple_value!(ModuleCtx);

impl fmt::Display for ModuleCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "module_ctx[{}]", self.extension_id)
    }
}

fn module_ctx_all_methods(builder: &mut MethodsBuilder) {
    crate::starlark::base_external_context::base_external_context_methods(builder);
    crate::bzlmod::module_extension_context::module_ctx_methods(builder);
}

#[starlark_value(type = "module_ctx")]
impl<'v> StarlarkValue<'v> for ModuleCtx {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("module_ctx", module_ctx_all_methods);
        Some(RES.methods())
    }

    // The flag-gated `is_isolated` field (Java `enableOnlyWithFlag =
    // "-experimental_isolated_extension_usages"`) exists only when the flag is on.
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "is_isolated" => crate::bzlmod::module_extension_context::is_isolated(&self.fields)
                .map(|b| heap.alloc(b)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "is_isolated"
            && crate::bzlmod::module_extension_context::is_isolated(&self.fields).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        match crate::bzlmod::module_extension_context::is_isolated(&self.fields) {
            Some(_) => vec!["is_isolated".to_owned()],
            None => Vec::new(),
        }
    }
}

/// `module_ctx` with `load_wasm` and `execute_wasm` (see [`RepositoryCtxWithWasm`]).
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ModuleCtxWithWasm(pub ModuleCtx);

starlark_simple_value!(ModuleCtxWithWasm);

impl fmt::Display for ModuleCtxWithWasm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

fn module_ctx_with_wasm_methods(builder: &mut MethodsBuilder) {
    module_ctx_all_methods(builder);
    crate::starlark::base_external_context::wasm_methods(builder);
}

#[starlark_value(type = "module_ctx")]
impl<'v> StarlarkValue<'v> for ModuleCtxWithWasm {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("module_ctx", module_ctx_with_wasm_methods);
        Some(RES.methods())
    }

    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        self.0.get_attr(attribute, heap)
    }

    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        self.0.has_attr(attribute, heap)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.0.dir_attr()
    }
}

/// Allocates the `module_ctx` handle, with the wasm methods when `execute_wasm` is set.
pub fn alloc_module_ctx<'v>(heap: Heap<'v>, ctx: ModuleCtx, execute_wasm: bool) -> Value<'v> {
    if execute_wasm {
        heap.alloc(ModuleCtxWithWasm(ctx))
    } else {
        heap.alloc(ctx)
    }
}

/// The [`ModuleCtx`] of a `module_ctx` handle of either type.
pub fn module_ctx_of<'v>(value: Value<'v>) -> Option<&'v ModuleCtx> {
    value
        .downcast_ref::<ModuleCtx>()
        .or_else(|| value.downcast_ref::<ModuleCtxWithWasm>().map(|c| &c.0))
}
