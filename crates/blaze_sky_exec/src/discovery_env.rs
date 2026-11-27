//! `ActionExecutionContext.getEnvironmentForDiscoveringInputs()`: the Skyframe environment of the
//! `ActionExecutionFunction` node that discovers an action's inputs.
//!
//! PORT: Java's `ActionExecutionContext.forInputDiscovery` holds the `SkyFunction.Environment`.
//! The execution context is shared (`&dyn ActionExecutionContext`) and cannot hold the function's
//! `&mut Environment`, so `ActionExecutionFunction` lends it for the call of
//! `SkyframeActionExecutor::discover_inputs` through this scoped, thread-local slot (the pattern
//! of `blaze_analysis::skyframe_env`): discovery runs synchronously on the function's thread
//! while the function's `&mut Environment` is borrowed by [`with_discovery_env`].

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::NestedSet;
use blaze_exec::actions::{ExecException, TreeArtifactValue};
use blaze_exec::exec::action_execution_context::DiscoveryEnvironment;
use blaze_skyframe::{Environment, SkyKey};

use crate::action_execution_value::ActionExecutionValue;
use crate::artifact_function::ArtifactValue;
use crate::keys::{ActionExecutionKey, artifact_key};

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

/// Runs `f` with `env` as the environment for discovering inputs.
pub fn with_discovery_env<R>(env: &mut Environment<'_>, f: impl FnOnce() -> R) -> R {
    let pointer = NonNull::from(env).cast::<Environment<'static>>();
    let _restore = Restore(CURRENT.with(|c| c.replace(Some(pointer))));
    f()
}

/// Calls `f` with the environment lent by the innermost [`with_discovery_env`] on this thread.
///
/// Panics when none is lent (Java: the context of an execution, whose environment is null), or
/// when called again inside `f`.
pub fn get_discovery_env<R>(f: impl for<'e> FnOnce(&mut Environment<'e>) -> R) -> R {
    let pointer = CURRENT.with(|c| c.take()).unwrap_or_else(|| {
        panic!("NullPointerException: getEnvironmentForDiscoveringInputs() outside input discovery")
    });
    let _restore = Restore(Some(pointer));
    // SAFETY: `pointer` was made from the `&mut Environment` that the innermost
    // `with_discovery_env` on this thread borrows for the whole call that is still running (the
    // slot is restored before that borrow ends). It was taken out of the slot above, so no other
    // reference to it is created until `f` returns; `f` is generic over the environment's
    // lifetime, so it cannot keep the reference.
    let env: &mut Environment<'static> = unsafe { &mut *pointer.as_ptr() };
    f(env)
}

/// The reads of input discovery over a Skyframe environment.
pub struct SkyframeDiscoveryEnvironment<'a, 'e> {
    pub env: &'a mut Environment<'e>,
}

impl DiscoveryEnvironment for SkyframeDiscoveryEnvironment<'_, '_> {
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:240 IncludeScanningHeaderDataHelper.handleTreeArtifacts
    fn get_tree_artifact_values(
        &mut self,
        trees: &[Artifact],
    ) -> Result<Option<Vec<Arc<TreeArtifactValue>>>, ExecException> {
        let keys: Vec<SkyKey> = trees.iter().map(artifact_key).collect();
        // Java: InterruptedException out of getValuesAndExceptions (review round 3, IS-3).
        let result = self
            .env
            .get_values_and_exceptions(&keys)
            .map_err(|_| ExecException::Interrupted)?;
        if self.env.values_missing() {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(keys.len());
        for (tree, key) in trees.iter().zip(&keys) {
            let Some(value) = result.get(key) else {
                return Ok(None);
            };
            match value.downcast_ref::<ArtifactValue>() {
                Some(ArtifactValue::Tree(tree_value)) => values.push(tree_value.clone()),
                _ => panic!("IllegalStateException: SkyValue of {tree} is not TreeArtifactValue"),
            }
        }
        Ok(Some(values))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:2074 CppCompileAction.computeTransitivelyUsedModules
    fn get_discovered_modules(
        &mut self,
        modules: &[Artifact],
    ) -> Result<Option<Vec<NestedSet<Artifact>>>, ExecException> {
        let keys: Vec<SkyKey> = modules
            .iter()
            .map(|m| SkyKey::new(ActionExecutionKey(m.get_generating_action_key().clone())))
            .collect();
        // Java: InterruptedException out of getValuesAndExceptions (review round 3, IS-3).
        let result = self
            .env
            .get_values_and_exceptions(&keys)
            .map_err(|_| ExecException::Interrupted)?;
        if self.env.values_missing() {
            return Ok(None);
        }
        let mut discovered = Vec::with_capacity(keys.len());
        for (module, key) in modules.iter().zip(&keys) {
            let Some(value) = result.get(key) else {
                return Ok(None);
            };
            let value = value
                .downcast_ref::<ActionExecutionValue>()
                .unwrap_or_else(|| panic!("NullPointerException: {module}"));
            discovered.push(value.get_discovered_modules());
        }
        Ok(Some(discovered))
    }
}
