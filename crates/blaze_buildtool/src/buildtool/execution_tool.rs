//! `ExecutionTool`: sets up the executor (strategies, contexts, action log directory, execroot
//! output directories) and runs the execution phase; plus the `BuildSummaryStatsModule` and
//! `UiStateTracker.buildComplete` summary lines.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use blaze_actions::Artifact;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_events::{
    Event, EventBus, EventHandler, ExtendedEventHandler, JavaClock, OutputFilter, Postable,
};
use blaze_exec::actions::CacheCheckAction;
use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::action_cache_checker::{
    ActionCacheChecker, ActionCacheCheckerCacheConfig, ExecutionFilter, NoProxies,
};
use blaze_exec::actions::cache::{
    ActionCache, ActionCacheStatistics, CompactPersistentActionCache, PostableActionCacheStats,
};
use blaze_exec::actions::resource_manager::{ResourceManager, configure_resource_manager};
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::executor_setup::{
    RemoteModuleSpawnStrategies, build_spawn_strategy_registry_with_remote_module,
};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_exec::sandbox::sandbox_module::SandboxModule;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_options::Options;
use blaze_remote::hooks::{RemoteHooks, RemoteOutputInjector};
use blaze_remote::remote_module::{RemoteModule, RemoteModuleEnvironment};
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionResultReceivedEvent, TargetCompleteEvent, downcast_event,
};
use blaze_sky_exec::keys::{BuildDriverKey, artifact_key};
use blaze_sky_exec::remote_output_tree::RemoteOutputTree;
use blaze_sky_exec::skyframe_action_executor::{
    ActionCacheSetup, ActionContexts, ArtifactFactoryResolver, ContainingPackageRootLookup,
    OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
};
use blaze_skyframe::{EvaluationContext, EvaluationResult, Evaluator, Interrupted, SkyKey};
use blaze_vfs::{FileSystemRef, Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// Where and how the execution phase runs.
pub struct ExecutionToolConfig {
    pub file_system: FileSystemRef,
    pub output_base: Path,
    /// `<output_base>/execroot/<workspace name>`.
    pub exec_root: Path,
    /// The install base holding `linux-sandbox`, `process-wrapper` and `embedded_tools`.
    pub install_base: Path,
    pub client_env: IndexMap<String, String>,
    pub keep_going: bool,
    pub verbose_failures: bool,
    /// `--jobs`.
    pub jobs: usize,
    /// The command's `ExecutionOptions`, `SandboxOptions` and `LocalExecutionOptions` (`None`:
    /// the defaults, with `verbose_failures` from above).
    pub execution_options: Option<ExecutionOptions>,
    pub sandbox_options: Option<SandboxOptions>,
    pub local_execution_options: Option<LocalExecutionOptions>,
    /// The command's `RemoteOptions`, `DynamicExecutionOptions` and `WorkerOptions` (`None`: the
    /// defaults), which the modules' `registerSpawnStrategies` read
    /// (`blaze_exec::exec::executor_setup`).
    pub remote_options: Option<RemoteOptions>,
    pub dynamic_execution_options: Option<DynamicExecutionOptions>,
    pub worker_options: Option<WorkerOptions>,
    /// `pkgLocator.getPathEntries()` (the sandbox mounts those under /tmp).
    pub package_path_entries: Vec<Path>,
    /// `--repo_contents_cache` (the sandbox mounts it when under /tmp).
    pub repo_contents_cache_path: Option<Path>,
    /// `--test_arg`s (`TestOptions.testArguments`).
    pub test_arguments: Vec<String>,
    /// `BlazeDirectories.getActionTempsDirectory(execRoot)`:
    /// `<execroot>/bazel-out/_tmp/actions`, where action stdout/stderr files live.
    pub action_temps_directory: Path,
    /// `--experimental_writable_outputs` (`CoreOptions.experimentalWritableOutputs`).
    pub writable_outputs: bool,
    /// The reporter's `OutputFilter` from `--output_filter` / `--auto_output_filter`
    /// (`Reporter.setOutputFilter`); `None` is `OutputFilter.OUTPUT_EVERYTHING`.
    pub output_filter: Option<Arc<dyn OutputFilter>>,
    /// The command's `RemoteModule` after `beforeCommand` ([`remote_module_before_command`]);
    /// `None` (test harnesses) registers the placeholder "remote" strategy and no spawn cache.
    pub remote_module: Option<Arc<CommandRemoteModule>>,
}

/// What `RemoteModule.beforeCommand` reads from the command's `CommandEnvironment`.
pub struct RemoteModuleCommandEnvironment {
    /// `env.getCommandName()`.
    pub command_name: String,
    /// `RemoteOptions`, `AuthAndTLSOptions` and `ExecutionOptions` of the command.
    pub remote_options: RemoteOptions,
    pub auth_and_tls_options: AuthAndTLSOptions,
    pub execution_options: Arc<ExecutionOptions>,
    /// `BuildRequestOptions.rewindLostInputs`.
    pub rewind_lost_inputs: bool,
    /// `BuildLanguageOptions.experimentalSiblingRepositoryLayout`.
    pub experimental_sibling_repository_layout: bool,
    /// `env.getReporter()`.
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getRuntime().getFileSystem()`.
    pub file_system: FileSystemRef,
    /// `env.getOutputBase()`, `env.getExecRoot()`, `env.getWorkingDirectory()`,
    /// `env.getWorkspace()`, `env.getWorkspaceName()`, `env.getActionTempsDirectory()`.
    pub output_base: Path,
    pub exec_root: Path,
    pub working_directory: Path,
    pub workspace: Option<Path>,
    pub workspace_name: String,
    pub action_temps_directory: Path,
    /// `directories.getInstallBase()` (`%install_base%` in `--credential_helper`).
    pub install_base: Path,
    /// `directories.getRelativeOutputPath()` (`bazel-out`): the outputs the action file system
    /// holds.
    pub relative_output_path: String,
    /// `env.getBuildRequestId()`, `env.getCommandId()`.
    pub build_request_id: String,
    pub command_id: String,
    /// `env.getRuntime().getProductName()`, `BlazeVersionInfo.instance().getVersion()`.
    pub product_name: String,
    pub tool_version: String,
    /// `env.getClientEnv()`.
    pub client_env: IndexMap<String, String>,
}

/// The `RemoteModule` of one command, between `beforeCommand` and `afterCommand`, with the
/// remote output tree of the actions' `RemoteActionFileSystem`s.
pub struct CommandRemoteModule {
    module: Mutex<Option<RemoteModule>>,
    remote_options: RemoteOptions,
    remote_output_tree: Arc<RemoteOutputTree>,
}

/// `RemoteActionFileSystem.injectRemoteFile` over the executor's remote output tree.
struct RemoteOutputTreeInjector {
    /// `RemoteActionFileSystem.outputBase`: `execRoot.getRelative(relativeOutputPath)`.
    output_base: PathFragment,
    remote_output_tree: Arc<RemoteOutputTree>,
}

impl RemoteOutputInjector for RemoteOutputTreeInjector {
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time: SystemTime,
        in_memory_output: bool,
    ) -> std::io::Result<()> {
        // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:768 RemoteActionFileSystem.isOutput
        if !path.starts_with(&self.output_base) {
            return Ok(());
        }
        // PORT: `Instant` to epoch milliseconds (`FileArtifactValue` keeps the expiration time
        // that way).
        let expiration_time_millis = expiration_time
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as i64);
        let metadata = FileArtifactValue::create_for_remote_file_with_materialization_data(
            digest,
            size,
            /* location_index= */ 1,
            expiration_time_millis,
            in_memory_output,
        );
        self.remote_output_tree.inject_file(path.clone(), metadata);
        Ok(())
    }
}

/// `RemoteModule.beforeCommand(env)`. An `Err` is the `AbruptExitException` Java's dispatcher
/// reports (`Event.error(e.getMessage())`) and exits with.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:403 RemoteModule.beforeCommand
pub fn remote_module_before_command(
    env: RemoteModuleCommandEnvironment,
) -> Result<Arc<CommandRemoteModule>, AbruptExitException> {
    let remote_output_tree = Arc::new(RemoteOutputTree::new());
    let output_base = env
        .exec_root
        .get_relative_str(&env.relative_output_path)
        .as_fragment()
        .clone();
    let hooks = RemoteHooks {
        remote_output_injector: Some(Arc::new(RemoteOutputTreeInjector {
            output_base,
            remote_output_tree: remote_output_tree.clone(),
        })),
    };
    let remote_options = env.remote_options.clone();
    let module = RemoteModule::before_command(RemoteModuleEnvironment {
        command_name: env.command_name,
        remote_options: env.remote_options,
        auth_and_tls_options: env.auth_and_tls_options,
        execution_options: env.execution_options,
        rewind_lost_inputs: Some(env.rewind_lost_inputs),
        experimental_sibling_repository_layout: env.experimental_sibling_repository_layout,
        reporter: env.reporter,
        file_system: env.file_system,
        output_base: env.output_base,
        exec_root: env.exec_root,
        working_directory: env.working_directory,
        workspace: env.workspace,
        install_base: Some(env.install_base),
        workspace_name: env.workspace_name,
        action_temps_directory: env.action_temps_directory,
        build_request_id: env.build_request_id,
        command_id: env.command_id,
        product_name: env.product_name,
        tool_version: env.tool_version,
        client_env: env.client_env,
        clock: Arc::new(JavaClock),
        // `--experimental_remote_scrubbing_config` fails loudly when its converter parses it
        // (blaze_optdefs's `Scrubber`), so no scrubber reaches the module.
        scrubber: None,
        hooks,
    })?;
    Ok(Arc::new(CommandRemoteModule {
        module: Mutex::new(Some(module)),
        remote_options,
        remote_output_tree,
    }))
}

impl CommandRemoteModule {
    /// `RemoteModule.afterCommand()` (with the task it submits to the `BlockWaitingModule`):
    /// waits for the background uploads (`--remote_cache_async`), closes the gRPC log and
    /// deletes the downloaded inputs. Runs once; later calls return `Ok`.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1098 RemoteModule.afterCommand
    pub fn after_command(&self) -> Result<(), AbruptExitException> {
        match self.module.lock().take() {
            Some(mut module) => module.after_command(),
            None => Ok(()),
        }
    }

    /// The `BuildCompleteEvent` subscriber (`RemoteExecutionService.onBuildComplete`).
    pub fn on_build_complete(&self, success: bool) {
        if let Some(module) = self.module.lock().as_ref() {
            module.on_build_complete(success);
        }
    }

    /// The `BuildInterruptedEvent` subscriber (`RemoteExecutionService.onBuildInterrupted`).
    pub fn on_build_interrupted(&self) {
        if let Some(module) = self.module.lock().as_ref() {
            module.on_build_interrupted();
        }
    }
}

/// `BuildSummaryStatsModule` + the `UiStateTracker` counters, as an event-bus subscriber.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:56 BuildSummaryStatsModule
pub struct ExecutionEventCollector {
    pub spawn_stats: SpawnStats,
    /// `UiStateTracker.actionsCompleted`.
    pub actions_completed: AtomicI32,
    pub target_complete_events: Mutex<Vec<TargetCompleteEvent>>,
    /// Further subscribers (`AggregatingTestListener`, BEP).
    pub forward: Mutex<Vec<Arc<dyn EventBus>>>,
}

impl ExecutionEventCollector {
    pub fn new() -> ExecutionEventCollector {
        ExecutionEventCollector {
            spawn_stats: SpawnStats::new(),
            actions_completed: AtomicI32::new(0),
            target_complete_events: Mutex::new(Vec::new()),
            forward: Mutex::new(Vec::new()),
        }
    }
}

impl EventBus for ExecutionEventCollector {
    fn post(&self, event: Arc<dyn Postable>) {
        if let Some(e) = downcast_event::<ActionResultReceivedEvent>(&event) {
            // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:139 BuildSummaryStatsModule.actionResultReceived
            self.spawn_stats
                .count_action_result(e.action_result.spawn_results());
        } else if downcast_event::<ActionCompletionEvent>(&event).is_some() {
            // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:145 BuildSummaryStatsModule.actionCompletion
            self.spawn_stats.increment_action_count();
            // Java: com/google/devtools/build/lib/runtime/UiStateTracker.java:620 UiStateTracker.actionCompletion
            self.actions_completed.fetch_add(1, Ordering::SeqCst);
        } else if let Some(e) = downcast_event::<PostableActionCacheStats>(&event) {
            // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:149 BuildSummaryStatsModule.actionCacheStats
            self.spawn_stats
                .record_action_cache_stats(e.as_proto().hits);
        } else if let Some(e) = downcast_event::<TargetCompleteEvent>(&event) {
            self.target_complete_events.lock().push(e.clone());
        }
        let forward = self.forward.lock().clone();
        for f in forward {
            f.post(event.clone());
        }
    }
}

/// The executor of one build plus the modules whose lifetime it spans.
pub struct PreparedExecution {
    pub executor: Arc<SkyframeActionExecutor>,
    pub sandbox_module: SandboxModule,
    pub spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    /// The persistent action cache (`BlazeWorkspace.getOrLoadPersistentActionCache`), `None` with
    /// `--nouse_action_cache`.
    pub action_cache: Option<Arc<dyn ActionCache>>,
    pub action_cache_checker: Arc<ActionCacheChecker>,
    /// The `AbruptExitException` of `SpawnStrategyRegistry.Builder.build` (an unknown or
    /// unsandboxed strategy in `--spawn_strategy`, `--strategy`, `--strategy_regexp`, ...),
    /// which Java's `ExecutionTool.<init>` throws. The executor then has no spawn strategy
    /// registry; the build command throws it where Java constructs the `ExecutionTool` (see
    /// [`PreparedExecution::check_execution_tool_created`]).
    ///
    /// PORT: Blaze creates the executor with the command's `SkyframeExecutor`, before the target
    /// patterns are loaded, and Java only after them, so the exception is kept for later.
    pub spawn_strategy_registry_error: Option<AbruptExitException>,
}

impl PreparedExecution {
    /// `new ExecutionTool(env, request)`: the `AbruptExitException` its constructor throws, if
    /// any (see [`PreparedExecution::spawn_strategy_registry_error`]).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
    pub fn check_execution_tool_created(&self) -> Result<(), AbruptExitException> {
        match &self.spawn_strategy_registry_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    /// Writes the action cache files to disk, reporting any errors that occurred during writing
    /// and capturing statistics: posts `PostableActionCacheStats` (which
    /// [`ExecutionEventCollector`] records for the `N processes: ... action cache hit` line).
    /// Call once after the execution phase, before the build summary (Java calls it from
    /// `ExecutionTool.nonCatastrophicFinalizations`, before `BuildCompleteEvent`).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:973 ExecutionTool.saveActionCache
    pub fn save_action_cache(&self, reporter: &dyn ExtendedEventHandler) {
        let mut builder = ActionCacheStatistics::default();

        if let Some(action_cache) = &self.action_cache {
            action_cache.merge_into_action_cache_statistics(&mut builder);
            if let Some(duration) = action_cache.get_load_time() {
                builder.load_time_in_ms = duration.as_millis() as u64;
            }
            let total_cache_check_semaphore_wait_millis = self
                .action_cache_checker
                .get_total_cache_check_semaphore_wait_millis();
            if total_cache_check_semaphore_wait_millis > 0 {
                builder.cache_check_semaphore_wait_time_in_ms =
                    total_cache_check_semaphore_wait_millis as u64;
            }

            let start = Instant::now();
            match action_cache.save() {
                Ok(size) => builder.size_in_bytes = size as u64,
                Err(e) => {
                    builder.size_in_bytes = 0;
                    reporter.handle(&Event::error(format!(
                        "I/O error while writing action log: {e}"
                    )));
                }
            }
            builder.save_time_in_ms = start.elapsed().as_millis() as u64;
        }
        reporter.post(Arc::new(PostableActionCacheStats::new(builder)));
    }
}

/// What `ExecutionTool` reads to set up the action cache of a build.
pub struct ActionCacheOptions {
    /// `--use_action_cache` (`BuildRequestOptions.useActionCache`, default true).
    pub use_action_cache: bool,
    /// `--explain` is set (`BuildRequestOptions.explanationPath != null`).
    pub explain: bool,
    /// `--check_up_to_date` (`ExecutionOptions.checkUpToDate`).
    pub check_up_to_date: bool,
    /// `--check_tests_up_to_date` (`ExecutionOptions.testCheckUpToDate`).
    pub test_check_up_to_date: bool,
    /// `env.getSkyframeBuildView().getArtifactFactory()`: resolves the source inputs that
    /// input-discovering actions (C++ compiles) recorded in the action cache.
    pub artifact_factory: Option<Arc<ArtifactFactory>>,
    /// The `CONTAINING_PACKAGE_LOOKUP` requests of `PackageRootResolverWithEnvironment`, needed
    /// when a cached input is not a source artifact analysis already created.
    pub containing_package_root_lookup: Option<Arc<dyn ContainingPackageRootLookup>>,
}

impl Default for ActionCacheOptions {
    fn default() -> Self {
        ActionCacheOptions {
            use_action_cache: true,
            explain: false,
            check_up_to_date: false,
            test_check_up_to_date: false,
            artifact_factory: None,
            containing_package_root_lookup: None,
        }
    }
}

/// `ExecutionTool.<init>` + `prepareForExecution`: creates the action log directory, builds the
/// spawn strategy registry (`linux-sandbox` first on Linux) and the `SkyframeActionExecutor`.
/// `register_contexts` adds the action contexts other crates own (`FileWriteActionContext`,
/// `SymlinkTreeActionContext`, `TestActionContext`, ...).
///
/// The action cache is disabled (`--nouse_action_cache`), as before the action cache was wired;
/// builds use [`prepare_for_execution_with_action_cache`].
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:264 ExecutionTool.prepareForExecution
pub fn prepare_for_execution(
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
    spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    register_contexts: &dyn Fn(&mut ActionContexts, &ExecCommandEnvironment),
) -> Result<PreparedExecution, String> {
    let prepared = prepare_for_execution_with_action_cache(
        config,
        reporter,
        spawn_log_context,
        register_contexts,
        ActionCacheOptions {
            use_action_cache: false,
            ..ActionCacheOptions::default()
        },
    )?;
    prepared
        .check_execution_tool_created()
        .map_err(|e| e.get_message().to_owned())?;
    Ok(prepared)
}

/// [`prepare_for_execution`] with the action cache: loads `<output_base>/action_cache` when
/// `--use_action_cache` (`BlazeWorkspace.getOrLoadPersistentActionCache`, then
/// `resetStatistics`) and builds the `ActionCacheChecker` (`ExecutionTool.createBuilder`). After
/// the execution phase call [`PreparedExecution::save_action_cache`].
///
/// An `Err` that starts with `Couldn't create action cache: ` is Java's `AbruptExitException`
/// with `FailureDetails.ActionCache.Code.INITIALIZATION_FAILURE`.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:264 ExecutionTool.prepareForExecution
pub fn prepare_for_execution_with_action_cache(
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
    spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    register_contexts: &dyn Fn(&mut ActionContexts, &ExecCommandEnvironment),
    action_cache_options: ActionCacheOptions,
) -> Result<PreparedExecution, String> {
    let action_out_dir = create_action_log_directory(config)?;

    let mut action_cache: Option<Arc<dyn ActionCache>> = None;
    if action_cache_options.use_action_cache {
        let cache = get_or_load_action_cache(&config.output_base, &*reporter)?;
        cache.reset_statistics();
        action_cache = Some(cache);
    }
    let execution_options = match &config.execution_options {
        Some(o) => o.clone(),
        None => {
            let mut o = Options::<ExecutionOptions>::get_defaults();
            o.verbose_failures = config.verbose_failures;
            o
        }
    };
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:915 ExecutionTool.createBuilder
    // PORT: the ActionCacheOptions flags are kept for callers that set them there.
    let execution_filter = check_up_to_date_filter_from_options(
        execution_options.test_check_up_to_date || action_cache_options.test_check_up_to_date,
        execution_options.check_up_to_date || action_cache_options.check_up_to_date,
    );
    // LocalOutputService: no proxy metadata, no remote output metadata in the action cache.
    let action_cache_checker = Arc::new(ActionCacheChecker::new(
        action_cache.clone(),
        Arc::new(ArtifactFactoryResolver(
            action_cache_options.artifact_factory.clone(),
        )),
        execution_filter,
        Arc::new(NoProxies),
        Some(
            ActionCacheCheckerCacheConfig::builder()
                .set_enabled(action_cache_options.use_action_cache)
                .set_store_output_metadata(false)
                .build(),
        ),
    ));

    let show_subcommands = execution_options.show_subcommands;
    let bin_tools = Arc::new(
        BinTools::for_production(config.install_base.clone())
            .map_err(|e| format!("cannot read the install base: {e}"))?,
    );
    let env = ExecCommandEnvironment {
        file_system: config.file_system.clone(),
        exec_root: config.exec_root.clone(),
        output_base: config.output_base.clone(),
        client_env: config.client_env.clone(),
        bin_tools,
        sandbox_options: Arc::new(
            config
                .sandbox_options
                .clone()
                .unwrap_or_else(Options::<SandboxOptions>::get_defaults),
        ),
        execution_options: Arc::new(execution_options),
        local_execution_options: Arc::new(
            config
                .local_execution_options
                .clone()
                .unwrap_or_else(Options::<LocalExecutionOptions>::get_defaults),
        ),
        remote_options: Arc::new(
            config
                .remote_options
                .clone()
                .unwrap_or_else(Options::<RemoteOptions>::get_defaults),
        ),
        dynamic_execution_options: Arc::new(
            config
                .dynamic_execution_options
                .clone()
                .unwrap_or_else(Options::<DynamicExecutionOptions>::get_defaults),
        ),
        worker_options: Arc::new(
            config
                .worker_options
                .clone()
                .unwrap_or_else(Options::<WorkerOptions>::get_defaults),
        ),
        resource_manager: ResourceManager::new(),
        reporter: reporter.clone() as Arc<dyn EventHandler>,
        package_path_entries: config.package_path_entries.clone(),
        repo_contents_cache_path: config.repo_contents_cache_path.clone(),
        test_arguments: config.test_arguments.clone(),
    };
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1010 ExecutionTool.configureResourceManager
    configure_resource_manager(&env.resource_manager, &env.execution_options);
    let mut sandbox_module = SandboxModule::new();
    sandbox_module.before_command(&env);
    let mut contexts = ActionContexts::default();
    let mut spawn_strategy_registry_error = None;
    let remote_module_guard = config.remote_module.as_ref().map(|m| m.module.lock());
    let remote_module: Option<&RemoteModule> =
        remote_module_guard.as_ref().and_then(|g| g.as_ref());
    match build_spawn_strategy_registry_with_remote_module(
        &env,
        &mut sandbox_module,
        remote_module.map(|m| m as &dyn RemoteModuleSpawnStrategies),
    ) {
        Ok(registry) => contexts.spawn_strategy_registry = Some(Arc::new(registry)),
        Err(e) => spawn_strategy_registry_error = Some(e),
    }
    contexts.spawn_log_context = spawn_log_context.clone();
    // Java: NoSpawnCacheModule registers SpawnCache.NO_CACHE ("no-cache"); RemoteModule,
    // registered later, replaces it with its RemoteSpawnCache ("remote-cache").
    // PORT: Java registers the RemoteSpawnCache of the placeholder provider too (no cache: it
    // looks nothing up and stores nothing); Blaze keeps NO_CACHE then.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1190 RemoteModule.registerActionContexts
    contexts.spawn_cache = Some(match remote_module.filter(|m| m.cache_available()) {
        Some(m) => m
            .register_action_contexts()
            .expect("a module with a cache has a provider"),
        None => blaze_exec::exec::action_execution_context::no_cache(),
    });
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
    // (ExecutorBuilder.getActionExecutionSalt(): RemoteModule is the only module that sets one).
    let action_execution_salt = match (remote_module, &config.remote_module) {
        (Some(m), Some(command_module)) => m
            .executor_init(&command_module.remote_options)
            .unwrap_or_default(),
        _ => String::new(),
    };
    // The RemoteOutputService's action file systems, when a cache is available.
    let remote_output_tree = match (remote_module, &config.remote_module) {
        (Some(m), Some(command_module)) if m.cache_available() => {
            Some(command_module.remote_output_tree.clone())
        }
        _ => None,
    };
    drop(remote_module_guard);
    register_contexts(&mut contexts, &env);
    let mut executor = SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
        exec_root: config.exec_root.clone(),
        action_out_dir,
        reporter,
        contexts,
        client_env: config.client_env.clone(),
        keep_going: config.keep_going,
        verbose_failures: config.verbose_failures,
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:452 SkyframeActionExecutor.getOutputPermissions
        output_permissions: if config.writable_outputs {
            OutputPermissions::WRITABLE
        } else {
            OutputPermissions::READONLY
        },
    })
    .with_action_cache(ActionCacheSetup {
        action_cache_checker: action_cache_checker.clone(),
        action_execution_salt,
        explain: action_cache_options.explain,
        containing_package_root_lookup: action_cache_options.containing_package_root_lookup,
    })
    .with_show_subcommands(show_subcommands);
    if let Some(output_filter) = &config.output_filter {
        executor = executor.with_output_filter(output_filter.clone());
    }
    if let Some(remote_output_tree) = remote_output_tree {
        executor = executor.with_remote_output_tree(remote_output_tree);
    }
    let executor = Arc::new(executor);
    Ok(PreparedExecution {
        executor,
        sandbox_module,
        spawn_log_context,
        action_cache,
        action_cache_checker,
        spawn_strategy_registry_error,
    })
}

