//! `blaze cquery`: `runtime/commands/CqueryCommand.java` (the body is in
//! `blaze_cquery::commands::cquery_command`).

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::top_level::create_build_options;
use blaze_buildtool::buildtool::execution_tool::{
    ActionCacheOptions, ExecutionToolConfig, prepare_for_execution_with_action_cache,
};
use blaze_cquery::commands::PostAnalysisCommandEnvironment;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::CQUERY_COMMAND;
use crate::commands::query::{
    AfterCommand, create_skyframe_executor, query_output_stream, reporter,
    to_runtime_detailed_exit_code,
};
use crate::options_result::OptionsParsingResult;

/// Handles the 'cquery' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:77 CqueryCommand
pub struct CqueryCommand;

impl BlazeCommand for CqueryCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &CQUERY_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:79 CqueryCommand.editOptions
    fn edit_options(&self, options_parser: &mut blaze_options::OptionsParser) {
        blaze_cquery::commands::cquery_command::edit_options(options_parser);
    }

    // Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:118 CqueryCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // BuildTool.buildTargets: runtime.createBuildOptions(request).
        // PORT: the executor takes the build options as its baseline when it is created (see
        // blaze_sky_top::skyframe_executor).
        // The options carry the MODULE.bazel flag aliases the dispatcher injected
        // (BlazeCommandDispatcher.execExclusively).
        let build_options = create_build_options(&BazelRuleClassProvider::create(), options);
        let skyframe_executor = match create_skyframe_executor(env, Some(build_options)) {
            Ok(e) => e,
            Err(result) => return result,
        };
        let _after_command = AfterCommand(&skyframe_executor);
        // new ExecutionTool(env, request) (see `construct_execution_tool`).
        // BuildSummaryStatsModule's state for this request: executorInit when the ExecutionTool
        // is constructed, buildComplete in stopRequest.
        let collector = blaze_buildtool::buildtool::execution_tool::ExecutionEventCollector::new();
        let execution_tool = || construct_execution_tool(env, options, &collector);
        // BuildRequest.getOptionsDescription(): the command's options.
        if let Some(bep) = env.get_bep() {
            bep.set_build_request_options(options);
        }
        // The build event stream's posting points (BuildTool's events).
        let bep = env.get_bep().cloned();
        let build_event_hooks = crate::commands::cquery::PostAnalysisBuildEventHooks {
            ui_event_handler: env.get_ui_event_handler().cloned(),
            bep: bep
                .as_deref()
                .map(|b| b as &dyn blaze_sky_top::build_tool::BuildEventHooks),
        };
        let command_env = PostAnalysisCommandEnvironment {
            skyframe_executor: &skyframe_executor,
            reporter: reporter(env),
            output: query_output_stream(env),
            relative_working_directory: env.get_relative_working_directory().clone(),
            working_directory: env.get_working_directory().clone(),
            product_name: env.get_runtime().get_product_name().to_owned(),
            tracks_state_for_incrementality: env.tracks_state_for_incrementality(),
            available_processors: crate::local_resources::available_processors(),
            stop_request: &|detailed_exit_code| {
                crate::commands::build::stop_request_without_execution(
                    env,
                    detailed_exit_code,
                    &collector,
                )
            },
            options,
            execution_tool: &execution_tool,
            build_event_hooks: Some(&build_event_hooks),
        };
        let detailed_exit_code = blaze_cquery::commands::cquery_command::exec(&command_env);
        BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(detailed_exit_code))
    }
}

/// The subscribers of `BuildTool`'s events for the commands whose `BuildTool` is constructed in
/// `blaze_cquery` (`cquery`, `aquery`): the UI's (`UiEventHandler.buildStarted` on
/// `BuildStartingEvent`), then the build event stream's.
pub(crate) struct PostAnalysisBuildEventHooks<'a> {
    pub(crate) ui_event_handler: Option<std::sync::Arc<blaze_events::ui::UiEventHandler>>,
    pub(crate) bep: Option<&'a dyn blaze_sky_top::build_tool::BuildEventHooks>,
}

impl blaze_sky_top::build_tool::BuildEventHooks for PostAnalysisBuildEventHooks<'_> {
    fn build_starting(&self, request: &blaze_sky_top::build_tool::BuildRequest) {
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:658 UiEventHandler.buildStarted
        if let Some(handler) = &self.ui_event_handler {
            handler.build_started();
        }
        if let Some(bep) = self.bep {
            bep.build_starting(request);
        }
    }

    fn target_patterns_evaluated(
        &self,
        request: &blaze_sky_top::build_tool::BuildRequest,
        skyframe_executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
        target_pattern_phase_value: &blaze_sky_top::target_pattern_phase::TargetPatternPhaseValue,
    ) {
        if let Some(bep) = self.bep {
            bep.target_patterns_evaluated(request, skyframe_executor, target_pattern_phase_value);
        }
    }

    fn project_evaluated(&self) {
        if let Some(bep) = self.bep {
            bep.project_evaluated();
        }
    }

    fn ensure_build_info_posted(&self) {
        if let Some(bep) = self.bep {
            bep.ensure_build_info_posted();
        }
    }

    fn no_execution(&self) {
        if let Some(bep) = self.bep {
            bep.no_execution();
        }
    }

    fn analysis_complete(
        &self,
        request: &blaze_sky_top::build_tool::BuildRequest,
        skyframe_executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
        target_pattern_phase_value: &blaze_sky_top::target_pattern_phase::TargetPatternPhaseValue,
        analysis_result: &blaze_sky_top::build_view::AnalysisResult,
    ) {
        if let Some(bep) = self.bep {
            bep.analysis_complete(
                request,
                skyframe_executor,
                target_pattern_phase_value,
                analysis_result,
            );
        }
    }
}

