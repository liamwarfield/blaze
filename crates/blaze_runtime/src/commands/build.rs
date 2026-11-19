//! `blaze build` and `blaze test`: `runtime/commands/BuildCommand.java`,
//! `runtime/commands/TestCommand.java`, and the parts of `buildtool/BuildTool.java`
//! (`processRequest` / `stopRequest`), `buildtool/ExecutionTool.java` (`executeBuild`) and the
//! modules (`SpawnLogModule`, `BuildSummaryStatsModule`, the UI's `buildComplete`) that run
//! around them.
//!
//! PORT: Bazel 9 merges analysis and execution (Skymeld). Blaze analyzes the top-level targets
//! first (`BuildView.update`, which prints `Analyzed ...` and `Found ...`) and then evaluates the
//! `BuildDriverKey`s, whose `CONFIGURED_TARGET` dependencies are already done; the evaluation of
//! every node is the same, only the interleaving of the two phases differs.

use std::cell::{Cell, RefCell};
use std::io::Write as _;
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ActionLookupKeyRef;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_analysis::providers::output_group_info::ValidationMode;
use blaze_bep::events::buildtool_events::BuildInterruptedEvent;
use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::top_level::create_build_options;
use blaze_buildtool::buildtool::build_result::{BuildResult, ConfiguredTargetRef};
use blaze_buildtool::buildtool::build_result_printer::{
    BuildRequestView, BuildResultPrinter, OutputFileGeneratingRule, TopLevelArtifactContext,
};
use blaze_buildtool::buildtool::execution_tool::{
    ExecutionEventCollector, ExecutionToolConfig, build_complete_event, evaluate_build_driver_keys,
    report_build_summary_stats,
};
use blaze_buildtool::buildtool::output_directory_links_utils::{
    BlazeDirectoriesView, SymlinkCreationResult, create_output_directory_links, get_symlink_prefix,
};
use blaze_buildtool::buildtool::symlink_forest::SymlinkForest;
use blaze_buildtool::runtime::test_command_support::{
    TestCommandSupport, get_exit_code, make_test_log_path_formatter,
};
use blaze_events::{Clock, Event, EventHandler, ExtendedEventHandler, JavaClock, OutErr};
use blaze_exec::actions::action_execution::ActionExecutionException;
use blaze_exec::exec::spawn_log_module::{SpawnLogModule, SpawnLogModuleEnv};
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::build_request_options::{BuildRequestOptions, ConvenienceSymlinksMode};
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::command::workspace_status_action::WorkspaceStatusActionOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_analysis::top_level_status_events::{
    TopLevelTargetAnalyzedEvent, TopLevelTargetSkippedEvent,
};
use blaze_sky_exec::artifact_function::ArtifactNestedSetEvalException;
use blaze_sky_exec::build_driver_function::TopLevelSkyValue;
use blaze_sky_exec::completion_function::InputFileErrorException;
use blaze_sky_exec::events::TestAnalyzedEvent;
use blaze_sky_exec::keys::{AspectCompletionKey, BuildDriverKey, TargetCompletionKey, TestType};
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_sky_top::build_tool::{
    AnalysisPostProcessor, BuildRequest, BuildTool, BuildToolError, ExecutionPhase,
};
use blaze_sky_top::build_view::{AnalysisResult, construct_label_to_target_map};
use blaze_sky_top::execution_phase::ExecutionSetup;
use blaze_sky_top::execution_seam::{
    BuildDriverKeyTestContext, build_driver_aspect_keys, build_driver_keys,
    configured_aspect_views, configured_target_views, get_explicit_target_patterns,
    top_level_output_groups, validation_mode,
};
use blaze_sky_top::skyframe_executor::{SkyframeExecutor, get_configuration_from_graph};
use blaze_sky_top::target_pattern_phase::TargetPatternPhaseValue;
use blaze_vfs::{Path, PathFragment};

#[path = "build_remote.rs"]
pub(crate) mod build_remote;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::{BUILD_COMMAND, TEST_COMMAND};
use crate::commands::bep::BepCommandSupport;
use crate::commands::query::{
    AfterCommand, create_skyframe_executor_with_execution, reporter, to_runtime_detailed_exit_code,
};
use crate::commands::target_patterns_helper;
use crate::directories::BlazeDirectories;
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::options_result::OptionsParsingResult;

/// Handles the 'build' command on the Blaze command line, including targets named by arguments
/// passed to Blaze.
// Java: com/google/devtools/build/lib/runtime/commands/BuildCommand.java:68 BuildCommand
pub struct BuildCommand;

impl BlazeCommand for BuildCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &BUILD_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/BuildCommand.java:84 BuildCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        run_build(env, options, false)
    }
}

/// Handles the 'test' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:75 TestCommand
pub struct TestCommand;

impl BlazeCommand for TestCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &TEST_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:98 TestCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        run_build(env, options, true)
    }
}

fn options_or_default<T: blaze_options::OptionsClass>(options: &OptionsParsingResult) -> T {
    options
        .get_options::<T>()
        .unwrap_or_else(Options::<T>::get_defaults)
}

/// `AbruptExitException.getDetailedExitCode()` of a module's exception: Java's explicit
/// `ExitCode` when the module passed one (`DetailedExitCode.of(exitCode, failureDetail)`, e.g.
/// `RemoteModule.createExitException`), else the failure detail's metadata.
// Java: com/google/devtools/build/lib/util/AbruptExitException.java:46 AbruptExitException.getDetailedExitCode
pub(crate) fn abrupt_exit_detailed_exit_code(
    e: blaze_exec::exec::spawn_strategy_registry::AbruptExitException,
) -> DetailedExitCode {
    match e.exit_code {
        Some(code) => DetailedExitCode::of_exit_code(
            ExitCode::for_code(code)
                .unwrap_or_else(|| panic!("No ExitCode for numericExitCode {code}")),
            e.failure_detail,
        ),
        None => DetailedExitCode::of(e.failure_detail),
    }
}

/// `BuildCommand.exec` / `TestCommand.doTest` with the Build Event Protocol modules around them:
/// `BuildEventServiceModule.beforeCommand` before, `env.precompleteCommand` (the
/// `BuildMetricsEvent`) and `BuildEventServiceModule.afterCommand` after.
///
/// PORT: Java runs the module hooks in `BlazeCommandDispatcher` / `BlazeRuntime.afterCommand`
/// around `command.exec`; the BEP of `build` and `test` lives here.
fn run_build(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
    run_tests: bool,
) -> BlazeCommandResult {
    // RemoteModule.beforeCommand: the dispatcher runs the modules' beforeCommand in module order
    // (RemoteModule before BuildEventServiceModule) and exits with the first module's
    // AbruptExitException, reported as `ERROR: <message>`.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:491 BlazeCommandDispatcher.execExclusively
    let remote = match build_remote::before_command(env, options) {
        Ok(remote) => remote.map(Arc::new),
        Err(e) => {
            env.get_reporter()
                .handle(&Event::error(e.get_message().to_owned()));
            return BlazeCommandResult::detailed_exit_code(abrupt_exit_detailed_exit_code(e));
        }
    };
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:337 BuildEventServiceModule.beforeCommand
    let bep = BepCommandSupport::before_command(
        env,
        options,
        &build_remote::build_event_artifact_uploader_factory_map(remote.as_ref()),
    );
    let result = run_build_with_bep(env, options, run_tests, bep.as_ref(), remote.as_ref());
    // BlazeRuntime.afterCommand: CommandPrecompleteEvent, then each module's afterCommand in
    // module order; each AbruptExitException is reported as `ERROR: <message>` and the more
    // important exit code (the first on a tie) replaces the result unless that is an
    // infrastructure failure.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:750 BlazeRuntime.afterCommand
    //
    // Order of the remote module and the BEP transports (review finding N8), from the pin:
    // - The streamer starts the transports' close (and with it the artifact uploads) itself when
    //   the last announced event is posted.
    //   Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:678 BuildEventStreamer.buildEvent
    //   Java: com/google/devtools/build/lib/runtime/BuildEventStreamer.java:476 BuildEventStreamer.close
    // - Module order: RemoteModule, ..., BlockWaitingModule, then BazelBuildEventServiceModule
    //   ("needs to come after BlockWaitingModule so that the BES isn't closed until the
    //   background tasks maintained by the module have completed").
    //   Java: com/google/devtools/build/lib/bazel/Bazel.java:60 Bazel.BAZEL_MODULES
    //   Java: com/google/devtools/build/lib/bazel/Bazel.java:92 Bazel.BAZEL_MODULES
    // - RemoteModule.afterCommand resets the uploader factory delegate and submits
    //   afterCommandTask (RemoteActionContextProvider.afterCommand ->
    //   RemoteExecutionService.shutdown) to the BlockWaitingModule, whose afterCommand waits for
    //   it. shutdown calls combinedCache.shutdownNow() only when the build was interrupted;
    //   otherwise it waits for the background tasks and release()s the cache, which the BEP
    //   uploader keeps alive with its own retain().
    //   Java: com/google/devtools/build/lib/remote/RemoteModule.java:1098 RemoteModule.afterCommand
    //   Java: com/google/devtools/build/lib/runtime/BlockWaitingModule.java:81 BlockWaitingModule.afterCommand
    //   Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:244 RemoteActionContextProvider.afterCommand
    //   Java: com/google/devtools/build/lib/remote/RemoteExecutionService.java:2117 RemoteExecutionService.shutdown
    //   Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderFactory.java:70 ByteStreamBuildEventArtifactUploaderFactory.create
    // - BuildEventServiceModule.afterCommand then waits for the transports' close futures
    //   (closeBepTransports).
    //   Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:672 BuildEventServiceModule.afterCommand
    // So the remote module's afterCommand (with its afterCommandTask) runs first and the wait for
    // the BEP transports second, as here.
    if let Some(bep) = &bep {
        // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:802 CommandEnvironment.precompleteCommand
        bep.command_precomplete();
    }
    let mut module_exit_code: Option<DetailedExitCode> = None;
    // RemoteModule.afterCommand and its BlockWaitingModule task (reports its own error).
    let remote_exit = match &remote {
        Some(remote) => remote.after_command(env.get_reporter().as_ref()),
        None => {
            build_remote::placeholder_after_command(env, options);
            None
        }
    };
    if let Some(e) = remote_exit {
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:780 BlazeRuntime.afterCommand
        module_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
            module_exit_code,
            Some(abrupt_exit_detailed_exit_code(e)),
        );
    }
    // BuildEventServiceModule.afterCommand.
    if let Some(bep) = &bep
        && let Err(e) = bep.after_command()
    {
        env.get_reporter()
            .handle(&Event::error(e.get_message().to_owned()));
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:780 BlazeRuntime.afterCommand
        module_exit_code = DetailedExitCode::choose_more_important_with_first_if_tie(
            module_exit_code,
            Some(e.detailed_exit_code),
        );
    }
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:795 BlazeRuntime.afterCommand
    match module_exit_code {
        Some(code) if !result.get_exit_code().is_infrastructure_failure() => {
            BlazeCommandResult::detailed_exit_code(code)
        }
        _ => result,
    }
}

