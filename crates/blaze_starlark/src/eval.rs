//! Compiling and executing Starlark files and calling frozen functions.
//!
//! Java: com/google/devtools/build/lib/skyframe/BzlCompileFunction.java (compile),
//! BzlLoadFunction.executeBzlFile and PackageFunction's BUILD execution (exec), and
//! `Starlark.call` for rule implementations. Threading rules (docs/research/05-starlark-rust.md
//! 6.2):
//! - one `Module` per evaluation unit, created and dropped on one thread;
//! - only `FrozenModule` / `OwnedFrozenValue` or plain Rust data leave an evaluation;
//! - evaluation recurses on the native stack (calls, repr, freezing, GC of deep values), so run
//!   it on a thread from [`run_on_evaluation_thread`] or with at least [`EVAL_STACK_SIZE`].

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use starlark::environment::FrozenModule;
use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::values::OwnedFrozenValue;
use starlark::values::Value;

use crate::error::EvalError;
use crate::error::EvalErrorKind;
use crate::error::Location;
use crate::error::SyntaxError;
use crate::file_kind::FileKind;
use crate::loader::LoadMap;
use crate::loader::LoadRequest;
use crate::loader::load_requests;
use crate::syntax::BuildFileFacts;
use crate::thread::ThreadContext;

/// Native stack size for evaluation threads. Virtual memory only; pages are committed on use.
pub const EVAL_STACK_SIZE: usize = 256 << 20;

/// Default Starlark call depth limit (frames, natives included). starlark-rust's default is 50;
/// Bazel has no fixed limit (only the JVM stack).
pub const DEFAULT_MAX_CALLSTACK: usize = 1000;

/// Knobs for one evaluation.
#[derive(Clone)]
pub struct EvalOptions {
    pub max_callstack_size: usize,
    /// Set to true to interrupt the evaluation (Java `InterruptedException`).
    pub cancelled: Option<Arc<AtomicBool>>,
}

impl Default for EvalOptions {
    fn default() -> Self {
        EvalOptions {
            max_callstack_size: DEFAULT_MAX_CALLSTACK,
            cancelled: None,
        }
    }
}

/// A parsed and statically checked file (Java `BzlCompileValue` / `CompiledBuildFile`).
pub struct CompiledFile {
    pub kind: FileKind,
    pub ast: AstModule,
    pub loads: Vec<LoadRequest>,
    /// BUILD only: glob/subpackages prefetch patterns and generator names.
    pub build_facts: BuildFileFacts,
}

impl std::fmt::Debug for CompiledFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledFile")
            .field("kind", &self.kind)
            .field("loads", &self.loads)
            .field("build_facts", &self.build_facts)
            .finish_non_exhaustive()
    }
}

/// Parses `source` as a file of `kind` and runs Bazel's static checks.
pub fn compile(kind: FileKind, file_name: &str, source: String) -> Result<CompiledFile, EvalError> {
    let ast = AstModule::parse(file_name, source, &kind.dialect()).map_err(|e| {
        let location = e
            .span()
            .map(Location::of_span_start)
            .unwrap_or_else(|| Location {
                file: file_name.to_owned(),
                line: 0,
                column: 0,
            });
        EvalError::syntax(vec![SyntaxError {
            location,
            message: crate::error::bare_message(&e),
        }])
    })?;
    let (errors, build_facts) = crate::syntax::check_file(kind, &ast);
    if !errors.is_empty() {
        return Err(EvalError::syntax(errors));
    }
    let loads = load_requests(&ast);
    Ok(CompiledFile {
        kind,
        ast,
        loads,
        build_facts,
    })
}

fn panic_error(p: Box<dyn std::any::Any + Send>) -> EvalError {
    let what = p
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    EvalError {
        kind: EvalErrorKind::Panic,
        message: format!("internal error: Starlark evaluation panicked: {what}"),
        callstack: Vec::new(),
        include_stack: true,
    }
}