fn options_or_default<T: blaze_options::OptionsClass>(options: &OptionsParsingResult) -> T {
    options
        .get_options::<T>()
        .unwrap_or_else(Options::<T>::get_defaults)
}

/// `new ExecutionTool(env, request)` for cquery and aquery: Java constructs the tool after the
/// analysis (BuildTool.java:520) and its constructor registers the spawn strategies, failing
/// with an `AbruptExitException` for an unknown `--spawn_strategy` / `--strategy` value.
/// The executor setup is built as `build` builds it (commands/build.rs).
///
/// PORT: blaze_buildtool exposes the constructor only together with `prepareForExecution`
/// (`prepare_for_execution_with_action_cache`), which also recreates the action log directory
/// (`<output_base>/action_outs`); the action cache is not loaded. No spawn log context, no
/// test action context and no output filter are given: none of them takes part in the
/// constructor's checks. The remote module's contributions are not built (its support is private
/// to commands/build.rs): its strategy is registered under the same name, "remote", with or
/// without remote caching or execution (RemoteActionContextProvider.registerRemoteSpawnStrategy),
/// so the remote options are passed with the cache, executor, output service and downloader
/// cleared (the placeholder provider) and the remote-local fallback identifier kept; the
/// validation of strategy names is the same.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:151 ExecutionTool.<init>
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:520 BuildTool.buildTargetsWithoutMergedAnalysisExecution
pub(crate) fn construct_execution_tool(
    env: &CommandEnvironment,
    options: &OptionsParsingResult,
    collector: &blaze_buildtool::buildtool::execution_tool::ExecutionEventCollector,
) -> Result<(), blaze_query_engine::DetailedExitCode> {
    let build_request_options: BuildRequestOptions = options_or_default(options);
    let execution_options: ExecutionOptions = options_or_default(options);
    let test_options: TestConfigurationTestOptions = options_or_default(options);
    let keep_going = options
        .get_options::<blaze_optdefs::command::keep_going_option::KeepGoingOption>()
        .map(|o| o.keep_going)
        .unwrap_or(false);
    let directories = env.get_directories();
    let exec_root = env.get_exec_root();
    let jobs = if build_request_options.jobs > 0 {
        build_request_options.jobs as usize
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    let remote_options = options
        .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
        .map(|mut o| {
            o.remote_executor = None;
            o.remote_cache = None;
            o.disk_cache = None;
            o.remote_output_service = None;
            o.remote_downloader = None;
            o
        });
    crate::commands::build::check_strategy_policy_is_default(env);
    let config = ExecutionToolConfig {
        file_system: env.get_output_base().get_file_system().clone(),
        output_base: env.get_output_base().clone(),
        exec_root: exec_root.clone(),
        install_base: directories.get_install_base().clone(),
        client_env: env.get_client_env().clone(),
        keep_going,
        verbose_failures: execution_options.verbose_failures,
        jobs,
        execution_options: Some(execution_options.clone()),
        sandbox_options: Some(options_or_default::<SandboxOptions>(options)),
        local_execution_options: Some(options_or_default::<LocalExecutionOptions>(options)),
        remote_options,
        dynamic_execution_options: options.get_options::<
            blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions,
        >(),
        worker_options: options
            .get_options::<blaze_optdefs::misc::worker_options::WorkerOptions>(),
        package_path_entries: Vec::new(),
        repo_contents_cache_path: None,
        test_arguments: test_options.test_arguments.clone(),
        action_temps_directory: directories.get_action_temps_directory(&exec_root),
        writable_outputs: options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .map(|o| o.experimental_writable_outputs)
            .unwrap_or(false),
        output_filter: None,
        remote: Default::default(),
    };
    let prepared = match prepare_for_execution_with_action_cache(
        &config,
        reporter(env),
        None,
        &|_, _| {},
        ActionCacheOptions {
            use_action_cache: false,
            ..ActionCacheOptions::default()
        },
    ) {
        Ok(prepared) => prepared,
        // Not a constructor failure in Java (prepareForExecution runs only for builds).
        Err(e) => panic!(
            "cannot set up the executor for {}: {e}",
            env.get_command_name()
        ),
    };
    // ExecutionTool.<init>: each module's executorInit (BuildSummaryStatsModule's registers the
    // CriticalPathComputer) runs before the spawn strategy registry is built.
    // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:91 BuildSummaryStatsModule.executorInit
    collector.executor_init(
        execution_options.enable_critical_path_profiling,
        execution_options.stats_summary,
        blaze_buildtool::buildtool::execution_tool::can_be_shared_with_executor(
            prepared.executor.clone(),
        ),
    );
    // catch (AbruptExitException e)
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:999 BuildTool.processRequest
    prepared.check_execution_tool_created().map_err(|e| {
        let failure_detail = e.failure_detail;
        blaze_query_engine::DetailedExitCode::of_exit_code(
            blaze_query_engine::DetailedExitCode::get_exit_code_for(&failure_detail),
            failure_detail,
        )
    })
}
