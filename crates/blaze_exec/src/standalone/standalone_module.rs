//! Port of `lib/standalone/StandaloneModule.java` (the spawn-strategy part).

use std::sync::Arc;

use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::local::local_env_provider;
use crate::exec::local::local_spawn_runner::LocalSpawnRunner;
use crate::exec::runfiles_tree_updater::RunfilesTreeUpdater;
use crate::exec::spawn_strategy_registry::SpawnStrategyRegistryBuilder;
use crate::runtime::ProcessWrapper;
use crate::standalone::standalone_spawn_strategy::StandaloneSpawnStrategy;

/// StandaloneModule provides pluggable functionality for blaze.
// Java: com/google/devtools/build/lib/standalone/StandaloneModule.java:47 StandaloneModule
#[derive(Debug, Default, Clone, Copy)]
pub struct StandaloneModule;

impl StandaloneModule {
    /// `new StandaloneModule()`.
    pub fn new() -> StandaloneModule {
        StandaloneModule
    }

    // PORT: registerActionContexts (C++ include scanning dummies, test strategies, file-write and
    // template-expansion contexts) belongs to the action-context registry, which is not ported
    // here.

    /// `registerSpawnStrategies(registryBuilder, env)`.
    // Java: com/google/devtools/build/lib/standalone/StandaloneModule.java:80 StandaloneModule.registerSpawnStrategies
    pub fn register_spawn_strategies(
        &self,
        registry_builder: &mut SpawnStrategyRegistryBuilder,
        env: &ExecCommandEnvironment,
    ) {
        // ProcessWrapper.fromCommandEnvironment(env)
        // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:72 ProcessWrapper.fromCommandEnvironment
        let process_wrapper = ProcessWrapper::from_bin_tools(
            &env.bin_tools,
            Some(
                env.local_execution_options
                    .get_local_sigkill_grace_seconds(),
            ),
            env.local_execution_options.process_wrapper_graceful_sigterm,
        );
        let local_spawn_runner = Arc::new(LocalSpawnRunner::new(
            env.exec_root.clone(),
            env.local_execution_options.clone(),
            env.resource_manager.clone(),
            Arc::new(local_env_provider::for_current_os(&env.client_env)),
            env.bin_tools.clone(),
            process_wrapper,
            Arc::new(RunfilesTreeUpdater::for_command_environment(env)),
        ));

        let execution_options = env.execution_options.clone();
        // Order of strategies passed to builder is significant - when there are many strategies
        // that could potentially be used and a spawnActionContext doesn't specify which one it
        // wants, the last one from strategies list will be used
        registry_builder.register_strategy(
            Arc::new(StandaloneSpawnStrategy::new(
                env.exec_root.clone(),
                local_spawn_runner,
                execution_options,
            )),
            &["standalone", "local"],
        );

        // This makes the "standalone" strategy the default Spawn strategy, unless it is overridden
        // by a later BlazeModule.
        registry_builder.set_default_strategies(&["standalone".to_string()]);
    }
}
