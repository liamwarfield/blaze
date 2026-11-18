//! Building the command's `SpawnStrategyRegistry`: the spawn-strategy part of
//! `buildtool/ExecutionTool.<init>`, calling each Bazel module's `registerSpawnStrategies` in
//! `Bazel.BAZEL_MODULES` order:
//!
//! 1. `RemoteModule` ("remote" and the remote-local fallback identifier; the remote module's
//!    contributions come from the caller, [`RemoteModuleContributions`]),
//! 2. `WorkerModule` ("worker"),
//! 3. `StandaloneModule` ("standalone", "local"; default "standalone"),
//! 4. `SandboxModule` ("sandboxed", "linux-sandbox", "processwrapper-sandbox", ...),
//! 5. `DynamicExecutionModule` ("dynamic", "dynamic_worker" and the dynamic local/remote lists),
//! 6. `BazelStrategyModule` (`--spawn_strategy`, `--genrule_strategy`, `--strategy`,
//!    `--strategy_regexp`, `--allowed_strategies_by_exec_platform`).
//!
//! The other modules in the list do not override `registerSpawnStrategies`.

use std::fmt;
use std::sync::Arc;

use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{
    self, FailureDetail, execution_options::Code as ExecutionOptionsCode, failure_detail,
    sandbox::Code as SandboxCode,
};
use indexmap::IndexMap;

use crate::actions::spawn_strategy::SpawnStrategy;
use crate::actions::{ExecException, Spawn, SpawnResult, spawns};
use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, ActionInputPrefetcher, RemoteOutputTree,
    SpawnCache,
};
use crate::exec::exec_command_environment::ExecCommandEnvironment;
use crate::exec::spawn_runner::{SpawnExecutionContext, SpawnRunner};
use crate::exec::spawn_strategy_registry::{
    AbruptExitException, SpawnStrategyRegistry, SpawnStrategyRegistryBuilder,
};
use crate::standalone::standalone_module::StandaloneModule;

/// What `RemoteModule` contributes to the executor of a build, built by the caller from
/// `blaze_remote` (this crate cannot depend on it): the parts of `RemoteModule
/// .registerSpawnStrategies`, `registerActionContexts` and `executorInit`.
///
/// The default (no contribution) is the placeholder `RemoteActionContextProvider` of a command
/// without disk cache, remote cache or remote executor, and `NoSpawnCacheModule`'s
/// `SpawnCache.NO_CACHE`.
#[derive(Clone, Default)]
pub struct RemoteModuleContributions {
    /// `RemoteActionContextProvider.registerRemoteSpawnStrategy`: the `RemoteSpawnStrategy`
    /// registered as "remote" (`None`: the placeholder provider's).
    pub remote_spawn_strategy: Option<Arc<dyn SpawnStrategy>>,
    /// `RemoteActionContextProvider.registerSpawnCache`: the `RemoteSpawnCache` registered as
    /// `SpawnCache` ("remote-cache"), which replaces `NoSpawnCacheModule`'s `NO_CACHE`.
    pub spawn_cache: Option<Arc<dyn SpawnCache>>,
    /// `RemoteModule.executorInit`: `builder.setActionInputPrefetcher(actionInputFetcher)`.
    pub action_input_prefetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    /// The remote output tree of the build (`RemoteOutputService`'s action file systems), which
    /// the output metadata stores consult.
    pub remote_output_tree: Option<Arc<RemoteOutputTree>>,
    /// `RemoteModule.executorInit`: `builder.setActionExecutionSalt(computeActionExecutionSalt(
    /// remoteOptions))`; `None` keeps the executor's salt.
    pub action_execution_salt: Option<String>,
}

impl RemoteModuleContributions {
    /// `RemoteModule.registerActionContexts` and the action-context part of `executorInit`: the
    /// spawn cache replaces `SpawnCache.NO_CACHE` (the last registration of a context type
    /// wins), the prefetcher and the remote output tree are registered as contexts.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1190 RemoteModule.registerActionContexts
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
    pub fn register_action_contexts(
        &self,
        spawn_cache: &mut Option<Arc<dyn SpawnCache>>,
        register: &mut dyn FnMut(
            std::any::TypeId,
            crate::actions::action_execution::ActionContextRef,
        ),
    ) {
        if let Some(cache) = &self.spawn_cache {
            *spawn_cache = Some(cache.clone());
        }
        if let Some(prefetcher) = &self.action_input_prefetcher {
            register(
                std::any::TypeId::of::<Arc<dyn ActionInputPrefetcher>>(),
                Arc::new(prefetcher.clone()),
            );
        }
        if let Some(tree) = &self.remote_output_tree {
            register(std::any::TypeId::of::<RemoteOutputTree>(), tree.clone());
        }
    }
}

