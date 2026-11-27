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
    self, FailureDetail, failure_detail, sandbox::Code as SandboxCode,
};

use crate::actions::spawn_strategy::SpawnStrategy;
use crate::actions::{ExecException, Spawn, SpawnResult};
use crate::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use crate::exec::action_execution_context::{ActionInputPrefetcher, RemoteOutputTree, SpawnCache};
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
    /// The command's output service is `RemoteOutputService` (`RemoteModule.setup` created it):
    /// its `getOutputChecker()` (the `RemoteOutputChecker`), which the action cache checker and
    /// `SkyframeActionExecutor.checkActionCache` use, and its
    /// `shouldStoreRemoteOutputMetadataInActionCache()` is true. `None`: `LocalOutputService`
    /// (`OutputChecker.TRUST_LOCAL_ONLY`, no remote metadata in the action cache).
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:182 RemoteOutputService.shouldStoreRemoteOutputMetadataInActionCache
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:187 RemoteOutputService.getOutputChecker
    pub output_checker: Option<Arc<dyn crate::actions::OutputChecker>>,
    /// `RemoteOutputService.finalizeAction` when `--output_tree_tracking` is set (see
    /// [`ActionFinalizer`](crate::exec::action_execution_context::ActionFinalizer)).
    pub action_finalizer: Option<Arc<dyn crate::exec::action_execution_context::ActionFinalizer>>,
    /// `RemoteModule.registerActionContexts`: the `RemoteImportantOutputHandler` (see
    /// [`ImportantOutputHandler`](crate::exec::action_execution_context::ImportantOutputHandler)).
    pub important_output_handler:
        Option<Arc<dyn crate::exec::action_execution_context::ImportantOutputHandler>>,
    /// How the action executor treats the lost inputs of a build with a remote cache (see
    /// [`LostInputsPolicy`]); `None` without one (no input can be lost then).
    pub lost_inputs_policy: Option<LostInputsPolicy>,
    /// `outputService.getRewoundActionSynchronizer()` of the remote output service
    /// (`RemoteRewoundActionSynchronizer` with `--rewind_lost_inputs`); `None` is `NOOP`.
    pub rewound_action_synchronizer:
        Option<Arc<dyn crate::exec::rewound_action_synchronizer::RewoundActionSynchronizer>>,
}

/// `SkyframeActionExecutor.rewindingEnabled()` / `invocationRetriesEnabled()` and the
/// `LostInputsEvent` subscriber (`RemoteExecutionService.onLostInputs`), which
/// `ActionRewindStrategy.checkRewindingEnabled` uses when an action lost inputs.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:355 SkyframeActionExecutor.prepareForExecution
#[derive(Clone)]
pub struct LostInputsPolicy {
    /// `--rewind_lost_inputs` (`BuildRequestOptions.rewindLostInputs`).
    pub rewinding_enabled: bool,
    /// `--experimental_remote_cache_eviction_retries` > 0
    /// (`ExecutionOptions.remoteRetryOnTransientCacheError`).
    pub invocation_retries_enabled: bool,
    /// `post(new LostInputsEvent(missingDigests))`: the subscribers' handler (the digests are
    /// `hash/size`).
    pub on_lost_inputs: Arc<dyn Fn(&[String]) + Send + Sync>,
}

