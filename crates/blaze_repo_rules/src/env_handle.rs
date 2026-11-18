//! Access to the Skyframe [`Environment`] from Starlark natives.
//!
//! Java: a repository rule or module extension implementation runs in a worker thread
//! (`WorkerSkyKeyComputeState`) whose `Environment` the `repository_ctx` / `module_ctx` object
//! holds and calls during Starlark evaluation (`rctx.path(Label)` requests REPOSITORY_DIRECTORY
//! and FILE values, `rctx.getenv` requests REPOSITORY_ENVIRONMENT_VARIABLE, ...).
//!
//! PORT: Blaze's evaluator blocks inside `Environment::get_value` instead of restarting, so the
//! implementation runs directly on the SkyFunction's thread (whose stack is the evaluator's
//! 256 MB `thread_stack_size`) and natives reach the environment through a [`SkyEnv`] stored in
//! the thread's payload. A `SkyEnv` is only valid inside [`SkyEnv::scope`]; it must not escape
//! the Starlark evaluation started in that scope.

use std::marker::PhantomData;

use blaze_skyframe::Environment;

/// A handle to the `Environment` of the SkyFunction running the current Starlark evaluation.
pub struct SkyEnv {
    ptr: *mut Environment<'static>,
    // Not Send/Sync: the evaluator records per-thread state (wait-for graph) for the environment.
    _not_send: PhantomData<*mut ()>,
}

impl SkyEnv {
    /// Runs `f` with a handle to `env`. Everything holding the handle must be dropped (or no
    /// longer used) when `f` returns.
    pub fn scope<R>(env: &mut Environment<'_>, f: impl FnOnce(SkyEnv) -> R) -> R {
        // SAFETY: only the lifetime is erased; the handle is used during `f`, while `env` is
        // mutably borrowed by this call and cannot be used by anything else.
        let ptr = (env as *mut Environment<'_>).cast::<Environment<'static>>();
        f(SkyEnv {
            ptr,
            _not_send: PhantomData,
        })
    }

    /// The environment. Callers must not hold the returned reference across another call that
    /// can reach the same handle (natives re-borrow it per use).
    #[allow(clippy::mut_from_ref)]
    pub fn get(&self) -> &mut Environment<'static> {
        // SAFETY: see `scope`; natives run one at a time on the evaluation thread and each use is
        // a short re-borrow.
        unsafe { &mut *self.ptr }
    }
}
