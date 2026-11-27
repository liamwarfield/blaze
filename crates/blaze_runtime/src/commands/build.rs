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
use blaze_buildtool::buildtool::action_execution_inactivity_watchdog::ActionExecutionInactivityWatchdog;
use blaze_buildtool::buildtool::action_execution_status_reporter::ActionExecutionStatusReporter;
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
use blaze_exec::exec::spawn_log_module::{
    DeferredSpawnLogContext, SpawnLogModule, SpawnLogModuleEnv,
};
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
use blaze_sky_analysis::top_level_status_events::TopLevelTargetAnalyzedEvent;
use blaze_sky_exec::build_driver_function::TopLevelSkyValue;
use blaze_sky_exec::events::TestAnalyzedEvent;
use blaze_sky_exec::keys::{
    AspectCompletionKey, BuildDriverKey, TargetCompletionKey, TestCompletionKey, TestType,
};
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
use blaze_sky_top::skyframe_error_processor::{
    ExecutionExceptionCause, RethrownException, rethrow,
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
    AfterCommand, create_skyframe_executor_with_execution_and_repo_precomputed, reporter,
    to_runtime_detailed_exit_code,
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
        run_build(env, options, false, None)
    }
}

/// Handles the 'test' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:75 TestCommand
pub struct TestCommand;

