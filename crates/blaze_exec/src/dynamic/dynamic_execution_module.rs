//! Port of `lib/dynamic/DynamicExecutionModule.java`: the parts that make the
//! `DynamicSpawnStrategy` (its registration with the local and remote strategy lists is
//! `executor_setup::dynamic_execution_module_register_spawn_strategies`).

use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_protos::failure_details::{
    self, FailureDetail, execution_options::Code as ExecutionOptionsCode, failure_detail,
};
use indexmap::IndexMap;

use super::dynamic_spawn_strategy::{IgnoreFailureCheck, PostProcessingSpawn};
use crate::actions::{Spawn, spawns};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::execution_policy::ExecutionPolicy;
use crate::exec::spawn_strategy_registry::AbruptExitException;
use crate::util::FileOutErr;

/// Use the `Spawn` metadata to determine if it can be executed locally, remotely, or both.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:213 DynamicExecutionModule.getExecutionPolicy
pub fn get_execution_policy(spawn: &dyn Spawn) -> ExecutionPolicy {
    if !spawns::may_be_executed_remotely(spawn) {
        return ExecutionPolicy::LOCAL_EXECUTION_ONLY;
    }
    if !spawns::may_be_executed_locally(spawn) {
        return ExecutionPolicy::REMOTE_EXECUTION_ONLY;
    }
    ExecutionPolicy::ANYWHERE
}

/// `getPostProcessingSpawnForLocalExecution`: Bazel has none.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:232 DynamicExecutionModule.getPostProcessingSpawnForLocalExecution
pub struct NoPostProcessingSpawn;

impl PostProcessingSpawn for NoPostProcessingSpawn {
    fn get(&self, _spawn: &dyn Spawn) -> Option<Box<dyn Spawn>> {
        None
    }
}

/// `canIgnoreFailure` with the module's command state (`beforeCommand`: `ignoreLocalSignals`,
/// `verboseFailures`, `reporter`).
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:249 DynamicExecutionModule.canIgnoreFailure
pub struct ModuleIgnoreFailureCheck {
    /// `ignoreLocalSignals` (`--experimental_dynamic_ignore_local_signals`, empty when unset).
    pub ignore_local_signals: Vec<i32>,
    pub verbose_failures: bool,
    pub reporter: Arc<dyn EventHandler>,
}

impl IgnoreFailureCheck for ModuleIgnoreFailureCheck {
    fn can_ignore_failure(
        &self,
        spawn: &dyn Spawn,
        _context: &dyn ActionExecutionContext,
        exit_code: i32,
        _error_message: &str,
        _out_err: Option<&FileOutErr>,
        is_local: bool,
    ) -> bool {
        // By convention, when killed by a signal, a process gives exit code (128 + signal
        // number). More accurate information could be had through waitid(2), but Java does not
        // expose that. But accuracy is not critical here, at worst we are a bit slower in
        // getting either a success or a failure.
        let signal = exit_code.wrapping_sub(128);
        if is_local && self.ignore_local_signals.contains(&signal) {
            if self.verbose_failures {
                self.reporter.handle(&Event::info(format!(
                    "Local execution for {} stopped by signal {}, ignoring in favor of remote execution.",
                    spawn.get_resource_owner().pretty_print(),
                    signal
                )));
            }
            // PORT: skipped, logging ("Ignoring dynamic local branch killed by signal %d").
            return true;
        }
        false
    }
}

/// The local strategies of dynamic execution per mnemonic (`""` for the default).
/// `local_options` is the module's `localOptions` (null before `beforeCommand`).
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:105 DynamicExecutionModule.getLocalStrategies
pub fn get_local_strategies(
    options: &DynamicExecutionOptions,
    local_options: Option<&LocalExecutionOptions>,
    sandboxing_supported: bool,
) -> Result<IndexMap<String, Vec<String>>, AbruptExitException> {
    // Options that set "allowMultiple" to true ignore the default value, so we replicate that
    // functionality here.
    // ImmutableMap.Builder + buildKeepingLast: a repeated key keeps its first position and its
    // last value (measured on the oracle's Guava), as IndexMap::insert does.
    let mut local_and_worker_strategies: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut default_local_strategies = vec!["worker".to_string()];
    if sandboxing_supported {
        default_local_strategies.push("sandboxed".to_string());
    }
    if local_options.is_some_and(|o| o.local_lockfree_output) {
        // Without local lock free, having standalone execution risks very bad performance.
        default_local_strategies.push("standalone".to_string());
    }
    local_and_worker_strategies.insert(String::new(), default_local_strategies);

    for (mnemonic, strategies) in &options.dynamic_local_strategy {
        local_and_worker_strategies.insert(mnemonic.clone(), strategies.clone());
        throw_if_contains_dynamic(strategies, "--dynamic_local_strategy")?;
    }
    Ok(local_and_worker_strategies)
}

