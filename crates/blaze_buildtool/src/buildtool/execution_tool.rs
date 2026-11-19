//! `ExecutionTool`: sets up the executor (strategies, contexts, action log directory, execroot
//! output directories) and runs the execution phase; plus the `BuildSummaryStatsModule` and
//! `UiStateTracker.buildComplete` summary lines.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::time::Instant;

use blaze_actions::Artifact;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_bep::metrics::criticalpath::{
    AggregatedCriticalPath, CanBeShared, CriticalPathComputer, CriticalPathEvent,
};
use blaze_events::java_format::java_format_fixed;
use blaze_events::{
    Clock, Event, EventBus, EventHandler, ExtendedEventHandler, JavaClock, OutputFilter, Postable,
    nano_time_of,
};
use blaze_exec::actions::CacheCheckAction;
use blaze_exec::actions::action_cache_checker::{
    ActionCacheChecker, ActionCacheCheckerCacheConfig, ExecutionFilter, NoProxies,
};
use blaze_exec::actions::cache::{
    ActionCache, ActionCacheStatistics, CompactPersistentActionCache, PostableActionCacheStats,
};
use blaze_exec::actions::resource_manager::{ResourceManager, configure_resource_manager};
use blaze_exec::analysis::test::test_runner_action::SpawnExecutedEventChangePhase;
use blaze_exec::exec::action_execution_context::SpawnExecutedEvent;
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::evaluation_stack::on_evaluation_stack;
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::executor_setup::{
    RemoteModuleContributions, build_spawn_strategy_registry_with_remote,
};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_exec::sandbox::sandbox_module::SandboxModule;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_options::Options;
use blaze_sky_exec::action_execution_function::SkyframeContainingPackageRootLookup;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionResultReceivedEvent, ActionStartedEvent, TargetCompleteEvent,
    TopLevelTargetPendingExecutionEvent, downcast_event,
};
use blaze_sky_exec::keys::{BuildDriverKey, TestCompletionKey, artifact_key};
use blaze_sky_exec::skyframe_action_executor::{
    ActionCacheSetup, ActionContexts, ArtifactFactoryResolver, CachedActionEvent,
    ContainingPackageRootLookup, OutputPermissions, SkyframeActionExecutor,
    SkyframeActionExecutorConfig,
};
use blaze_skyframe::{EvaluationContext, EvaluationResult, Evaluator, Interrupted, SkyKey};
use blaze_vfs::{FileSystemRef, Path};
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
    /// What the command's `RemoteModule` contributes (strategy "remote", spawn cache, input
    /// prefetcher, remote output tree, action execution salt); the default is a command without
    /// any remote option.
    pub remote: RemoteModuleContributions,
}

/// `BuildSummaryStatsModule` + the `UiStateTracker` counters, as an event-bus subscriber.
///
/// PORT: the `CriticalPathComputer` that `executorInit` registers on the event bus is called from
/// here (its `@Subscribe` methods take the events' parts, see `blaze_bep::metrics::criticalpath`).
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:56 BuildSummaryStatsModule
pub struct ExecutionEventCollector {
    pub spawn_stats: SpawnStats,
    /// `UiStateTracker.actionsCompleted`.
    pub actions_completed: AtomicI32,
    pub target_complete_events: Mutex<Vec<TargetCompleteEvent>>,
    /// Further subscribers (`AggregatingTestListener`, BEP).
    pub forward: Mutex<Vec<Arc<dyn EventBus>>>,
    /// `criticalPathComputer`: set by `executorInit` with `--experimental_enable_critical_path_profiling`.
    critical_path_computer: Mutex<Option<Arc<CriticalPathComputer>>>,
    /// `statsSummary` (`--experimental_stats_summary`), set by `executorInit`.
    stats_summary: AtomicBool,
    /// `commandStartMillis` (`env.getCommandStartTime()`).
    command_start_millis: AtomicI64,
    /// `executionStartMillis`.
    execution_start_millis: AtomicI64,
    /// `executionEndMillis`.
    execution_end_millis: AtomicI64,
    /// `executionStarted`.
    execution_started: AtomicBool,
    /// Whether `executorInit` ran (the build's `ExecutionTool` was constructed).
    executor_initialized: AtomicBool,
    /// The `criticalPath` of the last `buildComplete` when a computer existed: the BEP's
    /// `BuildToolLogCollection` value "critical path" (`criticalPath.toString()`).
    ///
    /// PORT: Java adds the value to the build result inside `buildComplete`; the BEP side of the
    /// command (which owns the collection) reads it from here.
    critical_path_tool_log: Mutex<Option<String>>,
    /// The clock of the summary (`BlazeClock.instance()`).
    clock: Arc<dyn Clock>,
}

