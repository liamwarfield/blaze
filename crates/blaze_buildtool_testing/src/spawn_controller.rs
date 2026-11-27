//! Port of `SpawnController` and `ControllableActionStrategyModule`
//! (`src/test/java/com/google/devtools/build/lib/testutil/`): spawn strategies whose `exec` a
//! test can override per spawn description, registered over Bazel's strategies of the given
//! identifiers.

use std::fmt;
use std::sync::Arc;

use blaze_exec::actions::spawn_strategy::{
    SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns,
};
use blaze_exec::actions::{ExecException, Spawn, SpawnResult};
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::spawn_strategy_registry::{
    AbruptExitException, SpawnStrategyRegistryBuilder,
};
use blaze_runtime::module::BlazeModule;
use indexmap::IndexMap;
use parking_lot::Mutex;

/// The desired behavior of a `SpawnStrategy.exec` call: a [`SpawnResult`] ([`ExecResult::of`]),
/// an [`ExecException`] ([`ExecResult::of_exception`]), or the underlying strategy
/// ([`ExecResult::delegate`]).
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:61 SpawnController.ExecResult
pub enum ExecResult {
    /// Override the action by returning the provided [`SpawnResult`].
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:74 SpawnController.ExecResult.of
    Of(SpawnResult),
    /// Override the action by throwing the provided [`ExecException`].
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:79 SpawnController.ExecResult.ofException
    OfException(ExecException),
    /// Do not override the action. Allow the underlying strategy to handle it.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:84 SpawnController.ExecResult.delegate
    Delegate,
}

impl ExecResult {
    pub fn of(spawn_result: SpawnResult) -> ExecResult {
        ExecResult::Of(spawn_result)
    }

    pub fn of_exception(exec_exception: ExecException) -> ExecResult {
        ExecResult::OfException(exec_exception)
    }

    pub fn delegate() -> ExecResult {
        ExecResult::Delegate
    }
}

/// The means of controlling `SpawnStrategy.exec` calls.
///
/// PORT: Java's shim may throw `IOException` (turned into a `SpawnShimException`); a Rust shim
/// returns `Err(message)` for it.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:46 SpawnController.SpawnShim
pub type SpawnShim =
    Box<dyn FnOnce(&dyn Spawn, &dyn ActionExecutionContext) -> Result<ExecResult, String> + Send>;

/// Injects custom spawn behavior and records the descriptions of the spawns it sees.
// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:43 SpawnController
#[derive(Default)]
pub struct SpawnController {
    executed_spawn_descriptions: Mutex<Vec<String>>,
    /// `LinkedListMultimap<String, SpawnShim>`: per description, in insertion order.
    spawn_shims: Mutex<IndexMap<String, Vec<SpawnShim>>>,
}

impl SpawnController {
    pub fn new() -> Arc<SpawnController> {
        Arc::new(SpawnController::default())
    }

    /// All executed spawn descriptions seen by strategies created via [`SpawnController::wrap`]
    /// (in order) since the last call to [`SpawnController::clear_executed_spawn_descriptions`].
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:99 SpawnController.getExecutedSpawnDescriptions
    pub fn get_executed_spawn_descriptions(&self) -> Vec<String> {
        self.executed_spawn_descriptions.lock().clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:104 SpawnController.clearExecutedSpawnDescriptions
    pub fn clear_executed_spawn_descriptions(&self) {
        self.executed_spawn_descriptions.lock().clear();
    }

    /// Enqueues `spawn_shim` for a single execution of a spawn with the given description. When a
    /// matching spawn is seen, an associated shim is dequeued and used in a FIFO manner. If there
    /// are no matching shims enqueued, the delegate strategy is used.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:115 SpawnController.addSpawnShim
    pub fn add_spawn_shim(&self, spawn_description: &str, spawn_shim: SpawnShim) {
        self.spawn_shims
            .lock()
            .entry(spawn_description.to_owned())
            .or_default()
            .push(spawn_shim);
    }

    /// A new strategy that picks up custom behavior added via
    /// [`SpawnController::add_spawn_shim`] and delegates to `delegate` if necessary.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:123 SpawnController.wrap
    pub fn wrap(self: &Arc<Self>, delegate: Arc<dyn SpawnStrategy>) -> Arc<dyn SpawnStrategy> {
        Arc::new(ControllableSpawnStrategy {
            controller: self.clone(),
            delegate,
        })
    }

    /// Checks that all spawn shims added via [`SpawnController::add_spawn_shim`] have been
    /// consumed, panicking (Java's `IllegalStateException`) if any remain.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:133 SpawnController.verifyAllShimsConsumed
    pub fn verify_all_shims_consumed(&self) {
        let shims = self.spawn_shims.lock();
        let remaining: Vec<&String> = shims
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, _)| k)
            .collect();
        assert!(remaining.is_empty(), "Remaining spawn shims: {remaining:?}");
    }

    /// The next shim for `description` (`events.remove(0)`), if any.
    fn take_shim(&self, description: &str) -> Option<SpawnShim> {
        let mut shims = self.spawn_shims.lock();
        let events = shims.get_mut(description)?;
        if events.is_empty() {
            return None;
        }
        let shim = events.remove(0);
        if events.is_empty() {
            shims.shift_remove(description);
        }
        Some(shim)
    }
}

// Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:137 SpawnController.ControllableSpawnStrategy
struct ControllableSpawnStrategy {
    controller: Arc<SpawnController>,
    delegate: Arc<dyn SpawnStrategy>,
}

impl fmt::Display for ControllableSpawnStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.delegate, f)
    }
}

impl SpawnStrategy for ControllableSpawnStrategy {
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:154 SpawnController.ControllableSpawnStrategy.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        let description = spawn.get_resource_owner().describe();
        self.controller
            .executed_spawn_descriptions
            .lock()
            .push(description.clone());

        if let Some(shim) = self.controller.take_shim(&description) {
            match shim(spawn, action_execution_context) {
                Err(io_message) => {
                    // Java: a SpawnShimException with a crash failure detail; only a broken test
                    // reaches it.
                    panic!(
                        "In a test shim, failed to determine ExecException for action: \
                         {description}: {io_message}"
                    );
                }
                Ok(ExecResult::OfException(e)) => return Err(e),
                Ok(ExecResult::Of(result)) => return Ok(vec![result]),
                Ok(ExecResult::Delegate) => {}
            }
        }

        self.delegate.exec(spawn, action_execution_context)
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:180 SpawnController.ControllableSpawnStrategy.canExec
    fn can_exec(
        &self,
        spawn: &dyn Spawn,
        action_context_registry: &dyn ActionContextRegistry,
    ) -> bool {
        self.delegate.can_exec(spawn, action_context_registry)
    }

    fn get_simple_class_name(&self) -> String {
        "ControllableSpawnStrategy".to_owned()
    }

    // Java: `implements SandboxedSpawnStrategy`.
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        Some(self)
    }
}

impl SandboxedSpawnStrategy for ControllableSpawnStrategy {
    // Java: src/test/java/com/google/devtools/build/lib/testutil/SpawnController.java:145 SpawnController.ControllableSpawnStrategy.exec
    fn exec_with_stop_concurrent_spawns(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
        _stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.exec(spawn, action_execution_context)
    }
}

/// A module that registers, for each identifier, a [`SpawnController::wrap`] of the strategy
/// Bazel's modules registered under it.
// Java: src/test/java/com/google/devtools/build/lib/testutil/ControllableActionStrategyModule.java:36 ControllableActionStrategyModule
pub struct ControllableActionStrategyModule {
    spawn_controller: Arc<SpawnController>,
    identifiers: Vec<String>,
}

impl ControllableActionStrategyModule {
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ControllableActionStrategyModule.java:41 ControllableActionStrategyModule.<init>
    pub fn new(
        spawn_controller: Arc<SpawnController>,
        identifiers: &[&str],
    ) -> ControllableActionStrategyModule {
        assert!(!identifiers.is_empty(), "No identifiers given");
        ControllableActionStrategyModule {
            spawn_controller,
            identifiers: identifiers.iter().map(|i| i.to_string()).collect(),
        }
    }
}

impl BlazeModule for ControllableActionStrategyModule {
    fn name(&self) -> &'static str {
        "ControllableActionStrategyModule"
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ControllableActionStrategyModule.java:48 ControllableActionStrategyModule.registerSpawnStrategies
    fn register_spawn_strategies(
        &self,
        registry_builder: &mut SpawnStrategyRegistryBuilder,
        _env: &ExecCommandEnvironment,
    ) -> Result<(), AbruptExitException> {
        for identifier in &self.identifiers {
            let delegate =
                registry_builder.to_strategy(identifier, "ControllableActionStrategyModule")?;
            registry_builder
                .register_strategy(self.spawn_controller.wrap(delegate), &[identifier.as_str()]);
        }
        Ok(())
    }
}