impl BlazeCommand for TestCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &TEST_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:83 TestCommand.editOptions
    fn edit_options(&self, options_parser: &mut blaze_options::OptionsParser) {
        let test_output = options_parser
            .get_options::<ExecutionOptions>()
            .expect("ExecutionOptions")
            .test_output;
        if test_output == blaze_optdefs::command::execution_options::TestOutputFormat::STREAMED
            && let Err(e) = options_parser.parse_with_priority(
                blaze_options::PriorityCategory::SOFTWARE_REQUIREMENT,
                Some("streamed output requires locally run tests, without sharding"),
                vec![
                    "--test_sharding_strategy=disabled".to_string(),
                    "--test_strategy=exclusive".to_string(),
                ],
            )
        {
            panic!(
                "IllegalStateException: Known options failed to parse: {}",
                e.get_message()
            );
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:98 TestCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        run_build(env, options, true, None)
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
/// What `RunCommand.runBuild` passes to `BuildTool.processRequest` besides the request (its
/// targets and its `TargetValidator`), and the rest of `RunCommand.exec`, which runs on the
/// `BuildResult` once `processRequest` returned (crate::commands::run).
pub(crate) struct RunBuild<'a> {
    /// `BuildRequest.Builder.setTargets(targetsToBuild)`.
    pub(crate) targets: Vec<String>,
    pub(crate) validator: &'a dyn blaze_sky_top::build_tool::TargetValidator,
    pub(crate) after_build: Box<dyn FnOnce(RunBuildOutcome<'_>) -> BlazeCommandResult + 'a>,
    /// The precomputed values the command injected into `env.getSkyframeExecutor()` before the
    /// build (`fetch --force`'s FORCE_FETCH); the default for `run`.
    pub(crate) repo_precomputed: blaze_repo_rules::registration::RepoPrecomputedValues,
}

/// What `BuildTool.processRequest` leaves to `RunCommand`: its `BuildResult` and the command's
/// Skyframe executor (`env.getSkyframeExecutor()`).
pub(crate) struct RunBuildOutcome<'b> {
    pub(crate) env: &'b mut CommandEnvironment,
    pub(crate) build_result: &'b BuildResult,
    /// `buildResult.getDetailedExitCode()`.
    pub(crate) detailed_exit_code: DetailedExitCode,
    pub(crate) skyframe_executor: &'b SkyframeExecutor,
    /// The configured targets analyzed by the build (`BuildResultListener`), from which the
    /// `ConfiguredTarget`s behind `build_result`'s views are taken.
    pub(crate) analyzed_targets: Vec<ConfiguredTarget>,
    /// The command's options parser after `BuildTool.buildTargets` parsed a project file's
    /// flags into it (Java mutates the parser the command holds); `None` when no project file
    /// set flags.
    pub(crate) updated_options: Option<Arc<OptionsParsingResult>>,
}

pub(crate) fn run_build(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
    run_tests: bool,
    run: Option<RunBuild<'_>>,
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
    // The command's BEP, set up by BuildEventServiceModule.beforeCommand in the dispatcher;
    // the remote module's uploader factory is known from here on
    // (BuildEventArtifactUploaderFactoryDelegate.init).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1351 BuildEventArtifactUploaderFactoryDelegate.init
    let bep: Option<Arc<BepCommandSupport>> = env.get_bep().cloned();
    if let Some(bep) = &bep {
        bep.set_uploader_factory_map(build_remote::build_event_artifact_uploader_factory_map(
            remote.as_ref(),
        ));
    }
    // MetricsModule.beforeCommand: the MetricsCollector exists for every command; with a BEP
    // transport it is the BEP support's, otherwise this one's.
    // Java: com/google/devtools/build/lib/metrics/MetricsModule.java:78 MetricsModule.beforeCommand
    let metrics_only = bep
        .is_none()
        .then(|| crate::commands::bep::MetricsOnlyCommandSupport::before_command(env));
    let result = run_build_with_bep(
        env,
        options,
        run_tests,
        bep.as_ref(),
        metrics_only.as_deref(),
        remote.as_ref(),
        run,
    );
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
    // BlockWaitingModule's executor is replaced by running its one submitter's task
    // (RemoteModule.afterCommandTask) inline, in the module order Java's afterCommand waits in.
    // PORT: skipped, replaced by an inline call: BlockWaitingModule.beforeCommand, BlockWaitingModule.submit
    // PORT: skipped, replaced by an inline call: BlockWaitingModule.call
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
    // BuildEventServiceModule.afterCommand (the dispatcher's call of the module's afterCommand
    // then finds it done).
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
            // A run keeps its exec request: BlazeCommandResult.execute(execRequest,
            // moduleExitCode).
            match result.get_exec_request() {
                Some(exec_request) => {
                    BlazeCommandResult::execute_with_detailed_exit_code(exec_request.clone(), code)
                }
                None => BlazeCommandResult::detailed_exit_code(code),
            }
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
    metrics_only: Option<&crate::commands::bep::MetricsOnlyCommandSupport>,
    remote: Option<&Arc<build_remote::RemoteSupport>>,
    run: Option<RunBuild<'_>>,
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

    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:161 TestCommand.doTest
    // (builder.setNeedsInstrumentationFilter(true), on the request below).
    let needs_instrumentation_filter = run_tests
        && options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .is_some_and(|o| o.collect_code_coverage)
        && !options.contains_explicit_option(
            blaze_sky_top::instrumentation_filter_support::INSTRUMENTATION_FILTER_FLAG,
        );
    // The modules subscribed to the command's event bus.
    let collector = Arc::new(ExecutionEventCollector::new());
    // BuildSummaryStatsModule.beforeCommand: commandStartMillis = env.getCommandStartTime().
    collector.set_command_start_millis(start_time_millis);
    env.get_event_bus().register(collector.clone());
    let ui_test_summaries = Arc::new(UiTestSummaryCounter::default());
    env.get_event_bus().register(ui_test_summaries.clone());
    // CommandEnvironment.<init>: eventBus.register(buildResultListener)
    let build_result_listener =
        Arc::new(blaze_sky_top::build_result_listener::BuildResultListener::default());
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
    // RunCommand.runBuild: the run target (and the --run_under label), not the residue.
    let (targets, run, repo_precomputed) = match run {
        Some(RunBuild {
            targets,
            validator,
            after_build,
            repo_precomputed,
        }) => (targets, Some((validator, after_build)), repo_precomputed),
        None => match target_patterns_helper::read_from(env, options) {
            Ok(targets) => (targets, None, Default::default()),
            Err(e) => {
                reporter.handle(&Event::error(e.get_message()));
                return BlazeCommandResult::failure_detail(e.get_failure_detail());
            }
        },
    };
    // BuildCommand.exec only: RunCommand, TargetFetcher and PrintActionCommand call
    // BuildTool.processRequest directly.
    // Java: com/google/devtools/build/lib/runtime/commands/BuildCommand.java:80 BuildCommand.exec
    if run.is_none() && !run_tests && targets.is_empty() {
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

    // SpawnLogModule (--execution_log_*_file): its executorInit runs where Java constructs the
    // ExecutionTool (`ExecutionToolConstruction`); the executor registers the deferred context
    // it fills (see `DeferredSpawnLogContext`).
    let spawn_log = SpawnLogSetup {
        module: RefCell::new(SpawnLogModule::new()),
        deferred: blaze_exec::exec::spawn_log_module::execution_log_requested(Some(
            &execution_options,
        ))
        .then(|| Arc::new(DeferredSpawnLogContext::new())),
        initialized: Cell::new(false),
        execution_options: execution_options.clone(),
        // env.getOptions().getOptions(RemoteOptions.class)
        remote_options: options
            .get_options::<blaze_optdefs::misc::remote_options::RemoteOptions>()
            .map(Arc::new),
        sibling_repository_layout: build_language_options.experimental_sibling_repository_layout,
        file_system: file_system.clone(),
        output_base: env.get_output_base().clone(),
        exec_root: exec_root.clone(),
        workspace: workspace.clone(),
        working_directory: env.get_working_directory().clone(),
        workspace_name: env.get_workspace_name().to_string(),
        command_id: env.get_command_id().to_string(),
        reporter: env.get_reporter().clone() as Arc<dyn EventHandler>,
        module_environment: env.get_blaze_module_environment().clone(),
    };
    let spawn_log_context = spawn_log
        .deferred
        .clone()
        .map(|d| d as Arc<dyn blaze_exec::exec::spawn_log_context::SpawnLogContext>);

    // OutputFilteringModule: the reporter's filter (see `output_filter`).
    let (reporter_output_filter, auto_output_filter) =
        output_filter(options, &build_request_options);

    let jobs = if build_request_options.jobs > 0 {
        build_request_options.jobs as usize
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    check_strategy_policy_is_default(env);
    let config = ExecutionToolConfig {
        file_system: file_system.clone(),
        output_base: env.get_output_base().clone(),
        exec_root: exec_root.clone(),
        install_base: directories.get_install_base().clone(),
        client_env: env.get_client_env().clone(),
        keep_going,
        verbose_failures: execution_options.verbose_failures,
        jobs,
        command_id: env.get_command_id().to_string(),
        // The modules' registerSpawnStrategies read `env.getOptions()`: the options of the
        // first round of parsing, which the CommandEnvironment keeps (the dispatcher re-parses
        // into a new parser with the main repo mapping, which only the command sees). Its labels
        // were converted without a repo context (`Label.parseCanonical`), so
        // `--allowed_strategies_by_exec_platform=@platforms//host` is `@@platforms//host:host`
        // there; the other fields are the same in both rounds.
        // Java: com/google/devtools/build/lib/bazel/rules/BazelStrategyModule.java:64 BazelStrategyModule.registerSpawnStrategies
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:706 BlazeCommandDispatcher.execExclusively
        execution_options: Some(options_or_default::<ExecutionOptions>(env.get_options())),
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
    let skyframe_executor = match create_skyframe_executor_with_execution_and_repo_precomputed(
        env,
        env.get_options().clone(),
        Some(build_options),
        Some(setup),
        repo_precomputed,
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
    request.set_needs_instrumentation_filter(needs_instrumentation_filter);
    // BuildTool.buildTargets: the project file's options go through the command's parser.
    request.project_options_context.config_flag_definitions =
        env.get_config_flag_definitions().clone();
    // `processRequest(request, validator, options)`: the command's parser (not
    // env.getOptions(), which keeps the dispatcher's first parse).
    let mut applier = crate::project_options::BuildProjectOptionsApplier::new(
        Arc::new(options.clone()),
        env.get_relative_working_directory().clone(),
        reporter.clone(),
    );
    if let Some(bep) = bep {
        let bep = bep.clone();
        let product_name = env.get_runtime().get_product_name().to_string();
        let startup_options = env.get_runtime().get_startup_options_provider_arc();
        let command_name = command_name.to_string();
        applier = applier.with_canonical_command_line_poster(Box::new(move |parser| {
            bep.post_project_canonical_command_line(
                &product_name,
                startup_options.clone(),
                &command_name,
                parser,
            )
        }));
    }
    let project_options_applier = Arc::new(applier);
    request.project_options_applier = Some(project_options_applier.clone());
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
    // ActionExecutionFunction's SkyframeInputMetadataProvider reads done values from the
    // evaluator (`skyframeActionExecutor.getEvaluator()`).
    if let Some(execution) = skyframe_executor.get_execution() {
        execution
            .prepared
            .executor
            .set_memoizing_evaluator_graph(skyframe_executor.get_walkable_graph());
    }
    if let Some(remote) = remote {
        remote.set_merged_analysis_and_execution(merged_analysis_and_execution);
        if let Some(execution) = skyframe_executor.get_execution() {
            remote.set_was_rewound(execution.prepared.executor.clone());
        }
        remote.set_rewinding_action_graph(skyframe_executor.get_walkable_graph());
    }
    if let Some(bep) = bep {
        bep.set_merged_analysis_and_execution(merged_analysis_and_execution);
    }
    // BlazeCommandDispatcher.createEventHandler: `new UiEventHandler(...,
    // env.withMergedAnalysisAndExecutionSourceOfTruth(), newStatsSummary)` picks the
    // SkymeldUiStateTracker; its @Subscribe methods are the UI subscriber on the event bus.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:1037 BlazeCommandDispatcher.createEventHandler
    // PORT: the dispatcher created the handler with a UiStateTracker before this decision; the
    // tracker is replaced here, and the new one is told the one event the old one received
    // (MainRepoMappingComputationStartingEvent, posted by the dispatcher).
    let ui_subscriber = env.get_ui_event_subscriber().cloned().map(|subscriber| {
        let tracker = Arc::new(crate::dispatcher::new_ui_state_tracker(
            subscriber.get_handler(),
            &ui_options,
            merged_analysis_and_execution,
            execution_options.stats_summary,
        ));
        tracker.main_repo_mapping_computation_started();
        subscriber.set_state_tracker(tracker);
        subscriber
    });
    // ExecutionTool.prepareForExecution: the ExecutionProgressReceiverSetup is registered on the
    // event bus; it creates the build's ExecutionProgressReceiver on the first
    // SomeExecutionStartedEvent.
    // PORT: registered for every build that executes (see ExecutionProgressReceiverSetup).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:343 ExecutionTool.prepareForExecution
    // The inactivity watchdog of the execution (SkyframeExecutor.watchdog) and
    // SkyframeExecutor.isBuildingExclusiveArtifacts.
    let watchdog: Arc<parking_lot::Mutex<Option<Arc<ActionExecutionInactivityWatchdog>>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let is_building_exclusive_artifacts = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if skyframe_executor.get_execution().is_some() {
        let skyframe_progress_receiver = skyframe_executor.get_progress_receiver().clone();
        let skyframe_action_executor = skyframe_executor
            .get_execution()
            .map(|execution| execution.prepared.executor.clone());
        let status_event_bus = env.get_event_bus().clone();
        let status_event_handler: Arc<dyn blaze_events::EventHandler> = reporter.clone();
        let progress_report_interval = build_request_options.progress_report_interval;
        let watchdog = watchdog.clone();
        let is_building_exclusive_artifacts = is_building_exclusive_artifacts.clone();
        env.get_event_bus().register(Arc::new(
            blaze_buildtool::buildtool::execution_progress_receiver::ExecutionProgressReceiverSetup::new(
                env.get_event_bus().clone(),
                Box::new(move |execution_progress_receiver| {
                    // ActionExecutionStatusReporter.create(env.getReporter(),
                    //     skyframeExecutor.getEventBus()): registered on the event bus.
                    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1059 ExecutionTool.ExecutionProgressReceiverSetup.setupExecutionProgressReceiver
                    let status_reporter = ActionExecutionStatusReporter::create(
                        status_event_handler.clone(),
                        None,
                    );
                    status_event_bus.register(status_reporter.clone());
                    // skyframeExecutor.setActionExecutionProgressReportingObjects(
                    //     executionProgressReceiver, executionProgressReceiver, statusReporter)
                    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1061 ExecutionTool.ExecutionProgressReceiverSetup.setupExecutionProgressReceiver
                    if let Some(executor) = &skyframe_action_executor {
                        executor.set_action_execution_progress_reporting_objects(
                            Some(execution_progress_receiver.clone()),
                            Some(execution_progress_receiver.clone()),
                        );
                        executor.set_action_execution_status_reporter(Some(status_reporter.clone()));
                    }
                    // skyframeExecutor.setExecutionProgressReceiver(executionProgressReceiver)
                    skyframe_progress_receiver
                        .set_execution_progress_receiver(Some(execution_progress_receiver.clone()));
                    // skyframeExecutor.setAndStartWatchdog(new ActionExecutionInactivityWatchdog(
                    //     executionProgressReceiver.createInactivityMonitor(statusReporter),
                    //     executionProgressReceiver.createInactivityReporter(statusReporter,
                    //         skyframeExecutor.getIsBuildingExclusiveArtifacts()),
                    //     progressReportInterval))
                    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1065 ExecutionTool.ExecutionProgressReceiverSetup.setupExecutionProgressReceiver
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3483 SkyframeExecutor.setAndStartWatchdog
                    let new_watchdog = Arc::new(ActionExecutionInactivityWatchdog::new(
                        execution_progress_receiver.create_inactivity_monitor(status_reporter.clone()),
                        execution_progress_receiver.create_inactivity_reporter(
                            status_reporter,
                            is_building_exclusive_artifacts.clone(),
                            status_event_bus.clone(),
                        ),
                        progress_report_interval,
                    ));
                    *watchdog.lock() = Some(new_watchdog.clone());
                    new_watchdog.start();
                }),
            ),
        ));
    }

    let execution_phase = RuntimeExecutionPhase {
        watchdog: watchdog.clone(),
        is_building_exclusive_artifacts: is_building_exclusive_artifacts.clone(),
        expand_filesets: options
            .get_options::<blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions>()
            .map(|o| o.expand_filesets)
            .unwrap_or(false),
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
        extra_action_filter: options
            .get_options::<blaze_optdefs::command::analysis_options::AnalysisOptions>()
            .map(|o| o.extra_action_filter),
        sibling_repository_layout: build_language_options.experimental_sibling_repository_layout,
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
            running_in_emacs: ui_options.running_in_emacs,
        },
        bep: bep.cloned(),
        remote: remote.cloned(),
        // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:78 BazelCoverageReportModule.getCoverageReportFactory
        coverage_report_action_factory:
            blaze_analysis::bazel::coverage::bazel_coverage_report_module::get_coverage_report_factory(
                options.get_options::<
                    blaze_optdefs::misc::bazel_coverage_report_module::BazelCoverageReportModuleOptions,
                >(),
            ),
        non_merged_coverage_artifacts: std::cell::RefCell::new(None),
        event_bus: env.get_event_bus().clone(),
        incremental_package_roots: RefCell::new(None),
        tracks_state_for_incrementality: env.tracks_state_for_incrementality(),
        is_run_command: request.command_name == "run",
        top_level_target_analysis_watcher: parking_lot::Mutex::new(None),
    };
    let post_processor = ExecutionToolConstruction {
        execution_phase: &execution_phase,
        spawn_log: &spawn_log,
        cache_file_digests_initialized: Cell::new(false),
    };
    let build_tool = BuildTool::new(
        &skyframe_executor,
        reporter.clone(),
        env.get_relative_working_directory().clone(),
        &post_processor,
    )
    .with_execution_phase(&execution_phase);
    // BuildTool.dumpSkyframeStateAfterBuild (--experimental_aquery_dump_after_build_format).
    let aquery_dump_after_build =
        blaze_cquery::buildtool::aquery_processor::DumpSkyframeStateAfterBuild {
            reporter: reporter.clone(),
            output_base: env.get_output_base().clone(),
            streaming_log_file_uploads: options_or_default::<
                blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions,
            >(options)
            .streaming_log_file_uploads,
        };
    let build_tool = build_tool.with_aquery_dump_after_build(&aquery_dump_after_build);
    // BuildTool.processRequest(request, validator, ...): RunCommand's TargetValidator.
    let build_tool = match &run {
        Some((validator, _)) => build_tool.with_target_validator(*validator),
        None => build_tool,
    };
    // The build event stream's loading and analysis posting points.
    let event_hooks: Option<&dyn blaze_sky_top::build_tool::BuildEventHooks> =
        match (bep, metrics_only) {
            (Some(bep), _) => Some(bep.as_ref()),
            // Without a transport, the MetricsCollector's analysis hook.
            (None, Some(metrics_only)) => Some(metrics_only),
            (None, None) => None,
        };
    // OutputFilteringModule.targetParsingComplete, a subscriber of the TargetParsingCompleteEvent
    // posted after the target patterns are evaluated.
    let output_filtering_hooks = auto_output_filter.map(|(auto, slot)| OutputFilteringHooks {
        inner: event_hooks,
        auto_output_filter: auto,
        slot,
    });
    let build_tool = match (&output_filtering_hooks, event_hooks) {
        (Some(hooks), _) => build_tool.with_build_event_hooks(hooks),
        (None, Some(hooks)) => build_tool.with_build_event_hooks(hooks),
        (None, None) => build_tool,
    };
    // BuildTool.buildTargets: BuildStartingEvent. Its subscriber OutputFilteringModule sets the
    // reporter's output filter.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:288 BuildTool.buildTargets
    // Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:78 OutputFilteringModule.buildStarting
    if let Some(filter) = &reporter_output_filter {
        env.get_reporter().set_output_filter(filter.clone());
    }
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:658 UiEventHandler.buildStarted
    if let Some(handler) = env.get_ui_event_handler() {
        handler.build_started();
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
    // WorkerModule.buildStarting (a BuildStartingEvent subscriber): the worker factory, pool and
    // lifecycle manager of the build.
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:92 WorkerModule.buildStarting
    let worker_options = Arc::new(options_or_default::<
        blaze_optdefs::misc::worker_options::WorkerOptions,
    >(options));
    // env.getBlazeWorkspace().getBinTools(): only read for the hardened worker sandbox (scanning
    // the install base is not free).
    let worker_bin_tools = if worker_options.sandbox_hardening {
        blaze_exec::exec::bin_tools::BinTools::for_production(
            env.get_directories().get_install_base().clone(),
        )
        .ok()
        .map(Arc::new)
    } else {
        None
    };
    blaze_exec::worker::worker_module::WorkerModule::build_starting(
        blaze_exec::worker::worker_module::WorkerBuildStarting {
            worker_options,
            sandbox_options: Some(&options_or_default::<SandboxOptions>(options)),
            output_base: env.get_output_base(),
            reporter: env.get_reporter().clone() as Arc<dyn EventHandler>,
            bin_tools: worker_bin_tools,
            file_system: Some(env.get_output_base().get_file_system().clone()),
        },
    );
    let detailed_exit_code = to_runtime_detailed_exit_code(build_tool.process_request(&request));
    // The project file's flags were parsed into the command's options parser
    // (BuildTool.buildTargets); RunCommand reads that parser after the build.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:352 BuildTool.buildTargets
    let updated_options = project_options_applier.take_updated_options();
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
    // The ExecutionFinishedEvent of ExecutionTool.unconditionalExecutionPhaseFinalizations is
    // posted whenever the ExecutionTool exists (the Skymeld path constructs it when it enters
    // buildTargetsWithMergedAnalysisExecution: ExecutionToolConstruction::before_merged_analysis).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:601 ExecutionTool.unconditionalExecutionPhaseFinalizations
    if collector.executor_initialized() {
        collector.execution_phase_finish();
    }
    build_result.set_detailed_exit_code(Arc::new(to_util_detailed_exit_code(&detailed_exit_code)));
    // The stop time has to be captured before we send the BuildCompleteEvent.
    build_result.set_stop_time(JavaClock.current_time_millis());
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1174 BuildTool.stopRequest
    blaze_util::profiler::Profiler::instance()
        .mark_phase(blaze_util::profiler::ProfilePhase::FINISH);
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
    if let Some(ui_subscriber) = &ui_subscriber {
        ui_subscriber.begin_build_complete();
    }
    if let Some(test_support) = &test_support {
        test_support.build_complete_event(&build_result);
    }
    // RemoteExecutionService.onBuildComplete (a BuildCompleteEvent subscriber).
    if let Some(remote) = remote {
        remote.build_complete(build_result.get_success());
    }
    // WorkerModule.buildComplete (a BuildCompleteEvent subscriber).
    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:246 WorkerModule.buildComplete
    blaze_exec::worker::worker_module::WorkerModule::build_complete();
    report_build_summary_stats(
        reporter.as_ref(),
        &collector,
        build_result.get_elapsed_seconds(),
        build_result.get_stop_time(),
    );
    // UiEventHandler.buildComplete (UiStateTracker.buildComplete's line, written by the UI only).
    match &ui_subscriber {
        Some(ui_subscriber) => ui_subscriber.build_complete(build_result.get_success()),
        None => reporter.handle(&build_complete_event(
            build_result.get_success(),
            failed_tests,
            &collector,
        )),
    }
    // SpawnLogModule.buildComplete: the log is finalized, and the file it publishes joins the
    // build tool logs of the BuildToolLogs event below (after BuildSummaryStatsModule's entries,
    // see BepCommandSupport::add_execution_log).
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:205 SpawnLogModule.buildComplete
    if let Some((name, path)) = spawn_log.module.borrow_mut().build_complete()
        && let Some(bep) = bep
    {
        bep.add_execution_log(name, path);
    }
    // The BuildCompleteEvent's BEP subscribers (BuildSummaryStatsModule's tool logs, the UI's
    // progress bar while the transports are open, the streamer), then the BuildToolLogs event.
    if let Some(bep) = bep {
        bep.build_complete(
            &mut build_result,
            &to_util_detailed_exit_code(&detailed_exit_code),
            &collector,
            skyframe_executor
                .get_execution()
                .map(|e| &*e.prepared.executor),
        );
    }
    // RunCommand.exec after BuildTool.processRequest returned; the modules' afterCommand below
    // runs after the command (BlazeRuntime.afterCommand), with the ExecRequest kept.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:415 RunCommand.runBuild
    let run_result = run.map(|(_, after_build)| {
        let analyzed_targets = build_result_listener.get_analyzed_targets();
        after_build(RunBuildOutcome {
            env: &mut *env,
            build_result: &build_result,
            detailed_exit_code: detailed_exit_code.clone(),
            skyframe_executor: &skyframe_executor,
            analyzed_targets,
            updated_options,
        })
    });
    // BlazeRuntime.afterCommand: SkyframeExecutor.notifyCommandComplete posts the evaluator's
    // SkyframeGraphStatsEvent (resetting its statistics) for every command, before the
    // buildtool.CommandPrecompleteEvent whose MetricsCollector subscriber builds the
    // BuildMetricsEvent.
    // PORT: taken here, where the command's executor ends (the build's last evaluation is done);
    // the precomplete follows in `run_build`.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:759 BlazeRuntime.afterCommand
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1196 SkyframeExecutor.notifyCommandComplete
    let graph_stats = skyframe_executor.get_evaluator().post_logging_stats();
    if let Some(bep) = bep {
        bep.skyframe_graph_stats(&graph_stats);
    }
    // SandboxModule.afterCommand.
    if let Some(execution) = skyframe_executor.get_execution() {
        execution.after_command();
    }
    let spawn_log_after_command = spawn_log.module.borrow_mut().after_command();
    if let Err(e) = spawn_log_after_command {
        reporter.handle(&Event::error(e.failure_detail.message.clone()));
        // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:795 BlazeRuntime.afterCommand
        if let Some(run_result) = &run_result
            && let Some(exec_request) = run_result.get_exec_request()
            && !run_result.get_exit_code().is_infrastructure_failure()
        {
            return BlazeCommandResult::execute_with_detailed_exit_code(
                exec_request.clone(),
                DetailedExitCode::of(e.failure_detail),
            );
        }
        return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(e.failure_detail));
    }
    if let Some(run_result) = run_result {
        return run_result;
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
    // TestCommand.analyzeTestResults: with the validation aspect, the base targets of the
    // successful ValidateTarget aspects.
    // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:237 TestCommand.analyzeTestResults
    let validated_targets: Option<
        Vec<blaze_buildtool::runtime::aggregating_test_listener::ConfiguredTargetKey>,
    > = if validation_mode(&build_request_options) == ValidationMode::Aspect {
        Some(
            build_result
                .get_successful_aspects()
                .into_iter()
                .flatten()
                .filter_map(|key| {
                    key.as_any()
                        .downcast_ref::<blaze_analysis::aspect_key_creator::AspectKey>()
                })
                .filter(|key| {
                    key.get_aspect_name()
                        == blaze_buildtool::buildtool::build_result_printer::VALIDATION_ASPECT_NAME
                })
                .map(|key| {
                    let base = key.get_base_configured_target_key();
                    blaze_buildtool::runtime::aggregating_test_listener::ConfiguredTargetKey {
                        label: base.get_label().clone(),
                        configuration_checksum: base
                            .get_configuration_checksum()
                            .map(str::to_owned),
                    }
                })
                .collect(),
        )
    } else {
        None
    };
    let code = test_support.finish(
        &build_result,
        reporter.as_ref(),
        printer,
        formatter,
        main_repo_mapping,
        validated_targets.as_deref(),
    );
    let exit_code = get_exit_code(&code);
    let detailed = match code.get_failure_detail() {
        None => DetailedExitCode::success(),
        Some(failure_detail) => DetailedExitCode::of_exit_code(exit_code, failure_detail.clone()),
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
/// With a build event stream, the `BuildCompleteEvent` then reaches its BEP subscribers and the
/// `BuildToolLogs` event is posted ([`BepCommandSupport::build_complete`]).
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1155 BuildTool.stopRequest
pub(crate) fn stop_request_without_execution(
    env: &CommandEnvironment,
    detailed_exit_code: &blaze_query_engine::DetailedExitCode,
    collector: &ExecutionEventCollector,
) {
    let reporter = reporter(env);
    let mut result = BuildResult::new(env.get_command_start_time());
    // The stop time has to be captured before we send the BuildCompleteEvent.
    result.set_stop_time(JavaClock.current_time_millis());
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1174 BuildTool.stopRequest
    blaze_util::profiler::Profiler::instance()
        .mark_phase(blaze_util::profiler::ProfilePhase::FINISH);
    // No action ran: the counters of BuildSummaryStatsModule and of the UI are zero.
    report_build_summary_stats(
        reporter.as_ref(),
        collector,
        result.get_elapsed_seconds(),
        result.get_stop_time(),
    );
    // BuildResult.getSuccess()
    reporter.handle(&build_complete_event(
        detailed_exit_code.is_success(),
        0,
        collector,
    ));
    // The BuildCompleteEvent's BEP subscribers, then the BuildToolLogs event.
    if let Some(bep) = env.get_bep() {
        let detailed_exit_code = to_runtime_detailed_exit_code(detailed_exit_code.clone());
        result.set_detailed_exit_code(Arc::new(detailed_exit_code.clone()));
        // No execution state: the action executor's metrics consumers are not reached.
        bep.build_complete(&mut result, &detailed_exit_code, collector, None);
    }
}

/// The runtime's `DetailedExitCode` as `blaze_util`'s (what `BuildResult` holds).
fn to_util_detailed_exit_code(code: &DetailedExitCode) -> blaze_util::DetailedExitCode {
    match code.get_failure_detail() {
        None => blaze_util::DetailedExitCode::success(),
        // The same type (crate::exit_code re-exports blaze_util's): the ExitCode is kept, an
        // unregistered one (PARSING_FAILURE) included.
        Some(_) => code.clone(),
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
/// `BuildSummaryStatsModule.executorInit` of the build's `ExecutionTool`.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:91 BuildSummaryStatsModule.executorInit
fn executor_init_build_summary_stats(
    collector: &ExecutionEventCollector,
    execution_options: &ExecutionOptions,
    state: &blaze_sky_top::execution_phase::ExecutionState,
) {
    collector.executor_init(
        execution_options.enable_critical_path_profiling,
        execution_options.stats_summary,
        blaze_buildtool::buildtool::execution_tool::can_be_shared_with_executor(
            state.prepared.executor.clone(),
        ),
    );
}

/// `SpawnLogModule` and what its `executorInit` reads of the command environment.
struct SpawnLogSetup {
    module: RefCell<SpawnLogModule>,
    /// The context the executor registered (when a log is requested).
    deferred: Option<Arc<DeferredSpawnLogContext>>,
    /// Whether `executorInit` ran (the ExecutionTool was constructed).
    initialized: Cell<bool>,
    execution_options: ExecutionOptions,
    remote_options: Option<Arc<blaze_optdefs::misc::remote_options::RemoteOptions>>,
    sibling_repository_layout: bool,
    file_system: blaze_vfs::FileSystemRef,
    output_base: Path,
    exec_root: Path,
    workspace: Path,
    working_directory: Path,
    workspace_name: String,
    command_id: String,
    reporter: Arc<dyn EventHandler>,
    module_environment: crate::command_environment::BlazeModuleEnvironment,
}

impl SpawnLogSetup {
    /// `SpawnLogModule.executorInit` and `registerActionContexts`, run by
    /// `new ExecutionTool(env, request)` (once per command). An error is the module's
    /// `env.getBlazeModuleEnvironment().exit(...)`: a pending exit, which the build does not see
    /// until it is thrown (before a merged analysis, or when the command completes).
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:188 SpawnLogModule.executorInit
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:178 SpawnLogModule.registerActionContexts
    fn executor_init(&self) {
        if self.initialized.replace(true) {
            return;
        }
        let env = SpawnLogModuleEnv {
            execution_options: Some(&self.execution_options),
            remote_options: self.remote_options.clone(),
            sibling_repository_layout: self.sibling_repository_layout,
            file_system: &self.file_system,
            output_base: &self.output_base,
            exec_root: &self.exec_root,
            workspace: Some(&self.workspace),
            working_directory: &self.working_directory,
            workspace_name: &self.workspace_name,
            command_id: &self.command_id,
            reporter: self.reporter.clone(),
        };
        let created = blaze_exec::exec::spawn_log_module::create_spawn_log_context(
            &mut self.module.borrow_mut(),
            &env,
        );
        let context = match created {
            Ok(context) => context,
            Err(e) => {
                self.module_environment
                    .exit(crate::module::AbruptExitException::new(
                        DetailedExitCode::of(e.failure_detail),
                    ));
                None
            }
        };
        if let Some(deferred) = &self.deferred {
            deferred.set(context);
        }
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:520 BuildTool.buildTargetsWithoutMergedAnalysisExecution
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:613 BuildTool.buildTargetsWithMergedAnalysisExecution
/// `skyframeExecutor.setSaltAndDeleteActionsIfChanged(actionExecutionSalt)` of
/// `ExecutionTool.prepareForExecution`, after `configureActionExecutor` (the executor's salt:
/// `RemoteModule.executorInit`'s, "" without one).
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:332 ExecutionTool.prepareForExecution
fn set_salt_and_delete_actions_if_changed(skyframe_executor: &SkyframeExecutor) {
    if let Some(state) = skyframe_executor.get_execution() {
        skyframe_executor.set_salt_and_delete_actions_if_changed(
            &state
                .config
                .remote
                .action_execution_salt
                .clone()
                .unwrap_or_default(),
        );
    }
}

struct ExecutionToolConstruction<'a> {
    execution_phase: &'a RuntimeExecutionPhase,
    spawn_log: &'a SpawnLogSetup,
    /// Whether `CacheFileDigestsModule.executorInit` ran for this command (once, when Java
    /// constructs the `ExecutionTool`: before a merged analysis, else after the analysis).
    cache_file_digests_initialized: Cell<bool>,
}

impl AnalysisPostProcessor for ExecutionToolConstruction<'_> {
    /// Skymeld constructs the ExecutionTool before the analysis (`new ExecutionTool(env,
    /// request)` at the start of `buildTargetsWithMergedAnalysisExecution`): the modules'
    /// `executorInit` and the executor ([`Self::construct`]).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:613 BuildTool.buildTargetsWithMergedAnalysisExecution
    fn before_merged_analysis(
        &self,
        _request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<(), BuildToolError> {
        self.construct(skyframe_executor)
    }

    /// `env.throwPendingException()` of `AnalysisAndExecutionPhaseRunner.execute`: the exit a
    /// module requested (`SpawnLogModule.executorInit`'s).
    // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:113 AnalysisAndExecutionPhaseRunner.execute
    fn throw_pending_exception(&self) -> Result<(), BuildToolError> {
        if let Err(e) = self.spawn_log.module_environment.throw_pending_exception() {
            // catch (AbruptExitException e): result.setCatastrophe().
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:987 BuildTool.processRequest
            self.execution_phase
                .build_result
                .borrow_mut()
                .set_catastrophe();
            let failure_detail = e
                .get_detailed_exit_code()
                .get_failure_detail()
                .cloned()
                .expect("an AbruptExitException has a FailureDetail");
            return Err(BuildToolError::Exit(
                blaze_query_engine::DetailedExitCode::of_exit_code(
                    blaze_query_engine::DetailedExitCode::get_exit_code_for(&failure_detail),
                    failure_detail,
                ),
            ));
        }
        Ok(())
    }

    fn process(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        self.construct(skyframe_executor)?;
        // Without merged analysis and execution, the result's configuration and targets are set
        // once the analysis returned, before the post-processing and the execution (which
        // replaces them); with it, ExecutionTool's finalizations set them.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:540 BuildTool.buildTargetsWithoutMergedAnalysisExecution
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            // BuildView.update → createResult: `artifactsToBuild.addAll(
            // memoizedGetCoverageArtifactsHelper(configuredTargets, allTargetsToTest, ...))`.
            // A build that collects no coverage gets no wrapper and no artifacts.
            // Java: com/google/devtools/build/lib/analysis/BuildView.java:657 BuildView.createResult
            self.execution_phase
                .memoized_non_merged_coverage_artifacts(skyframe_executor, analysis_result);
        }
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            let mut build_result = self.execution_phase.build_result.borrow_mut();
            build_result.set_build_configuration(analysis_result.get_configuration().clone());
            build_result.set_actual_targets(Some(configured_target_views(
                analysis_result.get_targets_to_build(),
            )));
            build_result.set_test_targets(
                analysis_result
                    .get_targets_to_test()
                    .map(configured_target_views),
            );
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:520 BuildTool.buildTargetsWithoutMergedAnalysisExecution
    fn process_without_analysis(
        &self,
        _request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<(), BuildToolError> {
        self.construct(skyframe_executor)
    }
}

impl ExecutionToolConstruction<'_> {
    /// `CacheFileDigestsModule.executorInit`, once per command.
    // Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:53 CacheFileDigestsModule.executorInit
    fn cache_file_digests_executor_init(&self) {
        if self.cache_file_digests_initialized.replace(true) {
            return;
        }
        crate::bazel_modules::CacheFileDigestsModule::executor_init(
            self.execution_phase
                .execution_options
                .cache_size_for_computed_file_digests,
        );
    }

    /// `new ExecutionTool(env, request)`, and `init()`: `getExecutor()` creates the
    /// `BlazeExecutor` (here the prepared execution state, `check_execution_tool_created`).
    ///
    /// PORT: skipped, ExecutionTool.shutdown and the executorCreated call of getExecutor notify ExecutorLifecycleListeners, whose only implementation at the pin, IncludeScanningModule, is not ported (include scanning stops with unported!).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:171 ExecutionTool.<init>
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:239 ExecutionTool.init
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:216 ExecutionTool.getExecutor
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:227 ExecutionTool.createExecutor
    fn construct(&self, skyframe_executor: &SkyframeExecutor) -> Result<(), BuildToolError> {
        let state = skyframe_executor
            .get_execution()
            .expect("IllegalStateException: the executor was created without execution");
        // SpawnLogModule.executorInit (already run before a merged analysis).
        self.spawn_log.executor_init();
        // CacheFileDigestsModule.executorInit, next in the module order (already run before a
        // merged analysis).
        self.cache_file_digests_executor_init();
        // ExecutionTool.<init>: each module's executorInit (BuildSummaryStatsModule's registers
        // the CriticalPathComputer) runs before the spawn strategy registry is built.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:171 ExecutionTool.<init>
        executor_init_build_summary_stats(
            &self.execution_phase.collector,
            &self.execution_phase.execution_options,
            state,
        );
        if let Err(e) = state.prepared.check_execution_tool_created() {
            // catch (InterruptedException e): an interrupted sandbox probe (see
            // blaze_exec::exec::executor_setup); not a catastrophe.
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:943 BuildTool.processRequest
            if matches!(
                e.failure_detail.category,
                Some(blaze_protos::failure_details::failure_detail::Category::Interrupted(_))
            ) {
                return Err(BuildToolError::Interrupted);
            }
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

/// What `BuildResultPrinter` reads from Java's `BuildRequest`.
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:59 BuildRequest
struct RuntimeBuildRequestView {
    build_options: BuildRequestOptions,
    execution_options: ExecutionOptions,
    should_run_tests: bool,
    out_err: OutErr,
    /// `isRunningInEmacs()`: `UiOptions.runningInEmacs` (`--emacs`, which the client adds when
    /// `EMACS=t` or `INSIDE_EMACS` is set).
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:305 BuildRequest.isRunningInEmacs
    running_in_emacs: bool,
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
    /// `SkyframeExecutor.watchdog` (set when the execution progress receiver is set up).
    watchdog: Arc<parking_lot::Mutex<Option<Arc<ActionExecutionInactivityWatchdog>>>>,
    /// `SkyframeExecutor.isBuildingExclusiveArtifacts`.
    is_building_exclusive_artifacts: Arc<std::sync::atomic::AtomicBool>,
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
    /// `--experimental_extra_action_filter` (`AnalysisOptions`).
    extra_action_filter: Option<blaze_optdefs::common::regex_filter::RegexFilter>,
    /// `request.getOptions(BuildLanguageOptions.class).experimentalSiblingRepositoryLayout`.
    sibling_repository_layout: bool,
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
    build_result_listener: Arc<blaze_sky_top::build_result_listener::BuildResultListener>,
    /// The `BuildRequest` as `BuildResultPrinter` reads it.
    request_view: RuntimeBuildRequestView,
    /// The command's Build Event Protocol support (`None` without `--build_event_*_file`).
    bep: Option<Arc<BepCommandSupport>>,
    /// The command's `RemoteModule` (`None` without any remote option).
    remote: Option<Arc<build_remote::RemoteSupport>>,
    /// `TopLevelArtifactContext.expandFilesets()` (`BuildEventProtocolOptions.expandFilesets`),
    /// a component of the hash code of the build's `BuildDriverKey`s.
    expand_filesets: bool,
    /// `env.getRuntime().getCoverageReportActionFactory(request)`: the factory of
    /// `BazelCoverageReportModule`, the only module that has one.
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:283 BlazeRuntime.getCoverageReportActionFactory
    coverage_report_action_factory:
        blaze_analysis::bazel::coverage::bazel_coverage_report_module::BazelCoverageReportActionFactory,
    /// `BuildView.memoizedCoverageArtifacts` of the path without merged analysis and execution:
    /// computed when the analysis returned (`BuildView.update` adds them to `artifactsToBuild`).
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:941 BuildView.memoizedGetCoverageArtifactsHelper
    non_merged_coverage_artifacts: std::cell::RefCell<Option<Vec<blaze_actions::Artifact>>>,
    /// The command's event bus, on which the Skymeld package roots are registered.
    event_bus: Arc<crate::command_environment::CommandEventBus>,
    /// `ExecutionTool.prepareForExecution`'s `IncrementalPackageRoots` (merged analysis and
    /// execution only).
    incremental_package_roots: RefCell<
        Option<Arc<blaze_sky_top::incremental_package_roots::IncrementalPackageRoots>>,
    >,
    /// `env.getSkyframeExecutor().tracksStateForIncrementality()`.
    tracks_state_for_incrementality: bool,
    /// The request is the `run` command's (`TopLevelArtifactContext` of a run request).
    is_run_command: bool,
    /// The merged phase's `TopLevelTargetAnalysisWatcher`, while it is registered.
    top_level_target_analysis_watcher: parking_lot::Mutex<Option<Arc<TopLevelTargetAnalysisWatcher>>>,
}

impl RuntimeExecutionPhase {
    /// The `IncrementalPackageRoots` part of `ExecutionTool.prepareForExecution`: eagerly plants
    /// the symlinks to the single source root's entries, registers the roots on the event bus
    /// (they plant the external repositories as top-level targets finish their analysis) and
    /// makes them the artifact factory's package roots.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:270 ExecutionTool.prepareForExecution
    fn prepare_incremental_package_roots(
        &self,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<(), BuildToolError> {
        let state = skyframe_executor
            .get_execution()
            .expect("IllegalStateException: the executor was created without execution");
        let path_entries = skyframe_executor.get_package_locator().get_path_entries();
        // Single package path is a Skymeld prerequisite.
        if path_entries.len() != 1 {
            panic!(
                "IllegalArgumentException: expected one element but was: {:?}",
                path_entries
            );
        }
        let ignored_paths = skyframe_executor
            .get_ignored_paths(self.reporter.clone())
            .map_err(|_| BuildToolError::Interrupted)?;
        let incremental_package_roots =
            blaze_sky_top::incremental_package_roots::IncrementalPackageRoots::create(
                state.config.exec_root.clone(),
                path_entries[0].clone(),
                format!("{}-", self.product_name),
                ignored_paths,
                self.sibling_repository_layout,
                // runtime.getWorkspace().doesAllowExternalRepositories(): true in Bazel.
                true,
                Some(skyframe_executor.get_root_for_done_package()),
            );
        self.event_bus.register(incremental_package_roots.clone());
        let planted = state
            .config
            .exec_root
            .create_directory_and_parents()
            .map_err(
                |e| blaze_sky_top::incremental_package_roots::SymlinkForestAbruptExit {
                    failure_detail: FailureDetail {
                        message: format!(
                            "Failed to prepare the symlink forest: java.io.IOException: {e}"
                        ),
                        category: Some(Category::SymlinkForest(failure_details::SymlinkForest {
                            code: failure_details::symlink_forest::Code::CreationFailed as i32,
                        })),
                    },
                },
            )
            .and_then(|()| {
                incremental_package_roots.eagerly_plant_symlinks_to_single_source_root()
            });
        if let Err(e) = planted {
            return Err(BuildToolError::Exit(
                blaze_query_engine::DetailedExitCode::of_exit_code(
                    blaze_query_engine::DetailedExitCode::get_exit_code_for(&e.failure_detail),
                    e.failure_detail,
                ),
            ));
        }
        if let Some(services) = skyframe_executor.get_analysis_services() {
            services
                .artifact_factory
                .set_package_roots(incremental_package_roots.clone());
        }
        *self.incremental_package_roots.borrow_mut() = Some(incremental_package_roots);
        Ok(())
    }

    /// `BuildView.memoizedGetCoverageArtifactsHelper(configuredTargets, allTargetsToTest,
    /// eventHandler, eventBus)` of `BuildView.update` (no merged analysis and execution): the
    /// coverage report actions are created (and their errors reported) when the analysis
    /// returns; `BazelCoverageReportModule`'s factory registers the `CoverageReportCollector`.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:941 BuildView.memoizedGetCoverageArtifactsHelper
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:953 BuildView.constructCoverageArtifacts
    fn memoized_non_merged_coverage_artifacts(
        &self,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) {
        if self.non_merged_coverage_artifacts.borrow().is_some() {
            return;
        }
        let (coverage_report_artifacts, actions_wrapper) =
            blaze_sky_top::build_view::construct_coverage_artifacts(
                skyframe_executor,
                Some(&self.coverage_report_action_factory),
                analysis_result.get_targets_to_build(),
                analysis_result.get_targets_to_test().unwrap_or(&[]),
                self.reporter.as_ref(),
            );
        // eventBus.register(new CoverageReportCollector(wrapper))
        // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:114 BazelCoverageReportModule.getCoverageReportFactory
        if let (Some(bep), Some(wrapper)) = (&self.bep, &actions_wrapper) {
            bep.register_coverage_report_collector(
                blaze_analysis::bazel::coverage::bazel_coverage_report_module::coverage_report_build_tool_log_files(wrapper),
            );
        }
        *self.non_merged_coverage_artifacts.borrow_mut() = Some(coverage_report_artifacts);
    }
}

/// What the exclusive-test phase leaves to the error processing of the build.
#[derive(Default)]
struct ExclusiveTestsOutcome {
    /// The `executionDetailedExitCode()` of each exclusive test evaluation in error
    /// (`--keep_going`), in run order.
    detailed_exit_codes: Vec<Option<blaze_util::DetailedExitCode>>,
    /// The exception `SkyframeErrorProcessor.processErrors` threw for the first exclusive test in
    /// error (no `--keep_going`); no exclusive test runs after it.
    rethrown: Option<RethrownException>,
}

impl RuntimeExecutionPhase {
    /// Runs the exclusive tests sequentially, one Skyframe evaluation of its
    /// `TestCompletionKey(exclusiveTesting = true)` each, after the main evaluation
    /// (SkyframeBuilder.buildArtifacts, the path without merged analysis and execution):
    /// `analysisResult.getExclusiveTests()` and then `getExclusiveIfLocalTests()` (the `Sets.union`
    /// ExecutionTool passes) without the skipped targets, after the main evaluation's errors
    /// were processed (which throws without `--keep_going`). The merged path runs its exclusive
    /// tests in `SkyframeBuildView.analyzeAndExecuteTargets`, in the order of
    /// [`SkymeldExecutionHooks::get_exclusive_tests`].
    ///
    /// The coverage report artifacts that Java evaluates after the exclusive tests are
    /// `execute_build`'s.
    // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:203 SkyframeBuilder.buildArtifacts
    fn run_exclusive_tests(
        &self,
        skyframe_executor: &SkyframeExecutor,
        config: &blaze_buildtool::buildtool::execution_tool::ExecutionToolConfig,
        reporter: Arc<dyn ExtendedEventHandler>,
        keep_going: bool,
        keys_to_execute: &[BuildDriverKey],
        main_evaluation_result: &blaze_skyframe::EvaluationResult,
    ) -> Result<ExclusiveTestsOutcome, blaze_skyframe::Interrupted> {
        let mut outcome = ExclusiveTestsOutcome::default();
        let of_type = |test_type: TestType| {
            keys_to_execute
                .iter()
                .filter(move |key| key.test_type == test_type)
        };
        let exclusive_tests_to_run: indexmap::IndexSet<&BuildDriverKey> =
            of_type(TestType::EXCLUSIVE)
                .chain(of_type(TestType::EXCLUSIVE_IF_LOCAL))
                .collect();
        let continue_with_exclusive_tests = !main_evaluation_result.has_error() || keep_going;
        if !continue_with_exclusive_tests || exclusive_tests_to_run.is_empty() {
            return Ok(outcome);
        }
        // SkyframeBuilder.buildArtifacts: isBuildingExclusiveArtifacts.set(true).
        // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:205 SkyframeBuilder.buildArtifacts
        self.is_building_exclusive_artifacts
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Run exclusive tests sequentially.
        for key in exclusive_tests_to_run {
            let test_completion_key = TestCompletionKey {
                configured_target_key: key.action_lookup_key.clone(),
                output_groups: key.output_groups.clone(),
                exclusive_testing: true,
            };
            let test_run_result =
                blaze_buildtool::buildtool::execution_tool::run_exclusive_test_skymeld(
                    skyframe_executor.get_evaluator(),
                    &test_completion_key,
                    config,
                    reporter.clone(),
                )?;
            // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:228 SkyframeBuilder.buildArtifacts
            blaze_util::check_state!(
                test_run_result.has_error() || !test_run_result.values.is_empty(),
                "Build reported as successful but test {:?} not executed: {:?}",
                key.action_lookup_key,
                test_run_result
            );
            if test_run_result.has_error() {
                match process_execution_errors(
                    &test_run_result,
                    keep_going,
                    skyframe_executor.get_cycles_reporter(),
                    &reporter,
                    self.uses_process_errors(/* merged= */ false),
                ) {
                    Ok(code) => outcome.detailed_exit_codes.push(code),
                    Err(rethrown) => {
                        outcome.rethrown = Some(rethrown);
                        break;
                    }
                }
            }
        }
        Ok(outcome)
    }
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
        handler.println(&format!("Build options: {options_description}"));
        handler
    }

    /// `log.println(s)` of a `PrintWriter` over `OutputStreamWriter(log, UTF_8)`: the UTF-8 of
    /// the string (for an internal string, each byte >= 0x80 becomes two bytes, as in Java).
    /// `PrintWriter` swallows `IOException`s.
    fn println(&self, s: &str) {
        if let Some(log) = self.log.lock().as_mut() {
            let _ = log.write_all(s.as_bytes());
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
            self.println(&event.get_message());
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

impl RuntimeExecutionPhase {
    /// Whether the execution errors go through `SkyframeErrorProcessor.processErrors`: always with
    /// merged analysis and execution; without it, `SkyframeBuilder` calls
    /// `processExecutionErrors`, which uses `processErrors` only with
    /// `--experimental_skyframe_error_handling_refactor` and `processResult` otherwise.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:245 SkyframeErrorProcessor.processExecutionErrors
    // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:119 SkyframeBuilder.buildArtifacts
    // PORT: skipped, the builder's parts are the prepared execution state (blaze_buildtool ExecutionToolConfig, the action executor's cache checker, file cache and prefetcher): SkyframeBuilder.<init> SkyframeBuilder.getActionCacheChecker SkyframeBuilder.getFileCache SkyframeBuilder.getActionOutputDirectoryHelper SkyframeBuilder.getActionInputPrefetcher
    // PORT: skipped, without merged analysis and execution Java's progress receiver counts the exclusive tests' status artifacts; Blaze registers ExecutionTool's receiver (count 0) for every build, so the "[done / total]" totals of a non-merged build with exclusive tests are lower (open item): SkyframeBuilder.countTestActions
    fn uses_process_errors(&self, merged: bool) -> bool {
        merged
            || self
                .request_view
                .build_options
                .skyframe_error_handling_refactor
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:516 ExecutionTool.announceEnteringDirIfEmacs
    fn announce_entering_dir_if_emacs(&self, exec_root: &blaze_vfs::Path) {
        if self.request_view.running_in_emacs {
            // The syntax of this message is tightly constrained by lisp/progmodes/compile.el in
            // emacs
            self.request_view.out_err.print_err_ln(&format!(
                "{}: Entering directory `{}/'",
                self.product_name, exec_root
            ));
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:525 ExecutionTool.announceLeavingDirIfEmacs
    fn announce_leaving_dir_if_emacs(&self, exec_root: &blaze_vfs::Path) {
        if self.request_view.running_in_emacs {
            self.request_view.out_err.print_err_ln(&format!(
                "{}: Leaving directory `{}/'",
                self.product_name, exec_root
            ));
        }
    }
}

impl ExecutionPhase for RuntimeExecutionPhase {
    /// `ExecutionTool.executeBuild`: the execution phase of a build without merged analysis and
    /// execution (`--noexperimental_merged_skyframe_analysis_execution`). The merged build is
    /// `blaze_sky_top::skyframe_build_view::analyze_and_execute_targets` with this phase's
    /// hooks (`SkymeldExecutionHooks`, `merged_finalizations`).
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:367 ExecutionTool.executeBuild
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
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1007 ExecutionTool.getReporter
        let reporter = self.reporter.clone();

        // The loading and analysis BEP events are posted by BuildTool's hooks
        // (blaze_sky_top BuildEventHooks, crate::commands::bep).

        // ExecutionTool.prepareForExecution of the path without merged analysis and execution
        // (Skymeld runs it from BuildView.update: `prepare_for_execution`).
        set_salt_and_delete_actions_if_changed(skyframe_executor);

        // Without merged analysis and execution, ExecutionTool.executeBuild plants the symlink
        // forest of the exec root ("plantSymlinkForest"); Skymeld planted it incrementally
        // (prepare_incremental_package_roots).
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:620 ExecutionTool.executeBuild
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:615 ExecutionTool.prepare
        // PORT: skipped, ExecutionTool.logDeleteTreeFailure: logging only (Java log).
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1011 ExecutionTool.getExecRoot
        let exec_root = state.config.exec_root.clone();
        let planted = exec_root.create_directory_and_parents().and_then(|_| {
            SymlinkForest::with_sibling_repository_layout(
                skyframe_executor.get_package_roots(),
                exec_root.clone(),
                &self.product_name,
                self.sibling_repository_layout,
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

        // The convenience symlinks of the path without merged analysis and execution, at the
        // start of ExecutionTool.executeBuild with `analysisResult.getTargetsToBuild()`.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:393 ExecutionTool.executeBuild
        let symlinks = self.handle_convenience_symlinks(
            skyframe_executor,
            analysis_result.get_targets_to_build(),
            analysis_result.get_configuration(),
        );

        // `extraActionFilterSupplier` of BuildDriverFunction: the build's
        // `AnalysisOptions.extraActionFilter`.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3479 SkyframeExecutor.setExtraActionFilter
        if let Some(services) = skyframe_executor.get_analysis_services() {
            *services.extra_action_filter.write() = self.extra_action_filter.clone();
        }

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
        // RemoteModule.afterAnalysis: the outputs of the top-level targets are downloaded under
        // --remote_download_outputs=toplevel.
        // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:153 AnalysisPhaseRunner.execute
        if let Some(remote) = &self.remote {
            remote.after_analysis(
                analysis_result,
                &output_groups,
                /* merged= */ false,
                // analysisResult.getArtifactsToBuild(): read only for coverage.
                &self
                    .non_merged_coverage_artifacts
                    .borrow()
                    .clone()
                    .unwrap_or_default(),
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
                // executionTool.getTestActionContext(): no TestActionContext at the pin
                // overrides the defaults (false).
                // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:249 ExecutionTool.getTestActionContext
                force_exclusive_tests_in_parallel: false,
                force_exclusive_if_local_tests_in_parallel: false,
            },
            reporter.as_ref(),
            request.keep_going,
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
            request.keep_going,
        ));
        // PORT: Java's path without merged analysis and execution builds the artifacts of
        // `getTargetsToBuild()` without `getTargetsToSkip()` (SkyframeBuilder.buildArtifacts);
        // Blaze builds them through the BuildDriverKeys of those targets (BuildDriverFunction
        // without its analysis steps: `AnalysisSeam::is_merged_analysis_and_execution` false).
        // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:119 SkyframeBuilder.buildArtifacts
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
            if !skipped_labels.contains(&label) && analyzed_labels.contains(&label) {
                keys_to_execute.push(key.clone());
            }
        }
        // Conditionally record dependency-checker log (uninstalled and closed when dropped, where
        // ExecutionTool.nonCatastrophicFinalizations does it: after the results are shown).
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:421 ExecutionTool.executeBuild
        let _explanation_handler = self.install_explanation_handler();

        // announceEnteringDirIfEmacs() of executeBuild.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:426 ExecutionTool.executeBuild
        self.announce_entering_dir_if_emacs(&exec_root);

        let evaluated = evaluate_build_driver_keys(
            skyframe_executor.get_evaluator(),
            &keys_to_execute,
            &state.seam.get_workspace_status_artifacts(),
            &state.config,
            reporter.clone(),
        );
        // An interrupt still reaches the finalizations below, with buildCompleted false: the
        // action cache is not saved and the targets built so far are shown.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:492 ExecutionTool.executeBuild
        let mut interrupted = evaluated.is_err();
        let evaluation = match evaluated {
            Ok(result) => result.evaluation,
            Err(blaze_skyframe::Interrupted) => blaze_skyframe::EvaluationResult {
                values: Vec::new(),
                errors: Vec::new(),
                catastrophe: None,
            },
        };

        // The exclusive tests, each in a sequential Skyframe evaluation of its own.
        let exclusive_tests = if interrupted {
            ExclusiveTestsOutcome::default()
        } else {
            match self.run_exclusive_tests(
                skyframe_executor,
                &state.config,
                reporter.clone(),
                request.keep_going,
                &keys_to_execute,
                &evaluation,
            ) {
                Ok(outcome) => outcome,
                Err(blaze_skyframe::Interrupted) => {
                    interrupted = true;
                    ExclusiveTestsOutcome::default()
                }
            }
        };
        // Coverage report generation should only be requested after all tests have executed.
        // When --nokeep_going and there's an earlier error, we should skip this and fail fast.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:796 SkyframeBuildView.analyzeAndExecuteTargets
        let mut coverage_report_errors: Option<
            Result<Option<blaze_util::DetailedExitCode>, RethrownException>,
        > = None;
        let has_exclusive_tests_error =
            !exclusive_tests.detailed_exit_codes.is_empty() || exclusive_tests.rethrown.is_some();
        if !interrupted
            && ((!evaluation.has_error() && !has_exclusive_tests_error) || request.keep_going)
        {
            // BuildView.update computed them (with the collector) when the analysis returned,
            // and they are built with the targets.
            // PORT: Java builds them in the same evaluation as the top-level targets
            // (ExecutionTool.executeBuild, artifactsToBuild); they are evaluated here, after the
            // tests.
            let coverage_report_artifacts: Vec<blaze_actions::Artifact> = self
                .non_merged_coverage_artifacts
                .borrow()
                .clone()
                .unwrap_or_default();
            match blaze_buildtool::buildtool::execution_tool::evaluate_artifacts(
                skyframe_executor.get_evaluator(),
                &coverage_report_artifacts,
                &state.config,
                blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT,
                reporter.clone(),
            ) {
                Ok(additional_artifacts_result) => {
                    if additional_artifacts_result.has_error() {
                        coverage_report_errors = Some(process_execution_errors(
                            &additional_artifacts_result,
                            request.keep_going,
                            skyframe_executor.get_cycles_reporter(),
                            &reporter,
                            self.uses_process_errors(/* merged= */ false),
                        ));
                    }
                }
                Err(blaze_skyframe::Interrupted) => interrupted = true,
            }
        }

        // No more action execution beyond this point; the watchdog stops.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2520 SkyframeExecutor.clearExecutionStatesSkymeld
        if let Some(watchdog) = self.watchdog.lock().take() {
            watchdog.stop();
        }

        // ExecutionTool.nonCatastrophicFinalizations: announceLeavingDirIfEmacs(), then
        // `if (buildCompleted) saveActionCache(actionCache)`.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:542 ExecutionTool.nonCatastrophicFinalizations
        self.announce_leaving_dir_if_emacs(&exec_root);
        if !interrupted {
            // RemoteOutputService.onExecutionPhaseCompleteEvent → LeaseService
            // .finalizeExecution (Java: after the save, on the server's in-memory cache).
            if let Some(remote) = &self.remote {
                remote.finalize_execution(
                    state.prepared.action_cache.as_deref(),
                    skyframe_executor.get_evaluator(),
                );
            }
            state.prepared.save_action_cache(reporter.as_ref());
        }

        // SkyframeErrorProcessor.processResult of the evaluation (SkyframeBuilder.buildArtifacts).
        // Without --keep_going it throws, and Java reaches the finally block below ("Show
        // results") with the exception pending.
        // Java: com/google/devtools/build/lib/buildtool/SkyframeBuilder.java:191 SkyframeBuilder.buildArtifacts
        let mut test_exec_exception: Option<String> = None;
        let execution_error: Option<BuildToolError> = if interrupted {
            Some(BuildToolError::Interrupted)
        } else {
            // Without --keep_going the first exclusive test in error threw; with it, its
            // DetailedExitCode joins the main evaluation's, which SkyframeBuilder.buildArtifacts
            // lists first; Collections.max keeps the first of the most important.
            let processed = match (exclusive_tests.rethrown, coverage_report_errors) {
                (Some(rethrown), _) => Err(rethrown),
                (None, Some(Err(rethrown))) => Err(rethrown),
                (None, coverage_report_errors) => {
                    process_execution_errors(
                        &evaluation,
                        request.keep_going,
                        skyframe_executor.get_cycles_reporter(),
                        &reporter,
                        self.uses_process_errors(/* merged= */ false),
                    )
                    .map(|main| {
                        let mut codes = exclusive_tests.detailed_exit_codes;
                        // The coverage report's executionDetailedExitCode() follows the exclusive
                        // tests' (SkyframeBuildView.analyzeAndExecuteTargets).
                        if let Some(Ok(coverage_code)) = coverage_report_errors {
                            codes.push(coverage_code);
                        }
                        codes.insert(0, main);
                        codes.into_iter().fold(None, |max, code| {
                            blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(
                                max, code,
                            )
                        })
                    })
                }
            };
            match processed {
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

        // ExecutionTool.nonCatastrophicFinalizations ("Show results").
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:534 ExecutionTool.nonCatastrophicFinalizations
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

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:620 BuildTool.buildTargetsWithMergedAnalysisExecution
    fn merged_analysis_and_execution_args(
        &self,
        request: &BuildRequest,
    ) -> blaze_sky_top::skyframe_build_view::MergedAnalysisAndExecutionArgs<'_> {
        blaze_sky_top::skyframe_build_view::MergedAnalysisAndExecutionArgs {
            hooks: self,
            build_result_listener: self.build_result_listener.clone(),
            top_level_artifact_context_output_groups: top_level_output_groups(
                &self.build_request_options,
                request.should_run_tests(),
            ),
            extra_action_top_level_only: self.extra_action_top_level_only,
            extra_action_filter: self.extra_action_filter.clone(),
            // new BuildDriverKeyTestContext() { ... }
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:629 BuildTool.buildTargetsWithMergedAnalysisExecution
            build_driver_key_test_context: BuildDriverKeyTestContext {
                test_strategy: self.execution_options.test_strategy.clone(),
                // executionTool.getTestActionContext(): no TestActionContext at the pin
                // overrides the defaults (false).
                // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:249 ExecutionTool.getTestActionContext
                force_exclusive_tests_in_parallel: false,
                force_exclusive_if_local_tests_in_parallel: false,
            },
            skymeld_analysis_overlap_percentage: self
                .build_request_options
                .skymeld_analysis_overlap_percentage,
            tracks_state_for_incrementality: self.tracks_state_for_incrementality,
        }
    }

    fn build_result_listener(
        &self,
    ) -> Arc<blaze_sky_top::build_result_listener::BuildResultListener> {
        self.build_result_listener.clone()
    }

    // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:309 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.createAndRegisterWithEventBus
    fn register_top_level_target_analysis_watcher(&self, request: &BuildRequest) {
        let watcher = Arc::new(TopLevelTargetAnalysisWatcher {
            remote: self.remote.clone(),
            output_groups: top_level_output_groups(
                &self.build_request_options,
                request.should_run_tests(),
            ),
            registered: std::sync::atomic::AtomicBool::new(true),
        });
        self.event_bus.register(watcher.clone());
        *self.top_level_target_analysis_watcher.lock() = Some(watcher);
    }

    /// PORT: the command's event bus has no `unregister`; the closed watcher ignores the events.
    // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:351 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.close
    fn close_top_level_target_analysis_watcher(&self) {
        if let Some(watcher) = self.top_level_target_analysis_watcher.lock().take() {
            watcher
                .registered
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:343 AnalysisPhaseRunner.postAbortedEventsForSkippedTargets
    fn post_aborted_events_for_skipped_targets(&self, skipped_targets: &[ConfiguredTarget]) {
        if let Some(bep) = &self.bep {
            bep.post_aborted_events_for_skipped_targets(skipped_targets);
        }
    }

    /// The `finally` block of `BuildTool.buildTargetsWithMergedAnalysisExecution` and
    /// `ExecutionTool.nonCatastrophicFinalizations(result, actionCache, /* explanationHandler= */
    /// null, buildCompleted)`.
    ///
    /// PORT: `unconditionalExecutionPhaseFinalizations` (the `ExecutionFinishedEvent` and the
    /// `ExecutionPhaseCompleteEvent`) is the command's, after `processRequest` returned
    /// (`ExecutionEventCollector::execution_phase_finish`, the build event stream's timer).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:664 BuildTool.buildTargetsWithMergedAnalysisExecution
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:534 ExecutionTool.nonCatastrophicFinalizations
    fn merged_finalizations(
        &self,
        _request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        build_completed: bool,
        error: Option<
            &blaze_sky_top::analysis_and_execution_phase_runner::AnalysisAndExecutionError,
        >,
    ) {
        let state = skyframe_executor
            .get_execution()
            .expect("IllegalStateException: the executor was created without execution");
        let reporter = self.reporter.clone();
        // BuildTool.processRequest: `if (e.isCatastrophic()) result.setCatastrophe()` for the
        // BuildFailedException the merged phase threw.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:938 BuildTool.processRequest
        if let Some(
            blaze_sky_top::analysis_and_execution_phase_runner::AnalysisAndExecutionError::Execution(
                RethrownException::BuildFailed {
                    catastrophic: true, ..
                },
            ),
        ) = error
        {
            self.build_result.borrow_mut().set_catastrophe();
        }

        let analyzed_targets = self.build_result_listener.get_analyzed_targets();
        let configuration = self
            .build_result
            .borrow()
            .get_build_configuration()
            .cloned();
        if let Some(configuration) = &configuration {
            // We still need to do this even in case of an exception.
            let symlinks = self.handle_convenience_symlinks(
                skyframe_executor,
                &analyzed_targets,
                configuration,
            );
            self.build_result
                .borrow_mut()
                .set_convenience_symlinks(symlinks.get_created_symlinks().clone());
        }

        // For the --noskymeld code path, this is done after the analysis phase.
        let views = configured_target_views(&analyzed_targets);
        {
            let mut build_result = self.build_result.borrow_mut();
            build_result.set_actual_targets(Some(views.clone()));
            build_result.set_test_targets(Some(configured_target_views(
                &self.build_result_listener.get_analyzed_tests(),
            )));
        }

        // ExecutionTool.nonCatastrophicFinalizations
        // PORT: skipped, env.recordLastExecutionTime() (only read by the Google-internal
        // ExecutionTool logging).
        self.announce_leaving_dir_if_emacs(&state.config.exec_root);
        if build_completed {
            reporter.handle(&Event::progress("Building complete."));
        }
        if build_completed {
            // RemoteOutputService.onExecutionPhaseCompleteEvent → LeaseService
            // .finalizeExecution (Java: after the save, on the server's in-memory cache).
            if let Some(remote) = &self.remote {
                remote.finalize_execution(
                    state.prepared.action_cache.as_deref(),
                    skyframe_executor.get_evaluator(),
                );
            }
            state.prepared.save_action_cache(reporter.as_ref());
        }

        // "Show results"
        self.results_shown.set(true);
        let top_level_keys = self.build_result_listener.get_top_level_keys();
        // determineSuccessfulTargets(analyzedTargets, builtTargets): maintain the ordering.
        // PORT: Java's builtTargets holds ConfiguredTargetKeys; a top-level target's
        // BuildDriverKey is found by the label it was requested under.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:877 ExecutionTool.determineSuccessfulTargets
        let mut successful_targets: Vec<ConfiguredTargetRef> = Vec::new();
        for (target, view) in analyzed_targets.iter().zip(views.iter()) {
            let top_level_label = target.get_original_label();
            let built = top_level_keys.iter().any(|key| {
                !key.is_top_level_aspect_driver()
                    && key.action_lookup_key.get_label().as_ref() == Some(&top_level_label)
                    && top_level_target_built_in_graph(skyframe_executor.get_evaluator(), key)
            });
            if built {
                successful_targets.push(view.clone());
            }
        }
        let analyzed_aspects = self.build_result_listener.get_analyzed_aspects();
        // determineSuccessfulAspects(analyzedAspects.keySet(), builtAspects): maintain the
        // ordering.
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:890 ExecutionTool.determineSuccessfulAspects
        let successful_aspects: indexmap::IndexSet<ActionLookupKeyRef> = analyzed_aspects
            .keys()
            .map(|aspect_key| ActionLookupKeyRef::new(aspect_key.clone()))
            .filter(|aspect_key| {
                top_level_aspect_built_in_graph(
                    skyframe_executor.get_evaluator(),
                    &top_level_keys,
                    aspect_key,
                )
            })
            .collect();
        let skipped_targets =
            configured_target_views(&self.build_result_listener.get_skipped_targets());
        let mut build_result = self.build_result.borrow_mut();
        build_result.set_successful_targets(Some(successful_targets));
        build_result.set_successful_aspects(Some(successful_aspects));
        build_result.set_skipped_targets(Some(skipped_targets.clone()));
        let generating_rule_lookup =
            |target: &ConfiguredTargetRef| output_file_generating_rule(&analyzed_targets, target);
        BuildResultPrinter::new(&self.product_name, self.relative_working_directory.clone())
            .show_build_result(
                &self.request_view,
                &build_result,
                &views,
                &skipped_targets,
                &configured_aspect_views(&analyzed_aspects),
                &generating_rule_lookup,
            );
        // PORT: skipped, the output service's finalizeBuild: informedOutputServiceToStartTheBuild
        // is false without an output service that is told of the build (no remote output
        // service ported: --experimental_remote_output_service fails loudly).
    }
}

/// `AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher`: the modules' hooks for each
/// top-level entity as the merged phase analyzes it. Only `RemoteModule` implements them at the
/// pin.
///
/// PORT: `handleKnownCoverageArtifacts` (`coverageArtifactsKnown`) is called by the command's
/// coverage hook (`get_coverage_report_artifacts`), where the event is posted.
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:291 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:297 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.<init>
// PORT: skipped, see above: AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.handleKnownCoverageArtifacts
struct TopLevelTargetAnalysisWatcher {
    remote: Option<Arc<build_remote::RemoteSupport>>,
    /// `request::getTopLevelArtifactContext`'s output groups.
    output_groups: Vec<String>,
    registered: std::sync::atomic::AtomicBool,
}

impl blaze_events::EventBus for TopLevelTargetAnalysisWatcher {
    fn post(&self, event: Arc<dyn blaze_events::Postable>) {
        if !self.registered.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(remote) = &self.remote else {
            return;
        };
        if let Some(e) =
            blaze_sky_exec::events::downcast_event::<TopLevelTargetAnalyzedEvent>(&event)
        {
            // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:318 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.handleTopLevelTargetAnalysisConcluded
            remote.after_top_level_target_analysis(&e.configured_target, &self.output_groups);
        } else if let Some(e) = blaze_sky_exec::events::downcast_event::<TestAnalyzedEvent>(&event)
        {
            // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:334 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.handleTestAnalyzed
            let view = e
                .configured_target
                .as_any()
                .downcast_ref::<blaze_sky_top::execution_seam::SkyframeConfiguredTargetView>()
                .unwrap_or_else(|| {
                    panic!(
                        "ClassCastException: {:?} is not a ConfiguredTarget",
                        e.configured_target
                    )
                });
            remote.after_single_test_analysis(view.get_configured_target());
        } else if blaze_sky_exec::events::downcast_event::<
            blaze_sky_top::analysis_phase_runner::AspectAnalyzedEvent,
        >(&event)
        .is_some()
        {
            // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:327 AnalysisAndExecutionPhaseRunner.TopLevelTargetAnalysisWatcher.handleAspectAnalyzed
            remote.after_single_aspect_analysis();
        }
    }
}

/// Whether a top-level target was built, as `BuildResultListener.getBuiltTargets()` records it
/// from the `TopLevelTargetBuiltEvent`s of `ExecutionProgressReceiver.evaluated`: its
/// `TARGET_COMPLETION` node succeeded, or its `BUILD_DRIVER` node succeeded with a value that is
/// not skipped.
///
/// PORT: the evaluator has no `EvaluationProgressReceiver`; the same facts are read from the
/// graph after the evaluation. A merged build's key carries no test type (the function resolves
/// it), so both `TargetCompletionKey`s it may have requested are read.
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:95 ExecutionProgressReceiver.evaluated
fn top_level_target_built_in_graph(
    evaluator: &blaze_skyframe::Evaluator,
    key: &BuildDriverKey,
) -> bool {
    for will_test in [false, true] {
        let target_completion_key = blaze_skyframe::SkyKey::new(TargetCompletionKey {
            action_lookup_key: key.action_lookup_key.clone(),
            output_groups: key.output_groups.clone(),
            will_test,
        });
        if evaluator
            .get_existing_value(&target_completion_key)
            .is_some()
        {
            return true;
        }
    }
    evaluator
        .get_existing_value(&blaze_skyframe::SkyKey::new(key.clone()))
        .and_then(|value| {
            value
                .downcast::<blaze_sky_exec::build_driver_function::BuildDriverValue>()
                .ok()
        })
        .is_some_and(|value| !value.skipped)
}

/// Whether a top-level aspect was built, as `BuildResultListener.getBuiltAspects()` records it
/// from the `AspectBuiltEvent`s of `ExecutionProgressReceiver.evaluated` (see
/// [`top_level_target_built_in_graph`]).
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:121 ExecutionProgressReceiver.evaluated
fn top_level_aspect_built_in_graph(
    evaluator: &blaze_skyframe::Evaluator,
    top_level_keys: &[BuildDriverKey],
    aspect_key: &ActionLookupKeyRef,
) -> bool {
    for key in top_level_keys {
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
        if let Some(value) = evaluator
            .get_existing_value(&blaze_skyframe::SkyKey::new(key.clone()))
            .and_then(|value| {
                value
                    .downcast::<blaze_sky_exec::build_driver_function::BuildDriverValue>()
                    .ok()
            })
            && !value.skipped
            && let TopLevelSkyValue::TopLevelAspects(aspects) = &value.top_level_sky_value
            && aspects
                .iter()
                .any(|aspect| &aspect.get_aspect_key() == aspect_key)
        {
            return true;
        }
    }
    false
}

/// The callbacks of `SkyframeBuildView.analyzeAndExecuteTargets` (and `BuildView.update`) on a
/// build with merged analysis and execution.
impl blaze_sky_top::skyframe_build_view::SkymeldExecutionHooks for RuntimeExecutionPhase {
    /// `ExecutionTool.prepareForExecution`: the incremental package roots, the action execution
    /// salt, `announceEnteringDirIfEmacs()`.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:260 ExecutionTool.prepareForExecution
    fn prepare_for_execution(
        &self,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<(), BuildToolError> {
        self.prepare_incremental_package_roots(skyframe_executor)?;
        set_salt_and_delete_actions_if_changed(skyframe_executor);
        // The end of prepareForExecution: announceEnteringDirIfEmacs().
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:358 ExecutionTool.prepareForExecution
        if let Some(state) = skyframe_executor.get_execution() {
            self.announce_entering_dir_if_emacs(&state.config.exec_root);
        }
        Ok(())
    }

    /// `result::setBuildConfiguration`.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:627 BuildTool.buildTargetsWithMergedAnalysisExecution
    fn build_configuration_created(&self, configuration: &Arc<BuildConfigurationValue>) {
        self.build_result
            .borrow_mut()
            .set_build_configuration(configuration.clone());
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3488 SkyframeExecutor.getIsBuildingExclusiveArtifacts
    fn set_is_building_exclusive_artifacts(&self, value: bool) {
        self.is_building_exclusive_artifacts
            .store(value, std::sync::atomic::Ordering::SeqCst);
    }

    /// The exclusive tests whose analysis succeeded i.e. those that can be run.
    ///
    /// PORT: java-hash-order. `evaluationResult.values()` iterates the HashMap of the result
    /// (crate::commands::exclusive_test_order).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1103 SkyframeBuildView.getExclusiveTests
    fn get_exclusive_tests(
        &self,
        main_evaluation_result: &blaze_skyframe::EvaluationResult,
        root_keys: &[BuildDriverKey],
    ) -> Vec<BuildDriverKey> {
        let flags = super::exclusive_test_order::TopLevelArtifactContextFlags {
            run_tests_exclusively: self.execution_options.test_strategy == "exclusive",
            expand_filesets: self.expand_filesets,
            for_run_command: self.is_run_command,
        };
        // Every distinct key of the result: the BuildDriverKeys in root order, with their Java
        // hash code, then the other roots (the workspace status artifacts).
        let mut seen: std::collections::HashSet<&blaze_skyframe::SkyKey> =
            std::collections::HashSet::new();
        let mut evaluated_driver_keys: Vec<(usize, &BuildDriverKey, bool)> = Vec::new();
        let mut other_entries: Vec<usize> = Vec::new();
        for (index, (sky_key, value)) in main_evaluation_result.values.iter().enumerate() {
            if !seen.insert(sky_key) {
                continue;
            }
            let Some(key) = sky_key.downcast::<BuildDriverKey>() else {
                other_entries.push(index);
                continue;
            };
            let key = root_keys
                .iter()
                .find(|k| *k == key)
                .expect("an evaluated BuildDriverKey is a root");
            let is_exclusive = value
                .clone()
                .downcast::<blaze_sky_exec::build_driver_function::BuildDriverValue>()
                .is_ok_and(|value| value.exclusive_test);
            evaluated_driver_keys.push((index, key, is_exclusive));
        }
        let mut result_entries: Vec<(usize, Option<i32>)> = Vec::new();
        let mut exclusive: Vec<(usize, &BuildDriverKey)> = Vec::new();
        for root in root_keys {
            let Some(&(index, key, is_exclusive)) =
                evaluated_driver_keys.iter().find(|(_, k, _)| *k == root)
            else {
                // In error: not a value of the result.
                continue;
            };
            let hash_code = super::exclusive_test_order::build_driver_key_hash_code(key, &flags);
            result_entries.push((index, hash_code));
            if is_exclusive {
                assert!(
                    hash_code.is_some(),
                    "an exclusive test's key is a ConfiguredTargetKey"
                );
                exclusive.push((index, key));
            }
        }
        for index in other_entries {
            result_entries.push((index, None));
        }
        super::exclusive_test_order::evaluation_result_values_order(result_entries)
            .into_iter()
            .filter_map(|index| {
                exclusive
                    .iter()
                    .find(|(i, _)| *i == index)
                    .map(|(_, key)| (*key).clone())
            })
            .collect()
    }

    /// `memoizedGetCoverageArtifactsHelper(configuredTargets, allTargetsToTest, eventHandler,
    /// eventBus)` and `CoverageArtifactsKnownEvent`, whose only subscriber is
    /// `RemoteModule.coverageArtifactsKnown`.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:941 BuildView.memoizedGetCoverageArtifactsHelper
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1062 RemoteModule.coverageArtifactsKnown
    fn get_coverage_report_artifacts(
        &self,
        skyframe_executor: &SkyframeExecutor,
        analyzed_targets: &[ConfiguredTarget],
        analyzed_tests: &[ConfiguredTarget],
    ) -> Vec<blaze_actions::Artifact> {
        let (coverage_report_artifacts, actions_wrapper) =
            blaze_sky_top::build_view::construct_coverage_artifacts(
                skyframe_executor,
                Some(&self.coverage_report_action_factory),
                analyzed_targets,
                analyzed_tests,
                self.reporter.as_ref(),
            );
        // eventBus.register(new CoverageReportCollector(wrapper))
        if let (Some(bep), Some(wrapper)) = (&self.bep, &actions_wrapper) {
            bep.register_coverage_report_collector(
                blaze_analysis::bazel::coverage::bazel_coverage_report_module::coverage_report_build_tool_log_files(wrapper),
            );
        }
        if let Some(remote) = &self.remote {
            remote.coverage_artifacts_known(&coverage_report_artifacts);
        }
        coverage_report_artifacts
    }

    /// The watchdog stops (`clearExecutionStatesSkymeld`).
    /// PORT: the rest of the method (the execution progress receiver, the status reporter, the
    /// consumed artifacts tracker) belongs to state that Blaze drops with the evaluation.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2520 SkyframeExecutor.clearExecutionStatesSkymeld
    fn clear_execution_states_skymeld(&self) {
        if let Some(watchdog) = self.watchdog.lock().take() {
            watchdog.stop();
        }
    }
}

/// The part of `SkyframeErrorProcessor.processErrors` (with `includeExecutionPhase`) that
/// handles execution exceptions, for the evaluation of the build driver keys.
///
/// - `--keep_going`: returns the most important exit code of the execution errors (`None` when
///   there was none); nothing is printed for them (`logOrPrintWarningsKeepGoing`).
/// - `--nokeep_going`: throws for the first error (`throwOrReturnAspectAnalysisException` →
///   `rethrow`).
///
/// With `uses_process_errors` false it is `processResult` (the path without merged analysis and
/// execution): the same results, but no keep-going warning for a cycle; its keep-going exit code
/// (`getDetailedExitCodeKeepGoing`) is the most important detailed one, a cycle's `CYCLE_CODE`,
/// as here.
///
/// PORT: Blaze analyzes before it executes (module comment), so the loading and analysis errors
/// of the top-level keys were processed by blaze_sky_top's `SkyframeErrorProcessor` and the keys
/// evaluated here are those of analyzed targets: an error that is not an execution exception or
/// an execution cycle fails loudly. The failure events of execution errors
/// (`maybePostFailureEventsForNonConflictError` posts an `AnalysisFailureEvent` only for
/// analysis errors) are none.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:302 SkyframeErrorProcessor.processErrors
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:899 SkyframeErrorProcessor.processResult
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:936 SkyframeErrorProcessor.getDetailedExitCodeKeepGoing
fn process_execution_errors(
    result: &blaze_skyframe::EvaluationResult,
    keep_going: bool,
    cycles_reporter: &blaze_sky_top::cycles_reporter::CyclesReporter,
    event_handler: &Arc<dyn ExtendedEventHandler>,
    uses_process_errors: bool,
) -> Result<Option<blaze_util::DetailedExitCode>, RethrownException> {
    let mut execution_detailed_exit_code: Option<blaze_util::DetailedExitCode> = None;
    for (error_key, error_info) in result.error_map() {
        // The cycle reporter requires that the path to the cycle starts at the top level key
        // (requested via SkyframeExecutor), hence we need to provide the original top level key
        // here.
        cycles_reporter.report_cycles(error_info.get_cycle_info(), error_key, event_handler);
        let nullable_cause = error_info.get_exception();
        blaze_util::check_state!(
            nullable_cause.is_some() || !error_info.get_cycle_info().is_empty(),
            "{:?}",
            error_info
        );
        let Some(cause) = nullable_cause else {
            // processIndividualError: a cycle (the cause is null). Analysis cycles were
            // processed before execution (module comment).
            if !blaze_sky_top::skyframe_error_processor::is_execution_cycle(
                error_info.get_cycle_info(),
            ) {
                blaze_util::unported!(
                    "SkyframeErrorProcessor.processErrors",
                    "analysis cycle at {:?} during execution",
                    error_key
                );
            }
            // If we have a cycle, cause would be null, so it's guaranteed that this
            // executionDetailedExitCode is final.
            let cycle_code = blaze_sky_top::skyframe_error_processor::cycle_code();
            if keep_going {
                execution_detailed_exit_code =
                    blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(
                        execution_detailed_exit_code.take(),
                        Some(cycle_code),
                    );
                // logOrPrintWarningsKeepGoing: a cycle is not an execution exception (the cause
                // is null).
                // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:665 SkyframeErrorProcessor.logOrPrintWarningsKeepGoing
                if uses_process_errors {
                    let label = effective_error_key_label(error_key)
                        .map_or_else(|| "null".to_string(), |l| l.to_string());
                    event_handler.handle(&Event::warn(format!(
                        "errors encountered while analyzing target '{}', it will not be built.",
                        label
                    )));
                }
                continue;
            }
            // If a --nokeep_going build found a cycle, that means there were no other errors
            // thrown during evaluation (otherwise, it wouldn't have bothered to find a cycle). So
            // the best we can do is throw a generic build failure exception, since we've already
            // reported the cycles above.
            // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:490 SkyframeErrorProcessor.throwOrReturnAspectAnalysisException
            return Err(RethrownException::build_failed(
                BuildToolError::BuildFailed {
                    // new BuildFailedException(null, CYCLE_CODE): no message (already shown).
                    message: String::new(),
                    failure_detail: cycle_code
                        .get_failure_detail()
                        .cloned()
                        .expect("CYCLE_CODE has a failure detail"),
                },
            ));
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

/// `getLabel(getEffectiveErrorKey(errorEntry))`: the label of the action lookup key a build
/// driver, test completion, target completion or aspect completion key stands for.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:698 SkyframeErrorProcessor.getEffectiveErrorKey
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:717 SkyframeErrorProcessor.getLabel
fn effective_error_key_label(error_key: &blaze_skyframe::SkyKey) -> Option<blaze_cmdline::Label> {
    if let Some(key) = error_key.downcast::<BuildDriverKey>() {
        key.action_lookup_key.get_label()
    } else if let Some(key) = error_key.downcast::<TestCompletionKey>() {
        key.configured_target_key.get_label()
    } else if let Some(key) = error_key.downcast::<TargetCompletionKey>() {
        key.action_lookup_key.get_label()
    } else if let Some(key) = error_key.downcast::<AspectCompletionKey>() {
        key.action_lookup_key.get_label()
    } else {
        // errorKey instanceof ActionLookupKey: the other keys of these evaluations are not.
        None
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
/// default, left as it is), and for `--auto_output_filter` the slot that the target-parsing hook
/// fills.
///
/// Java's module sets the filter on the reporter at `BuildStartingEvent` (`--output_filter`) or
/// at `TargetParsingCompleteEvent` (`--auto_output_filter`); the command sets it at
/// `BuildStartingEvent`, and the action executor (which reports through the reporter but asks
/// `reporter.showOutput` itself before it reports an action's output) takes it when it is
/// created.
///
/// PORT: for `--auto_output_filter`, the reporter and the executor get an
/// [`AutoOutputFilterSlot`] at `BuildStartingEvent`: it shows everything (Java's reporter has no
/// filter yet) until [`OutputFilteringHooks`] sets the filter of the parsed targets in it.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1132 BuildTool.initializeOutputFilter
// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:78 OutputFilteringModule.buildStarting
#[allow(clippy::type_complexity)]
fn output_filter(
    options: &OptionsParsingResult,
    request_options: &BuildRequestOptions,
) -> (
    Option<Arc<dyn blaze_events::OutputFilter>>,
    Option<(
        blaze_optdefs::command::auto_output_filter::AutoOutputFilter,
        Arc<AutoOutputFilterSlot>,
    )>,
) {
    use blaze_optdefs::command::output_filtering_module::OutputFilteringModuleOptions;
    if let Some(output_filter) = &request_options.output_filter {
        return (
            Some(blaze_events::RegexOutputFilter::for_pattern(
                output_filter.regex_pattern().clone(),
            )),
            None,
        );
    }
    match options
        .get_options::<OutputFilteringModuleOptions>()
        .map(|o| o.auto_output_filter)
    {
        None => (None, None),
        Some(auto) => {
            let slot = Arc::new(AutoOutputFilterSlot::default());
            (
                Some(slot.clone() as Arc<dyn blaze_events::OutputFilter>),
                Some((auto, slot)),
            )
        }
    }
}

/// The reporter's filter under `--auto_output_filter`: everything until the target patterns are
/// parsed, then `AutoOutputFilter.getFilter(labels)`.
#[derive(Default)]
struct AutoOutputFilterSlot {
    filter: std::sync::OnceLock<Arc<dyn blaze_events::OutputFilter>>,
}

impl blaze_events::OutputFilter for AutoOutputFilterSlot {
    fn show_output(&self, tag: &str) -> bool {
        match self.filter.get() {
            Some(filter) => filter.show_output(tag),
            // OutputFilter.OUTPUT_EVERYTHING, the reporter's default.
            None => true,
        }
    }
}

/// `OutputFilteringModule.targetParsingComplete` as a build hook, forwarding every hook to the
/// build event stream's (if any).
struct OutputFilteringHooks<'a> {
    inner: Option<&'a dyn blaze_sky_top::build_tool::BuildEventHooks>,
    auto_output_filter: blaze_optdefs::command::auto_output_filter::AutoOutputFilter,
    slot: Arc<AutoOutputFilterSlot>,
}

impl blaze_sky_top::build_tool::BuildEventHooks for OutputFilteringHooks<'_> {
    fn build_starting(&self, request: &BuildRequest) {
        if let Some(inner) = self.inner {
            inner.build_starting(request);
        }
    }

    // Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:98 OutputFilteringModule.targetParsingComplete
    fn target_patterns_evaluated(
        &self,
        request: &BuildRequest,
        skyframe_executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
        target_pattern_phase_value: &TargetPatternPhaseValue,
    ) {
        if let Some(inner) = self.inner {
            inner.target_patterns_evaluated(request, skyframe_executor, target_pattern_phase_value);
        }
        // `event.getLabels()`: the targets before test suite expansion.
        let labels: Vec<&blaze_cmdline::Label> = target_pattern_phase_value
            .get_non_expanded_labels()
            .iter()
            .collect();
        let _ = self.slot.filter.set(auto_output_filter_get_filter(
            self.auto_output_filter,
            &labels,
        ));
    }

    fn project_evaluated(&self, project_flags_applied: bool) {
        if let Some(inner) = self.inner {
            inner.project_evaluated(project_flags_applied);
        }
    }

    fn ensure_build_info_posted(&self) {
        if let Some(inner) = self.inner {
            inner.ensure_build_info_posted();
        }
    }

    fn no_analyze(&self) {
        if let Some(inner) = self.inner {
            inner.no_analyze();
        }
    }

    fn no_execution(&self) {
        if let Some(inner) = self.inner {
            inner.no_execution();
        }
    }

    fn analysis_complete(
        &self,
        request: &BuildRequest,
        skyframe_executor: &blaze_sky_top::skyframe_executor::SkyframeExecutor,
        target_pattern_phase_value: &TargetPatternPhaseValue,
        analysis_result: &blaze_sky_top::build_view::AnalysisResult,
    ) {
        if let Some(inner) = self.inner {
            inner.analysis_complete(
                request,
                skyframe_executor,
                target_pattern_phase_value,
                analysis_result,
            );
        }
    }
}

/// `AutoOutputFilter.getFilter(targets)`.
// Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:103 AutoOutputFilter.getFilter
fn auto_output_filter_get_filter(
    auto: blaze_optdefs::command::auto_output_filter::AutoOutputFilter,
    targets: &[&blaze_cmdline::Label],
) -> Arc<dyn blaze_events::OutputFilter> {
    use blaze_optdefs::command::auto_output_filter::AutoOutputFilter;
    // Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:99 AutoOutputFilter.SYSTEM_ACTION_REGEX
    let system_action_regex = format!(
        "(unknown)|{}",
        blaze_cmdline::Label::parse_canonical_unchecked("@bazel_tools//tools:internal_platform")
            .get_canonical_form()
    );
    let regex = match auto {
        // Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:40 AutoOutputFilter.NONE.getFilter
        AutoOutputFilter::NONE => return blaze_events::output_everything(),
        // Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:50 AutoOutputFilter.ALL.getFilter
        AutoOutputFilter::ALL => return blaze_events::output_nothing(),
        // Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:62 AutoOutputFilter.PACKAGES.getFilter
        AutoOutputFilter::PACKAGES => format!(
            "{system_action_regex}|^//{}:",
            auto_output_filter_get_pkg_regex(auto_output_filter_get_packages(targets).iter())
        ),
        // Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:77 AutoOutputFilter.SUBPACKAGES.getFilter
        AutoOutputFilter::SUBPACKAGES => {
            let mut packages: Vec<String> = Vec::new();
            let mut previous: Option<String> = None;
            for pkg in auto_output_filter_get_packages(targets) {
                if let Some(previous) = &previous
                    && pkg.starts_with(&format!("{previous}/"))
                {
                    // We already have a super-package in the list, so this package does
                    // not need to be added.
                    continue;
                }
                packages.push(pkg.clone());
                previous = Some(pkg);
            }
            format!(
                "{system_action_regex}|^//{}[/:]",
                auto_output_filter_get_pkg_regex(packages.iter())
            )
        }
    };
    // `Pattern.compile`: the package names are inserted unquoted, as in Java.
    match blaze_events::RegexOutputFilter::for_regex(&regex) {
        Ok(filter) => filter,
        Err(e) => panic!("PatternSyntaxException: {e}"),
    }
}

/// `getPackages(targets)`: the sorted package names; `java/...` and `javatests/...` are one.
// Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:110 AutoOutputFilter.getPackages
fn auto_output_filter_get_packages(
    targets: &[&blaze_cmdline::Label],
) -> std::collections::BTreeSet<String> {
    let mut packages = std::collections::BTreeSet::new();
    for label in targets {
        let mut name = label.get_package_name().to_owned();
        // Treat //java/foo and //javatests/foo as one package
        if name.starts_with("java/") || name.starts_with("javatests/") {
            name = format!("java(tests)?{}", &name[name.find('/').unwrap()..]);
        }
        packages.insert(name);
    }
    packages
}

/// `getPkgRegex(strings)`: `(a|b|...)`.
// Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:127 AutoOutputFilter.getPkgRegex
fn auto_output_filter_get_pkg_regex<'a>(strings: impl Iterator<Item = &'a String>) -> String {
    format!("({})", strings.cloned().collect::<Vec<_>>().join("|"))
}

#[cfg(test)]
mod tests {
    use blaze_exec::actions::action_execution::ActionExecutionException;
    use blaze_exec::actions::action_execution::FailedActionInfo;
    use blaze_skyframe::{
        ERROR_TRANSIENCE_KEY, ErrorInfo, EvaluationResult, SkyFunctionException, Transience,
    };

    use super::*;

    /// `process_execution_errors` of an evaluation without cycles.
    fn process_execution_errors_without_cycles(
        result: &blaze_skyframe::EvaluationResult,
        keep_going: bool,
    ) -> Result<Option<blaze_util::DetailedExitCode>, RethrownException> {
        let event_handler: Arc<dyn ExtendedEventHandler> =
            Arc::new(blaze_events::StoredEventHandler::new());
        process_execution_errors(
            result,
            keep_going,
            &blaze_sky_top::cycles_reporter::CyclesReporter::new(Vec::new()),
            &event_handler,
            /* uses_process_errors= */ true,
        )
    }

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
        }) = process_execution_errors_without_cycles(&result, /* keep_going= */ false)
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
        }) = process_execution_errors_without_cycles(&result, /* keep_going= */ false)
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
            process_execution_errors_without_cycles(&result, /* keep_going= */ false)
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
            process_execution_errors_without_cycles(&result_with_error(catastrophic), false)
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
        let code = process_execution_errors_without_cycles(&result, /* keep_going= */ true)
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
        assert!(matches!(
            process_execution_errors_without_cycles(&result, true),
            Ok(None)
        ));
        assert!(matches!(
            process_execution_errors_without_cycles(&result, false),
            Ok(None)
        ));
    }
}

/// `SpawnStrategyRegistry.builder(env.getInvocationPolicy().getStrategyPolicy())` in
/// `ExecutionTool`'s constructor. blaze_exec builds the registry with the policy
/// (`executor_setup::build_spawn_strategy_registry_with_policy`), but blaze_buildtool's
/// `ExecutionToolConfig` does not carry it from the command environment yet, so the registry is
/// built with the default policy. That is the same registry when every `MnemonicPolicy` of the
/// strategy policy is empty (`SpawnStrategyPolicy.create`: "Empty proto policy implies everything
/// allowed"); any other strategy policy fails loudly.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:166 ExecutionTool.<init>
pub(crate) fn check_strategy_policy_is_default(env: &CommandEnvironment) {
    use blaze_protos::blaze::strategy_policy::MnemonicPolicy;
    let Some(strategy_policy) = &env.get_invocation_policy().strategy_policy else {
        return;
    };
    let allows_all = [
        &strategy_policy.mnemonic_policy,
        &strategy_policy.dynamic_remote_policy,
        &strategy_policy.dynamic_local_policy,
    ]
    .iter()
    .all(|p| (*p).clone().unwrap_or_default() == MnemonicPolicy::default());
    if !allows_all {
        blaze_util::unported!(
            "ExecutionTool.<init>",
            "SpawnStrategyRegistry.builder(env.getInvocationPolicy().getStrategyPolicy()) with a \
             strategy policy (ExecutionToolConfig does not carry it to blaze_exec)"
        );
    }
}