impl ExecutionEventCollector {
    pub fn new() -> ExecutionEventCollector {
        ExecutionEventCollector::with_clock(Arc::new(JavaClock))
    }

    /// `beforeCommand` with an injectable clock (Java's `BlazeClock.instance()`).
    // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:71 BuildSummaryStatsModule.beforeCommand
    pub fn with_clock(clock: Arc<dyn Clock>) -> ExecutionEventCollector {
        ExecutionEventCollector {
            spawn_stats: SpawnStats::new(),
            actions_completed: AtomicI32::new(0),
            target_complete_events: Mutex::new(Vec::new()),
            forward: Mutex::new(Vec::new()),
            critical_path_computer: Mutex::new(None),
            stats_summary: AtomicBool::new(false),
            command_start_millis: AtomicI64::new(0),
            execution_start_millis: AtomicI64::new(0),
            execution_end_millis: AtomicI64::new(0),
            execution_started: AtomicBool::new(false),
            executor_initialized: AtomicBool::new(false),
            critical_path_tool_log: Mutex::new(None),
            clock,
        }
    }

    /// `commandStartMillis = env.getCommandStartTime()` (`beforeCommand`).
    pub fn set_command_start_millis(&self, command_start_millis: i64) {
        self.command_start_millis
            .store(command_start_millis, Ordering::SeqCst);
    }

    /// `executorInit`: called when the build's `ExecutionTool` is constructed. With
    /// `--experimental_enable_critical_path_profiling` (the default) a `CriticalPathComputer` is
    /// registered; `can_be_shared` is `Actions.canBeShared` with the executor's action key
    /// context (see [`can_be_shared_with_executor`]).
    // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:91 BuildSummaryStatsModule.executorInit
    pub fn executor_init(
        &self,
        enable_critical_path_profiling: bool,
        stats_summary: bool,
        can_be_shared: CanBeShared,
    ) {
        // PORT: Blaze creates the executor before the analysis; the command calls this where
        // Java constructs the ExecutionTool, which happens once per build.
        if self.executor_initialized.swap(true, Ordering::SeqCst) {
            return;
        }
        let enabled = enable_critical_path_profiling;
        self.stats_summary.store(stats_summary, Ordering::SeqCst);
        if enabled {
            *self.critical_path_computer.lock() =
                Some(Arc::new(CriticalPathComputer::new(can_be_shared)));
        }
    }

    /// Whether `executorInit` ran (the build's `ExecutionTool` was constructed).
    pub fn executor_initialized(&self) -> bool {
        self.executor_initialized.load(Ordering::SeqCst)
    }

    /// Whether `executorInit` registered a critical path computer.
    pub fn has_critical_path_computer(&self) -> bool {
        self.critical_path_computer.lock().is_some()
    }

    // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:121 BuildSummaryStatsModule.markExecutionPhaseStarted
    fn mark_execution_phase_started(&self) {
        // TODO(ulfjack): Make sure to use the same clock as for commandStartMillis.
        self.execution_start_millis
            .store(self.clock.current_time_millis(), Ordering::SeqCst);
    }

    /// `@Subscribe executionPhaseFinish(ExecutionFinishedEvent)`.
    ///
    /// PORT: the `ExecutionFinishedEvent` (`ExecutionTool.unconditionalExecutionPhaseFinalizations`)
    /// is not posted by the execution side; the command calls this where Java posts it.
    // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:132 BuildSummaryStatsModule.executionPhaseFinish
    pub fn execution_phase_finish(&self) {
        self.execution_end_millis
            .store(self.clock.current_time_millis(), Ordering::SeqCst);
    }