impl fmt::Debug for LostInputsPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LostInputsPolicy")
            .field("rewinding_enabled", &self.rewinding_enabled)
            .field(
                "invocation_retries_enabled",
                &self.invocation_retries_enabled,
            )
            .finish_non_exhaustive()
    }
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
        if let Some(handler) = &self.important_output_handler {
            register(
                std::any::TypeId::of::<
                    Arc<dyn crate::exec::action_execution_context::ImportantOutputHandler>,
                >(),
                Arc::new(handler.clone()),
            );
        }
        if let Some(synchronizer) = &self.rewound_action_synchronizer {
            register(
                std::any::TypeId::of::<
                    Arc<dyn crate::exec::rewound_action_synchronizer::RewoundActionSynchronizer>,
                >(),
                Arc::new(synchronizer.clone()),
            );
        }
        if let Some(finalizer) = &self.action_finalizer {
            register(
                std::any::TypeId::of::<
                    Arc<dyn crate::exec::action_execution_context::ActionFinalizer>,
                >(),
                Arc::new(finalizer.clone()),
            );
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
///
/// PORT: the strategy policy is the default one (every strategy allowed). The runtime fails
/// loudly before it gets here for an invocation policy with any other strategy policy, because
/// blaze_buildtool's `ExecutionToolConfig` does not carry it yet; see
/// [`build_spawn_strategy_registry_with_policy`].
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
pub fn build_spawn_strategy_registry_with_remote(
    env: &ExecCommandEnvironment,
    sandbox_module: &mut crate::sandbox::sandbox_module::SandboxModule,
    remote: &RemoteModuleContributions,
) -> Result<SpawnStrategyRegistry, AbruptExitException> {
    build_spawn_strategy_registry_with_policy(
        env,
        sandbox_module,
        remote,
        &blaze_protos::blaze::strategy_policy::StrategyPolicy::default(),
    )
}

/// [`build_spawn_strategy_registry_with_remote`] with the strategy policy of the invocation
/// policy (`env.getInvocationPolicy().getStrategyPolicy()`).
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
pub fn build_spawn_strategy_registry_with_policy(
    env: &ExecCommandEnvironment,
    sandbox_module: &mut crate::sandbox::sandbox_module::SandboxModule,
    remote: &RemoteModuleContributions,
    strategy_policy: &blaze_protos::blaze::strategy_policy::StrategyPolicy,
) -> Result<SpawnStrategyRegistry, AbruptExitException> {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:166 ExecutionTool.<init>
    let mut spawn_strategy_registry_builder =
        SpawnStrategyRegistry::builder_with_policy(strategy_policy);

    // com.google.devtools.build.lib.remote.RemoteModule
    remote_module_register_spawn_strategies(&mut spawn_strategy_registry_builder, env, remote);
    // com.google.devtools.build.lib.worker.WorkerModule
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:221 WorkerModule.registerSpawnStrategies
    crate::worker::worker_module::WorkerModule::register_spawn_strategies(
        &mut spawn_strategy_registry_builder,
        env,
    );
    // com.google.devtools.build.lib.standalone.StandaloneModule
    StandaloneModule::new().register_spawn_strategies(&mut spawn_strategy_registry_builder, env);
    // com.google.devtools.build.lib.sandbox.SandboxModule
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:145 SandboxModule.registerSpawnStrategies
    if let Err(e) =
        sandbox_module.register_spawn_strategies(&mut spawn_strategy_registry_builder, env)
    {
        if crate::sandbox::sandbox_helpers::is_interrupted_error(&e) {
            // An InterruptedException of registerSpawnStrategies (a sandbox's isSupported probe):
            // BuildTool reports "build interrupted". PORT: the executor is made before Java
            // makes its ExecutionTool, so the interruption travels as an AbruptExitException
            // with the Interrupted detail of InterruptedFailureDetails.detailedExitCode, which the
            // build command turns back into an interrupted build.
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:943 BuildTool.processRequest
            return Err(AbruptExitException::new(FailureDetail {
                message: "build interrupted".to_string(),
                category: Some(failure_detail::Category::Interrupted(
                    failure_details::Interrupted {
                        code: failure_details::interrupted::Code::Interrupted as i32,
                    },
                )),
            }));
        }
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

    let registry = spawn_strategy_registry_builder.build()?;
    // PORT: Java logs the registry when `ExecutionTool.prepareBuild` makes the `BlazeExecutor`
    // (once per build, after the registry is built here); Blaze has no `BlazeExecutor`, so the
    // registry is logged as soon as it is built. A build that stops before Java's executor is made
    // logs it here and not in Java (only the server log differs).
    // Java: com/google/devtools/build/lib/exec/BlazeExecutor.java:78 BlazeExecutor.<init>
    registry.log_spawn_strategies();
    Ok(registry)
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

/// `DynamicExecutionModule.registerSpawnStrategies`.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:146 DynamicExecutionModule.registerSpawnStrategies
fn dynamic_execution_module_register_spawn_strategies(
    registry_builder: &mut SpawnStrategyRegistryBuilder,
    env: &ExecCommandEnvironment,
) -> Result<(), AbruptExitException> {
    let options = &env.dynamic_execution_options;
    if !options.internal_spawn_scheduler {
        return Ok(());
    }
    // execOptions.getLocalResources().get(ResourceSet.CPU).intValue()
    let mut num_cpus =
        blaze_optdefs::common::local_host_capacity::get_local_host_capacity().cpu_usage;
    for (resource, value) in &env.execution_options.local_resources {
        if resource == "cpu" {
            num_cpus = *value;
        }
    }
    let jobs = env
        .jobs
        .expect("getOptions(BuildRequestOptions.class).jobs of the command");
    // DynamicExecutionModule.beforeCommand: ignoreLocalSignals, verboseFailures, reporter.
    let ignore_failure_check = crate::dynamic::dynamic_execution_module::ModuleIgnoreFailureCheck {
        ignore_local_signals: options.ignore_local_signals.clone().unwrap_or_default(),
        verbose_failures: env.execution_options.verbose_failures,
        reporter: env.reporter.clone(),
    };
    crate::dynamic::dynamic_execution_module::register_spawn_strategies(
        registry_builder,
        options,
        Some(&env.local_execution_options),
        num_cpus as i32,
        jobs,
        Arc::new(ignore_failure_check),
    )
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
