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

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::top_level::create_build_options;
use blaze_buildtool::buildtool::build_result::{BuildResult, ConfiguredTargetRef};
use blaze_buildtool::buildtool::execution_tool::{
    ExecutionEventCollector, ExecutionToolConfig, build_complete_event, evaluate_build_driver_keys,
    report_build_summary_stats,
};
use blaze_buildtool::buildtool::output_directory_links_utils::{
    BlazeDirectoriesView, create_output_directory_links, get_symlink_prefix,
};
use blaze_buildtool::buildtool::symlink_forest::SymlinkForest;
use blaze_buildtool::runtime::test_command_support::{
    TestCommandSupport, get_exit_code, make_test_log_path_formatter,
};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_exec::exec::spawn_log_module::{SpawnLogModule, SpawnLogModuleEnv};
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::test_summary_options::TestSummaryOptions;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::command::workspace_status_action::WorkspaceStatusActionOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_exec::keys::TestType;
use blaze_sky_top::build_tool::{
    AnalysisPostProcessor, BuildRequest, BuildTool, BuildToolError, ExecutionPhase,
};
use blaze_sky_top::build_view::{AnalysisResult, construct_label_to_target_map};
use blaze_sky_top::execution_phase::ExecutionSetup;
use blaze_sky_top::execution_seam::{
    BuildDriverKeyTestContext, build_driver_keys, configured_target_views,
    get_explicit_target_patterns, top_level_output_groups,
};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_sky_top::target_pattern_phase::TargetPatternPhaseValue;
use blaze_vfs::{Path, PathFragment};

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::{BUILD_COMMAND, TEST_COMMAND};
use crate::commands::bep::BepCommandSupport;
use crate::commands::query::{
    AfterCommand, create_skyframe_executor_with_execution, reporter, to_runtime_detailed_exit_code,
};
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
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:337 BuildEventServiceModule.beforeCommand
    let bep = BepCommandSupport::before_command(env, options);
    let result = run_build_with_bep(env, options, run_tests, bep.as_ref());
    let Some(bep) = bep else {
        return result;
    };
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:802 CommandEnvironment.precompleteCommand
    bep.command_precomplete();
    // Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:750 BlazeRuntime.afterCommand
    match bep.after_command() {
        Ok(()) => result,
        Err(e) => {
            env.get_reporter()
                .handle(&Event::error(e.get_message().to_owned()));
            if result.get_exit_code().is_infrastructure_failure() {
                result
            } else {
                BlazeCommandResult::detailed_exit_code(e.detailed_exit_code)
            }
        }
    }
}

