//! Port of `lib/actions/SpawnStrategy.java` and `lib/actions/SandboxedSpawnStrategy.java`.

use std::fmt;
use std::sync::Arc;

use crate::actions::{ExecException, Spawn, SpawnResult};
use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use crate::util::FileOutErr;

/// An implementation of how to execute a [`Spawn`] instance.
///
/// Strategies are used during the execution phase based on how they were registered by a module
/// (`registerSpawnStrategies`) and whether they match a given spawn based on their abilities
/// ([`SpawnStrategy::can_exec`]).
///
/// `Display` is Java's `toString()` (the strategy's name in messages such as
/// `"... cannot be executed with any of the available strategies: [linux-sandbox, local]"`).
// Java: com/google/devtools/build/lib/actions/SpawnStrategy.java:28 SpawnStrategy
pub trait SpawnStrategy: Send + Sync + fmt::Display {
    /// Executes the given spawn and returns metadata about the execution. Implementations must
    /// guarantee that the first list entry represents the successful execution of the given spawn
    /// (if no execution was successful, the method must return an error instead). The list may
    /// contain further entries for (unsuccessful) retries as well as tree artifact management
    /// (which may require additional spawn executions).
    // Java: com/google/devtools/build/lib/actions/SpawnStrategy.java:37 SpawnStrategy.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException>;

    /// Returns whether this SpawnActionContext supports executing the given Spawn.
    // Java: com/google/devtools/build/lib/actions/SpawnStrategy.java:41 SpawnStrategy.canExec
    fn can_exec(
        &self,
        spawn: &dyn Spawn,
        action_context_registry: &dyn ActionContextRegistry,
    ) -> bool;

    /// Performs any actions conditional on this strategy not only being registered but triggered
    /// as used because its identifier was requested and it was not overridden. (Java passes a
    /// nullable registry.)
    // Java: com/google/devtools/build/lib/actions/SpawnStrategy.java:51 SpawnStrategy.usedContext
    fn used_context(&self, _action_context_registry: Option<&dyn ActionContextRegistry>) {}

    /// `getClass().getSimpleName()`: used by `SpawnStrategyRegistry`'s error messages.
    fn get_simple_class_name(&self) -> String;

    /// `this instanceof SandboxedSpawnStrategy`.
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        None
    }

    /// `this instanceof AbstractSpawnStrategy` (every Bazel strategy except the dynamic one).
    fn as_abstract_spawn_strategy(&self) -> Option<&AbstractSpawnStrategy> {
        None
    }
}

/// Lambda interface to stop other instances of the same spawn before writing outputs.
// Java: com/google/devtools/build/lib/actions/SandboxedSpawnStrategy.java:31 SandboxedSpawnStrategy.StopConcurrentSpawns
pub trait StopConcurrentSpawns: Send + Sync {
    /// Stops other instances of the same spawn before writing outputs. If `exit_code` != 0, this
    /// may stop this instance instead by returning `Err(ExecException::Interrupted)`, to allow the
    /// other instance to succeed instead.
    fn stop(
        &self,
        exit_code: i32,
        error_message: &str,
        out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException>;
}

/// A context that allows execution of [`Spawn`] instances similar to [`SpawnStrategy`], but with
/// the additional restriction that during execution the spawn must not be allowed to modify the
/// current execution root of the build. Instead, the spawn should be executed in a sandbox or on a
/// remote system and its output files only be moved to the execution root.
// Java: com/google/devtools/build/lib/actions/SandboxedSpawnStrategy.java:27 SandboxedSpawnStrategy
pub trait SandboxedSpawnStrategy: SpawnStrategy {
    /// Executes the given spawn. When the strategy is about to write output files into the
    /// execroot, it first asks any other concurrent instances of this same spawn (handled by other
    /// spawn runners when dynamic scheduling is enabled) to stop by invoking
    /// `stop_concurrent_spawns`.
    // Java: com/google/devtools/build/lib/actions/SandboxedSpawnStrategy.java:62 SandboxedSpawnStrategy.exec
    fn exec_with_stop_concurrent_spawns(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
        stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException>;
}

/// Java's `List<SpawnStrategy>.toString()`: `"[a, b]"`.
pub fn strategies_to_string(strategies: &[Arc<dyn SpawnStrategy>]) -> String {
    let parts: Vec<String> = strategies.iter().map(|s| s.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Java object identity (`==`, and `equals` for strategies, which never override it).
pub fn same_strategy(a: &Arc<dyn SpawnStrategy>, b: &Arc<dyn SpawnStrategy>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}
