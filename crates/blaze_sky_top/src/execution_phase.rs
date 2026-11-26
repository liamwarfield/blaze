//! The execution side of the command's `SkyframeExecutor`: what `ExecutionTool.<init>` /
//! `prepareForExecution` set up (executor, action contexts, action cache) and the execution
//! SkyFunctions registered next to analysis in the single evaluator
//! (`SkyframeExecutor.skyFunctions`: ACTION_EXECUTION, ARTIFACT, TARGET_COMPLETION,
//! TEST_COMPLETION, BUILD_DRIVER).
//!
//! PORT: Java's `SkyframeExecutor` registers the execution functions once per server and
//! `ExecutionTool` configures the `SkyframeActionExecutor` per build after analysis. Blaze
//! builds one evaluator per command, so a command that executes hands an [`ExecutionSetup`] to
//! `SkyframeExecutor::create`, which prepares the executor (`prepareForExecution`: action output
//! directory, sandbox module, spawn strategies, action contexts, action cache) before analysis
//! and registers the execution functions with it. The effects are the same; they happen earlier.

use std::sync::Arc;

use blaze_analysis::analysis_services::AnalysisServices;
use blaze_buildtool::buildtool::execution_tool::{
    ActionCacheOptions, ExecutionToolConfig, PreparedExecution,
    prepare_for_execution_with_action_cache,
};
use blaze_events::ExtendedEventHandler;
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::single_build_file_cache::SingleBuildFileCache;
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_sky_exec::skyframe_action_executor::ActionContexts;
use blaze_skyframe::EvaluatorBuilder;
use blaze_vfs::Path;

use crate::execution_seam::SkyframeAnalysisSeam;

/// Registers the action contexts the command's modules contribute (Java
/// `BlazeModule.registerActionContexts`); errors are `AbruptExitException` messages.
pub type RegisterContexts =
    Box<dyn Fn(&mut ActionContexts, &ExecCommandEnvironment) -> Result<(), String> + Send + Sync>;

/// What a command that executes actions hands the executor.
pub struct ExecutionSetup {
    pub config: ExecutionToolConfig,
    /// `--use_action_cache`.
    pub use_action_cache: bool,
    /// `--explain` is set.
    pub explain: bool,
    /// The `SpawnLogModule`'s context (`--execution_log_*_file`).
    pub spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    /// The command modules' contexts beyond the ones registered here (test strategy, ...).
    pub register_contexts: RegisterContexts,
    /// `--embed_label`, `--workspace_status_command` (`BazelWorkspaceStatusModule`).
    pub embed_label: String,
    pub workspace_status_command: blaze_vfs::PathFragment,
    /// The workspace (`BazelWorkspaceStatusModule` runs the status command there).
    pub workspace: Path,
    /// `--experimental_sibling_repository_layout`.
    pub sibling_repository_layout: bool,
}

/// The prepared execution of the command.
pub struct ExecutionState {
    pub prepared: PreparedExecution,
    /// The `SandboxModule` of this command, until [`ExecutionState::after_command`].
    sandbox_module: parking_lot::Mutex<Option<blaze_exec::sandbox::sandbox_module::SandboxModule>>,
    pub seam: Arc<SkyframeAnalysisSeam>,
    pub config: ExecutionToolConfig,
}

impl ExecutionState {
    /// `SandboxModule.afterCommand`: cleans the sandbox base (Java runs it from
    /// `BlazeRuntime.afterCommand` for every module).
    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:452 SandboxModule.afterCommand
    pub fn after_command(&self) {
        if let Some(mut sandbox_module) = self.sandbox_module.lock().take() {
            sandbox_module.after_command();
        }
    }
}

/// Registers the `ExecutableAction` implementations of every crate that defines action classes
/// (Java: virtual dispatch on `Action.execute`). Idempotent.
pub fn register_executable_actions() {
    blaze_exec::analysis::actions::register();
    blaze_rules::executable_actions::register_executable_actions();
    blaze_analysis::test::test_runner_action_execution::register_executable_actions();
}

