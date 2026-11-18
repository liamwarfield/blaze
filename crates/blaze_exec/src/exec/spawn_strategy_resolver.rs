//! Port of `lib/exec/SpawnStrategyResolver.java`.

use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, spawn::Code};

use crate::actions::spawn_strategy::{SpawnStrategy, strategies_to_string};
use crate::actions::{ExecException, Spawn, SpawnResult, UserExecException, spawns};
use crate::exec::action_execution_context::ActionExecutionContext;

/// Resolver that looks up the right strategy for a spawn during `exec` (via a
/// `SpawnStrategyRegistry`) and uses it to execute the spawn.
// Java: com/google/devtools/build/lib/exec/SpawnStrategyResolver.java:36 SpawnStrategyResolver
#[derive(Debug, Default, Clone, Copy)]
pub struct SpawnStrategyResolver;

impl SpawnStrategyResolver {
    /// `new SpawnStrategyResolver()`.
    pub fn new() -> SpawnStrategyResolver {
        SpawnStrategyResolver
    }

    /// Executes the given spawn with the highest priority strategy that can be found for it.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyResolver.java:44 SpawnStrategyResolver.exec
    pub fn exec(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.resolve_one(spawn, action_execution_context)?
            .exec(spawn, action_execution_context)
    }

    // Java: com/google/devtools/build/lib/exec/SpawnStrategyResolver.java:49 SpawnStrategyResolver.resolveOne
    fn resolve_one(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Arc<dyn SpawnStrategy>, ExecException> {
        let strategies = self.resolve(spawn, action_execution_context)?;

        // Because the strategies are ordered by preference, we can execute the spawn with the best
        // possible one by simply filtering out the ones that can't execute it and then picking the
        // first one from the remaining strategies in the list.
        Ok(strategies[0].clone())
    }

    /// Returns the list of strategies that should be used to execute the given spawn.
    // Java: com/google/devtools/build/lib/exec/SpawnStrategyResolver.java:65 SpawnStrategyResolver.resolve
    pub fn resolve(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<Arc<dyn SpawnStrategy>>, ExecException> {
        let strategies = action_execution_context
            .get_spawn_strategy_registry()
            .expect("NullPointerException: getContext(SpawnStrategyRegistry.class)")
            .get_strategies(
                spawn,
                Some(action_execution_context.get_event_handler() as &dyn EventHandler),
            );

        let execable_strategies: Vec<Arc<dyn SpawnStrategy>> = strategies
            .iter()
            .filter(|s| s.can_exec(spawn, action_execution_context))
            .cloned()
            .collect();

        if execable_strategies.is_empty() {
            let message = format!(
                "{} spawn{} cannot be executed with any of the available strategies: {}. Your --spawn_strategy, --genrule_strategy, --strategy and/or --allowed_strategies_by_exec_platform flags are probably too strict. Visit https://github.com/bazelbuild/bazel/issues/7480 for advice.\n",
                spawn.get_mnemonic(),
                if spawns::uses_path_mapping(spawn) {
                    ", which requires sandboxing due to path mapping,"
                } else {
                    ""
                },
                strategies_to_string(&strategies)
            );
            return Err(UserExecException::new(FailureDetail {
                message,
                category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                    code: Code::NoUsableStrategyFound as i32,
                    ..Default::default()
                })),
            })
            .into());
        }

        Ok(execable_strategies)
    }
}
