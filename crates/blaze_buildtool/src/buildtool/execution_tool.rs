//! `ExecutionTool`: sets up the executor (strategies, contexts, action log directory, execroot
//! output directories) and runs the execution phase; plus the `BuildSummaryStatsModule` and
//! `UiStateTracker.buildComplete` summary lines.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Instant;

use blaze_actions::Artifact;
use blaze_events::{Event, EventBus, EventHandler, ExtendedEventHandler, Postable};
use blaze_exec::actions::resource_manager::{ResourceManager, configure_resource_manager};
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::executor_setup::build_spawn_strategy_registry;
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_exec::sandbox::sandbox_module::SandboxModule;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionResultReceivedEvent, TargetCompleteEvent, downcast_event,
};
use blaze_sky_exec::keys::{BuildDriverKey, artifact_key};
use blaze_sky_exec::skyframe_action_executor::{
    ActionContexts, OutputPermissions, SkyframeActionExecutor, SkyframeActionExecutorConfig,
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
}

/// `ExecutionTool.<init>` + `prepareForExecution`: creates the action log directory, builds the
/// spawn strategy registry (`linux-sandbox` first on Linux) and the `SkyframeActionExecutor`.
/// `register_contexts` adds the action contexts other crates own (`FileWriteActionContext`,
/// `SymlinkTreeActionContext`, `TestActionContext`, ...).
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:264 ExecutionTool.prepareForExecution
pub fn prepare_for_execution(
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
    spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    register_contexts: &dyn Fn(&mut ActionContexts, &ExecCommandEnvironment),
) -> Result<PreparedExecution, String> {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:956 ExecutionTool.createActionLogDirectory
    let action_out_dir = config.output_base.get_child("action_outs");
    let _ = action_out_dir.delete_tree();
    action_out_dir
        .create_directory_and_parents()
        .map_err(|e| format!("Couldn't delete action output directory: {e}"))?;

    let mut execution_options = Options::<ExecutionOptions>::get_defaults();
    execution_options.verbose_failures = config.verbose_failures;
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
        sandbox_options: Arc::new(Options::<SandboxOptions>::get_defaults()),
        execution_options: Arc::new(execution_options),
        local_execution_options: Arc::new(Options::<LocalExecutionOptions>::get_defaults()),
        remote_options: Arc::new(Options::get_defaults()),
        dynamic_execution_options: Arc::new(Options::get_defaults()),
        worker_options: Arc::new(Options::get_defaults()),
        resource_manager: ResourceManager::new(),
        reporter: reporter.clone() as Arc<dyn EventHandler>,
        package_path_entries: vec![],
        repo_contents_cache_path: None,
        test_arguments: Vec::new(),
    };
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1010 ExecutionTool.configureResourceManager
    configure_resource_manager(&env.resource_manager, &env.execution_options);
    let mut sandbox_module = SandboxModule::new();
    sandbox_module.before_command(&env);
    let registry =
        build_spawn_strategy_registry(&env, &mut sandbox_module).map_err(|e| format!("{e:?}"))?;
    let mut contexts = ActionContexts::default();
    contexts.spawn_strategy_registry = Some(Arc::new(registry));
    contexts.spawn_log_context = spawn_log_context.clone();
    contexts.spawn_cache = Some(blaze_exec::exec::action_execution_context::no_cache());
    register_contexts(&mut contexts, &env);
    let executor = Arc::new(SkyframeActionExecutor::new(SkyframeActionExecutorConfig {
        exec_root: config.exec_root.clone(),
        action_out_dir,
        reporter,
        contexts,
        client_env: config.client_env.clone(),
        keep_going: config.keep_going,
        verbose_failures: config.verbose_failures,
        output_permissions: OutputPermissions::READONLY,
    }));
    Ok(PreparedExecution {
        executor,
        sandbox_module,
        spawn_log_context,
    })
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
        ..EvaluationContext::default()
    };
    let evaluation = evaluator.evaluate(&roots, &ctx)?;
    Ok(ExecutionPhaseResult {
        evaluation,
        start,
        stop: Instant::now(),
    })
}

/// The `BuildSummaryStatsModule.buildComplete` lines: `Elapsed time: ..., Critical Path: ...` and
/// the process summary.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
pub fn report_build_summary_stats(
    reporter: &dyn EventHandler,
    collector: &ExecutionEventCollector,
    elapsed_seconds: f64,
    critical_path_seconds: f64,
) {
    let mut items = vec![format!("Elapsed time: {elapsed_seconds:.3}s")];
    // Java: com/google/devtools/build/lib/runtime/AggregatedCriticalPath.java:113 AggregatedCriticalPath.toStringSummaryNoRemote
    // PORT: CriticalPathComputer is not ported; the caller passes the longest action chain it
    // measured. The line is time-dependent and not compared by the oracle.
    items.push(format!("Critical Path: {critical_path_seconds:.2}s"));
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