/// `ExecutionTool.<init>` + `prepareForExecution` and the registration of the execution
/// SkyFunctions on `builder`.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:600 SkyframeExecutor.skyFunctions
pub fn prepare_execution(
    builder: EvaluatorBuilder,
    setup: ExecutionSetup,
    services: &Arc<AnalysisServices>,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<(EvaluatorBuilder, ExecutionState), String> {
    register_executable_actions();
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:158 ExecutionTool.<init>
    setup
        .config
        .exec_root
        .create_directory_and_parents()
        .map_err(|e| format!("Execroot creation failed: {e}"))?;
    let workspace_name = services.directories.workspace_name.clone();
    let artifact_factory = services.artifact_factory.clone();
    let registration_error: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);
    let register = |contexts: &mut ActionContexts, env: &ExecCommandEnvironment| {
        // Java: the Bazel modules' registerActionContexts (BazelRulesModule / StandaloneModule
        // FileWriteStrategy, LocalTemplateExpansionStrategy, SymlinkTreeStrategy, ...).
        for (type_id, ctx) in
            blaze_exec::analysis::actions::default_action_contexts(&workspace_name)
        {
            contexts.register_raw(type_id, ctx);
        }
        // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java BazelWorkspaceStatusModule.registerActionContexts
        use blaze_exec::analysis::actions::bazel_workspace_status_module as ws;
        let status: Arc<dyn ws::WorkspaceStatusActionContext> =
            Arc::new(ws::BazelWorkspaceStatusActionContext::new(
                ws::WorkspaceStatusOptions {
                    embed_label: setup.embed_label.clone(),
                    workspace_status_command: setup.workspace_status_command.clone(),
                },
                env.client_env.clone(),
                setup.workspace.clone(),
            ));
        let (type_id, ctx) = ws::workspace_status_context_entry(status);
        contexts.register_raw(type_id, ctx);
        // Java: CppIncludeExtractionContext / CppIncludeScanningContext (BazelRulesModule).
        for (type_id, ctx) in blaze_rules::executable_actions::executable_action_contexts(
            artifact_factory.clone(),
            setup.sibling_repository_layout,
        ) {
            contexts.register_raw(type_id, ctx);
        }
        if let Err(e) = (setup.register_contexts)(contexts, env) {
            *registration_error.lock() = Some(e);
        }
    };
    let mut prepared = prepare_for_execution_with_action_cache(
        &setup.config,
        reporter,
        setup.spawn_log_context.clone(),
        &register,
        ActionCacheOptions {
            use_action_cache: setup.use_action_cache,
            explain: setup.explain,
            artifact_factory: Some(artifact_factory.clone()),
            // ExecutionOptions --check_up_to_date / --check_tests_up_to_date.
            check_up_to_date: setup
                .config
                .execution_options
                .as_ref()
                .is_some_and(|o| o.check_up_to_date),
            test_check_up_to_date: setup
                .config
                .execution_options
                .as_ref()
                .is_some_and(|o| o.test_check_up_to_date),
            ..ActionCacheOptions::default()
        },
    )?;
    if let Some(e) = registration_error.into_inner() {
        return Err(e);
    }
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1044 CommandEnvironment.getFileCache
    // (`new SingleBuildFileCache(execRoot, PathFragment.create(directories
    // .getRelativeOutputPath()), fs, syscallCache)`), which ExecutionTool hands to
    // SkyframeActionExecutor.configure.
    prepared
        .executor
        .configure_file_cache(Arc::new(SingleBuildFileCache::new(
            setup.config.exec_root.clone(),
            // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:257 BlazeDirectories.getRelativeOutputPath
            blaze_vfs::PathFragment::create(&format!("{}-out", services.directories.product_name)),
        )));
    let seam = Arc::new(SkyframeAnalysisSeam::new(services.clone()));
    // SequencedSkyframeExecutor.getGraphInconsistencyReceiverForCommand: a
    // RewindableGraphInconsistencyReceiver with --rewind_lost_inputs, else THROWING.
    // PORT: the executor answers `rewindingEnabled()` only when the remote module gave it a
    // lost-inputs policy; without one no input can be lost, so no inconsistency can arise from
    // rewinding and THROWING behaves the same.
    // Java: com/google/devtools/build/lib/skyframe/SequencedSkyframeExecutor.java:321 SequencedSkyframeExecutor.getGraphInconsistencyReceiverForCommand
    let builder = if prepared.executor.rewinding_enabled() {
        builder.graph_inconsistency_receiver(Arc::new(
            blaze_sky_exec::rewinding::rewindable_graph_inconsistency_receiver::RewindableGraphInconsistencyReceiver::new(),
        ))
    } else {
        builder
    };
    let builder = blaze_sky_exec::register_execution_functions(
        builder,
        prepared.executor.clone(),
        seam.clone(),
        None,
    );
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:940 SkyframeExecutor.skyFunctions
    // (`SkyFunctions.ACTION_TEMPLATE_EXPANSION, new ActionTemplateExpansionFunction(actionKeyContext)`).
    let action_template_expansion = Arc::new(
        crate::action_template_expansion_function::ActionTemplateExpansionFunction {
            skyframe_action_executor: prepared.executor.clone(),
            seam: seam.clone(),
        },
    );
    let builder = builder.typed_function::<
        blaze_sky_exec::action_template_expansion_value::ActionTemplateExpansionKey,
        _,
    >(move |k, env| action_template_expansion.compute(k, env));
    // The sandbox module lives until afterCommand (see ExecutionState::after_command).
    let sandbox_module = std::mem::replace(
        &mut prepared.sandbox_module,
        blaze_exec::sandbox::sandbox_module::SandboxModule::new(),
    );
    Ok((
        builder,
        ExecutionState {
            sandbox_module: parking_lot::Mutex::new(Some(sandbox_module)),
            prepared,
            seam,
            config: setup.config,
        },
    ))
}