/// Creates the directory for the actions' stdout/stderr files, deleting an existing one.
///
/// PORT: no `BulkDeleter` (remote output service) is ported, so the tree is deleted directly. An
/// `Err` is Java's `AbruptExitException` with `Execution.Code
/// .TEMP_ACTION_OUTPUT_DIRECTORY_DELETION_FAILURE` / `..._CREATION_FAILURE`.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:667 ExecutionTool.createActionLogDirectory
fn create_action_log_directory(config: &ExecutionToolConfig) -> Result<Path, String> {
    let directory = config.action_temps_directory.clone();
    if directory.exists()
        && let Err(e) = directory.delete_tree()
    {
        // PORT: logDeleteTreeFailure (logging) is not ported.
        return Err(format!("Couldn't delete action output directory: {e}"));
    }

    if let Err(e) = directory.create_directory_and_parents() {
        return Err(format!("Couldn't create action output directory: {e}"));
    }
    Ok(directory)
}

/// Determines an execution filter based on the --check_up_to_date and --check_tests_up_to_date
/// options.
// Java: com/google/devtools/build/lib/exec/CheckUpToDateFilter.java:32 CheckUpToDateFilter.fromOptions
fn check_up_to_date_filter_from_options(
    test_check_up_to_date: bool,
    check_up_to_date: bool,
) -> ExecutionFilter {
    if !test_check_up_to_date && !check_up_to_date {
        return Box::new(|_: &dyn CacheCheckAction| true);
    }
    // Java: com/google/devtools/build/lib/exec/CheckUpToDateFilter.java:46 CheckUpToDateFilter.<init>
    // If we want to check whether test is up-to-date, we should disallow test execution.
    let allow_test_action_execution = !test_check_up_to_date;
    // Build action execution should be prohibited in two cases - if we are checking whether
    // build is up-to-date or if we are checking that tests are up-to-date (and test execution is
    // not allowed).
    let allow_build_action_execution = allow_test_action_execution && !check_up_to_date;
    // Java: com/google/devtools/build/lib/exec/CheckUpToDateFilter.java:64 CheckUpToDateFilter.apply
    Box::new(move |action: &dyn CacheCheckAction| {
        if action.is_test_runner_action() {
            allow_test_action_execution
        } else {
            allow_build_action_execution
        }
    })
}