    /// The "critical path" value of the build tool logs, when `buildComplete` computed one.
    pub fn critical_path_tool_log(&self) -> Option<String> {
        self.critical_path_tool_log.lock().clone()
    }

    /// The `@Subscribe` methods of the `CriticalPathComputer` registered by `executorInit`.
    fn post_to_critical_path_computer(&self, event: &Arc<dyn Postable>) {
        let Some(computer) = self.critical_path_computer.lock().clone() else {
            return;
        };
        if let Some(e) = downcast_event::<ActionStartedEvent>(event) {
            // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:234 CriticalPathComputer.actionStarted
            computer.action_started(&e.action, nano_time_of(e.nano_time_start));
        } else if let Some(e) = downcast_event::<ActionCompletionEvent>(event) {
            // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:307 CriticalPathComputer.actionComplete
            computer.action_complete(
                &e.action,
                nano_time_of(e.related_time_start),
                nano_time_of(e.finish_time),
            );
        } else if let Some(e) = downcast_event::<CachedActionEvent>(event) {
            // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:293 CriticalPathComputer.actionCached
            computer.action_cached(
                &e.action,
                nano_time_of(e.nano_time_start),
                nano_time_of(e.nano_time_finish),
            );
        } else if let Some(e) = downcast_event::<SpawnExecutedEvent>(event) {
            // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:150 CriticalPathComputer.spawnExecuted
            // getActionMetadata().getPrimaryOutput(): the action's primary output artifact.
            let primary_output = e
                .get_action_primary_output()
                .as_any()
                .downcast_ref::<Artifact>();
            let spawn_result = e.get_spawn_result();
            computer.spawn_executed(
                primary_output,
                spawn_result.get_metrics(),
                spawn_result.get_runner_name(),
                spawn_result.get_runner_subtype(),
                spawn_result.was_remote(),
            );
        } else if let Some(e) = downcast_event::<SpawnExecutedEventChangePhase>(event) {
            // Java: com/google/devtools/build/lib/metrics/criticalpath/CriticalPathComputer.java:139 CriticalPathComputer.nextCriticalPathPhase
            computer.next_critical_path_phase(&blaze_actions::Action::get_primary_output(
                &**e.get_action(),
            ));
        }
        // PORT: DiscoveredInputsEvent, ActionChangePrunedEvent and ActionRewoundEvent are not
        // posted by the execution side (input discovery metrics, change pruning and action
        // rewinding are not ported there).
    }
}

impl EventBus for ExecutionEventCollector {
    fn post(&self, event: Arc<dyn Postable>) {
        self.post_to_critical_path_computer(&event);
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
        } else if downcast_event::<TopLevelTargetPendingExecutionEvent>(&event).is_some() {
            // Skymeld-specific marking of the start of execution. Multiple instances of this
            // event might be fired during the build, but we make sure to only mark the start of
            // the execution phase when the first one is received.
            // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:114 BuildSummaryStatsModule.executionPhaseStarting
            if self
                .execution_started
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                self.mark_execution_phase_started();
            }
        }
        let forward = self.forward.lock().clone();
        for f in forward {
            f.post(event.clone());
        }
    }
}