/// Builds the `SpawnStrategyRegistry` of a build command, as `ExecutionTool`'s constructor does:
/// every module's `registerSpawnStrategies` in module order, then `Builder.build()`.
///
/// Requires on [`ExecCommandEnvironment`] (seam): `remote_options`, `dynamic_execution_options`
/// and `worker_options`. A command whose remote options enable a cache or remote execution
/// needs [`build_spawn_strategy_registry_with_remote`].
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
pub fn build_spawn_strategy_registry(
    env: &ExecCommandEnvironment,
    sandbox_module: &mut crate::sandbox::sandbox_module::SandboxModule,
) -> Result<SpawnStrategyRegistry, AbruptExitException> {
    build_spawn_strategy_registry_with_remote(
        env,
        sandbox_module,
        &RemoteModuleContributions::default(),
    )
}

/// [`build_spawn_strategy_registry`] with the remote module's contributions.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
pub fn build_spawn_strategy_registry_with_remote(
    env: &ExecCommandEnvironment,
    sandbox_module: &mut crate::sandbox::sandbox_module::SandboxModule,
    remote: &RemoteModuleContributions,
) -> Result<SpawnStrategyRegistry, AbruptExitException> {
    // PORT: Java uses SpawnStrategyRegistry.builder(env.getInvocationPolicy().getStrategyPolicy());
    // Blaze has no invocation policy, whose default strategy policy allows every strategy, which
    // is what builder() does.
    let mut spawn_strategy_registry_builder = SpawnStrategyRegistry::builder();

    // com.google.devtools.build.lib.remote.RemoteModule
    remote_module_register_spawn_strategies(&mut spawn_strategy_registry_builder, env, remote);
    // com.google.devtools.build.lib.worker.WorkerModule
    worker_module_register_spawn_strategies(&mut spawn_strategy_registry_builder, env);
    // com.google.devtools.build.lib.standalone.StandaloneModule
    StandaloneModule::new().register_spawn_strategies(&mut spawn_strategy_registry_builder, env);
    // com.google.devtools.build.lib.sandbox.SandboxModule
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:145 SandboxModule.registerSpawnStrategies
    if let Err(e) =
        sandbox_module.register_spawn_strategies(&mut spawn_strategy_registry_builder, env)
    {
        return Err(AbruptExitException::new(FailureDetail {
            message: format!("Failed to initialize sandbox: {e}"),
            category: Some(failure_detail::Category::Sandbox(
                failure_details::Sandbox {
                    code: SandboxCode::InitializationFailure as i32,
                },
            )),
        }));
    }
    // com.google.devtools.build.lib.dynamic.DynamicExecutionModule
    dynamic_execution_module_register_spawn_strategies(&mut spawn_strategy_registry_builder, env)?;
    // com.google.devtools.build.lib.bazel.rules.BazelStrategyModule
    bazel_strategy_module_register_spawn_strategies(&mut spawn_strategy_registry_builder, env);

    spawn_strategy_registry_builder.build()
}

fn is_null_or_empty(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(str::is_empty)
}

/// `RemoteModule.shouldEnableRemoteExecution(options)`.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:235 RemoteModule.shouldEnableRemoteExecution
fn should_enable_remote_execution(options: &RemoteOptions) -> bool {
    !is_null_or_empty(&options.remote_executor)
}