/// Get action cache if present or reload it from the on-disk cache
/// (`BlazeWorkspace.getOrLoadPersistentActionCache`: `<output_base>/action_cache`, with
/// `action_cache.bad` and `action_cache.tmp`).
///
/// PORT: Blaze runs one command per server process, so the cache is always loaded from disk.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:896 ExecutionTool.getOrLoadActionCache
// Java: com/google/devtools/build/lib/runtime/BlazeWorkspace.java:387 BlazeWorkspace.getOrLoadPersistentActionCache
fn get_or_load_action_cache(
    output_base: &Path,
    reporter: &dyn ExtendedEventHandler,
) -> Result<Arc<dyn ActionCache>, String> {
    match CompactPersistentActionCache::create(
        &output_base.get_child("action_cache"),
        &output_base.get_child("action_cache.bad"),
        &output_base.get_child("action_cache.tmp"),
        Arc::new(JavaClock),
        reporter as &dyn EventHandler,
    ) {
        Ok(cache) => Ok(Arc::new(cache)),
        Err(e) => Err(format!(
            "Couldn't create action cache: {e}. If error persists, use 'bazel clean'."
        )),
    }
}

/// The outcome of the execution phase.
pub struct ExecutionPhaseResult {
    pub evaluation: EvaluationResult,
    pub start: Instant,
    pub stop: Instant,
}

