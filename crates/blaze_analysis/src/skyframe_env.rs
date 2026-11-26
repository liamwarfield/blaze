//! Java `AnalysisEnvironment.getSkyframeEnv()` for the native rule factories that request
//! Skyframe values while they run (`GenQuery`).
//!
//! PORT: Java's `CachingAnalysisEnvironment` holds the `SkyFunction.Environment` of the
//! `ConfiguredTargetFunction` that creates it. Blaze's `CachingAnalysisEnvironment` is shared
//! (`Arc`) and cannot hold the function's `&mut Environment`, so `ConfiguredTargetFunction`
//! lends it to the factory call through this scoped, thread-local slot: the factory runs
//! synchronously on the function's thread while the function's `&mut Environment` is borrowed
//! by [`with_skyframe_env`], so the environment is reachable from exactly one place at a time.

use std::cell::Cell;
use std::ptr::NonNull;

use blaze_skyframe::Environment;

thread_local! {
    static CURRENT: Cell<Option<NonNull<Environment<'static>>>> = const { Cell::new(None) };
}

/// Restores the slot's previous content when dropped (also on unwinding).
struct Restore(Option<NonNull<Environment<'static>>>);

impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0));
    }
}

/// Runs `f` with `env` as the analysis environment's Skyframe environment. Calls nest: a
/// dependency request inside `f` may compute another node on this thread, which lends its own
/// environment and gives it back when it returns.
pub fn with_skyframe_env<R>(env: &mut Environment<'_>, f: impl FnOnce() -> R) -> R {
    let pointer = NonNull::from(env).cast::<Environment<'static>>();
    let _restore = Restore(CURRENT.with(|c| c.replace(Some(pointer))));
    f()
}

/// Calls `f` with the Skyframe environment lent by the innermost [`with_skyframe_env`] (Java
/// `ruleContext.getAnalysisEnvironment().getSkyframeEnv()`).
///
/// Panics when no environment is lent (a factory called outside `ConfiguredTargetFunction`), or
/// when called again inside `f` (the environment is taken out of the slot while `f` runs).
// Java: com/google/devtools/build/lib/analysis/CachingAnalysisEnvironment.java:334 CachingAnalysisEnvironment.getSkyframeEnv
pub fn get_skyframe_env<R>(f: impl for<'e> FnOnce(&mut Environment<'e>) -> R) -> R {
    let pointer = CURRENT
        .with(|c| c.take())
        .expect("no Skyframe environment: the factory must run inside ConfiguredTargetFunction");
    let _restore = Restore(Some(pointer));
    // SAFETY: `pointer` was made from the `&mut Environment` that the innermost
    // `with_skyframe_env` on this thread borrows for the whole call that is still running (the
    // slot is restored before that borrow ends). It was taken out of the slot above, so no other
    // reference to it is created until `f` returns; `f` is generic over the environment's
    // lifetime, so it cannot keep the reference.
    let env: &mut Environment<'static> = unsafe { &mut *pointer.as_ptr() };
    f(env)
}