/// `RemoteModule.registerSpawnStrategies` with the `RemoteActionContextProvider` that
/// `RemoteModule.setup` creates for the command.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1176 RemoteModule.registerSpawnStrategies
fn remote_module_register_spawn_strategies(
    registry_builder: &mut SpawnStrategyRegistryBuilder,
    env: &ExecCommandEnvironment,
    remote: &RemoteModuleContributions,
) {
    let remote_options = &env.remote_options;
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:526 RemoteModule.setup
    // Without disk, HTTP or gRPC caching, remote execution, a remote output service or a remote
    // downloader, setup() installs the placeholder provider (no cache, no executor).
    let any_remote = !is_null_or_empty(&remote_options.remote_executor)
        || !is_null_or_empty(&remote_options.remote_cache)
        || remote_options
            .disk_cache
            .as_ref()
            .is_some_and(|p| !p.get().is_empty())
        || !is_null_or_empty(&remote_options.remote_output_service)
        || !is_null_or_empty(&remote_options.remote_downloader);
    if any_remote && remote.remote_spawn_strategy.is_none() {
        blaze_util::unported!(
            "RemoteModule.setup (remote caching, execution or downloader without the remote module's contributions)"
        );
    }

    registry_builder.set_remote_local_fallback_strategy_identifier(
        &remote_options.remote_local_fallback_strategy,
    );
    if let Some(strategy) = &remote.remote_spawn_strategy {
        // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:201 RemoteActionContextProvider.registerRemoteSpawnStrategy
        registry_builder.register_strategy(strategy.clone(), &["remote"]);
        return;
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:201 RemoteActionContextProvider.registerRemoteSpawnStrategy
    // Java: com/google/devtools/build/lib/remote/RemoteSpawnStrategy.java:30 RemoteSpawnStrategy.toString
    registry_builder.register_strategy(
        Arc::new(
            AbstractSpawnStrategy::new(
                env.exec_root.clone(),
                Arc::new(PlaceholderRemoteSpawnRunner),
                env.execution_options.clone(),
                "remote",
            )
            .with_simple_class_name("RemoteSpawnStrategy"),
        ),
        &["remote"],
    );
}

/// `RemoteSpawnRunner` of the placeholder `RemoteActionContextProvider` (no remote cache and no
/// remote executor): it can execute nothing.
// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:83 RemoteSpawnRunner
struct PlaceholderRemoteSpawnRunner;

impl SpawnRunner for PlaceholderRemoteSpawnRunner {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        blaze_util::unported!("RemoteSpawnRunner.exec")
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:517 RemoteSpawnRunner.canExec
    fn can_exec(&self, _spawn: &dyn Spawn) -> bool {
        // RemoteExecutionService.mayBeExecutedRemotely: the placeholder's combined cache is null,
        // so `combinedCache instanceof RemoteExecutionCache` is false.
        // Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:369 RemoteExecutionService.mayBeExecutedRemotely
        false
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:522 RemoteSpawnRunner.handlesCaching
    fn handles_caching(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/remote/RemoteSpawnRunner.java:135 RemoteSpawnRunner.getName
    fn get_name(&self) -> String {
        "remote".to_string()
    }
}

/// `WorkerModule.registerSpawnStrategies`.
// Java: com/google/devtools/build/lib/worker/WorkerModule.java:234 WorkerModule.registerSpawnStrategies
fn worker_module_register_spawn_strategies(
    registry_builder: &mut SpawnStrategyRegistryBuilder,
    env: &ExecCommandEnvironment,
) {
    // Java: com/google/devtools/build/lib/worker/WorkerSpawnStrategy.java:30 WorkerSpawnStrategy.toString
    registry_builder.register_strategy(
        Arc::new(
            AbstractSpawnStrategy::new(
                env.exec_root.clone(),
                Arc::new(WorkerSpawnRunner {
                    allowlist: env.worker_options.allowlist.clone(),
                }),
                env.execution_options.clone(),
                "worker",
            )
            .with_simple_class_name("WorkerSpawnStrategy"),
        ),
        &["worker"],
    );
}

/// `WorkerSpawnRunner`: `canExec` is ported; executing with persistent workers is not.
// Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:83 WorkerSpawnRunner
struct WorkerSpawnRunner {
    /// `WorkerOptions.allowlist`.
    allowlist: Option<Vec<String>>,
}

impl SpawnRunner for WorkerSpawnRunner {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        blaze_util::unported!("WorkerSpawnRunner.exec")
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:135 WorkerSpawnRunner.canExec
    fn can_exec(&self, spawn: &dyn Spawn) -> bool {
        if !spawns::supports_workers(spawn) && !spawns::supports_multiplex_workers(spawn) {
            return false;
        }
        // Note: `allowlist` is sorted, we could binary search.
        if let Some(allowlist) = &self.allowlist
            && !allowlist.is_empty()
            && !allowlist.contains(&spawns::get_worker_key_mnemonic(spawn))
        {
            return false;
        }
        if spawn.get_tool_files().is_empty() {
            return false;
        }
        true
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:152 WorkerSpawnRunner.handlesCaching
    fn handles_caching(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/worker/WorkerSpawnRunner.java:130 WorkerSpawnRunner.getName
    fn get_name(&self) -> String {
        "worker".to_string()
    }
}

/// `DynamicExecutionModule.registerSpawnStrategies`.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:146 DynamicExecutionModule.registerSpawnStrategies
fn dynamic_execution_module_register_spawn_strategies(
    registry_builder: &mut SpawnStrategyRegistryBuilder,
    env: &ExecCommandEnvironment,
) -> Result<(), AbruptExitException> {
    let options = &env.dynamic_execution_options;
    // Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:161 DynamicExecutionModule.registerSpawnStrategies
    if !options.internal_spawn_scheduler {
        return Ok(());
    }

    // PORT: the DynamicSpawnStrategy itself is not ported; it is registered so that strategy
    // resolution and its errors match Bazel, and fails loudly if a spawn ever reaches it.
    registry_builder.register_strategy(
        Arc::new(DynamicSpawnStrategy),
        &["dynamic", "dynamic_worker"],
    );
    let sandboxing_supported = registry_builder.is_strategy_registered("sandboxed");
    let local_strategies = get_local_strategies(env, sandboxing_supported)?;
    registry_builder.add_dynamic_local_strategies(&local_strategies);
    let remote_strategies = get_remote_strategies(env)?;
    registry_builder.add_dynamic_remote_strategies(&remote_strategies);
    Ok(())
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:105 DynamicExecutionModule.getLocalStrategies
fn get_local_strategies(
    env: &ExecCommandEnvironment,
    sandboxing_supported: bool,
) -> Result<IndexMap<String, Vec<String>>, AbruptExitException> {
    // Options that set "allowMultiple" to true ignore the default value, so we replicate that
    // functionality here.
    // ImmutableMap.Builder + buildKeepingLast: first position, last value.
    let mut local_and_worker_strategies: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut default_local_strategies = vec!["worker".to_string()];
    if sandboxing_supported {
        default_local_strategies.push("sandboxed".to_string());
    }
    if env.local_execution_options.local_lockfree_output {
        // Without local lock free, having standalone execution risks very bad performance.
        default_local_strategies.push("standalone".to_string());
    }
    local_and_worker_strategies.insert(String::new(), default_local_strategies);

    for (mnemonic, strategies) in &env.dynamic_execution_options.dynamic_local_strategy {
        local_and_worker_strategies.insert(mnemonic.clone(), strategies.clone());
        throw_if_contains_dynamic(strategies, "--dynamic_local_strategy")?;
    }
    Ok(local_and_worker_strategies)
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:128 DynamicExecutionModule.getRemoteStrategies
fn get_remote_strategies(
    env: &ExecCommandEnvironment,
) -> Result<IndexMap<String, Vec<String>>, AbruptExitException> {
    let options = &env.dynamic_execution_options;
    // Needed to dedup. (Java's HashMap order does not survive: the builder copies the entries
    // into its own HashMap.)
    let mut strategies: IndexMap<String, Vec<String>> = IndexMap::new();
    for (mnemonic, values) in &options.dynamic_remote_strategy {
        throw_if_contains_dynamic(values, "--dynamic_remote_strategy")?;
        strategies.insert(mnemonic.clone(), values.clone());
    }
    if options.dynamic_remote_strategy.is_empty() {
        // Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:141 DynamicExecutionModule.remoteStrategyName
        let mut m = IndexMap::new();
        m.insert(String::new(), vec!["remote".to_string()]);
        return Ok(m);
    }
    Ok(strategies)
}

// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:186 DynamicExecutionModule.throwIfContainsDynamic
fn throw_if_contains_dynamic(
    strategies: &[String],
    flag_name: &str,
) -> Result<(), AbruptExitException> {
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

/// `DynamicSpawnStrategy`: registered as "dynamic" / "dynamic_worker"; not ported.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:78 DynamicSpawnStrategy
struct DynamicSpawnStrategy;

impl fmt::Display for DynamicSpawnStrategy {
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:745 DynamicSpawnStrategy.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dynamic")
    }
}

impl SpawnStrategy for DynamicSpawnStrategy {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        blaze_util::unported!("DynamicSpawnStrategy.exec")
    }

    fn can_exec(
        &self,
        _spawn: &dyn Spawn,
        _action_context_registry: &dyn ActionContextRegistry,
    ) -> bool {
        blaze_util::unported!("DynamicSpawnStrategy.canExec")
    }

    fn get_simple_class_name(&self) -> String {
        "DynamicSpawnStrategy".to_string()
    }
}

/// `BazelStrategyModule.registerSpawnStrategies`.
// Java: com/google/devtools/build/lib/bazel/rules/BazelStrategyModule.java:62 BazelStrategyModule.registerSpawnStrategies
fn bazel_strategy_module_register_spawn_strategies(
    registry_builder: &mut SpawnStrategyRegistryBuilder,
    env: &ExecCommandEnvironment,
) {
    let options = &env.execution_options;
    let remote_options = &env.remote_options;

    let mut spawn_strategies: Vec<String> = options.spawn_strategy.clone();

    if spawn_strategies.is_empty() {
        if should_enable_remote_execution(remote_options) {
            spawn_strategies.push("remote".to_string());
        }
        spawn_strategies.push("worker".to_string());
        // Sandboxing is not yet available on Windows.
        spawn_strategies.push("sandboxed".to_string());
        spawn_strategies.push("local".to_string());
    }
    registry_builder.set_default_strategies(&spawn_strategies);

    // By adding this filter before the ones derived from --strategy the latter can override the
    // former.
    registry_builder.add_mnemonic_filter("Genrule", &options.genrule_strategy);

    for (mnemonic, strategies) in &options.strategy {
        registry_builder.add_mnemonic_filter(mnemonic, strategies);
    }

    for entry in &options.strategy_by_regexp {
        registry_builder.add_description_filter(entry.key.clone(), &entry.value);
    }

    for entry in &options.allowed_strategies_by_exec_platform {
        registry_builder.add_exec_platform_filter(entry.key.clone(), &entry.value);
    }
}