/// `SkyframeBuilder.buildArtifacts` / `SkyframeExecutor.evaluateBuildDriverKeys`: evaluates the
/// build driver keys and the additional artifacts (workspace status) together.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2170 SkyframeExecutor.evaluateBuildDriverKeys
pub fn evaluate_build_driver_keys(
    evaluator: &Evaluator,
    build_driver_keys: &[BuildDriverKey],
    additional_artifacts: &[Artifact],
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<ExecutionPhaseResult, Interrupted> {
    let start = Instant::now();
    let mut roots: Vec<SkyKey> = build_driver_keys.iter().cloned().map(SkyKey::new).collect();
    roots.extend(additional_artifacts.iter().map(artifact_key));
    let ctx = EvaluationContext {
        keep_going: config.keep_going,
        parallelism: config.jobs,
        event_handler: reporter,
        // PORT: Blaze only (see EvaluationContext::thread_stack_size). Spawns fork the process
        // (blaze_shell's JavaProcess runs Java's child-side steps in a pre_exec hook), and with
        // the kernel's heuristic overcommit a fork must account every writable private mapping
        // of the parent, including the reserved evaluator thread stacks: hundreds of threads at
        // the analysis default (256 MB) exceed RAM and the fork fails with ENOMEM. The Starlark
        // code of the execution phase (map_each callbacks of command lines and action keys) does
        // not run on these stacks: SkyframeActionExecutor runs `Action.execute` and the action
        // key computations on an EVAL_STACK_SIZE thread
        // (blaze_exec::exec::evaluation_stack::on_evaluation_stack).
        thread_stack_size: 16 << 20,
        ..EvaluationContext::default()
    };
    let evaluation = evaluator.evaluate(&roots, &ctx)?;
    Ok(ExecutionPhaseResult {
        evaluation,
        start,
        stop: Instant::now(),
    })
}

/// The `BuildSummaryStatsModule.buildComplete` lines: `Elapsed time: ...` and the process
/// summary. `elapsed_seconds` is `event.getResult().getElapsedSeconds()`.
///
/// PORT: skipped, profiling: critical path profiling (`CriticalPathComputer`,
/// `--experimental_enable_critical_path_profiling`) is not ported, so the summary is the one Java
/// prints without a `criticalPathComputer`: no `, Critical Path: ...` item, no `CriticalPathEvent`
/// and no "critical path" build tool log. The `--experimental_stats_summary` form of the summary
/// is not ported either; the build command fails loudly when the flag is set.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
pub fn report_build_summary_stats(
    reporter: &dyn EventHandler,
    collector: &ExecutionEventCollector,
    elapsed_seconds: f64,
) {
    // We might want to make this conditional on a flag; it can sometimes be a bit of a nuisance.
    let items = [format!("Elapsed time: {elapsed_seconds:.3}s")];
    let spawn_summary = collector.spawn_stats.get_summary();
    let spawn_summary_string = SpawnStats::convert_summary_to_string(&spawn_summary);
    reporter.handle(&Event::info(items.join(", ")));
    reporter.handle(&Event::info(spawn_summary_string));
}

/// `UiStateTracker.buildComplete`: `Build completed successfully, N total actions` or
/// `Build did NOT complete successfully`.
// Java: com/google/devtools/build/lib/runtime/UiStateTracker.java:481 UiStateTracker.buildComplete
pub fn build_complete_event(
    success: bool,
    failed_tests: i32,
    collector: &ExecutionEventCollector,
) -> Event {
    if success {
        let actions_completed = collector.actions_completed.load(Ordering::SeqCst);
        let mut s = String::from("Build completed");
        if failed_tests == 0 {
            s.push_str(" successfully");
        } else {
            s.push_str(&format!(
                ", {}{} FAILED",
                failed_tests,
                pluralize(" test", failed_tests)
            ));
        }
        // newStatsSummary is false by default.
        s.push_str(&format!(
            ", {}{}",
            actions_completed,
            pluralize(" total action", actions_completed)
        ));
        Event::info(s)
    } else {
        Event::error("Build did NOT complete successfully")
    }
}

// Java: com/google/devtools/build/lib/runtime/UiStateTracker.java:512 UiStateTracker.pluralize
fn pluralize(noun: &str, count: i32) -> String {
    format!("{}{}", noun, if count == 1 { "" } else { "s" })
}