/// `BuildCommand.exec` / `TestCommand.doTest`: builds (and tests) the targets of the residue.
fn run_build_with_bep(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
    run_tests: bool,
    bep: Option<&Arc<BepCommandSupport>>,
    remote: Option<&Arc<build_remote::RemoteSupport>>,
) -> BlazeCommandResult {
    // BuildRequest.Builder.setStartTimeMillis(env.getCommandStartTime())
    let start_time_millis = env.get_command_start_time();
    let reporter = reporter(env);
    let command_name = env.get_command_name();

    let build_request_options: BuildRequestOptions = options_or_default(options);
    let execution_options: ExecutionOptions = options_or_default(options);
    let test_summary_options: TestSummaryOptions = options_or_default(options);
    let build_language_options: BuildLanguageOptions = options_or_default(options);
    let ui_options: UiOptions = options_or_default(options);
    let test_options: TestConfigurationTestOptions = options_or_default(options);
    let status_options: WorkspaceStatusActionOptions = options_or_default(options);
    let keep_going = options
        .get_options::<blaze_optdefs::command::keep_going_option::KeepGoingOption>()
        .map(|o| o.keep_going)
        .unwrap_or(false);

    // Flags whose implementation is not ported: fail before anything runs.
    if execution_options.stats_summary {
        // The other summary of BuildSummaryStatsModule.buildComplete (total action wall time,
        // preparation and execution times).
        blaze_util::unported!(
            "BuildSummaryStatsModule.buildComplete",
            "--experimental_stats_summary"
        );
    }
    if build_request_options
        .aquery_dump_after_build_format
        .is_some()
        || build_request_options
            .aquery_dump_after_build_output_file
            .is_some()
    {
        blaze_util::unported!(
            "BuildTool.dumpSkyframeStateAfterBuild",
            "--experimental_aquery_dump_after_build_format / _output_file"
        );
    }
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:162 TestCommand.doTest
    if run_tests
        && options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .is_some_and(|o| o.collect_code_coverage)
        && !options.contains_explicit_option("instrumentation_filter")
    {
        // builder.setNeedsInstrumentationFilter(true)
        blaze_util::unported!("InstrumentationFilterSupport.computeInstrumentationFilter");
    }
    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:78 BazelCoverageReportModule.getCoverageReportFactory
    if run_tests
        && options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .is_some_and(|o| o.collect_code_coverage)
        && options
            .get_options::<blaze_optdefs::misc::bazel_coverage_report_module::BazelCoverageReportModuleOptions>()
            .is_some_and(|o| {
                o.combined_report != blaze_optdefs::misc::bazel_coverage_report_module::ReportType::NONE
            })
    {
        blaze_util::unported!(
            "BazelCoverageReportModule.getCoverageReportFactory",
            "--collect_code_coverage with --combined_report (the coverage report action)"
        );
    }

    // The modules subscribed to the command's event bus.
    let collector = Arc::new(ExecutionEventCollector::new());
    env.get_event_bus().register(collector.clone());
    let ui_test_summaries = Arc::new(UiTestSummaryCounter::default());
    env.get_event_bus().register(ui_test_summaries.clone());
    // CommandEnvironment.<init>: eventBus.register(buildResultListener)
    let build_result_listener = Arc::new(BuildResultListener::default());
    env.get_event_bus().register(build_result_listener.clone());
    let test_support = run_tests.then(|| {
        TestCommandSupport::new(
            Arc::new(execution_options.clone()),
            Arc::new(test_summary_options.clone()),
            env.get_event_bus().clone() as Arc<dyn blaze_events::EventBus>,
            reporter.as_ref(),
        )
    });
    if let Some(test_support) = &test_support {
        env.get_event_bus().register(test_support.subscriber());
    }

    // Java: com/google/devtools/build/lib/runtime/commands/BuildCommand.java:74 BuildCommand.exec
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:131 TestCommand.doTest
    let targets: Vec<String> = match target_patterns_helper::read_from(env, options) {
        Ok(targets) => targets,
        Err(e) => {
            reporter.handle(&Event::error(e.get_message()));
            return BlazeCommandResult::failure_detail(e.get_failure_detail());
        }
    };
    if !run_tests && targets.is_empty() {
        let product_name = env.get_runtime().get_product_name();
        reporter.handle(&Event::warn(format!(
            "Usage: {product_name} build <options> <targets>.\
             \nInvoke `{product_name} help build` for full description of usage and options.\
             \nYour request is correct, but requested an empty set of targets. \
             Nothing will be built."
        )));
    }

    let directories = env.get_directories().clone();
    let workspace = env
        .get_workspace()
        .cloned()
        .expect("the build command runs in a workspace");
    let exec_root = env.get_exec_root();
    let file_system = env.get_output_base().get_file_system().clone();

    // SpawnLogModule.executorInit (--execution_log_*_file).
    let mut spawn_log_module = SpawnLogModule::new();
    let spawn_log_context = match blaze_exec::exec::spawn_log_module::create_spawn_log_context(
        &mut spawn_log_module,
        &SpawnLogModuleEnv {
            execution_options: Some(&execution_options),
            // env.getOptions().getOptions(RemoteOptions.class)
            remote_options: options
                .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
                .map(Arc::new),
            sibling_repository_layout: build_language_options
                .experimental_sibling_repository_layout,
            file_system: &file_system,
            output_base: env.get_output_base(),
            exec_root: &exec_root,
            workspace: Some(&workspace),
            working_directory: env.get_working_directory(),
            workspace_name: env.get_workspace_name(),
            command_id: env.get_command_id(),
            reporter: env.get_reporter().clone() as Arc<dyn EventHandler>,
        },
    ) {
        Ok(c) => c,
        Err(e) => {
            return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(e.failure_detail));
        }
    };

    // OutputFilteringModule: the reporter's filter (see `output_filter`).
    let reporter_output_filter = output_filter(options, &build_request_options);

    let jobs = if build_request_options.jobs > 0 {
        build_request_options.jobs as usize
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    let config = ExecutionToolConfig {
        file_system: file_system.clone(),
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
        remote_options: options
            .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>(),
        dynamic_execution_options: options.get_options::<
            blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions,
        >(),
        worker_options: options
            .get_options::<blaze_optdefs::misc::worker_options::WorkerOptions>(),
        // Filled by the executor from its package locator and repository setup.
        package_path_entries: Vec::new(),
        repo_contents_cache_path: None,
        test_arguments: test_options.test_arguments.clone(),
        action_temps_directory: directories.get_action_temps_directory(&exec_root),
        writable_outputs: options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .map(|o| o.experimental_writable_outputs)
            .unwrap_or(false),
        output_filter: reporter_output_filter.clone(),
        // RemoteModule.registerSpawnStrategies / registerActionContexts / executorInit.
        remote: remote
            .map(|remote| {
                remote.contributions(
                    options
                        .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
                        .as_ref(),
                    // ExecutionTool: request.getBuildOptions().finalizeActions.
                    build_request_options.finalize_actions,
                    build_request_options.rewind_lost_inputs,
                    execution_options.remote_retry_on_transient_cache_error,
                )
            })
            // No remote option set: RemoteModule.setup's placeholder; executorInit still sets
            // the salt.
            .unwrap_or_else(|| {
                build_remote::placeholder_contributions(
                    options
                        .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
                        .as_ref(),
                )
            }),
    };
    let summary_for_contexts = Arc::new(test_summary_options.clone());
    let workspace_for_contexts = workspace.clone();
    let setup = ExecutionSetup {
        config,
        use_action_cache: build_request_options.use_action_cache,
        explain: build_request_options.explanation_path.is_some(),
        spawn_log_context,
        register_contexts: Box::new(move |contexts, exec_env| {
            // Java: StandaloneModule.registerActionContexts (the test strategy).
            blaze_buildtool::runtime::standalone_module::register_test_action_contexts(
                contexts,
                exec_env,
                &workspace_for_contexts,
                Some(summary_for_contexts.clone()),
            )
            .map_err(|e| e.to_string())
        }),
        embed_label: status_options.embed_label.clone(),
        workspace_status_command: status_options.workspace_status_command.0.clone(),
        workspace: workspace.clone(),
        sibling_repository_layout: build_language_options.experimental_sibling_repository_layout,
    };

    // BuildTool.buildTargets: runtime.createBuildOptions(request).
    let build_options = create_build_options(&BazelRuleClassProvider::create(), options);
    let skyframe_executor = match create_skyframe_executor_with_execution(
        env,
        env.get_options().clone(),
        Some(build_options),
        Some(setup),
    ) {
        Ok(e) => e,
        Err(result) => return result,
    };
    let _after_command = AfterCommand(&skyframe_executor);
    // PORT: Java's dispatcher computed the main repository mapping on the server's executor
    // (BlazeCommandDispatcher.execExclusively) and reported any error; this command's executor
    // computes it again. `test` also asks for it in TestCommand.doTest, whose error handling
    // this is.
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:139 TestCommand.doTest
    if let Err(e) = skyframe_executor.get_main_repo_mapping(
        keep_going,
        blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT,
        reporter.clone(),
    ) {
        return match e {
            RepositoryMappingResolutionOrInterrupted::Interrupted => {
                // BuildTool.processRequest reports an interrupted `build` as "build interrupted".
                let message = if run_tests {
                    "test command interrupted"
                } else {
                    "build interrupted"
                };
                reporter.handle(&Event::error(message));
                BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
                    ExitCode::INTERRUPTED,
                    FailureDetail {
                        message: message.to_owned(),
                        category: Some(Category::Interrupted(failure_details::Interrupted {
                            code: failure_details::interrupted::Code::Interrupted as i32,
                        })),
                    },
                ))
            }
            RepositoryMappingResolutionOrInterrupted::Resolution(e) => {
                reporter.handle(&Event::error(e.to_string()));
                BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(
                    blaze_query::compat::failure_details::from_packages(&e.detailed_exit_code),
                ))
            }
        };
    }

    // BuildRequest.Builder: checkForActionConflicts = true, reportIncompatibleTargets = true.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:88 BuildRequest.Builder
    let mut request = BuildRequest::new(command_name, options, targets, true, true);
    request.set_run_tests(run_tests);
    // SkymeldModule.beforeCommand: env.setMergedAnalysisAndExecution(...)
    let merged_analysis_and_execution = determine_if_merging_analysis_execution(
        options,
        &build_request_options,
        skyframe_executor
            .get_package_path()
            .get_path_entries()
            .len(),
    );
    request.set_merged_analysis_and_execution(merged_analysis_and_execution);

    let execution_phase = RuntimeExecutionPhase {
        reporter: reporter.clone(),
        directories: directories.clone(),
        workspace: workspace.clone(),
        workspace_name: env.get_workspace_name().to_owned(),
        product_name: env.get_runtime().get_product_name().to_owned(),
        relative_working_directory: env.get_relative_working_directory().clone(),
        build_request_options: build_request_options.clone(),
        execution_options: execution_options.clone(),
        extra_action_top_level_only: options
            .get_options::<blaze_optdefs::command::analysis_options::AnalysisOptions>()
            .map(|o| o.extra_action_top_level_only)
            .unwrap_or(false),
        collector: collector.clone(),
        event_reporter: env.get_reporter().clone(),
        explanation: build_request_options
            .explanation_path
            .as_ref()
            // BuildRequest.<init>: OptionsUtils.asShellEscapedString(options)
            .map(|path| {
                (
                    path.0.clone(),
                    crate::commands::bep::as_shell_escaped_string(options),
                )
            }),
        build_result: RefCell::new(new_build_result(
            start_time_millis,
            keep_going,
            &execution_options,
        )),
        results_shown: Cell::new(false),
        execution_tool_failed: Cell::new(false),
        build_result_listener: build_result_listener.clone(),
        request_view: RuntimeBuildRequestView {
            build_options: build_request_options.clone(),
            execution_options: execution_options.clone(),
            should_run_tests: run_tests,
            // BuildRequest.Builder.setOutErr(env.getReporter().getOutErr())
            out_err: env.get_reporter_out_err().clone(),
        },
        bep: bep.cloned(),
        remote: remote.cloned(),
    };
    let post_processor = ExecutionToolConstruction {
        execution_phase: &execution_phase,
    };
    let build_tool = BuildTool::new(
        &skyframe_executor,
        reporter.clone(),
        env.get_relative_working_directory().clone(),
        &post_processor,
    )
    .with_execution_phase(&execution_phase);
    // BuildTool.buildTargets: BuildStartingEvent. Its subscriber OutputFilteringModule sets the
    // reporter's output filter.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:288 BuildTool.buildTargets
    // Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:78 OutputFilteringModule.buildStarting
    if let Some(filter) = &reporter_output_filter {
        env.get_reporter().set_output_filter(filter.clone());
    }
    if let Some(bep) = bep {
        bep.build_starting(
            env,
            options,
            &request.targets,
            start_time_millis,
            blaze_sky_top::skyframe_executor::short_host_name(),
        );
    }
    let detailed_exit_code = to_runtime_detailed_exit_code(build_tool.process_request(&request));
    // BuildTool.processRequest, `catch (InterruptedException e)`:
    // `env.getEventBus().post(new BuildInterruptedEvent())`.
    // PORT: blaze_sky_top's BuildTool cannot name the event (blaze_bep declares it); the exit
    // code of an interrupted build is the only one it returns with the category Interrupted.
    // AggregatingTestListener.buildInterrupted is called directly (blaze_buildtool does not see
    // the event's type either).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:957 BuildTool.processRequest
    if detailed_exit_code
        .get_failure_detail()
        .is_some_and(|f| matches!(f.category, Some(Category::Interrupted(_))))
    {
        blaze_events::EventBus::post(&**env.get_event_bus(), Arc::new(BuildInterruptedEvent));
        if let Some(test_support) = &test_support {
            test_support.listener().build_interrupted();
        }
        // RemoteExecutionService.onBuildInterrupted (an event-bus subscriber).
        if let Some(remote) = remote {
            remote.build_interrupted();
        }
    }

    // BuildTool.stopRequest.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1155 BuildTool.stopRequest
    let mut build_result = execution_phase.build_result.replace(new_build_result(
        start_time_millis,
        keep_going,
        &execution_options,
    ));
    if !execution_phase.results_shown.get()
        && !execution_phase.execution_tool_failed.get()
        && merged_analysis_and_execution
        && target_patterns_were_evaluated(
            &skyframe_executor,
            &request,
            env.get_relative_working_directory(),
        )
    {
        // The finally block of BuildTool.buildTargetsWithMergedAnalysisExecution also runs when
        // the analysis (or the preparation of the execution) throws: the targets analyzed so far
        // are the build result's targets, none of them was built, and
        // ExecutionTool.nonCatastrophicFinalizations shows them as failed. The sets are empty,
        // not null, when nothing was analyzed, which is how TestCommand.doTest tells this from a
        // build that never started.
        // PORT: Java also creates the convenience symlinks there when the top-level
        // configuration exists (result.getBuildConfiguration() != null); blaze_sky_top's
        // BuildTool does not hand the configuration of a failed analysis to the command.
        // PORT: a failure of the PROJECT.scl evaluation, which Java raises before this method,
        // is not told apart (blaze_sky_top::project fails loudly for project files).
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:664 BuildTool.buildTargetsWithMergedAnalysisExecution
        let analyzed_targets = build_result_listener.get_analyzed_targets();
        let views = configured_target_views(&analyzed_targets);
        let skipped_targets = configured_target_views(&build_result_listener.get_skipped_targets());
        build_result.set_actual_targets(Some(views.clone()));
        // getAnalyzedTests(): the TestAnalyzedEvents are posted when the execution starts.
        build_result.set_test_targets(Some(Vec::new()));
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:552 ExecutionTool.nonCatastrophicFinalizations
        build_result.set_successful_targets(Some(Vec::new()));
        build_result.set_successful_aspects(Some(indexmap::IndexSet::new()));
        build_result.set_skipped_targets(Some(skipped_targets.clone()));
        let generating_rule_lookup =
            |target: &ConfiguredTargetRef| output_file_generating_rule(&analyzed_targets, target);
        BuildResultPrinter::new(
            env.get_runtime().get_product_name(),
            env.get_relative_working_directory().clone(),
        )
        .show_build_result(
            &execution_phase.request_view,
            &build_result,
            &views,
            &skipped_targets,
            &indexmap::IndexMap::new(),
            &generating_rule_lookup,
        );
    }
    build_result.set_detailed_exit_code(Arc::new(to_util_detailed_exit_code(&detailed_exit_code)));
    // The stop time has to be captured before we send the BuildCompleteEvent.
    build_result.set_stop_time(JavaClock.current_time_millis());
    // The BuildCompleteEvent subscribers: AggregatingTestListener, BuildSummaryStatsModule, the
    // UI.
    // The TestSummary events that AggregatingTestListener.buildCompleteEvent posts (targets that
    // failed to build or were skipped) are dispatched after every subscriber of the
    // BuildCompleteEvent has run (the EventBus queues an event posted from a subscriber), so the
    // UI's buildComplete does not count them: read its counter first.
    // Java: com/google/devtools/build/lib/runtime/UiStateTracker.java:481 UiStateTracker.buildComplete
    let failed_tests = ui_test_summaries
        .failed_tests
        .load(std::sync::atomic::Ordering::SeqCst);
    if let Some(test_support) = &test_support {
        test_support.build_complete_event(&build_result);
    }
    // RemoteExecutionService.onBuildComplete (a BuildCompleteEvent subscriber).
    if let Some(remote) = remote {
        remote.build_complete(build_result.get_success());
    }
    report_build_summary_stats(
        reporter.as_ref(),
        &collector,
        build_result.get_elapsed_seconds(),
    );
    reporter.handle(&build_complete_event(
        build_result.get_success(),
        failed_tests,
        &collector,
    ));
    // The BuildCompleteEvent's BEP subscribers (BuildSummaryStatsModule's tool logs, the UI's
    // progress bar while the transports are open, the streamer), then the BuildToolLogs event.
    if let Some(bep) = bep {
        bep.build_complete(
            &mut build_result,
            &to_util_detailed_exit_code(&detailed_exit_code),
            &collector,
        );
    }
    // SpawnLogModule.buildComplete / afterCommand.
    spawn_log_module.build_complete();
    // SandboxModule.afterCommand.
    if let Some(execution) = skyframe_executor.get_execution() {
        execution.after_command();
    }
    if let Err(e) = spawn_log_module.after_command() {
        reporter.handle(&Event::error(e.failure_detail.message.clone()));
        return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(e.failure_detail));
    }

    let Some(test_support) = test_support else {
        return BlazeCommandResult::detailed_exit_code(detailed_exit_code);
    };
    // TestCommand.doTest after processRequest.
    let printer = Arc::new(blaze_events::terminal::AnsiTerminalPrinter::new(
        env.get_reporter_out_err().get_output_stream().clone(),
        ui_options.use_color(),
    ));
    let formatter = make_test_log_path_formatter(
        build_result.get_convenience_symlinks(),
        &test_summary_options,
        env.get_relative_working_directory(),
        &get_symlink_prefix(&build_request_options, env.get_runtime().get_product_name()),
    );
    let main_repo_mapping = skyframe_executor
        .get_analysis_services()
        .and_then(|s| s.main_repo_mapping.read().clone())
        .expect("the main repository mapping was computed");
    let code = test_support.finish(
        &build_result,
        reporter.as_ref(),
        printer,
        formatter,
        main_repo_mapping,
        None,
    );
    let exit_code = get_exit_code(&code);
    let detailed = match code.get_failure_detail() {
        None => DetailedExitCode::success(),
        Some(failure_detail) => DetailedExitCode::of_exit_code(
            ExitCode::for_code(exit_code.get_numeric_exit_code()).expect("a registered exit code"),
            failure_detail.clone(),
        ),
    };
    BlazeCommandResult::detailed_exit_code(detailed)
}