/// `Actions.canBeShared(actionKeyContext, a, b)` with the executor's `ActionKeyContext`, for the
/// `CriticalPathComputer` of `executorInit`.
///
/// PORT: the same port as `blaze_sky_exec::skyframe_action_executor::can_be_shared` (private
/// there); the action keys may run Starlark callbacks, so they are computed on an
/// evaluation-sized stack. An interrupted key computation answers false (the computer then
/// reports the duplicate output as Java's `IllegalStateException` does).
// Java: com/google/devtools/build/lib/actions/Actions.java:85 Actions.canBeShared
pub fn can_be_shared_with_executor(executor: Arc<SkyframeActionExecutor>) -> CanBeShared {
    Arc::new(move |a, b| {
        let action_key_context = executor.get_action_key_context();
        let (key_a, key_b) = on_evaluation_stack(|| {
            (
                a.get_key(action_key_context, None).ok(),
                b.get_key(action_key_context, None).ok(),
            )
        });
        let (Some(key_a), Some(key_b)) = (key_a, key_b) else {
            return false;
        };
        if !(a.get_mnemonic() == b.get_mnemonic() && key_a == key_b) {
            return false;
        }
        // Uses a standard comparison technique for shareable actions.
        if a.is_shareable() && b.is_shareable() {
            return artifacts_equal_without_owner(
                &a.get_mandatory_inputs().to_list(),
                &b.get_mandatory_inputs().to_list(),
            ) && artifacts_equal_without_owner(&a.get_outputs(), &b.get_outputs());
        }
        // If this is reached, at least one action is not shareable. If the actions differ on
        // this, they cannot be shared.
        if a.is_shareable() || b.is_shareable() {
            return false;
        }
        // If the artifacts are in fact equal (with owners), these are aliases of the same action
        // and not in conflict with each other.
        *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
            && a.get_outputs() == b.get_outputs()
    })
}