/// The remote strategies of dynamic execution per mnemonic.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:128 DynamicExecutionModule.getRemoteStrategies
pub fn get_remote_strategies(
    options: &DynamicExecutionOptions,
) -> Result<IndexMap<String, Vec<String>>, AbruptExitException> {
    // A HashMap, needed to dedup; ImmutableMap.copyOf keeps its iteration order (JDK String
    // hash order).
    let mut by_key: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut order = blaze_util::java_hash_map::JavaHashMap::new();
    for (mnemonic, values) in &options.dynamic_remote_strategy {
        throw_if_contains_dynamic(values, "--dynamic_remote_strategy")?;
        by_key.insert(mnemonic.clone(), values.clone());
        order.put(mnemonic.clone(), String::new());
    }
    if options.dynamic_remote_strategy.is_empty() {
        let mut m = IndexMap::new();
        m.insert(String::new(), vec![remote_strategy_name().to_string()]);
        return Ok(m);
    }
    Ok(order
        .into_entries()
        .into_iter()
        .map(|(k, _)| {
            let v = by_key[&k].clone();
            (k, v)
        })
        .collect())
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:141 DynamicExecutionModule.remoteStrategyName
fn remote_strategy_name() -> &'static str {
    "remote"
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:186 DynamicExecutionModule.throwIfContainsDynamic
fn throw_if_contains_dynamic(
    strategies: &[String],
    flag_name: &str,
) -> Result<(), AbruptExitException> {
    // ImmutableSet.of("dynamic", "dynamic_worker").toString()
    let identifiers = ["dynamic", "dynamic_worker"];
    if strategies.iter().any(|s| identifiers.contains(&s.as_str())) {
        let message = format!(
            "Cannot use strategy [dynamic, dynamic_worker] in flag {flag_name} as it would create a cycle during execution"
        );
        return Err(AbruptExitException::new(FailureDetail {
            message,
            category: Some(failure_detail::Category::ExecutionOptions(
                failure_details::ExecutionOptions {
                    code: ExecutionOptionsCode::InvalidCyclicDynamicStrategy as i32,
                },
            )),
        }));
    }
    Ok(())
}

/// `registerSpawnStrategies(registryBuilder, options, numCpus, jobs)` (the `@VisibleForTesting`
/// overload the command's registration calls): registers the `DynamicSpawnStrategy` as
/// `dynamic` and `dynamic_worker`, and the local and remote strategy lists.
/// `local_options` and `ignore_failure_check` are the module's state from `beforeCommand`.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:159 DynamicExecutionModule.registerSpawnStrategies
pub fn register_spawn_strategies(
    registry_builder: &mut crate::exec::spawn_strategy_registry::SpawnStrategyRegistryBuilder,
    options: &Arc<DynamicExecutionOptions>,
    local_options: Option<&LocalExecutionOptions>,
    num_cpus: i32,
    jobs: i32,
    ignore_failure_check: Arc<dyn IgnoreFailureCheck>,
) -> Result<(), AbruptExitException> {
    if !options.internal_spawn_scheduler {
        return Ok(());
    }
    let strategy = super::dynamic_spawn_strategy::DynamicSpawnStrategy::new(
        options.clone(),
        Arc::new(get_execution_policy),
        Arc::new(NoPostProcessingSpawn),
        num_cpus,
        jobs,
        Some(ignore_failure_check),
    );
    registry_builder.register_strategy(strategy, &["dynamic", "dynamic_worker"]);
    let sandboxing_supported = registry_builder.is_strategy_registered("sandboxed");
    registry_builder.add_dynamic_local_strategies(&get_local_strategies(
        options,
        local_options,
        sandboxing_supported,
    )?);
    registry_builder.add_dynamic_remote_strategies(&get_remote_strategies(options)?);
    Ok(())
}