/// `BuildCommand.exec` / `TestCommand.doTest`: builds (and tests) the targets of the residue.
fn run_build_with_bep(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
    run_tests: bool,
    bep: Option<&Arc<BepCommandSupport>>,
) -> BlazeCommandResult {
    let start = Instant::now();
    let start_time_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let reporter = reporter(env);
    let command_name = env.get_command_name();
    // PORT: TargetPatternsHelper.readFrom: --target_pattern_file is not ported.
    let targets: Vec<String> = options.get_residue();

    let build_request_options: BuildRequestOptions = options_or_default(options);
    let execution_options: ExecutionOptions = options_or_default(options);
    let test_summary_options: TestSummaryOptions = options_or_default(options);
    let build_language_options: BuildLanguageOptions = options_or_default(options);
    let ui_options: UiOptions = options_or_default(options);
    let test_options: TestConfigurationTestOptions = options_or_default(options);
    let status_options: WorkspaceStatusActionOptions = options_or_default(options);

    // The modules subscribed to the command's event bus.
    let collector = Arc::new(ExecutionEventCollector::new());
    env.get_event_bus().register(collector.clone());
    let ui_test_summaries = Arc::new(UiTestSummaryCounter::default());
    env.get_event_bus().register(ui_test_summaries.clone());
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
            remote_options: None,
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

    let jobs = if build_request_options.jobs > 0 {
        build_request_options.jobs as usize
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    };
    let keep_going = options
        .get_options::<blaze_optdefs::command::keep_going_option::KeepGoingOption>()
        .map(|o| o.keep_going)
        .unwrap_or(false);
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
        // Filled by the executor from its package locator and repository setup.
        package_path_entries: Vec::new(),
        repo_contents_cache_path: None,
        test_arguments: test_options.test_arguments.clone(),
        action_temps_directory: directories.get_action_temps_directory(&exec_root),
        writable_outputs: options
            .get_options::<blaze_optdefs::fragment::core_options::CoreOptions>()
            .map(|o| o.experimental_writable_outputs)
            .unwrap_or(false),
        output_filter: output_filter(options, &build_request_options),
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
    // (BlazeCommandDispatcher.execExclusively); this command's executor computes it again (the
    // dispatcher already reported any error).
    if skyframe_executor
        .get_main_repo_mapping(
            keep_going,
            blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT,
            reporter.clone(),
        )
        .is_err()
    {
        return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
            message: "main repository mapping computation failed".to_owned(),
            category: Some(Category::ExternalDeps(failure_details::ExternalDeps {
                code: failure_details::external_deps::Code::ExternalDepsUnknown as i32,
            })),
        }));
    }

    let mut request = BuildRequest::new(command_name, options, targets, true, false);
    request.set_run_tests(run_tests);

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
        outcome: RefCell::new(None),
        bep: bep.cloned(),
    };
    let post_processor = NoPostProcessor;
    let build_tool = BuildTool::new(
        &skyframe_executor,
        reporter.clone(),
        env.get_relative_working_directory().clone(),
        &post_processor,
    )
    .with_execution_phase(&execution_phase);
    // BuildTool.buildTargets: BuildStartingEvent.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:288 BuildTool.buildTargets
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

    // BuildTool.stopRequest: the BuildCompleteEvent subscribers.
    let outcome = execution_phase.outcome.borrow_mut().take();
    let mut build_result = BuildResult::new(start_time_millis);
    build_result.set_detailed_exit_code(Arc::new(to_util_detailed_exit_code(&detailed_exit_code)));
    if let Some(outcome) = &outcome {
        build_result.set_convenience_symlinks(outcome.convenience_symlinks.clone());
        build_result.set_actual_targets(Some(outcome.actual_targets.clone()));
        build_result.set_test_targets(Some(outcome.test_targets.clone()));
        build_result.set_successful_targets(Some(outcome.successful_targets.clone()));
        build_result.set_skipped_targets(Some(Vec::new()));
    }
    build_result.set_stop_time(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
    );
    if let Some(test_support) = &test_support {
        test_support.build_complete_event(&build_result);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let critical_path = outcome.as_ref().map(|o| o.critical_path).unwrap_or(0.0);
    report_build_summary_stats(reporter.as_ref(), &collector, elapsed, critical_path);
    let failed_tests = ui_test_summaries
        .failed_tests
        .load(std::sync::atomic::Ordering::SeqCst);
    reporter.handle(&build_complete_event(
        detailed_exit_code.is_success(),
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

/// `build` and `test` have no analysis post processor.
struct NoPostProcessor;

impl AnalysisPostProcessor for NoPostProcessor {
    fn process(
        &self,
        _request: &BuildRequest,
        _skyframe_executor: &SkyframeExecutor,
        _analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        Ok(())
    }
}

/// What the execution phase hands back to `stopRequest`.
struct ExecutionOutcome {
    convenience_symlinks: indexmap::IndexMap<PathFragment, PathFragment>,
    actual_targets: Vec<ConfiguredTargetRef>,
    test_targets: Vec<ConfiguredTargetRef>,
    successful_targets: Vec<ConfiguredTargetRef>,
    critical_path: f64,
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
    outcome: RefCell<Option<ExecutionOutcome>>,
    /// The command's Build Event Protocol support (`None` without `--build_event_*_file`).
    bep: Option<Arc<BepCommandSupport>>,
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

        // The convenience symlinks (OutputDirectoryLinksUtils.createOutputDirectoryLinks).
        let symlinks = create_output_directory_links(
            &[],
            &self.build_request_options,
            &self.workspace_name,
            &self.workspace,
            &self.directories,
            reporter.as_ref(),
            std::slice::from_ref(analysis_result.get_configuration()),
            &self.product_name,
        );
        // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:705 ExecutionTool.handleConvenienceSymlinks
        if let Some(bep) = &self.bep {
            bep.convenience_symlinks_identified(&self.build_request_options, &symlinks);
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
        let keys = build_driver_keys(
            &label_to_target_map,
            analysis_result.get_configuration(),
            analysis_result.get_targets_to_build(),
            loading_result.get_tests_to_run_labels(),
            &explicit_target_patterns,
            &output_groups,
            false,
            self.extra_action_top_level_only,
            &BuildDriverKeyTestContext {
                test_strategy: self.execution_options.test_strategy.clone(),
                force_exclusive_tests_in_parallel: false,
                force_exclusive_if_local_tests_in_parallel: false,
            },
            reporter.as_ref(),
        );
        if keys.iter().any(|k| {
            matches!(
                k.test_type,
                TestType::EXCLUSIVE | TestType::EXCLUSIVE_IF_LOCAL
            )
        }) {
            blaze_util::unported!("BuildTool.runExclusiveTests");
        }

        let result = evaluate_build_driver_keys(
            skyframe_executor.get_evaluator(),
            &keys,
            &state.seam.get_workspace_status_artifacts(),
            &state.config,
            reporter.clone(),
        )
        .map_err(|_| BuildToolError::Interrupted)?;
        state.prepared.save_action_cache(reporter.as_ref());

        let views = configured_target_views(analysis_result.get_targets_to_build());
        let test_targets =
            configured_target_views(analysis_result.get_targets_to_test().unwrap_or(&[]));
        let mut successful_targets = Vec::new();
        for (key, view) in keys.iter().zip(views.iter()) {
            if result
                .evaluation
                .get_error(&blaze_skyframe::SkyKey::new(key.clone()))
                .is_none()
            {
                successful_targets.push(view.clone());
            }
        }
        *self.outcome.borrow_mut() = Some(ExecutionOutcome {
            convenience_symlinks: symlinks.get_created_symlinks().clone(),
            actual_targets: views.clone(),
            test_targets,
            successful_targets,
            critical_path: (result.stop - result.start).as_secs_f64(),
        });
        let _ = &self.collector;

        if result.evaluation.has_error() {
            // The failing actions reported themselves (SkyframeActionExecutor); Java throws a
            // BuildFailedException whose error was already shown.
            let failure_detail = result
                .evaluation
                .get_first_error()
                .and_then(|e| e.get_exception())
                .and_then(|e| {
                    if let Some(a) = e
                        .downcast_ref::<blaze_exec::actions::action_execution::ActionExecutionException>()
                    {
                        return a.get_failure_detail().cloned();
                    }
                    // Java: a BuildFailedException whose error is already shown.
                    e.downcast_ref::<blaze_sky_exec::completion_function::InputFileErrorException>()
                        .and_then(|i| i.detailed_exit_code.get_failure_detail().cloned())
                })
                .unwrap_or_else(|| FailureDetail {
                    message: "build failed".to_owned(),
                    category: Some(Category::Execution(failure_details::Execution {
                        code: failure_details::execution::Code::NonActionExecutionFailure as i32,
                    })),
                });
            return Err(BuildToolError::BuildFailed {
                message: String::new(),
                failure_detail,
            });
        }
        Ok(())
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

/// The reporter's `OutputFilter` (`None`: `OutputFilter.OUTPUT_EVERYTHING`).
///
/// PORT: Java's module sets the filter on the reporter at `BuildStartingEvent`
/// (`--output_filter`) or `TargetParsingCompleteEvent` (`--auto_output_filter`); the executor
/// takes it when it is created.
// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:78 OutputFilteringModule.buildStarting
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