// Java: com/google/devtools/build/lib/actions/Actions.java:146 Actions.artifactsEqualWithoutOwner
fn artifacts_equal_without_owner(collection1: &[Artifact], collection2: &[Artifact]) -> bool {
    if collection1.len() != collection2.len() {
        return false;
    }
    collection1
        .iter()
        .zip(collection2)
        .all(|(a1, a2)| a1.equals_without_owner(a2))
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
    /// when a cached input is not a source artifact analysis already created. The default is the
    /// Skyframe port ([`SkyframeContainingPackageRootLookup`]).
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
            // ActionExecutionFunction always resolves through PackageRootResolverWithEnvironment.
            containing_package_root_lookup: Some(Arc::new(SkyframeContainingPackageRootLookup)),
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
    // outputService.startBuild(...) (startBuildAndDetermineModifiedOutputFiles), before the
    // action log directory is created under the output path.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:291 ExecutionTool.prepareForExecution
    start_build(config)?;
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
    // outputService.getProxyMetadataFactory() is NO_PROXIES for LocalOutputService and
    // RemoteOutputService; outputService.shouldStoreRemoteOutputMetadataInActionCache() is true
    // for RemoteOutputService (the remote module's output service) only.
    let store_output_metadata = config.remote.output_checker.is_some();
    let mut action_cache_checker = ActionCacheChecker::new(
        action_cache.clone(),
        Arc::new(ArtifactFactoryResolver(
            action_cache_options.artifact_factory.clone(),
        )),
        execution_filter,
        Arc::new(NoProxies),
        Some(
            ActionCacheCheckerCacheConfig::builder()
                .set_enabled(action_cache_options.use_action_cache)
                .set_store_output_metadata(store_output_metadata)
                .build(),
        ),
    );
    // PORT: TreeFileArtifact.createTreeOutput / TreeArtifactValue.Builder for the cached tree
    // metadata of the action cache (only read when the output metadata is stored).
    action_cache_checker.set_tree_artifact_factory(Arc::new(
        blaze_exec::actions::action_adapters::ArtifactTreeArtifactFactory,
    ));
    let action_cache_checker = Arc::new(action_cache_checker);

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
    match build_spawn_strategy_registry_with_remote(&env, &mut sandbox_module, &config.remote) {
        Ok(registry) => contexts.spawn_strategy_registry = Some(Arc::new(registry)),
        Err(e) => spawn_strategy_registry_error = Some(e),
    }
    contexts.spawn_log_context = spawn_log_context.clone();
    // NoSpawnCacheModule.registerActionContexts, then RemoteModule.registerActionContexts (the
    // last registered SpawnCache wins) and the contexts of RemoteModule.executorInit.
    // Java: com/google/devtools/build/lib/runtime/NoSpawnCacheModule.java:26 NoSpawnCacheModule.registerActionContexts
    contexts.spawn_cache = Some(blaze_exec::exec::action_execution_context::no_cache());
    {
        let mut spawn_cache = contexts.spawn_cache.take();
        let mut registered = Vec::new();
        config
            .remote
            .register_action_contexts(&mut spawn_cache, &mut |type_id, ctx| {
                registered.push((type_id, ctx))
            });
        contexts.spawn_cache = spawn_cache;
        for (type_id, ctx) in registered {
            contexts.register_raw(type_id, ctx);
        }
    }
    register_contexts(&mut contexts, &env);
    let mut executor = SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
        exec_root: config.exec_root.clone(),
        action_out_dir,
        // Java: directories.getOutputBase().getRelative(LabelConstants.EXTERNAL_PATH_PREFIX)
        external_directory: config.output_base.get_child("external"),
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
        // ExecutorBuilder.getActionExecutionSalt(): RemoteModule.executorInit sets it for every
        // command with RemoteOptions (computeActionExecutionSalt, also without a cache: the
        // placeholder setup); "" only when no module set it.
        // Java: com/google/devtools/build/lib/exec/ExecutorBuilder.java:72 ExecutorBuilder.getActionExecutionSalt
        action_execution_salt: config
            .remote
            .action_execution_salt
            .clone()
            .unwrap_or_default(),
        explain: action_cache_options.explain,
        containing_package_root_lookup: action_cache_options.containing_package_root_lookup,
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:782 SkyframeActionExecutor.checkActionCache
        output_checker: config.remote.output_checker.clone(),
    })
    .with_show_subcommands(show_subcommands)
    .with_lost_inputs_policy(config.remote.lost_inputs_policy.clone());
    if let Some(output_filter) = &config.output_filter {
        executor = executor.with_output_filter(output_filter.clone());
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

/// `outputService.startBuild(buildId, workspaceName, eventHandler, finalizeActions)` of the
/// command's output service: `RemoteOutputService` when the remote module created one (it gave
/// the executor its output checker), else `LocalOutputService`. Both make sure the output path
/// (`directories.getOutputPath(workspaceName)`, `<execroot>/bazel-out`) is not a symbolic link
/// left by another output service. An `Err` is Java's `AbruptExitException` with
/// `Execution.Code.LOCAL_OUTPUT_DIRECTORY_SYMLINK_FAILURE`.
///
/// PORT: the returned `ModifiedFileSet` (`EVERYTHING_MODIFIED` for both) is not used: Blaze has
/// no incremental output-file checking. The product name is "bazel" (the relative output path is
/// `bazel-out`).
// Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:125 RemoteOutputService.startBuild
// Java: com/google/devtools/build/lib/vfs/LocalOutputService.java:59 LocalOutputService.startBuild
fn start_build(config: &ExecutionToolConfig) -> Result<(), String> {
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:232 BlazeDirectories.getRelativeOutputPath
    let output_path = config.exec_root.get_relative_str("bazel-out");
    if config.remote.output_checker.is_some() {
        // One of the responsibilities of OutputService.startBuild() is that it ensures the output
        // path is valid. If the previous OutputService redirected the output path to a remote
        // location, we must undo this.
        if output_path.is_symbolic_link()
            && let Err(e) = output_path.delete()
        {
            return Err(format!("Couldn't remove output path symlink: {e}"));
        }
        return Ok(());
    }
    // directories.getLocalOutputPath(): outputBase.getRelative(getRelativeOutputPath())
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:117 BlazeDirectories.<init>
    let local_output_path = config.output_base.get_relative_str("bazel-out");
    if output_path.is_symbolic_link() {
        let result = (|| -> std::io::Result<()> {
            // Remove the existing symlink first.
            output_path.delete()?;
            if local_output_path.exists() {
                // Pre-existing local output directory. Move to outputPath.
                local_output_path.rename_to(&output_path)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            return Err(format!(
                "Couldn't handle local output directory symlinks: {e}"
            ));
        }
    }
    Ok(())
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

/// Asks the Skyframe evaluator to run a single exclusive test (the `TestCompletionKey` of one
/// exclusive test, `exclusiveTesting = true`).
///
/// PORT: `resourceManager.resetResourceUsage()` before and after is not needed: Blaze's resource
/// manager holds no resources between evaluations.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2033 SkyframeExecutor.runExclusiveTestSkymeld
pub fn run_exclusive_test_skymeld(
    evaluator: &Evaluator,
    test_completion_key: &TestCompletionKey,
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<EvaluationResult, Interrupted> {
    let ctx = EvaluationContext {
        keep_going: config.keep_going,
        parallelism: config.jobs,
        event_handler: reporter,
        // As in evaluate_build_driver_keys.
        thread_stack_size: 16 << 20,
        ..EvaluationContext::default()
    };
    evaluator.evaluate(&[SkyKey::new(test_completion_key.clone())], &ctx)
}

/// The `BuildSummaryStatsModule.buildComplete` lines: `Elapsed time: ...` (with
/// `, Critical Path: ...` when `executorInit` registered a critical path computer) and the
/// process summary, or the `--experimental_stats_summary` lines. `elapsed_seconds` is
/// `event.getResult().getElapsedSeconds()` and `stop_time_millis`
/// `event.getResult().getStopTime()`. Posts the `CriticalPathEvent`.
///
/// PORT: the build tool log values ("elapsed time", "critical path", "process stats") are added
/// by the command's BEP side, which owns the `BuildToolLogCollection`
/// ([`ExecutionEventCollector::critical_path_tool_log`]). The profiler (`ProfilerStartedEvent`,
/// `Profiler.instance().stop()`, the critical path profile tasks) is not ported. The logger
/// lines go to the server log, which is not ported.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
pub fn report_build_summary_stats(
    reporter: &dyn ExtendedEventHandler,
    collector: &ExecutionEventCollector,
    elapsed_seconds: f64,
    stop_time_millis: i64,
) {
    // We might want to make this conditional on a flag; it can sometimes be a bit of a nuisance.
    let mut items = vec![format!(
        "Elapsed time: {}s",
        java_format_fixed(elapsed_seconds, 3)
    )];

    let mut critical_path = AggregatedCriticalPath::empty();
    let critical_path_computer = collector.critical_path_computer.lock().clone();
    if let Some(critical_path_computer) = &critical_path_computer {
        critical_path = critical_path_computer.aggregate();
        reporter.post(Arc::new(CriticalPathEvent::new(critical_path.clone())));
        items.push(critical_path.to_string_summary_no_remote());
        *collector.critical_path_tool_log.lock() = Some(critical_path.to_string());
    }

    let spawn_summary = collector.spawn_stats.get_summary();
    let spawn_summary_string = SpawnStats::convert_summary_to_string(&spawn_summary);
    if collector.stats_summary.load(Ordering::SeqCst) {
        reporter.handle(&Event::info(spawn_summary_string));
        reporter.handle(&Event::info(format!(
            "Total action wall time {}s",
            java_format_fixed(
                collector.spawn_stats.get_total_wall_time_millis() as f64 / 1000.0,
                2
            )
        )));
        if !critical_path.is_empty_instance() {
            reporter.handle(&Event::info(critical_path.get_new_string_summary()));
        }
        let now = stop_time_millis;
        let command_start_millis = collector.command_start_millis.load(Ordering::SeqCst);
        let execution_time = collector.execution_end_millis.load(Ordering::SeqCst)
            - collector.execution_start_millis.load(Ordering::SeqCst);
        let overhead_time = now - command_start_millis - execution_time;
        reporter.handle(&Event::info(format!(
            "Elapsed time {}s (preparation {}s, execution {}s)",
            java_format_fixed((now - command_start_millis) as f64 / 1000.0, 2),
            java_format_fixed(overhead_time as f64 / 1000.0, 2),
            java_format_fixed(execution_time as f64 / 1000.0, 2)
        )));
    } else {
        reporter.handle(&Event::info(items.join(", ")));
        reporter.handle(&Event::info(spawn_summary_string));
    }

    // finally: eventBus.unregister(criticalPathComputer); criticalPathComputer = null.
    *collector.critical_path_computer.lock() = None;
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