/// `BuildTool.stopRequest` of a request without an execution phase (`cquery`, `aquery`: their
/// `editOptions` adds `--nobuild`): stops the clock and lets the `BuildCompleteEvent`
/// subscribers print the build summary. `detailed_exit_code` is the result of
/// `BuildTool.processRequest`.
///
/// `BuildSummaryStatsModule.buildComplete` prints `Elapsed time: ...` alone (no `ExecutionTool`
/// was created, so `executorInit` set up no critical path computer) and `0 processes.`; the UI
/// prints `Build completed successfully, 0 total actions` or
/// `Build did NOT complete successfully`.
///
/// PORT: the Build Event Protocol of these commands is not ported (the dispatcher fails loudly
/// when a `--build_event_*_file` is given to a command other than `build` and `test`), so the
/// `BuildCompleteEvent` and the `BuildToolLogs` event have no other subscriber.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1155 BuildTool.stopRequest
pub(crate) fn stop_request_without_execution(
    env: &CommandEnvironment,
    detailed_exit_code: &blaze_query_engine::DetailedExitCode,
) {
    let reporter = reporter(env);
    let mut result = BuildResult::new(env.get_command_start_time());
    // The stop time has to be captured before we send the BuildCompleteEvent.
    result.set_stop_time(JavaClock.current_time_millis());
    // No action ran: the counters of BuildSummaryStatsModule and of the UI are zero.
    let collector = ExecutionEventCollector::new();
    report_build_summary_stats(reporter.as_ref(), &collector, result.get_elapsed_seconds());
    // BuildResult.getSuccess()
    reporter.handle(&build_complete_event(
        detailed_exit_code.is_success(),
        0,
        &collector,
    ));
}

/// The runtime's `DetailedExitCode` as `blaze_util`'s (what `BuildResult` holds).
fn to_util_detailed_exit_code(code: &DetailedExitCode) -> blaze_util::DetailedExitCode {
    match code.get_failure_detail() {
        None => blaze_util::DetailedExitCode::success(),
        Some(failure_detail) => blaze_util::DetailedExitCode::of_exit_code(
            blaze_util::ExitCode::for_code(code.get_exit_code().get_numeric_exit_code())
                .expect("a registered exit code"),
            failure_detail.clone(),
        ),
    }
}

/// `build` and `test` have no analysis post processor. It is where the request's
/// `ExecutionTool` is constructed instead.
///
/// `new ExecutionTool(env, request)` throws the `AbruptExitException` of the spawn strategy
/// registry (`BuildTool.processRequest` reports it, sets the catastrophe and returns its exit
/// code). Java constructs the tool in `BuildTool.buildTargets` right after the analysis
/// (`--noexperimental_merged_skyframe_analysis_execution`, `--nobuild`) or right before the
/// merged analysis and execution (`buildTargetsWithMergedAnalysisExecution`, outside its
/// `try`/`finally`, so no build result is shown).
///
/// PORT: blaze_sky_top's `BuildTool` has no hook between the target patterns and the analysis;
/// with Skymeld, an analysis error that Java never reaches (the exception comes first) is
/// reported instead.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:520 BuildTool.buildTargetsWithoutMergedAnalysisExecution
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:613 BuildTool.buildTargetsWithMergedAnalysisExecution
struct ExecutionToolConstruction<'a> {
    execution_phase: &'a RuntimeExecutionPhase,
}

impl AnalysisPostProcessor for ExecutionToolConstruction<'_> {
    fn process(
        &self,
        _request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        _analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        let state = skyframe_executor
            .get_execution()
            .expect("IllegalStateException: the executor was created without execution");
        if let Err(e) = state.prepared.check_execution_tool_created() {
            // catch (AbruptExitException e): result.setCatastrophe().
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:999 BuildTool.processRequest
            self.execution_phase
                .build_result
                .borrow_mut()
                .set_catastrophe();
            self.execution_phase.execution_tool_failed.set(true);
            let failure_detail = e.failure_detail;
            return Err(BuildToolError::Exit(
                blaze_query_engine::DetailedExitCode::of_exit_code(
                    blaze_query_engine::DetailedExitCode::get_exit_code_for(&failure_detail),
                    failure_detail,
                ),
            ));
        }
        Ok(())
    }
}

/// `new BuildResult(request.getStartTime())` and `BuildTool.maybeSetStopOnFirstFailure`.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:898 BuildTool.processRequest
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1121 BuildTool.maybeSetStopOnFirstFailure
fn new_build_result(
    start_time_millis: i64,
    keep_going: bool,
    execution_options: &ExecutionOptions,
) -> BuildResult {
    let mut result = BuildResult::new(start_time_millis);
    // BuildTool.shouldStopOnFailure
    if !(keep_going && execution_options.test_keep_going) {
        result.set_stop_on_first_failure(true);
    }
    result
}