/// Runs `f` with a fresh module and an evaluator configured for Blaze: `ctx` in
/// `Evaluator::extra`, `loads` as the loader, recursion forbidden unless the semantics allow it,
/// the call-depth limit and cancellation hook from `opts`. Panics inside starlark-rust become
/// [`EvalErrorKind::Panic`].
pub fn with_evaluator<R>(
    ctx: &ThreadContext,
    loads: Option<&LoadMap>,
    opts: &EvalOptions,
    f: impl for<'v, 'a> FnOnce(&'a Module<'v>, &mut Evaluator<'v, 'a, '_>) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(ctx);
            if let Some(l) = loads {
                eval.set_loader(l);
            }
            eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
            eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                .expect("fresh evaluator");
            if let Some(c) = &opts.cancelled {
                let c = c.clone();
                eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
            }
            f(&module, &mut eval)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

fn freeze(module: Module) -> Result<FrozenModule, EvalError> {
    module.freeze().map_err(|e| EvalError {
        kind: EvalErrorKind::Eval,
        message: format!("{e:?}"),
        callstack: Vec::new(),
        include_stack: true,
    })
}

/// Executes a compiled file and freezes its module. `loads` must hold every load string of
/// `compiled.loads`.
pub fn exec_module(
    compiled: CompiledFile,
    globals: &Globals,
    loads: &LoadMap,
    ctx: &ThreadContext,
    opts: &EvalOptions,
) -> Result<FrozenModule, EvalError> {
    let ast = compiled.ast;
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            {
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(ctx);
                eval.set_loader(loads);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                eval.eval_module(ast, globals)
                    .map_err(|e| EvalError::from_starlark(&e))?;
            }
            freeze(module)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Like [`exec_module`], but first sets the module's extra value to `extra_value(module)`
/// (starlark-rust `Module::set_extra_value`), so it is frozen with the module. Blaze stores the
/// .bzl's `BazelModuleContext` there (Java `Module.setClientData`).
pub fn exec_module_with_extra_value(
    compiled: CompiledFile,
    globals: &Globals,
    loads: &LoadMap,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    extra_value: impl for<'v> FnOnce(&Module<'v>) -> Value<'v>,
) -> Result<FrozenModule, EvalError> {
    let ast = compiled.ast;
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let extra = extra_value(&module);
            module.set_extra_value(extra);
            {
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(ctx);
                eval.set_loader(loads);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                eval.eval_module(ast, globals)
                    .map_err(|e| EvalError::from_starlark(&e))?;
            }
            freeze(module)
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Brings a value frozen elsewhere (a function from a loaded .bzl, a provider from a dependency's
/// analysis) into `module`'s heap.
///
/// starlark-rust's `OwnedFrozenValue::owned_value(&FrozenHeap)` cannot be used with
/// `Module::with_temp_heap`: the returned value's lifetime is the heap borrow, not the module's
/// invariant brand `'v`. This is Blaze's one audited `unsafe`.
pub fn frozen_into<'v>(module: &Module<'v>, v: &OwnedFrozenValue) -> Value<'v> {
    // SAFETY: `owned_frozen_value` adds `v`'s heap as a reference of `module`'s frozen heap, so
    // the value lives at least as long as the module and anything frozen from it.
    Value::new_frozen(unsafe { v.owned_frozen_value(module.frozen_heap()) })
}

/// Calls a frozen function in a fresh module (Java `Starlark.call` on a rule implementation).
/// `args` builds the arguments on the new heap; `extract` turns the result into Rust data (or
/// or use [`call_frozen_and_freeze`] to keep the result as a frozen value).
pub fn call_frozen<R>(
    function: &OwnedFrozenValue,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    args: impl for<'v> FnOnce(&Module<'v>) -> (Vec<Value<'v>>, Vec<(String, Value<'v>)>),
    extract: impl for<'v> FnOnce(
        Value<'v>,
        &Module<'v>,
        &mut Evaluator<'v, '_, '_>,
    ) -> Result<R, EvalError>,
) -> Result<R, EvalError> {
    with_evaluator(ctx, None, opts, |module, eval| {
        let f = frozen_into(module, function);
        let (pos, named) = args(module);
        let named: Vec<(&str, Value)> = named.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let res = eval
            .eval_function(f, &pos, &named)
            .map_err(|e| EvalError::from_starlark(&e))?;
        extract(res, module, eval)
    })
}

/// Like [`call_frozen`], but stores the result as `result` in the call's module and freezes it,
/// so the value outlives the call (Bazel keeps providers returned by analysis).
pub fn call_frozen_and_freeze(
    function: &OwnedFrozenValue,
    ctx: &ThreadContext,
    opts: &EvalOptions,
    args: impl for<'v> FnOnce(&Module<'v>) -> (Vec<Value<'v>>, Vec<(String, Value<'v>)>),
) -> Result<OwnedFrozenValue, EvalError> {
    let run = AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            {
                let mut eval = Evaluator::new(&module);
                eval.extra = Some(ctx);
                eval.set_forbid_recursion(!ctx.semantics.allow_recursion);
                eval.set_max_callstack_size(opts.max_callstack_size.max(1))
                    .expect("fresh evaluator");
                if let Some(c) = &opts.cancelled {
                    let c = c.clone();
                    eval.set_check_cancelled(Box::new(move || c.load(Ordering::Relaxed)));
                }
                let f = frozen_into(&module, function);
                let (pos, named) = args(&module);
                let named: Vec<(&str, Value)> =
                    named.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                let res = eval
                    .eval_function(f, &pos, &named)
                    .map_err(|e| EvalError::from_starlark(&e))?;
                module.set("result", res);
            }
            let frozen = freeze(module)?;
            frozen.get("result").map_err(|e| EvalError {
                kind: EvalErrorKind::Eval,
                message: e.to_string(),
                callstack: Vec::new(),
                include_stack: true,
            })
        })
    });
    std::panic::catch_unwind(run).unwrap_or_else(|p| Err(panic_error(p)))
}

/// Runs `f` on a new thread with an [`EVAL_STACK_SIZE`] stack and waits for it.
pub fn run_on_evaluation_thread<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("starlark-eval".to_owned())
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(s, f)
            .expect("spawn evaluation thread")
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}