/// Whether Java runs the request with merged analysis and execution (Skymeld), which decides
/// what `BuildTool.buildTargets` leaves in the `BuildResult` when the analysis fails.
///
/// PORT: the `logger.atInfo()` lines that say why the flag is ignored are not written (no server
/// logger). Blaze's own order of work (analysis of all top-level targets, then execution) does
/// not depend on this value.
// Java: com/google/devtools/build/lib/skyframe/SkymeldModule.java:35 SkymeldModule.determineIfMergingAnalysisExecution
fn determine_if_merging_analysis_execution(
    options: &OptionsParsingResult,
    build_request_options: &BuildRequestOptions,
    package_path_entries: usize,
) -> bool {
    // SkymeldModule.getPlainValueFromFlag
    let mut effective_value =
        build_request_options.merged_skyframe_analysis_execution_do_not_use_directly;

    // --nobuild means no execution will be carried out, hence it doesn't make sense to interleave
    // analysis and execution in that case and --experimental_merged_skyframe_analysis_execution
    // should be ignored.
    if effective_value && !build_request_options.perform_execution_phase {
        effective_value = false;
    }
    // TODO(b/245922903): Make --explain compatible with Skymeld.
    if effective_value && build_request_options.explanation_path.is_some() {
        effective_value = false;
    }

    let having_multi_package_path = package_path_entries > 1;
    // TODO(b/246324830): Skymeld and multi-package_path are incompatible.
    if effective_value && having_multi_package_path {
        effective_value = false;
    }

    if effective_value
        && options
            .get_options::<blaze_optdefs::command::analysis_options::AnalysisOptions>()
            .is_some_and(|o| o.cpu_heavy_sky_keys_thread_pool_size <= 0)
    {
        effective_value = false;
    }

    if effective_value
        && (build_request_options
            .aquery_dump_after_build_format
            .is_some()
            || build_request_options
                .aquery_dump_after_build_output_file
                .is_some())
    {
        effective_value = false;
    }

    // TODO(b/245873370) --check_licenses is going away.
    if effective_value
        && options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .is_some_and(|o| o.check_licenses)
    {
        effective_value = false;
    }

    effective_value
}

/// Whether `BuildTool.buildTargets` got past `evaluateTargetPatterns`, that is, entered
/// `buildTargetsWithMergedAnalysisExecution`.
///
/// PORT: Java knows by control flow (the `finally` block of that method). blaze_sky_top's
/// `BuildTool.process_request` returns only the exit code, so this asks the graph for the
/// `TARGET_PATTERN_PHASE` node the build tool evaluated (the key of
/// `SkyframeExecutor.loadTargetPatternsWithFilters` for the request): it is done without an
/// error exactly when the evaluation of the target patterns did not throw.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:468 BuildTool.evaluateTargetPatterns
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3179 SkyframeExecutor.loadTargetPatternsWithFilters
fn target_patterns_were_evaluated(
    skyframe_executor: &SkyframeExecutor,
    request: &BuildRequest,
    relative_working_directory: &PathFragment,
) -> bool {
    use blaze_sky_top::target_pattern_phase::{TargetPatternPhaseKey, TestFilter};
    let options = &request.loading_options;
    let key = blaze_skyframe::SkyKey::new(TargetPatternPhaseKey::key(
        request.targets.clone(),
        relative_working_directory.clone(),
        options.compile_one_dependency,
        options.build_tests_only,
        request.should_run_tests(),
        options.build_tag_filter_list.clone(),
        options.build_manual_tests,
        options.expand_test_suites,
        Some(TestFilter::for_options(options)),
    ));
    let evaluator = skyframe_executor.get_evaluator();
    evaluator.get_existing_value(&key).is_some()
        && evaluator.get_existing_error_for_testing(&key).is_none()
}

/// A listener for the results of a build: the top-level targets that were analyzed and the
/// ones that were skipped.
///
/// PORT: the analyzed tests are the analysis result's tests (Blaze analyzes every top-level
/// target before it executes), the built targets are read from the graph
/// ([`top_level_target_built`]), and there are no top-level aspects.
// Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:42 BuildResultListener
#[derive(Default)]
struct BuildResultListener {
    // Also includes test targets.
    analyzed_targets: parking_lot::Mutex<Vec<ConfiguredTarget>>,
    // Also includes test targets.
    skipped_targets: parking_lot::Mutex<Vec<ConfiguredTarget>>,
}

impl BuildResultListener {
    /// `Set.add`: a configured target is one instance per lookup key.
    ///
    /// PORT: java-hash-order. Java's sets are `ConcurrentHashMap.newKeySet()` over identity hash
    /// codes, so the order of the result lines of several targets changes from run to run; here
    /// it is the order of the events.
    fn add(set: &parking_lot::Mutex<Vec<ConfiguredTarget>>, target: &ConfiguredTarget) {
        let mut set = set.lock();
        if !set
            .iter()
            .any(|t| t.get_lookup_key() == target.get_lookup_key())
        {
            set.push(target.clone());
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:89 BuildResultListener.getAnalyzedTargets
    fn get_analyzed_targets(&self) -> Vec<ConfiguredTarget> {
        self.analyzed_targets.lock().clone()
    }

    // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:101 BuildResultListener.getSkippedTargets
    fn get_skipped_targets(&self) -> Vec<ConfiguredTarget> {
        self.skipped_targets.lock().clone()
    }
}

impl blaze_events::EventBus for BuildResultListener {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        if let Some(event) =
            blaze_sky_exec::events::downcast_event::<TopLevelTargetAnalyzedEvent>(&event)
        {
            // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:55 BuildResultListener.addAnalyzedTarget
            Self::add(&self.analyzed_targets, &event.configured_target);
        } else if let Some(event) =
            blaze_sky_exec::events::downcast_event::<TopLevelTargetSkippedEvent>(&event)
        {
            // Java: com/google/devtools/build/lib/skyframe/BuildResultListener.java:73 BuildResultListener.addSkippedTarget
            Self::add(&self.skipped_targets, &event.configured_target);
        }
    }
}

/// What `BuildResultPrinter` reads from Java's `BuildRequest`.
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:59 BuildRequest
struct RuntimeBuildRequestView {
    build_options: BuildRequestOptions,
    execution_options: ExecutionOptions,
    should_run_tests: bool,
    out_err: OutErr,
}

impl BuildRequestView for RuntimeBuildRequestView {
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:331 BuildRequest.getBuildOptions
    fn get_build_options(&self) -> &BuildRequestOptions {
        &self.build_options
    }

    fn get_execution_options(&self) -> &ExecutionOptions {
        &self.execution_options
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:448 BuildRequest.useValidationAspect
    fn use_validation_aspect(&self) -> bool {
        validation_mode(&self.build_options) == ValidationMode::Aspect
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:410 BuildRequest.getTopLevelArtifactContext
    fn get_top_level_artifact_context(&self) -> TopLevelArtifactContext {
        TopLevelArtifactContext {
            output_groups: top_level_output_groups(&self.build_options, self.should_run_tests)
                .into_iter()
                .collect(),
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:323 BuildRequest.getOutErr
    fn get_out_err(&self) -> &OutErr {
        &self.out_err
    }
}

/// Whether a top-level target was built, as `BuildResultListener.getBuiltTargets()` records it
/// from the `TopLevelTargetBuiltEvent`s of `ExecutionProgressReceiver.evaluated`: its
/// `TARGET_COMPLETION` node succeeded, or its `BUILD_DRIVER` node succeeded with a value that is
/// not skipped.
///
/// PORT: the evaluator has no `EvaluationProgressReceiver`; the same facts are read from the
/// graph after the evaluation.
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:95 ExecutionProgressReceiver.evaluated
fn top_level_target_built(
    evaluator: &blaze_skyframe::Evaluator,
    evaluation: &blaze_skyframe::EvaluationResult,
    key: &BuildDriverKey,
) -> bool {
    // BuildDriverFunction requests the target's completion with `willTest` set for a test.
    let target_completion_key = blaze_skyframe::SkyKey::new(TargetCompletionKey {
        action_lookup_key: key.action_lookup_key.clone(),
        output_groups: key.output_groups.clone(),
        will_test: key.test_type != TestType::NOT_TEST,
    });
    if evaluator
        .get_existing_value(&target_completion_key)
        .is_some()
    {
        return true;
    }
    evaluation.get(key).is_some_and(|value| !value.skipped)
}

/// Whether a top-level aspect was built, as `BuildResultListener.getBuiltAspects()` records it
/// from the `AspectBuiltEvent`s of `ExecutionProgressReceiver.evaluated`: its
/// `ASPECT_COMPLETION` node succeeded, or a `BUILD_DRIVER` node of a top-level aspect driver
/// succeeded (not skipped) with a `TopLevelAspectsValue` that holds the aspect.
///
/// PORT: the evaluator has no `EvaluationProgressReceiver`; the same facts are read from the
/// graph after the evaluation.
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:121 ExecutionProgressReceiver.evaluated
fn top_level_aspect_built(
    evaluator: &blaze_skyframe::Evaluator,
    evaluation: &blaze_skyframe::EvaluationResult,
    keys_to_execute: &[BuildDriverKey],
    aspect_key: &ActionLookupKeyRef,
) -> bool {
    for key in keys_to_execute {
        if !key.is_top_level_aspect_driver() {
            continue;
        }
        // BuildDriverFunction requests the AspectCompletionKey with the driver's output groups.
        let aspect_completion_key = blaze_skyframe::SkyKey::new(AspectCompletionKey::create(
            aspect_key.clone(),
            key.output_groups.clone(),
        ));
        if evaluator
            .get_existing_value(&aspect_completion_key)
            .is_some()
        {
            return true;
        }
        if let Some(value) = evaluation.get(key)
            && !value.skipped
            && let TopLevelSkyValue::TopLevelAspects(aspects) = &value.top_level_sky_value
            && aspects.iter().any(|a| a.get_aspect_key() == *aspect_key)
        {
            return true;
        }
    }
    false
}

/// What `BuildResultPrinter.filterTargetsToPrint` reads from the `OutputFileConfiguredTarget`
/// behind `target` (a `generated file` among the analyzed targets) and from its generating rule.
// Java: com/google/devtools/build/lib/buildtool/BuildResultPrinter.java:353 BuildResultPrinter.filterTargetsToPrint
fn output_file_generating_rule(
    analyzed_targets: &[ConfiguredTarget],
    target: &ConfiguredTargetRef,
) -> OutputFileGeneratingRule {
    let lookup_key = target.get_lookup_key();
    for configured_target in analyzed_targets {
        let unmerged = configured_target.unwrap_if_merged();
        if let ConfiguredTarget::OutputFile(output_file) = &unmerged
            && *unmerged.get_lookup_key() == lookup_key
        {
            let generating_rule = ConfiguredTarget::Rule(output_file.get_generating_rule().clone());
            return OutputFileGeneratingRule {
                generating_rule_key: generating_rule.get_lookup_key().clone(),
                generating_rule_files_to_build: generating_rule
                    .get_provider::<FileProvider>()
                    .unwrap_or_else(|| panic!("NullPointerException: FileProvider"))
                    .get_files_to_build()
                    .clone(),
                files_to_build: unmerged
                    .get_provider::<FileProvider>()
                    .unwrap_or_else(|| panic!("NullPointerException: FileProvider"))
                    .get_files_to_build()
                    .clone(),
            };
        }
    }
    panic!(
        "ClassCastException: {} is not an OutputFileConfiguredTarget of the analysis result",
        target.get_label()
    )
}

impl BlazeDirectoriesView for BlazeDirectories {
    fn get_exec_root(&self, workspace_name: &str) -> Path {
        BlazeDirectories::get_exec_root(self, workspace_name)
    }

    fn get_output_path(&self, workspace_name: &str) -> Path {
        BlazeDirectories::get_output_path(self, workspace_name)
    }

    fn get_output_base(&self) -> Path {
        BlazeDirectories::get_output_base(self).clone()
    }

    fn get_blaze_exec_root(&self) -> Path {
        BlazeDirectories::get_blaze_exec_root(self).clone()
    }
}

struct RuntimeExecutionPhase {
    reporter: Arc<dyn ExtendedEventHandler>,
    directories: BlazeDirectories,
    workspace: Path,
    workspace_name: String,
    product_name: String,
    relative_working_directory: PathFragment,
    build_request_options: BuildRequestOptions,
    execution_options: ExecutionOptions,
    /// `--experimental_extra_action_top_level_only` (`AnalysisOptions`).
    extra_action_top_level_only: bool,
    collector: Arc<ExecutionEventCollector>,
    /// `env.getReporter()`, for `Reporter.addHandler`.
    event_reporter: Arc<blaze_events::Reporter>,
    /// `--explain`: `request.getBuildOptions().explanationPath` and
    /// `request.getOptionsDescription()`.
    explanation: Option<(PathFragment, String)>,
    /// Java's `BuildResult result` of `BuildTool.processRequest`, which `buildTargets` fills.
    build_result: RefCell<BuildResult>,
    /// Whether `execute_build` reached `ExecutionTool.nonCatastrophicFinalizations` ("Show
    /// results").
    results_shown: Cell<bool>,
    /// Whether `new ExecutionTool(env, request)` threw (see [`ExecutionToolConstruction`]): Java
    /// then never entered the `try` whose `finally` shows the results.
    execution_tool_failed: Cell<bool>,
    /// `env.getBuildResultListener()`.
    build_result_listener: Arc<BuildResultListener>,
    /// The `BuildRequest` as `BuildResultPrinter` reads it.
    request_view: RuntimeBuildRequestView,
    /// The command's Build Event Protocol support (`None` without `--build_event_*_file`).
    bep: Option<Arc<BepCommandSupport>>,
    /// The command's `RemoteModule` (`None` without any remote option).
    remote: Option<Arc<build_remote::RemoteSupport>>,
}

impl RuntimeExecutionPhase {
    /// Handles what action to perform on the convenience symlinks. If the mode is
    /// `ConvenienceSymlinksMode::IGNORE`, then skip any creating or cleaning of convenience
    /// symlinks. Otherwise, manage the convenience symlinks and then post a
    /// `ConvenienceSymlinksIdentifiedEvent` build event.
    ///
    /// Returns the symlink creation result, whose `get_created_symlinks()` is the map of
    /// convenience symlink name to target.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:705 ExecutionTool.handleConvenienceSymlinks
    fn handle_convenience_symlinks(
        &self,
        skyframe_executor: &SkyframeExecutor,
        targets_to_build: &[ConfiguredTarget],
        configuration: &Arc<BuildConfigurationValue>,
    ) -> SymlinkCreationResult {
        // OutputDirectoryLinksUtils.EMPTY_SYMLINK_CREATION_RESULT
        let mut convenience_symlinks = SymlinkCreationResult::empty();
        if self.build_request_options.experimental_convenience_symlinks
            != ConvenienceSymlinksMode::IGNORE
        {
            convenience_symlinks = self.create_convenience_symlinks(
                skyframe_executor,
                targets_to_build,
                configuration,
            );
        }
        // request.getBuildOptions().experimentalConvenienceSymlinksBepEvent is read by the BEP
        // support.
        if let Some(bep) = &self.bep {
            bep.convenience_symlinks_identified(&self.build_request_options, &convenience_symlinks);
        }
        convenience_symlinks
    }

    /// Creates convenience symlinks based on the target configurations.
    ///
    /// Top-level targets may have different configurations than the top-level configuration.
    /// This is because targets may apply configuration transitions.
    ///
    /// If all top-level targets have the same configuration - even if that isn't the top-level
    /// configuration - symlinks point to that configuration.
    ///
    /// If top-level targets have mixed configurations and at least one of them has the top-level
    /// configuration, symliks point to the top-level configuration.
    ///
    /// If top-level targets have mixed configurations and none has the top-level configuration,
    /// symlinks aren't created. Furthermore, lingering symlinks from the last build are deleted.
    /// This is to prevent confusion by pointing to an outdated directory the current build never
    /// used.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:742 ExecutionTool.createConvenienceSymlinks
    fn create_convenience_symlinks(
        &self,
        skyframe_executor: &SkyframeExecutor,
        targets_to_build: &[ConfiguredTarget],
        configuration: &Arc<BuildConfigurationValue>,
    ) -> SymlinkCreationResult {
        // Gather configurations to consider.
        let target_configs: Vec<Arc<BuildConfigurationValue>> = if targets_to_build.is_empty() {
            vec![configuration.clone()]
        } else {
            // Collect the configuration of each top-level requested target. These may be
            // different than the build's top-level configuration because of self-transitions.
            // PORT: `executor.getConfiguration(reporter, key)` evaluates the key; the
            // configuration of an analyzed target is in the graph.
            let graph = skyframe_executor.get_walkable_graph();
            let mut requested_target_config_keys: Vec<
                blaze_build_config::build_configuration_key::BuildConfigurationKey,
            > = Vec::new();
            for target in targets_to_build {
                if let Some(key) = target.get_actual().get_configuration_key()
                    && !requested_target_config_keys.contains(&key)
                {
                    requested_target_config_keys.push(key);
                }
            }
            let requested_target_configs: Vec<Arc<BuildConfigurationValue>> =
                requested_target_config_keys
                    .iter()
                    .map(|key| {
                        get_configuration_from_graph(&*graph, Some(key))
                            .expect("NullPointerException: the configuration of a top-level target")
                    })
                    .collect();
            if requested_target_configs.len() == 1 {
                // All top-level targets have the same configuration, so use that one.
                requested_target_configs
            } else if requested_target_configs
                .iter()
                .any(|c| c.get_output_directory_name() == configuration.get_output_directory_name())
            {
                // Mixed configs but at least one matches the top-level config's output path (this
                // doesn't mean it's the same as the top-level config: --trim_test_configuration
                // means non-test targets use the default output path but lack the top-level
                // config's TestOptions). Set symlinks to the top-level config so at least
                // non-transitioned targets resolve. See
                // https://github.com/bazelbuild/bazel/issues/17081.
                vec![configuration.clone()]
            } else {
                // Mixed configs, none of which include the top-level config. Delete the symlinks
                // because they won't contain any relevant data. This is handled in the
                // createOutputDirectorySymlinks call below.
                requested_target_configs
            }
        };

        // PORT: `runtime.getRuleClassProvider().getSymlinkDefinitions()` is empty for Bazel's
        // rule class provider.
        create_output_directory_links(
            &[],
            &self.build_request_options,
            &self.workspace_name,
            &self.workspace,
            &self.directories,
            self.reporter.as_ref(),
            &target_configs,
            &self.product_name,
        )
    }

    /// Installs an `ExplanationHandler` (if `--explain` is set) and returns it on success.
    /// Reports an error and returns `None` otherwise.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:804 ExecutionTool.installExplanationHandler
    fn install_explanation_handler(&self) -> Option<InstalledExplanationHandler> {
        let (explanation_path, all_options) = self.explanation.as_ref()?;
        // InstrumentationOutputFactory.createInstrumentationOutput("explain", explanationPath,
        // DestinationRelativeTo.WORKSPACE_OR_HOME, ...).createOutputStream()
        let handler = match self
            .workspace
            .get_relative(explanation_path)
            .get_output_stream()
        {
            Ok(log) => Arc::new(ExplanationHandler::new(log, all_options)),
            Err(e) => {
                self.reporter.handle(&Event::warn(format!(
                    "Cannot write explanation of rebuilds to file '{}': {}",
                    explanation_path, e
                )));
                return None;
            }
        };
        self.reporter.handle(&Event::info(format!(
            "Writing explanation of rebuilds to '{}'",
            explanation_path
        )));
        let registered: Arc<dyn EventHandler> = handler.clone();
        self.event_reporter.add_handler(registered.clone());
        Some(InstalledExplanationHandler {
            reporter: self.event_reporter.clone(),
            handler,
            registered,
        })
    }
}

/// Records the dependency checker's explanations of rebuilds (`--explain`).
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:850 ExecutionTool.ExplanationHandler
struct ExplanationHandler {
    log: parking_lot::Mutex<Option<Box<dyn std::io::Write + Send>>>,
}

impl ExplanationHandler {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:853 ExplanationHandler.<init>
    fn new(log: Box<dyn std::io::Write + Send>, options_description: &str) -> ExplanationHandler {
        let handler = ExplanationHandler {
            log: parking_lot::Mutex::new(Some(log)),
        };
        handler.println(format!("Build options: {options_description}").as_bytes());
        handler
    }

    /// `log.println(s)` of a `PrintWriter` over `OutputStreamWriter(log, UTF_8)`: Bazel's strings
    /// are in the internal form (one character per byte), so the writer encodes every byte of
    /// the string as its own character. `PrintWriter` swallows `IOException`s.
    fn println(&self, internal: &[u8]) {
        if let Some(log) = self.log.lock().as_mut() {
            let line: String = internal.iter().map(|&b| b as char).collect();
            let _ = log.write_all(line.as_bytes());
            let _ = log.write_all(b"\n");
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:859 ExplanationHandler.close
    fn close(&self) {
        if let Some(mut log) = self.log.lock().take() {
            let _ = log.flush();
        }
    }
}

impl EventHandler for ExplanationHandler {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:864 ExplanationHandler.handle
    fn handle(&self, event: &Event) {
        if event.get_kind() == blaze_events::EventKind::Depchecker {
            self.println(event.get_message_bytes());
        }
    }
}

/// An installed [`ExplanationHandler`]; dropping it uninstalls the handler and closes the log
/// file.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:839 ExecutionTool.uninstallExplanationHandler
struct InstalledExplanationHandler {
    reporter: Arc<blaze_events::Reporter>,
    handler: Arc<ExplanationHandler>,
    registered: Arc<dyn EventHandler>,
}

impl Drop for InstalledExplanationHandler {
    fn drop(&mut self) {
        self.reporter.remove_handler(&self.registered);
        self.handler.close();
    }
}

impl ExecutionPhase for RuntimeExecutionPhase {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:367 ExecutionTool.executeBuild
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:595 BuildTool.buildTargetsWithMergedAnalysisExecution
    fn execute_build(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        loading_result: &TargetPatternPhaseValue,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        let state = skyframe_executor
            .get_execution()
            .expect("IllegalStateException: the executor was created without execution");
        let reporter = self.reporter.clone();

        // PORT: the loading and analysis BEP events (TargetParsingCompleteEvent, ExecRootEvent,
        // the top-level configuration, TargetConfiguredEvents) and the analysis metrics are
        // posted here, from the loading and analysis results, instead of while BuildTool's
        // loading and analysis run; the streamer orders events by their announcements.
        if let Some(bep) = &self.bep {
            bep.analysis_complete(request, skyframe_executor, loading_result, analysis_result);
        }

        // ExecutionTool.prepareForExecution: plant the symlink forest of the exec root.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:296 ExecutionTool.prepareForExecution
        let exec_root = state.config.exec_root.clone();
        let planted = exec_root.create_directory_and_parents().and_then(|_| {
            SymlinkForest::new(
                skyframe_executor.get_package_roots(),
                exec_root.clone(),
                &self.product_name,
            )
            .plant_symlink_forest()
        });
        if let Err(e) = planted {
            let message = format!("Source forest creation failed: {e}");
            return Err(BuildToolError::Exit(
                blaze_query_engine::DetailedExitCode::of_exit_code(
                    36,
                    FailureDetail {
                        message,
                        category: Some(Category::SymlinkForest(failure_details::SymlinkForest {
                            code: failure_details::symlink_forest::Code::CreationFailed as i32,
                        })),
                    },
                ),
            ));
        }

        // The convenience symlinks.
        // PORT: Java's Skymeld path does this in the finally block of
        // buildTargetsWithMergedAnalysisExecution with `buildResultListener.getAnalyzedTargets()`,
        // the other path at the start of ExecutionTool.executeBuild with
        // `analysisResult.getTargetsToBuild()`. Every top-level target is analyzed by now.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:668 BuildTool.buildTargetsWithMergedAnalysisExecution
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:393 ExecutionTool.executeBuild
        let symlinks = if request.with_merged_analysis_and_execution_source_of_truth() {
            self.handle_convenience_symlinks(
                skyframe_executor,
                &self.build_result_listener.get_analyzed_targets(),
                analysis_result.get_configuration(),
            )
        } else {
            self.handle_convenience_symlinks(
                skyframe_executor,
                analysis_result.get_targets_to_build(),
                analysis_result.get_configuration(),
            )
        };

        // The top-level BuildDriverKeys.
        let label_to_target_map = construct_label_to_target_map(skyframe_executor, loading_result);
        let main_repo_mapping = skyframe_executor
            .get_analysis_services()
            .and_then(|s| s.main_repo_mapping.read().clone())
            .expect("the main repository mapping was computed");
        let explicit_target_patterns = get_explicit_target_patterns(
            &self.relative_working_directory,
            &main_repo_mapping,
            &request.targets,
        );
        let output_groups =
            top_level_output_groups(&self.build_request_options, request.should_run_tests());
        // RemoteModule.afterAnalysis (or, with Skymeld, afterTopLevelTargetAnalysis for each
        // top-level target): the outputs of the top-level targets are downloaded under
        // --remote_download_outputs=toplevel.
        // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:153 AnalysisPhaseRunner.execute
        // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:324 AnalysisAndExecutionPhaseRunner.execute
        if let Some(remote) = &self.remote {
            remote.after_analysis(
                analysis_result,
                &output_groups,
                request.with_merged_analysis_and_execution_source_of_truth(),
            );
        }
        let keys = build_driver_keys(
            &label_to_target_map,
            analysis_result.get_configuration(),
            analysis_result.get_targets_to_build(),
            loading_result.get_tests_to_run_labels(),
            &explicit_target_patterns,
            &output_groups,
            // request.getViewOptions().skipIncompatibleExplicitTargets
            request.skip_incompatible_explicit_targets,
            self.extra_action_top_level_only,
            &BuildDriverKeyTestContext {
                test_strategy: self.execution_options.test_strategy.clone(),
                // executionTool.getTestActionContext(): StandaloneTestStrategy keeps the
                // TestActionContext defaults (false); only the remote test strategies force
                // exclusive tests to run in parallel.
                force_exclusive_tests_in_parallel: false,
                force_exclusive_if_local_tests_in_parallel: false,
            },
            reporter.as_ref(),
        );
        // Java's topLevelKeys: the union of the configured-target and the top-level aspect
        // BuildDriverKeys.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:680 SkyframeBuildView.analyzeAndExecuteTargets
        let mut keys = keys;
        keys.extend(build_driver_aspect_keys(
            analysis_result.get_top_level_aspects_keys(),
            &explicit_target_patterns,
            &output_groups,
            request.skip_incompatible_explicit_targets,
            self.extra_action_top_level_only,
        ));
        if keys.iter().any(|k| {
            matches!(
                k.test_type,
                TestType::EXCLUSIVE | TestType::EXCLUSIVE_IF_LOCAL
            )
        }) {
            blaze_util::unported!("BuildTool.runExclusiveTests");
        }

        // PORT: Blaze analyzed every top-level target before this point, and BuildView.update ran
        // the compatibility check of BuildDriverFunction for the Skymeld path (see
        // blaze_sky_top::build_view). What is left of a BuildDriverKey is the execution it
        // requests, so the keys are split by how far Java's BuildDriverFunction.compute gets:
        // - the target is not in `getTargetsToBuild()` (its analysis or its compatibility check
        //   failed, with --keep_going): the function threw, the key has no value, nothing is
        //   executed; BuildView.update reported the error;
        // - the target is skipped: the function returns `BuildDriverValue(skipped = true)`
        //   without requesting any execution;
        // - otherwise the function requests the execution of the target.
        // The non-Skymeld path (SkyframeBuilder.buildArtifacts) likewise builds
        // `getTargetsToBuild()` without `getTargetsToSkip()`.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:155 BuildDriverFunction.compute
        let skipped_labels: indexmap::IndexSet<blaze_cmdline::Label> = analysis_result
            .get_targets_to_skip()
            .iter()
            .map(|target| target.get_original_label())
            .collect();
        let analyzed_labels: indexmap::IndexSet<blaze_cmdline::Label> = analysis_result
            .get_targets_to_build()
            .iter()
            .map(|target| target.get_original_label())
            .collect();
        // PORT: a top-level aspect driver gets as far as Java's BuildDriverFunction.compute when
        // its TopLevelAspectsKey was analyzed (`env.getValueOrThrow(actionLookupKey)` has a
        // value); otherwise the function throws the analysis error, which BuildView.update
        // reported. Java never skips an aspect driver.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:174 BuildDriverFunction.compute
        let analyzed_top_level_aspects_keys: indexmap::IndexSet<ActionLookupKeyRef> =
            analysis_result
                .get_top_level_aspects_keys()
                .iter()
                .filter(|k| {
                    skyframe_executor
                        .get_evaluator()
                        .get_existing_value(&blaze_skyframe::SkyKey::new((*k).clone()))
                        .is_some()
                })
                .map(|k| ActionLookupKeyRef::new(k.clone()))
                .collect();
        let mut keys_to_execute: Vec<BuildDriverKey> = Vec::new();
        let mut skipped_keys: Vec<BuildDriverKey> = Vec::new();
        for key in &keys {
            if key.is_top_level_aspect_driver() {
                if analyzed_top_level_aspects_keys.contains(&key.action_lookup_key) {
                    keys_to_execute.push(key.clone());
                }
                continue;
            }
            let label = key
                .action_lookup_key
                .get_label()
                .expect("NullPointerException: the label of a top-level ConfiguredTargetKey");
            if skipped_labels.contains(&label) {
                skipped_keys.push(key.clone());
            } else if analyzed_labels.contains(&label) {
                keys_to_execute.push(key.clone());
            }
        }
        if self.bep.is_some() && !skipped_keys.is_empty() {
            // The AbortedEvent (AbortReason.SKIPPED) of each skipped target in the build event
            // stream.
            blaze_util::unported!(
                "AnalysisPhaseRunner.postAbortedEventsForSkippedTargets",
                "build event stream of a build with skipped (incompatible) top-level targets"
            );
        }
        if request.with_merged_analysis_and_execution_source_of_truth() {
            // We still record analyzed but skipped tests, as this information is needed for the
            // result summary.
            // PORT: the other path posts its TestAnalyzedEvents in
            // AnalysisPhaseRunner.postTopLevelStatusEvents.
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:274 BuildDriverFunction.compute
            let graph = skyframe_executor.get_walkable_graph();
            for key in &skipped_keys {
                if !key.test_type.is_test() {
                    continue;
                }
                let label = key.action_lookup_key.get_label();
                let configured_target = analysis_result
                    .get_targets_to_skip()
                    .iter()
                    .find(|target| Some(target.get_original_label()) == label)
                    .expect("the skipped target of a skipped key");
                let build_configuration_value = get_configuration_from_graph(
                    &*graph,
                    configured_target.get_configuration_key().as_ref(),
                )
                .expect("NullPointerException: buildConfigurationValue");
                let view = configured_target_views(std::slice::from_ref(configured_target))
                    .pop()
                    .expect("one view per configured target");
                reporter.post(Arc::new(TestAnalyzedEvent {
                    configured_target: view,
                    configuration_checksum: build_configuration_value.checksum().to_owned(),
                    configuration_mnemonic: build_configuration_value.get_mnemonic().to_owned(),
                    is_skipped: true,
                }));
            }
        }

        // Conditionally record dependency-checker log (uninstalled and closed when dropped, where
        // ExecutionTool.nonCatastrophicFinalizations does it: after the results are shown).
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:421 ExecutionTool.executeBuild
        let _explanation_handler = self.install_explanation_handler();

        let evaluated = evaluate_build_driver_keys(
            skyframe_executor.get_evaluator(),
            &keys_to_execute,
            &state.seam.get_workspace_status_artifacts(),
            &state.config,
            reporter.clone(),
        );
        // An interrupt (InterruptedException out of analyzeAndExecuteTargets) still reaches the
        // finally block below, with buildCompleted false: the action cache is not saved and the
        // targets built so far are shown.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:620 BuildTool.buildTargetsWithMergedAnalysisExecution
        let interrupted = evaluated.is_err();
        let evaluation = match evaluated {
            Ok(result) => result.evaluation,
            Err(blaze_skyframe::Interrupted) => blaze_skyframe::EvaluationResult {
                values: Vec::new(),
                errors: Vec::new(),
                catastrophe: None,
            },
        };
        // ExecutionTool.nonCatastrophicFinalizations: `if (buildCompleted)
        // saveActionCache(actionCache)`.
        if !interrupted {
            // RemoteOutputService.onExecutionPhaseCompleteEvent → LeaseService
            // .finalizeExecution (Java: after the save, on the server's in-memory cache).
            if let Some(remote) = &self.remote {
                remote.finalize_execution(state.prepared.action_cache.as_deref());
            }
            state.prepared.save_action_cache(reporter.as_ref());
        }

        // SkyframeErrorProcessor.processErrors of the evaluation (the Skymeld path:
        // SkyframeBuildView.analyzeAndExecuteTargets; the other path: SkyframeBuilder
        // .buildArtifacts, which rethrows the same way). Without --keep_going it throws, and Java
        // reaches the finally block below ("Show results") with the exception pending.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:853 SkyframeBuildView.analyzeAndExecuteTargets
        let mut test_exec_exception: Option<String> = None;
        let execution_error: Option<BuildToolError> = if interrupted {
            Some(BuildToolError::Interrupted)
        } else {
            match process_execution_errors(&evaluation, request.keep_going) {
                Ok(None) => None,
                // This is the --keep_going code path: Time to throw the delayed exceptions.
                // Keeping legacy behavior: for execution errors, keep the message of the
                // BuildFailedException empty.
                // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:690 BuildTool.buildTargetsWithMergedAnalysisExecution
                Ok(Some(execution_detailed_exit_code)) => Some(BuildToolError::BuildFailed {
                    message: String::new(),
                    failure_detail: execution_detailed_exit_code
                        .get_failure_detail()
                        .cloned()
                        .expect("an execution exit code has a failure detail"),
                }),
                Err(RethrownException::BuildFailed {
                    error,
                    catastrophic,
                }) => {
                    // BuildTool.processRequest: `if (e.isCatastrophic()) result.setCatastrophe()`.
                    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:938 BuildTool.processRequest
                    if catastrophic {
                        self.build_result.borrow_mut().set_catastrophe();
                    }
                    Some(error)
                }
                // Non-catastrophic: the results are shown below, then BuildTool.processRequest
                // reports it.
                Err(RethrownException::TestExec(message)) => {
                    test_exec_exception = Some(message);
                    None
                }
            }
        };

        // BuildView.update, after analyzeAndExecuteTargets returned (it does not return when
        // processErrors threw): the successful targets are the BuildDriverKeys with a value,
        // skipped targets included.
        // Java: com/google/devtools/build/lib/analysis/BuildView.java:440 BuildView.update
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1183 SkyframeBuildView.getSuccessfulConfiguredTargets
        if request.with_merged_analysis_and_execution_source_of_truth()
            && (request.keep_going || (execution_error.is_none() && test_exec_exception.is_none()))
        {
            let num_targets_to_analyze = label_to_target_map.len();
            let num_successful = skipped_keys.len()
                + keys_to_execute
                    .iter()
                    // buildDriverCTKeys: the configured-target keys only.
                    .filter(|key| !key.is_top_level_aspect_driver())
                    .filter(|key| evaluation.get(*key).is_some())
                    .count();
            // PORT: this is the line of BuildView.update for `includeExecutionPhase`.
            // blaze_sky_top's BuildView.update returns before the execution, so it must not print
            // it for a merged request (it only knows the targets that were analyzed).
            if 0 < num_successful && num_successful < num_targets_to_analyze {
                // includeExecutionPhase ? "Build" : "Analysis"
                let msg = format!(
                    "Build succeeded for only {} of {} top-level targets",
                    num_successful, num_targets_to_analyze
                );
                reporter.handle(&Event::info(msg));
            }
        }

        // The finally block of BuildTool.buildTargetsWithMergedAnalysisExecution: the build
        // result's targets, then ExecutionTool.nonCatastrophicFinalizations ("Show results").
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:664 BuildTool.buildTargetsWithMergedAnalysisExecution
        self.results_shown.set(true);
        // buildResultListener.getAnalyzedTargets(): also the targets whose compatibility check
        // failed after they were analyzed, which the analysis result leaves out.
        let analyzed_targets = self.build_result_listener.get_analyzed_targets();
        let views = configured_target_views(&analyzed_targets);
        let test_targets =
            configured_target_views(analysis_result.get_targets_to_test().unwrap_or(&[]));
        let skipped_targets =
            configured_target_views(&self.build_result_listener.get_skipped_targets());
        // Maintain the ordering by copying builtTargets in the same iteration order as
        // configuredTargets.
        // PORT: Java's builtTargets holds ConfiguredTargetKeys; a top-level target's
        // BuildDriverKey is found by the label it was requested under, as
        // `execution_seam::build_driver_keys` pairs them (there is a key for every top-level
        // label, also for the ones whose analysis failed with --keep_going).
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:877 ExecutionTool.determineSuccessfulTargets
        let mut successful_targets: Vec<ConfiguredTargetRef> = Vec::new();
        for (target, view) in analyzed_targets.iter().zip(views.iter()) {
            let top_level_label = target.get_original_label();
            let built = keys_to_execute.iter().any(|key| {
                !key.is_top_level_aspect_driver()
                    && key.action_lookup_key.get_label().as_ref() == Some(&top_level_label)
                    && top_level_target_built(skyframe_executor.get_evaluator(), &evaluation, key)
            });
            if built {
                successful_targets.push(view.clone());
            }
        }
        {
            let mut build_result = self.build_result.borrow_mut();
            // result::setBuildConfiguration, result.setConvenienceSymlinks
            build_result.set_build_configuration(analysis_result.get_configuration().clone());
            build_result.set_convenience_symlinks(symlinks.get_created_symlinks().clone());
            build_result.set_actual_targets(Some(views.clone()));
            build_result.set_test_targets(Some(test_targets));
            // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:552 ExecutionTool.nonCatastrophicFinalizations
            build_result.set_successful_targets(Some(successful_targets));
            // Maintain the ordering.
            // PORT: Java's analyzed aspects are BuildResultListener.getAnalyzedAspects() (the
            // AspectAnalyzedEvents of BuildView.update), which are the keys of
            // AnalysisResult.getAspectsMap().
            // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:890 ExecutionTool.determineSuccessfulAspects
            let successful_aspects: indexmap::IndexSet<ActionLookupKeyRef> = analysis_result
                .get_aspects_map()
                .keys()
                .map(|aspect_key| ActionLookupKeyRef::new(aspect_key.clone()))
                .filter(|aspect_key| {
                    top_level_aspect_built(
                        skyframe_executor.get_evaluator(),
                        &evaluation,
                        &keys_to_execute,
                        aspect_key,
                    )
                })
                .collect();
            build_result.set_successful_aspects(Some(successful_aspects));
            build_result.set_skipped_targets(Some(skipped_targets.clone()));
            let generating_rule_lookup = |target: &ConfiguredTargetRef| {
                output_file_generating_rule(&analyzed_targets, target)
            };
            BuildResultPrinter::new(&self.product_name, self.relative_working_directory.clone())
                .show_build_result(
                    &self.request_view,
                    &build_result,
                    &views,
                    &skipped_targets,
                    &configured_aspect_views(analysis_result.get_aspects_map()),
                    &generating_rule_lookup,
                );
        }
        let _ = &self.collector;

        if let Some(execution_error) = execution_error {
            return Err(execution_error);
        }
        if let Some(message) = test_exec_exception {
            // BuildTool.processRequest, `catch (TestExecException e)`: ExitCode.SUCCESS means that
            // build was successful. Real return code of program is going to be calculated in
            // TestCommand.doTest(). reportExceptionError(e).
            // PORT: the exception is caught here, not in blaze_sky_top's BuildTool: nothing of
            // BuildTool.buildTargets runs after the execution phase on this path (a delayed
            // analysis failure needs --keep_going, which never rethrows).
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:976 BuildTool.processRequest
            reporter.handle(&Event::error(message));
        }
        Ok(())
    }
}

/// `SkyframeErrorProcessor.isExecutionException(cause)`, with the cause's class.
///
/// PORT: `TestExecException` is not a class of its own in blaze_exec (the test strategy raises
/// it as a marked `UserExecException`); the `ActionExecutionException` it causes tells
/// (`is_caused_by_test_exec_exception`), which is the `cause.getCause() instanceof
/// TestExecException` of `rethrow`. A `TestExecException` is never the error of a node itself.
/// `TopLevelOutputException` (action rewinding, staging of top-level outputs) is not raised by
/// the ported functions.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:873 SkyframeErrorProcessor.isExecutionException
enum ExecutionExceptionCause<'a> {
    ActionExecution(&'a ActionExecutionException),
    InputFileError(&'a InputFileErrorException),
    ArtifactNestedSetEval(&'a ArtifactNestedSetEvalException),
}

impl<'a> ExecutionExceptionCause<'a> {
    fn of(cause: &'a blaze_skyframe::SkyError) -> Option<ExecutionExceptionCause<'a>> {
        if let Some(e) = cause.downcast_ref::<ActionExecutionException>() {
            return Some(ExecutionExceptionCause::ActionExecution(e));
        }
        if let Some(e) = cause.downcast_ref::<InputFileErrorException>() {
            return Some(ExecutionExceptionCause::InputFileError(e));
        }
        if let Some(e) = cause.downcast_ref::<ArtifactNestedSetEvalException>() {
            return Some(ExecutionExceptionCause::ArtifactNestedSetEval(e));
        }
        None
    }

    /// `DetailedException.getDetailedExitCode(cause)`, or the "unknown" code for a cause that is
    /// not a `DetailedException`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:621 SkyframeErrorProcessor.getExecutionDetailedExitCodeFromCause
    fn get_execution_detailed_exit_code(&self) -> blaze_util::DetailedExitCode {
        match self {
            ExecutionExceptionCause::ActionExecution(e) => match e.get_failure_detail() {
                Some(failure_detail) => blaze_util::DetailedExitCode::of(failure_detail.clone()),
                None => panic!(
                    "NullPointerException: detailedExitCode of {}",
                    e.get_message()
                ),
            },
            ExecutionExceptionCause::InputFileError(e) => e.detailed_exit_code.clone(),
            ExecutionExceptionCause::ArtifactNestedSetEval(e) => {
                create_unknown_execution_detailed_exit_code(&e.to_string())
            }
        }
    }
}

/// The exit code of an execution exception without a `DetailedExitCode`.
///
/// PORT: skipped, the non-fatal bug report Java sends first.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:631 SkyframeErrorProcessor.sendBugReportAndCreateUnknownExecutionDetailedExitCode
fn create_unknown_execution_detailed_exit_code(
    cause_message: &str,
) -> blaze_util::DetailedExitCode {
    let message =
        format!("Unexpected exception, please file an issue with the Bazel team: {cause_message}");
    blaze_util::DetailedExitCode::of(FailureDetail {
        message,
        category: Some(Category::Execution(failure_details::Execution {
            code: failure_details::execution::Code::UnexpectedException as i32,
        })),
    })
}

/// The part of `SkyframeErrorProcessor.processErrors` (with `includeExecutionPhase`) that
/// handles execution exceptions, for the evaluation of the build driver keys.
///
/// - `--keep_going`: returns the most important exit code of the execution errors (`None` when
///   there was none); nothing is printed for them (`logOrPrintWarningsKeepGoing`).
/// - `--nokeep_going`: throws for the first error (`throwOrReturnAspectAnalysisException` →
///   `rethrow`).
///
/// PORT: Blaze analyzes before it executes (module comment), so the loading and analysis errors
/// of the top-level keys were processed by blaze_sky_top's `SkyframeErrorProcessor` and the keys
/// evaluated here are those of analyzed targets: an error that is not an execution exception or
/// an execution cycle fails loudly. The failure events of execution errors
/// (`maybePostFailureEventsForNonConflictError` posts an `AnalysisFailureEvent` only for
/// analysis errors) are none.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:302 SkyframeErrorProcessor.processErrors
fn process_execution_errors(
    result: &blaze_skyframe::EvaluationResult,
    keep_going: bool,
) -> Result<Option<blaze_util::DetailedExitCode>, RethrownException> {
    let mut execution_detailed_exit_code: Option<blaze_util::DetailedExitCode> = None;
    for (error_key, error_info) in result.error_map() {
        let nullable_cause = error_info.get_exception();
        blaze_util::check_state!(
            nullable_cause.is_some() || !error_info.get_cycle_info().is_empty(),
            "{:?}",
            error_info
        );
        let Some(cause) = nullable_cause else {
            // cyclesReporter.reportCycles and CYCLE_CODE ("cycle found during execution").
            blaze_util::unported!(
                "SkyframeErrorProcessor.processErrors",
                "cycle found during execution at {:?}",
                error_key
            );
        };
        let Some(execution_exception) = ExecutionExceptionCause::of(cause) else {
            blaze_util::unported!(
                "SkyframeErrorProcessor.processErrors",
                "exception at {:?} during execution that is not an execution exception: {}",
                error_key,
                cause
            );
        };
        if keep_going {
            // aggregateSingleResult; for execution exceptions, we don't print any extra warning.
            execution_detailed_exit_code =
                blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(
                    execution_detailed_exit_code.take(),
                    Some(execution_exception.get_execution_detailed_exit_code()),
                );
        } else {
            // If the error is execution-related: straightaway rethrow. No further steps
            // required.
            return Err(rethrow(&execution_exception));
        }
    }
    Ok(execution_detailed_exit_code)
}

/// What [`rethrow`] throws.
#[derive(Debug)]
enum RethrownException {
    /// A `BuildFailedException` (`BuildToolError::BuildFailed`), with its `isCatastrophic()`.
    BuildFailed {
        error: BuildToolError,
        catastrophic: bool,
    },
    /// The `TestExecException` (its message) that caused an `ActionExecutionException`: a test
    /// failed without `--test_keep_going`.
    TestExec(String),
}

impl RethrownException {
    fn build_failed(error: BuildToolError) -> RethrownException {
        RethrownException::BuildFailed {
            error,
            catastrophic: false,
        }
    }
}

/// Figure out why an action's analysis/execution failed and rethrow the right kind of exception.
///
/// PORT: returns the `BuildFailedException`; an empty message stands for
/// `isErrorAlreadyShown()` (`BuildTool.processRequest` reports the others).
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:974 SkyframeErrorProcessor.rethrow
fn rethrow(cause: &ExecutionExceptionCause<'_>) -> RethrownException {
    if let ExecutionExceptionCause::ActionExecution(e) = cause
        && e.is_caused_by_test_exec_exception()
    {
        // innerCause instanceof TestExecException: throw testExecException.
        // PORT: the ActionExecutionException's message is the TestExecException's
        // (`ActionExecutionException.fromExecException` without a subtask).
        return RethrownException::TestExec(e.get_message().to_owned());
    }
    match cause {
        ExecutionExceptionCause::ActionExecution(action_execution_cause) => {
            let mut message = action_execution_cause.get_message().to_owned();
            if let Some(action) = action_execution_cause.get_action() {
                message = format!("{} failed: {}", action.describe, message);
                // Sometimes ActionExecutionExceptions are caused by Actions with no owner.
                // PORT: the snapshot always has the owner's location.
                message = format!("{} {}", action.location, message);
            }
            let failure_detail = cause
                .get_execution_detailed_exit_code()
                .get_failure_detail()
                .cloned()
                .expect("NullPointerException: detailedExitCode");
            RethrownException::BuildFailed {
                error: BuildToolError::BuildFailed {
                    // errorAlreadyShown = !actionExecutionCause.showError()
                    message: if action_execution_cause.show_error() {
                        message
                    } else {
                        String::new()
                    },
                    failure_detail,
                },
                catastrophic: action_execution_cause.is_catastrophe(),
            }
        }
        // throw inputFileErrorException: a BuildFailedException whose error is already shown.
        ExecutionExceptionCause::InputFileError(input_file_error_exception) => {
            RethrownException::build_failed(BuildToolError::BuildFailed {
                message: String::new(),
                failure_detail: input_file_error_exception
                    .detailed_exit_code
                    .get_failure_detail()
                    .cloned()
                    .expect("NullPointerException: detailedExitCode"),
            })
        }
        // We encountered an exception we don't think we should have encountered. This can
        // indicate an exception-processing bug in our code, such as lower level exceptions not
        // being properly handled, or in our expectations in this method.
        ExecutionExceptionCause::ArtifactNestedSetEval(_) => {
            let failure_detail = cause
                .get_execution_detailed_exit_code()
                .get_failure_detail()
                .cloned()
                .expect("NullPointerException: unknownExitCode.getFailureDetail()");
            RethrownException::build_failed(BuildToolError::BuildFailed {
                message: failure_detail.message.clone(),
                failure_detail,
            })
        }
    }
}

/// The UI's `TestSummary` subscription (`UiStateTracker.testSummary`): the failed-test count of
/// the `Build completed, N tests FAILED` line.
#[derive(Default)]
struct UiTestSummaryCounter {
    failed_tests: std::sync::atomic::AtomicI32,
}

impl blaze_events::EventBus for UiTestSummaryCounter {
    // Java: com/google/devtools/build/lib/runtime/UiStateTracker.java:1028 UiStateTracker.testSummary
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        use blaze_buildtool::runtime::test_summary::TestSummary;
        use blaze_protos::blaze::BlazeTestStatus;
        if let Some(summary) = blaze_sky_exec::events::downcast_event::<TestSummary>(&event)
            && summary.get_status() != BlazeTestStatus::Passed
            && summary.get_status() != BlazeTestStatus::Flaky
        {
            self.failed_tests
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// The reporter's `OutputFilter` (`None`: `OutputFilter.OUTPUT_EVERYTHING`, the reporter's
/// default, left as it is).
///
/// Java's module sets the filter on the reporter at `BuildStartingEvent` (`--output_filter`) or
/// at `TargetParsingCompleteEvent` (`--auto_output_filter`); the command sets it at
/// `BuildStartingEvent`, and the action executor (which reports through the reporter but asks
/// `reporter.showOutput` itself before it reports an action's output) takes it when it is
/// created.
///
/// PORT: the `--auto_output_filter` values that do not depend on the parsed targets (`none`,
/// `all`) are set at `BuildStartingEvent`, before the target patterns are parsed: the events of
/// that phase have no tag (only the analysis and execution functions tag their events), so the
/// filter has no effect on them either way. `packages` and `subpackages` need the labels of the
/// `TargetParsingCompleteEvent`, which blaze_sky_top's target pattern phase does not post; they
/// fail loudly.
// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:78 OutputFilteringModule.buildStarting
// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:98 OutputFilteringModule.targetParsingComplete
fn output_filter(
    options: &OptionsParsingResult,
    request_options: &BuildRequestOptions,
) -> Option<Arc<dyn blaze_events::OutputFilter>> {
    use blaze_optdefs::command::auto_output_filter::AutoOutputFilter;
    use blaze_optdefs::command::output_filtering_module::OutputFilteringModuleOptions;
    if let Some(output_filter) = &request_options.output_filter {
        return Some(blaze_events::RegexOutputFilter::for_pattern(
            output_filter.regex_pattern().clone(),
        ));
    }
    match options
        .get_options::<OutputFilteringModuleOptions>()
        .map(|o| o.auto_output_filter)
    {
        // AutoOutputFilter.NONE.getFilter: OutputFilter.OUTPUT_EVERYTHING.
        None | Some(AutoOutputFilter::NONE) => None,
        // AutoOutputFilter.ALL.getFilter: OutputFilter.OUTPUT_NOTHING.
        Some(AutoOutputFilter::ALL) => Some(blaze_events::output_nothing()),
        Some(AutoOutputFilter::PACKAGES) | Some(AutoOutputFilter::SUBPACKAGES) => {
            blaze_util::unported!("AutoOutputFilter.getFilter (packages, subpackages)")
        }
    }
}

#[cfg(test)]
mod tests {
    use blaze_exec::actions::action_execution::FailedActionInfo;
    use blaze_skyframe::{
        ERROR_TRANSIENCE_KEY, ErrorInfo, EvaluationResult, SkyFunctionException, Transience,
    };

    use super::*;

    // There is no JUnit test of SkyframeErrorProcessor.rethrow or of the execution half of
    // processErrors at the pin; the expected values are the oracle's (Bazel 9.3.2) transcripts
    // scripts/w4/scenarios/exec-source-errors/ORACLE.txt and ui-build-results/ORACLE.txt.

    fn source_input_missing() -> FailureDetail {
        FailureDetail {
            message: "missing input file '//p:missing_input.txt'".to_owned(),
            category: Some(Category::Execution(failure_details::Execution {
                code: failure_details::execution::Code::SourceInputMissing as i32,
            })),
        }
    }

    /// The exception of `ActionExecutionFunction.throwSourceErrorException` for
    /// `//p:uses_missing` (exec-source-errors step 4), with `<WS>` as the workspace.
    fn missing_input_exception() -> ActionExecutionException {
        ActionExecutionException::with_root_causes(
            "1 input file(s) do not exist",
            Some(FailedActionInfo {
                owner_label: None,
                configuration_checksum: None,
                primary_output: PathFragment::create(
                    "bazel-out/k8-fastbuild/bin/p/uses_missing.out",
                ),
                mnemonic: "Action".to_owned(),
                location: "<WS>/p/BUILD.bazel:12:5".to_owned(),
                pretty_print: "action 'Action p/uses_missing.out'".to_owned(),
                describe: "Action p/uses_missing.out".to_owned(),
            }),
            Vec::new(),
            false,
            Some(source_input_missing()),
        )
    }

    fn result_with_error(e: ActionExecutionException) -> EvaluationResult {
        EvaluationResult {
            values: Vec::new(),
            errors: vec![(
                ERROR_TRANSIENCE_KEY.clone(),
                ErrorInfo::from_exception(
                    &SkyFunctionException::new(e, Transience::Transient),
                    false,
                ),
            )],
            catastrophe: None,
        }
    }

    /// exec-source-errors step 4 (`build //p:uses_missing`): Bazel prints
    /// `ERROR: <WS>/p/BUILD.bazel:12:5 Action p/uses_missing.out failed: 1 input file(s) do not
    /// exist` (the BuildFailedException of `rethrow`, reported by `BuildTool.processRequest`)
    /// and exits with 1.
    #[test]
    fn rethrow_of_a_source_error_reports_location_and_action() {
        let result = result_with_error(missing_input_exception());
        let Err(RethrownException::BuildFailed {
            error:
                BuildToolError::BuildFailed {
                    message,
                    failure_detail,
                },
            catastrophic: false,
        }) = process_execution_errors(&result, /* keep_going= */ false)
        else {
            panic!("expected a BuildFailedException");
        };
        assert_eq!(
            message,
            "<WS>/p/BUILD.bazel:12:5 Action p/uses_missing.out failed: 1 input file(s) do not exist"
        );
        assert_eq!(failure_detail, source_input_missing());
        assert_eq!(
            blaze_util::DetailedExitCode::of(failure_detail)
                .get_exit_code()
                .get_numeric_exit_code(),
            1
        );
    }

    /// ui-build-results step 20 (`build //:fails`): the error of an executed action is printed
    /// once, by the action executor; the BuildFailedException of an
    /// AlreadyReportedActionExecutionException has its error already shown.
    #[test]
    fn rethrow_of_an_already_reported_exception_shows_nothing() {
        let result = result_with_error(ActionExecutionException::already_reported(
            missing_input_exception(),
        ));
        let Err(RethrownException::BuildFailed {
            error: BuildToolError::BuildFailed { message, .. },
            ..
        }) = process_execution_errors(&result, /* keep_going= */ false)
        else {
            panic!("expected a BuildFailedException");
        };
        assert_eq!(message, "");
    }

    /// The TestExecException of `TestRunnerAction.executeAllAttempts` without
    /// `--test_keep_going` ("Test failed, aborting", TestAction.NO_KEEP_GOING_TEST_FAILURE),
    /// already reported by the action executor as `<loc>: Testing //:fail_test failed: Test
    /// failed, aborting`.
    fn test_failed_aborting() -> ActionExecutionException {
        ActionExecutionException::already_reported(
            ActionExecutionException::with_root_causes(
                "Test failed, aborting",
                None,
                Vec::new(),
                false,
                Some(FailureDetail {
                    message: "Testing //:fail_test failed: Test failed, aborting".to_owned(),
                    category: Some(Category::TestAction(failure_details::TestAction {
                        code: failure_details::test_action::Code::NoKeepGoingTestFailure as i32,
                    })),
                }),
            )
            .with_test_exec_exception_cause_for_testing(),
        )
    }

    /// test-no-keep-going step 1 (`test --notest_keep_going //:fail_test`): `rethrow` throws
    /// the TestExecException (`SkyframeErrorProcessor.java:978`), which BuildTool reports as
    /// `ERROR: Test failed, aborting` with a successful build; TestCommand exits with 3.
    #[test]
    fn rethrow_of_a_test_exec_exception_throws_it() {
        let result = result_with_error(test_failed_aborting());
        let Err(RethrownException::TestExec(message)) =
            process_execution_errors(&result, /* keep_going= */ false)
        else {
            panic!("expected a TestExecException");
        };
        assert_eq!(message, "Test failed, aborting");
    }

    /// `rethrow` passes `actionExecutionCause.isCatastrophe()` to the BuildFailedException
    /// (`SkyframeErrorProcessor.java:992`), which BuildTool turns into `result.setCatastrophe()`
    /// (`BuildTool.java:939`).
    #[test]
    fn rethrow_keeps_the_catastrophe() {
        let catastrophic = ActionExecutionException::with_root_causes(
            "catastrophic failure",
            None,
            Vec::new(),
            true,
            Some(source_input_missing()),
        );
        let Err(RethrownException::BuildFailed { catastrophic, .. }) =
            process_execution_errors(&result_with_error(catastrophic), false)
        else {
            panic!("expected a BuildFailedException");
        };
        assert!(catastrophic);
    }

    /// exec-source-errors step 8 (`build --keep_going ...`): nothing more is printed for the
    /// execution errors (no `ERROR: <loc> Action ... failed` line), and the build fails with the
    /// execution exit code (1).
    #[test]
    fn keep_going_returns_the_execution_exit_code() {
        let result = result_with_error(missing_input_exception());
        let code = process_execution_errors(&result, /* keep_going= */ true)
            .unwrap_or_else(|_| panic!("--keep_going does not throw"))
            .expect("an execution exit code");
        assert_eq!(code.get_failure_detail(), Some(&source_input_missing()));
        assert_eq!(code.get_exit_code().get_numeric_exit_code(), 1);
    }

    #[test]
    fn no_errors_no_exit_code() {
        let result = EvaluationResult {
            values: Vec::new(),
            errors: Vec::new(),
            catastrophe: None,
        };
        assert!(matches!(process_execution_errors(&result, true), Ok(None)));
        assert!(matches!(process_execution_errors(&result, false), Ok(None)));
    }
}
