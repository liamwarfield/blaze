//! Port of `runtime/commands/RunCommand.java` (the `run` command), `RunCommandLine.java` (its
//! command line) and `PathToReplaceUtils.java`.
//!
//! The build goes through the `build` path (`crate::commands::build`, Java `BuildTool`) with
//! `RunCommand`'s targets and `TargetValidator`; the rest of `RunCommand.exec` runs on its
//! `BuildResult`. The `ExecRequest` it returns is executed by the client (blaze_client
//! `BlazeServer::communicate`, Java `blaze.cc`) or, with `--batch`, by `BlazeRuntime.batchMain`
//! (crate::runtime).
//!
//! `RunBuildCompleteEvent` and `ExecRequestEvent` are build events whose only subscriber is the
//! `BuildEventStreamer` (crate::commands::bep): they are posted only when the command streams
//! build events. The output service is Bazel's default
//! (`LocalOutputService`: `stagesTopLevelRunfiles()` is false, `flushOutputTree()` does nothing):
//! the remote output service, the only other one, fails loudly with `run`
//! (crate::unported_options).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Write as _;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::analysis::runfiles_support::RunfilesSupport;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::providers::file_provider::FilesToRunProvider;
use blaze_analysis::test::test_provider::TestProvider;
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::events::buildtool_events::{ExecRequestEvent, run_build_complete_event};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_buildtool::buildtool::output_directory_links_utils::get_symlink_prefix;
use blaze_buildtool::buildtool::path_pretty_printer::PathPrettyPrinter;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::{Clock as _, JavaClock};
use blaze_events::{Event, EventHandler};
use blaze_exec::actions::{ExecException, ExecutableAction as _};
use blaze_exec::analysis::actions::runfiles_tree_action::RunfilesTreeAdapter;
use blaze_exec::analysis::test::test_runner_action::TestRunnerAction;
use blaze_exec::analysis::test::test_strategy::TestStrategy;
use blaze_exec::exec::runfiles_tree_updater::{RunfilesTreeUpdater, UpdateRunfilesError};
use blaze_exec::exec::test_policy::TestPolicy;
use blaze_optdefs::command::action_execution_context::ShowSubcommands;
use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::run_command::RunCommandRunOptions;
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_optdefs::fragment::run_under::RunUnder;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_options::EnvVar;
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_protos::command_server::{
    EnvironmentVariable, ExecRequest, PathToReplace, ScriptPath, path_to_replace,
};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_shell::{CommandDescriptionForm, CommandFailureUtils, ShellEscaper};
use blaze_sky_loading::package_manager::{NoSuchThingOrInterrupted, TargetProvider as _};
use blaze_sky_top::project::LoadingFailedException;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_skyframe::SkyKey;
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_util::{check_argument, check_state};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::RUN_COMMAND;
use crate::commands::build::{RunBuild, RunBuildOutcome, run_build};
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

type Code = failure_details::run_command::Code;

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:209 RunCommand.NO_TARGET_MESSAGE
const NO_TARGET_MESSAGE: &str = "No targets found to run";

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:211 RunCommand.MULTIPLE_TESTS_MESSAGE
const MULTIPLE_TESTS_MESSAGE: &str = "'run' only works with tests with one shard \
     ('--test_sharding_strategy=disabled' is okay) and without --runs_per_test";

// These variables are all used by runfiles libraries to locate the runfiles directory or manifest
// and can cause incorrect behavior when set for the top-level binary run with bazel run.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:215 RunCommand.ENV_VARIABLES_TO_CLEAR_UNCONDITIONALLY
const ENV_VARIABLES_TO_CLEAR_UNCONDITIONALLY: [&str; 5] = [
    "JAVA_RUNFILES",
    "RUNFILES_DIR",
    "RUNFILES_MANIFEST_FILE",
    "RUNFILES_MANIFEST_ONLY",
    "TEST_SRCDIR",
];

fn env_variables_to_clear_unconditionally() -> BTreeSet<String> {
    ENV_VARIABLES_TO_CLEAR_UNCONDITIONALLY
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Builds and run a target with the given command line arguments.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:120 RunCommand
pub struct RunCommand {
    /// The test policy to determine the environment variables from when running tests
    test_policy: TestPolicy,
}

impl RunCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:229 RunCommand.<init>
    pub fn new(test_policy: TestPolicy) -> RunCommand {
        RunCommand { test_policy }
    }
}

/// `RunCommandException`: a failed result and the time the command finished.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:907 RunCommand.RunCommandException
struct RunCommandException {
    result: BlazeCommandResult,
    /// `finishTimeMillis` (the `RunBuildCompleteEvent`'s).
    finish_time_millis: i64,
}

impl RunCommandException {
    fn new(result: BlazeCommandResult, finish_time_millis: i64) -> RunCommandException {
        check_argument!(
            !result.get_detailed_exit_code().is_success(),
            "Success is not exceptional: {:?}",
            result
        );
        RunCommandException {
            result,
            finish_time_millis,
        }
    }
}

/// Contains the targets built as part of a run-command invocation.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:919 RunCommand.BuiltTargets
struct BuiltTargets {
    target_to_run: ConfiguredTarget,
    target_to_run_runfiles_dir: Option<Path>,
    target_to_run_runfiles_support: Option<Arc<RunfilesSupport>>,
    run_under_target: Option<ConfiguredTarget>,
    configuration: Arc<BuildConfigurationValue>,
    convenience_symlinks: IndexMap<PathFragment, PathFragment>,
    stop_time: i64,
}

impl BlazeCommand for RunCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &RUN_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:233 RunCommand.editOptions
    fn edit_options(&self, _options_parser: &mut blaze_options::OptionsParser) {}

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:250 RunCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let run_options: RunCommandRunOptions = options
            .get_options::<RunCommandRunOptions>()
            .expect("NullPointerException: RunOptions");
        // This list should look like: ["//executable:target", "arg1", "arg2"]
        let target_and_args: Vec<String> = options.get_residue();

        // The user must at the least specify an executable target.
        if target_and_args.is_empty() {
            post_run_build_complete_event(
                env,
                blaze_util::ExitCode::COMMAND_LINE_ERROR,
                JavaClock.current_time_millis(),
                Vec::new(),
            );
            return report_and_create_failure_result(
                env,
                "Must specify a target to run",
                Code::NoTargetSpecified,
            );
        }
        let target_string = target_and_args[0].clone();
        let run_under: Option<RunUnder> = options
            .get_options::<CoreOptions>()
            .and_then(|o| o.run_under.clone());

        // runBuild, then the rest of exec on the BuildResult.
        run_build_and_then(
            env,
            options,
            &target_string,
            run_under.as_ref(),
            |outcome| {
                let RunBuildOutcome {
                    env,
                    build_result,
                    detailed_exit_code: _,
                    skyframe_executor,
                    analyzed_targets,
                    updated_options,
                } = outcome;
                // The options after BuildTool.buildTargets parsed a project file's flags into
                // the command's parser.
                let options: &OptionsParsingResult = updated_options.as_deref().unwrap_or(options);
                let built_targets = match get_built_targets(
                    env,
                    skyframe_executor,
                    build_result,
                    &analyzed_targets,
                    &target_string,
                    run_under.as_ref(),
                ) {
                    Ok(built_targets) => built_targets,
                    Err(e) => {
                        post_run_build_complete_event(
                            env,
                            run_command_exception_exit_code(&e),
                            e.finish_time_millis,
                            Vec::new(),
                        );
                        return e.result;
                    }
                };
                self.exec_with_built_targets(
                    env,
                    skyframe_executor,
                    options,
                    &run_options,
                    &target_and_args,
                    &built_targets,
                )
            },
        )
    }
}

impl RunCommand {
    /// `RunCommand.exec` after `runBuild` returned.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:275 RunCommand.exec
    fn exec_with_built_targets(
        &self,
        env: &mut CommandEnvironment,
        skyframe_executor: &SkyframeExecutor,
        options: &OptionsParsingResult,
        run_options: &RunCommandRunOptions,
        target_and_args: &[String],
        built_targets: &BuiltTargets,
    ) -> BlazeCommandResult {
        let mut run_complete_children_events = vec![
            build_event_id_util::build_tool_logs(),
            build_event_id_util::build_metrics(),
        ];
        if run_options.script_path.is_none() {
            run_complete_children_events.push(build_event_id_util::exec_request_id());
        }
        post_run_build_complete_event(
            env,
            // If the build returned non-zero exit code, an error would have already been thrown.
            blaze_util::ExitCode::SUCCESS,
            built_targets.stop_time,
            run_complete_children_events,
        );
        let args_from_residue: Vec<String> = target_and_args[1..].to_vec();
        let run_command_line = match get_command_line_info(
            env,
            skyframe_executor,
            built_targets,
            options,
            args_from_residue,
            run_options,
            &self.test_policy,
        ) {
            Ok(c) => c,
            Err(e) => return e.result,
        };
        let batch_mode = env
            .get_runtime()
            .get_startup_options_provider()
            .get_options::<BlazeServerStartupOptions>()
            .is_some_and(|o| o.batch);
        let mut final_run_env: BTreeMap<String, String> =
            run_command_line.get_environment().clone();
        if batch_mode {
            // In --batch, prioritize original client env-var values over those added by the c++
            // launcher. Only necessary in --batch since the command runs as a subprocess of the
            // java server.
            for (k, v) in env.get_client_env() {
                final_run_env.insert(k.clone(), v.clone());
            }
        }

        let should_run_target = run_options.script_path.is_none() && run_options.run_built_target;
        let paths_to_replace: Vec<PathToReplace> = if run_options.portable_paths {
            get_paths_to_replace(
                env,
                /* test_log_dir= */
                &built_targets
                    .configuration
                    .get_test_logs_directory(&MAIN)
                    .get_exec_path_string(),
                run_command_line.is_test_target(),
            )
        } else {
            Vec::new()
        };
        let exec_request = match exec_request_builder(
            env,
            &run_command_line,
            &final_run_env,
            &built_targets.configuration,
            built_targets.stop_time,
            should_run_target,
            paths_to_replace,
        ) {
            Ok(r) => r,
            Err(e) => return e.result,
        };

        if run_options.script_path.is_some() {
            return handle_script_path(
                run_options,
                exec_request,
                &run_command_line,
                env,
                built_targets,
            );
        }

        let execution_options: ExecutionOptions = options
            .get_options::<ExecutionOptions>()
            .expect("NullPointerException: ExecutionOptions");
        let show_subcommands = execution_options.show_subcommands;

        let command_description = if show_subcommands != ShowSubcommands::FALSE {
            let mut sh_executable: Option<String> = None;
            if run_command_line.requires_sh_executable() {
                match get_shell_executable_or_throw(
                    env,
                    &built_targets.configuration,
                    "",
                    built_targets.stop_time,
                ) {
                    Ok(sh) => sh_executable = Some(sh),
                    Err(e) => return e.result,
                }
            }
            let args = run_command_line.get_args(sh_executable.as_deref());
            let environment: IndexMap<String, String> = final_run_env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let to_clear: Vec<String> = run_command_line
                .get_environment_variables_to_clear()
                .iter()
                .cloned()
                .collect();
            CommandFailureUtils::describe_command(
                CommandDescriptionForm::Complete,
                show_subcommands == ShowSubcommands::PRETTY_PRINT,
                &args,
                Some(&environment),
                Some(&to_clear),
                Some(run_command_line.get_working_dir().get_path_string()),
                /* configuration_checksum= */ None,
                /* execution_platform_label= */ None,
                /* spawn_runner= */ None,
            )
        } else {
            run_command_line.get_pretty_args(run_options.run_omit_run_args)
        };

        let prefix = if run_options.run_built_target {
            "Running"
        } else {
            "Runnable"
        };
        let separator = if show_subcommands != ShowSubcommands::FALSE {
            ":\n"
        } else {
            ": "
        };
        env.get_reporter().handle(&Event::info(format!(
            "{prefix} command line{separator}{command_description}"
        )));

        if let Some(bep) = env.get_bep().cloned() {
            let include_residue_in_run_bep_event = options
                .get_options::<BuildEventProtocolOptions>()
                .expect("NullPointerException: BuildEventProtocolOptions")
                .include_residue_in_run_bep_event;
            let redacted_argv = if include_residue_in_run_bep_event {
                exec_request.argv.clone()
            } else {
                match get_argv(
                    env,
                    &run_command_line,
                    /* include_residue= */ false,
                    &built_targets.configuration,
                    built_targets.stop_time,
                ) {
                    Ok(argv) => argv,
                    Err(e) => return e.result,
                }
            };
            bep.post_build_event(Arc::new(ExecRequestEvent::new(
                exec_request.working_directory.clone(),
                exec_request
                    .environment_variable
                    .iter()
                    .map(|v| (v.name.clone(), v.value.clone()))
                    .collect(),
                exec_request.environment_variable_to_clear.clone(),
                exec_request.should_exec,
                redacted_argv,
            )));
        }
        BlazeCommandResult::execute(exec_request)
    }
}

/// `runBuild`, followed by `then` on its outcome once the build succeeded (Java: `exec` after
/// `runBuild` returned). A failed build reports "Build failed. Not running target".
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:394 RunCommand.runBuild
fn run_build_and_then<F>(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
    target_string: &str,
    run_under: Option<&RunUnder>,
    then: F,
) -> BlazeCommandResult
where
    F: FnOnce(RunBuildOutcome<'_>) -> BlazeCommandResult,
{
    let targets_to_build: Vec<String> = match run_under {
        Some(RunUnder::LabelRunUnder { label, .. }) => {
            vec![target_string.to_owned(), label.to_string()]
        }
        _ => vec![target_string.to_owned()],
    };
    let validator = RunTargetValidator {
        reporter: env.get_reporter().clone(),
        target_pattern_strings: targets_to_build.clone(),
        run_under: run_under.cloned(),
    };
    run_build(
        env,
        options,
        /* run_tests= */ false,
        Some(RunBuild {
            targets: targets_to_build,
            validator: &validator,
            repo_precomputed: Default::default(),
            post_build_callback: None,
            after_build: Box::new(move |outcome: RunBuildOutcome<'_>| {
                if !outcome.build_result.get_success() {
                    outcome
                        .env
                        .get_reporter()
                        .handle(&Event::error("Build failed. Not running target"));
                    // RunCommandException(detailedExitCode(buildResult.getDetailedExitCode()),
                    // buildResult.getStopTime()), caught by exec.
                    post_run_build_complete_event(
                        outcome.env,
                        blaze_util::ExitCode::for_code(
                            outcome
                                .detailed_exit_code
                                .get_exit_code()
                                .get_numeric_exit_code(),
                        )
                        .unwrap_or_else(|| panic!("unknown exit code")),
                        outcome.build_result.get_stop_time(),
                        Vec::new(),
                    );
                    return BlazeCommandResult::detailed_exit_code(outcome.detailed_exit_code);
                }
                // Build succeeded - make sure outputs are available before attempting to use
                // them.
                // flushOutputs(env): the local output service has nothing to flush.
                then(outcome)
            }),
        }),
    )
    // PORT: a failure that the build path returns before BuildTool.processRequest (the command's
    // Skyframe executor or spawn log setup, the main repository mapping) is returned as it is,
    // without "Build failed. Not running target"; in Java those happen outside the build too.
}

/// `env.getReporter().post(new RunBuildCompleteEvent(exitCode, finishTimeMillis, children))`.
// Java: com/google/devtools/build/lib/buildtool/buildevent/RunBuildCompleteEvent.java:29 RunBuildCompleteEvent.<init>
fn post_run_build_complete_event(
    env: &CommandEnvironment,
    exit_code: blaze_util::ExitCode,
    finish_time_millis: i64,
    children: Vec<blaze_protos::build_event_stream::BuildEventId>,
) {
    if let Some(bep) = env.get_bep() {
        bep.post_build_event(Arc::new(run_build_complete_event(
            exit_code,
            finish_time_millis,
            children,
        )));
    }
}

/// `e.result.getDetailedExitCode().getExitCode()` of a `RunCommandException`.
fn run_command_exception_exit_code(e: &RunCommandException) -> blaze_util::ExitCode {
    blaze_util::ExitCode::for_code(e.result.get_exit_code().get_numeric_exit_code())
        .unwrap_or_else(|| panic!("unknown exit code {:?}", e.result.get_exit_code()))
}

/// The `ConfiguredTarget` behind one of the build result's views (the same lookup key).
pub(crate) fn configured_target_of(
    analyzed_targets: &[ConfiguredTarget],
    view: &blaze_buildtool::buildtool::build_result::ConfiguredTargetRef,
) -> ConfiguredTarget {
    let key = view.get_lookup_key();
    analyzed_targets
        .iter()
        .find(|ct| *ct.unwrap_if_merged().get_lookup_key() == key)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "IllegalStateException: successful target {} was not analyzed",
                view.get_label()
            )
        })
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:435 RunCommand.getBuiltTargets
fn get_built_targets(
    env: &mut CommandEnvironment,
    skyframe_executor: &SkyframeExecutor,
    result: &blaze_buildtool::buildtool::build_result::BuildResult,
    analyzed_targets: &[ConfiguredTarget],
    target_string: &str,
    run_under: Option<&RunUnder>,
) -> Result<BuiltTargets, RunCommandException> {
    let top_level_targets: Option<Vec<ConfiguredTarget>> =
        result.get_successful_targets().map(|views| {
            views
                .iter()
                .map(|view| configured_target_of(analyzed_targets, view))
                .collect()
        });
    let mut target_to_run: Option<ConfiguredTarget> = None;
    let mut run_under_target: Option<ConfiguredTarget> = None;
    let label_run_under = match run_under {
        Some(RunUnder::LabelRunUnder { label, .. }) => Some(label),
        _ => None,
    };

    if let Some(top_level_targets) = &top_level_targets {
        // Make sure that we have exactly 1 built target (excluding --run_under) and that it is
        // executable. These checks should only fail if keepGoing is true, because we already did
        // validation before the build began in validateTargets().
        let max_targets = if label_run_under.is_some() { 2 } else { 1 };
        if top_level_targets.len() > max_targets {
            return Err(RunCommandException::new(
                report_and_create_failure_result(
                    env,
                    &make_error_message_for_not_having_a_single_target(
                        target_string,
                        &top_level_targets
                            .iter()
                            .map(|ct| ct.get_label().to_string())
                            .collect::<Vec<_>>(),
                    ),
                    Code::TooManyTargetsSpecified,
                ),
                result.get_stop_time(),
            ));
        }

        for target in top_level_targets {
            let target_validation_result = fully_validate_target(env, skyframe_executor, target);
            if !target_validation_result
                .get_detailed_exit_code()
                .is_success()
            {
                return Err(RunCommandException::new(
                    target_validation_result,
                    result.get_stop_time(),
                ));
            }
            if let Some(label) = label_run_under
                && target.get_original_label() == *label
            {
                if run_under_target.is_some() {
                    return Err(RunCommandException::new(
                        report_and_create_failure_result(
                            env,
                            "Can't identify the run_under target from multiple options?",
                            Code::RunUnderTargetNotBuilt,
                        ),
                        result.get_stop_time(),
                    ));
                }
                run_under_target = Some(target.clone());
            } else if target_to_run.is_none() {
                target_to_run = Some(target.clone());
            } else {
                return Err(RunCommandException::new(
                    report_and_create_failure_result(
                        env,
                        &make_error_message_for_not_having_a_single_target(
                            target_string,
                            &top_level_targets
                                .iter()
                                .map(|ct| ct.get_label().to_string())
                                .collect::<Vec<_>>(),
                        ),
                        Code::TooManyTargetsSpecified,
                    ),
                    result.get_stop_time(),
                ));
            }
        }
    }

    // Handle target & run_under referring to the same target.
    if target_to_run.is_none() && run_under_target.is_some() {
        target_to_run = run_under_target.clone();
    }

    let Some(target_to_run) = target_to_run else {
        return Err(RunCommandException::new(
            report_and_create_failure_result(env, NO_TARGET_MESSAGE, Code::NoTargetSpecified),
            result.get_stop_time(),
        ));
    };

    let reporter = env.get_reporter().clone();
    let mut configuration = target_to_run
        .get_configuration_key()
        .and_then(|key| skyframe_executor.get_configuration_for_key(reporter.clone(), &key));
    if configuration.is_none() {
        // The target may be an input file, which doesn't have a configuration. In that case, we
        // choose any target configuration.
        configuration = result.get_build_configuration().cloned();
    }
    let configuration =
        configuration.expect("NullPointerException: the build's top-level configuration");

    // When --nobuild_runfile_manifests is enabled, the output service is responsible for staging
    // runfiles.
    if !blaze_analysis::rule::config_util::core_options(&configuration).build_runfile_manifests
        && !OUTPUT_SERVICE_STAGES_TOP_LEVEL_RUNFILES
    {
        return Err(RunCommandException::new(
            report_and_create_failure_result(
                env,
                "--nobuild_runfile_manifests is incompatible with the \"run\" command",
                Code::RunPrereqUnmet,
            ),
            result.get_stop_time(),
        ));
    }

    // Ensure runfiles directories are constructed, both for the target to run and the
    // --run_under target. The path of the runfiles directory of the target to run needs to be
    // preserved, as it acts as the working directory.
    let mut target_to_run_runfiles_dir: Option<Path> = None;
    let mut target_to_run_runfiles_support: Option<Arc<RunfilesSupport>> = None;
    // Java: com/google/devtools/build/lib/exec/RunfilesTreeUpdater.java:61 RunfilesTreeUpdater.forCommandEnvironment
    let runfiles_tree_updater = RunfilesTreeUpdater::new(env.get_exec_root());
    for target in top_level_targets.as_deref().unwrap_or_default() {
        let provider = target.get_provider::<FilesToRunProvider>();
        let Some(runfiles_support) = provider.and_then(|p| p.get_runfiles_support().cloned())
        else {
            continue;
        };
        let target_configuration = target
            .get_configuration_key()
            .and_then(|key| skyframe_executor.get_configuration_for_key(reporter.clone(), &key))
            .expect("NullPointerException: the configuration of a target with runfiles");
        match ensure_runfiles_built(
            env,
            &runfiles_support,
            &target_configuration,
            &runfiles_tree_updater,
        ) {
            Ok(runfiles_dir) => {
                if target.get_lookup_key() == target_to_run.get_lookup_key() {
                    target_to_run_runfiles_dir = Some(runfiles_dir);
                    target_to_run_runfiles_support = Some(runfiles_support);
                }
            }
            Err(e) => {
                env.get_reporter().handle(&Event::error(e.get_message()));
                return Err(RunCommandException::new(
                    BlazeCommandResult::failure_detail(e.create_failure_detail()),
                    result.get_stop_time(),
                ));
            }
        }
    }
    Ok(BuiltTargets {
        target_to_run,
        target_to_run_runfiles_dir,
        target_to_run_runfiles_support,
        run_under_target,
        configuration,
        convenience_symlinks: result.get_convenience_symlinks().clone(),
        stop_time: result.get_stop_time(),
    })
}

/// `env.getOutputService().stagesTopLevelRunfiles()` of Bazel's default `LocalOutputService`.
// Java: com/google/devtools/build/lib/exec/local/LocalOutputService.java LocalOutputService.stagesTopLevelRunfiles
const OUTPUT_SERVICE_STAGES_TOP_LEVEL_RUNFILES: bool = false;

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:552 RunCommand.execRequestBuilder
fn exec_request_builder(
    env: &CommandEnvironment,
    run_command_line: &RunCommandLine,
    run_env: &BTreeMap<String, String>,
    configuration: &Arc<BuildConfigurationValue>,
    stop_time: i64,
    should_run_target: bool,
    paths_to_replace: Vec<PathToReplace>,
) -> Result<ExecRequest, RunCommandException> {
    let mut exec_description = ExecRequest {
        working_directory: bytes(run_command_line.get_working_dir().get_path_string()),
        argv: get_argv_for_exec_request(env, run_command_line, configuration, stop_time)?,
        ..Default::default()
    };

    for (name, value) in run_env {
        exec_description
            .environment_variable
            .push(EnvironmentVariable {
                name: bytes(name),
                value: bytes(value),
            });
    }
    exec_description.environment_variable_to_clear = run_command_line
        .get_environment_variables_to_clear()
        .iter()
        .map(|s| bytes(s))
        .collect();
    exec_description.should_exec = should_run_target;
    exec_description.path_to_replace = paths_to_replace;
    Ok(exec_description)
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:581 RunCommand.getPathsToReplace
fn get_paths_to_replace(
    env: &CommandEnvironment,
    test_log_dir: &str,
    is_test_target: bool,
) -> Vec<PathToReplace> {
    let mut paths_to_replace = path_to_replace_utils_get_paths_to_replace(env);
    if is_test_target {
        paths_to_replace.push(PathToReplace {
            r#type: path_to_replace::Type::TestLogSubdir as i32,
            value: bytes(test_log_dir),
        });
    }
    paths_to_replace
}

/// Returns the common required `PathToReplace` list.
// Java: com/google/devtools/build/lib/runtime/commands/PathToReplaceUtils.java:28 PathToReplaceUtils.getPathsToReplace
pub(crate) fn path_to_replace_utils_get_paths_to_replace(
    env: &CommandEnvironment,
) -> Vec<PathToReplace> {
    vec![
        PathToReplace {
            r#type: path_to_replace::Type::OutputBase as i32,
            value: bytes(env.get_output_base().get_path_string()),
        },
        PathToReplace {
            r#type: path_to_replace::Type::BuildWorkingDirectory as i32,
            value: bytes(env.get_working_directory().get_path_string()),
        },
        PathToReplace {
            r#type: path_to_replace::Type::BuildWorkspaceDirectory as i32,
            value: bytes(
                env.get_workspace()
                    .expect("NullPointerException: the workspace")
                    .get_path_string(),
            ),
        },
    ]
}

/// Converts a string to bytes for use in `ExecRequest` bytes fields.
// Java: com/google/devtools/build/lib/runtime/commands/PathToReplaceUtils.java:46 PathToReplaceUtils.bytes
fn bytes(string: &str) -> Vec<u8> {
    // ByteString.copyFrom(string, ISO_8859_1)
    latin1_encode_lossy(string).into_owned()
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:596 RunCommand.getArgvForExecRequest
fn get_argv_for_exec_request(
    env: &CommandEnvironment,
    run_command_line: &RunCommandLine,
    configuration: &Arc<BuildConfigurationValue>,
    stop_time: i64,
) -> Result<Vec<Vec<u8>>, RunCommandException> {
    get_argv(
        env,
        run_command_line,
        /* include_residue= */ true,
        configuration,
        stop_time,
    )
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:614 RunCommand.getArgv
fn get_argv(
    env: &CommandEnvironment,
    run_command_line: &RunCommandLine,
    include_residue: bool,
    configuration: &Arc<BuildConfigurationValue>,
    stop_time: i64,
) -> Result<Vec<Vec<u8>>, RunCommandException> {
    let mut sh_executable: Option<String> = None;
    if run_command_line.requires_sh_executable() {
        sh_executable = Some(get_shell_executable_or_throw(
            env,
            configuration,
            /* reason= */ "",
            stop_time,
        )?);
    }
    let args = if include_residue {
        run_command_line.get_args(sh_executable.as_deref())
    } else {
        run_command_line.get_args_without_residue(sh_executable.as_deref())
    };
    Ok(args.iter().map(|s| bytes(s)).collect())
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:632 RunCommand.handleScriptPath
fn handle_script_path(
    run_options: &RunCommandRunOptions,
    mut exec_request: ExecRequest,
    run_command_line: &RunCommandLine,
    env: &CommandEnvironment,
    built_targets: &BuiltTargets,
) -> BlazeCommandResult {
    let sh_executable = match get_shell_executable_or_throw(
        env,
        &built_targets.configuration,
        "with \"--script_path\"",
        built_targets.stop_time,
    ) {
        Ok(sh) => sh,
        Err(e) => return e.result,
    };

    let script_contents = run_command_line.get_script_form(&sh_executable);
    let script_path = &run_options
        .script_path
        .as_ref()
        .expect("script_path is set")
        .0;

    if run_options.emit_script_path_in_exec_request {
        exec_request.script_path = Some(ScriptPath {
            script_path: bytes(script_path.get_path_string()),
            script_contents: bytes(&script_contents),
        });
        BlazeCommandResult::execute(exec_request)
    } else {
        if let Err(e) = write_script(env, script_path, &script_contents) {
            let message = format!("Error writing run script: {e}");
            return report_and_create_failure_result(env, &message, Code::ScriptWriteFailure);
        }
        BlazeCommandResult::success()
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:665 RunCommand.getCommandLineInfo
fn get_command_line_info(
    env: &CommandEnvironment,
    skyframe_executor: &SkyframeExecutor,
    built_targets: &BuiltTargets,
    options: &OptionsParsingResult,
    args_from_residue: Vec<String>,
    run_options: &RunCommandRunOptions,
    test_policy: &TestPolicy,
) -> Result<RunCommandLine, RunCommandException> {
    if built_targets
        .target_to_run
        .get_provider::<TestProvider>()
        .is_some()
    {
        return get_test_command_line(
            env,
            skyframe_executor,
            built_targets,
            options,
            args_from_residue,
            test_policy,
        );
    }

    let action_environment = built_targets
        .target_to_run_runfiles_support
        .as_ref()
        .map(|s| s.get_action_environment().clone());
    // The final run environment is a combination of the environment constructed here and the
    // unrestricted client environment. This means that there is a difference between a variable
    // that isn't included in runEnvironment (which will have its value inherited from the client
    // environment) and a variable that is explicitly removed (which will be unset in the run
    // environment). We thus track the environment variables to clear separately.
    let mut run_environment = make_mutable_run_environment(env);
    let mut env_variables_to_clear: HashSet<String> = HashSet::new();
    let client_env = env.get_client_env();
    // Process --run_env flags first
    for env_var in &run_options.run_environment {
        match env_var {
            EnvVar::Set { name, value } => {
                run_environment.insert(name.clone(), value.clone());
                env_variables_to_clear.remove(name);
            }
            EnvVar::Inherit { name } => {
                // If a value is missing, inherit from client environment if present, otherwise
                // leave unset. In the latter case, explicitly remove since the same name might be
                // given multiple times.
                match client_env.get(name) {
                    Some(value) => {
                        run_environment.insert(name.clone(), value.clone());
                    }
                    None => {
                        run_environment.remove(name);
                    }
                }
                env_variables_to_clear.remove(name);
            }
            EnvVar::Unset { name } => {
                run_environment.remove(name);
                env_variables_to_clear.insert(name.clone());
            }
        }
    }
    // Then let the target's environment override --run_env flags
    // (ActionEnvironment.EMPTY resolves to nothing.)
    if let Some(action_environment) = action_environment {
        let mut resolved: IndexMap<String, String> = run_environment
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        action_environment.resolve(&mut resolved, client_env);
        run_environment = resolved.into_iter().collect();
    }

    let mut to_clear: BTreeSet<String> = env_variables_to_clear.into_iter().collect();
    to_clear.extend(env_variables_to_clear_unconditionally());
    Ok(construct_command_line(
        env,
        built_targets,
        run_environment,
        to_clear,
        get_binary_args(&built_targets.target_to_run),
        args_from_residue,
        run_options,
    ))
}

/// Returns the arguments in a `ConfiguredTarget`'s `args` attribute.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:237 RunCommand.getBinaryArgs
fn get_binary_args(target_to_run: &ConfiguredTarget) -> Vec<String> {
    let Some(provider) = target_to_run.get_provider::<FilesToRunProvider>() else {
        return Vec::new();
    };
    let Some(runfiles_support) = provider.get_runfiles_support() else {
        return Vec::new();
    };
    // RunfilesSupport.getArgs() is a flat command line: its arguments do not expand.
    runfiles_support
        .get_args()
        .arguments()
        .unwrap_or_else(|e| panic!("RunfilesSupport.getArgs().arguments(): {e:?}"))
}

/// Returns the command line for the test, making a best effort to mimic the environment had we
/// run `test //target`.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:731 RunCommand.getTestCommandLine
fn get_test_command_line(
    env: &CommandEnvironment,
    skyframe_executor: &SkyframeExecutor,
    built_targets: &BuiltTargets,
    options: &OptionsParsingResult,
    args_from_residue: Vec<String>,
    test_policy: &TestPolicy,
) -> Result<RunCommandLine, RunCommandException> {
    let status_artifacts = TestProvider::get_test_status_artifacts(&built_targets.target_to_run);
    if status_artifacts.len() != 1 {
        return Err(RunCommandException::new(
            report_and_create_failure_result(
                env,
                MULTIPLE_TESTS_MESSAGE,
                Code::TooManyTestShardsOrRuns,
            ),
            built_targets.stop_time,
        ));
    }

    let test_action = get_generating_test_runner_action(skyframe_executor, &status_artifacts[0]);
    let settings = test_action.get_execution_settings();
    // ensureRunfilesBuilt does build the runfiles, but an extra consistency check won't hurt.
    check_state!(
        settings.get_runfiles_symlinks_created()
            == options
                .get_options::<CoreOptions>()
                .expect("NullPointerException: CoreOptions")
                .build_runfile_links
    );

    let exec_root = env.get_exec_root();
    let runfiles_dir: Path = match settings.get_runfiles_dir() {
        Some(dir) => dir.clone(),
        None => built_targets
            .target_to_run_runfiles_dir
            .as_ref()
            .expect("NullPointerException: targetToRunRunfilesDir")
            .get_parent_directory()
            .expect("NullPointerException: getParentDirectory"),
    };

    let execution_options: ExecutionOptions = options
        .get_options::<ExecutionOptions>()
        .expect("NullPointerException: ExecutionOptions");
    let tmp_dir_root = TestStrategy::get_tmp_root(
        env.get_workspace()
            .expect("NullPointerException: the workspace"),
        &exec_root,
        &execution_options,
    );
    let maybe_relative_tmp_dir: PathFragment = if tmp_dir_root.starts_with(&exec_root) {
        tmp_dir_root.relative_to(&exec_root)
    } else {
        tmp_dir_root.as_fragment().clone()
    };
    let mut run_environment = make_mutable_run_environment(env);
    for (k, v) in test_policy.compute_test_environment(
        &test_action,
        env.get_client_env(),
        &runfiles_dir.relative_to(&exec_root),
        &maybe_relative_tmp_dir.get_relative_str(&TestStrategy::get_tmp_dir_name(&test_action)),
    ) {
        run_environment.insert(k, v);
    }

    // testAction.prepare(execRoot, ArtifactPathResolver.IDENTITY, null, false)
    if let Err(e) = test_action.prepare(&env.get_exec_root()) {
        return Err(RunCommandException::new(
            report_and_create_failure_result(
                env,
                &format!("Error while setting up test: {e}"),
                Code::TestEnvironmentSetupFailure,
            ),
            built_targets.stop_time,
        ));
    }

    let test_args = match TestStrategy::get_args(&test_action) {
        Ok(args) => args,
        Err(ExecException::Interrupted) => {
            let message = "run: command line expansion interrupted";
            env.get_reporter().handle(&Event::error(message));
            return Err(RunCommandException::new(
                BlazeCommandResult::detailed_exit_code(interrupted_detailed_exit_code(message)),
                built_targets.stop_time,
            ));
        }
        Err(e) => {
            // Strings.nullToEmpty(e.getMessage())
            return Err(RunCommandException::new(
                report_and_create_failure_result(
                    env,
                    &e.get_message(),
                    Code::CommandLineExpansionFailure,
                ),
                built_targets.stop_time,
            ));
        }
    };

    let mut builder = RunCommandLineBuilder::new(
        run_environment,
        env_variables_to_clear_unconditionally(),
        /* working_dir= */ exec_root,
        /* is_test_target= */ true,
    );
    builder.add_args(test_args);
    builder.add_args_from_residue(args_from_residue);
    Ok(builder.build())
}

/// `(TestRunnerAction) env.getSkyframeExecutor().getActionGraph(reporter)
/// .getGeneratingAction(statusArtifact)`: the action at the artifact's generating action key in
/// its owner's `ConfiguredTargetValue`.
fn get_generating_test_runner_action(
    skyframe_executor: &SkyframeExecutor,
    status_artifact: &Artifact,
) -> Arc<TestRunnerAction> {
    let generating_action_key = status_artifact.get_generating_action_key();
    let owner = generating_action_key.get_action_lookup_key();
    let configured_target_key = owner
        .as_any()
        .downcast_ref::<ConfiguredTargetKey>()
        .unwrap_or_else(|| panic!("ClassCastException: {owner} is not a ConfiguredTargetKey"));
    let graph = skyframe_executor.get_walkable_graph();
    let value = blaze_sky_top::skyframe_executor::get_configured_target_value(
        &*graph,
        &SkyKey::new(configured_target_key.clone()),
    )
    .unwrap_or_else(|| panic!("NullPointerException: no value for {owner}"));
    let action = value.get_actions()[generating_action_key.get_action_index() as usize].clone();
    blaze_exec::analysis::test::as_test_runner_action(action.as_ref()).unwrap_or_else(|| {
        panic!(
            "ClassCastException: {} cannot be cast to TestRunnerAction",
            action.get_mnemonic()
        )
    })
}

/// Returns a new sorted map with environment variables common to all run invocations. The
/// return value is a new, mutable instance - this is necessary since we want to maintain order
/// and overwrite existing keys.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:837 RunCommand.makeMutableRunEnvironment
fn make_mutable_run_environment(env: &CommandEnvironment) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    result.insert(
        "BUILD_WORKSPACE_DIRECTORY".to_owned(),
        env.get_workspace()
            .expect("NullPointerException: the workspace")
            .get_path_string()
            .to_owned(),
    );
    result.insert(
        "BUILD_WORKING_DIRECTORY".to_owned(),
        env.get_working_directory().get_path_string().to_owned(),
    );
    result.insert(
        "BUILD_EXECROOT".to_owned(),
        env.get_exec_root().get_path_string().to_owned(),
    );
    result.insert("BUILD_ID".to_owned(), env.get_command_id().to_owned());
    result
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:846 RunCommand.constructCommandLine
fn construct_command_line(
    env: &CommandEnvironment,
    built_targets: &BuiltTargets,
    run_environment: BTreeMap<String, String>,
    env_variables_to_clear: BTreeSet<String>,
    args_from_binary: Vec<String>,
    args_from_residue: Vec<String>,
    run_options: &RunCommandRunOptions,
) -> RunCommandLine {
    let request_options: BuildRequestOptions = env
        .get_options()
        .get_options::<BuildRequestOptions>()
        .expect("NullPointerException: BuildRequestOptions");
    let pretty_printer = PathPrettyPrinter::new(
        env.get_relative_working_directory(),
        &get_symlink_prefix(&request_options, env.get_runtime().get_product_name()),
        built_targets.convenience_symlinks.clone(),
    );

    let working_dir = match &built_targets.target_to_run_runfiles_dir {
        Some(runfiles_dir) if !run_options.run_in_cwd => runfiles_dir.clone(),
        _ => env.get_working_directory().clone(),
    };
    let mut run_command_line = RunCommandLineBuilder::new(
        run_environment,
        env_variables_to_clear,
        working_dir,
        /* is_test_target= */ false,
    );

    let run_under: Option<RunUnder> = env
        .get_options()
        .get_options::<CoreOptions>()
        .and_then(|o| o.run_under.clone());
    // Insert the command prefix specified by the "--run_under=<command-prefix>" option at the
    // start of the command line.
    if let Some(run_under) = &run_under {
        if let Some(run_under_target) = &built_targets.run_under_target {
            // --run_under specifies a target. Get the corresponding executable, this will be an
            // absolute path because the run_under target is only in the runfiles of test targets
            let run_under_path = run_under_target
                .get_provider::<FilesToRunProvider>()
                .and_then(|p| p.get_executable())
                .expect("NullPointerException: the --run_under target's executable")
                .get_path();
            run_command_line.set_run_under_target(
                &run_under_path,
                run_under.options(),
                &pretty_printer,
            );
        } else {
            run_command_line.set_run_under_prefix(run_under.value());
        }
    }

    let executable = built_targets
        .target_to_run
        .get_provider::<FilesToRunProvider>()
        .and_then(|p| p.get_executable())
        .expect("NullPointerException: the executable")
        .get_path();
    run_command_line.add_arg_path(&executable, &pretty_printer);
    run_command_line.add_args(args_from_binary);
    run_command_line.add_args_from_residue(args_from_residue);
    run_command_line.build()
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:891 RunCommand.getShellExecutableOrThrow
fn get_shell_executable_or_throw(
    env: &CommandEnvironment,
    configuration: &Arc<BuildConfigurationValue>,
    reason: &str,
    stop_time: i64,
) -> Result<String, RunCommandException> {
    let sh_executable = blaze_analysis::rule::sh_toolchain::get_path_for_host(configuration);
    if sh_executable.is_empty() {
        return Err(RunCommandException::new(
            report_and_create_failure_result(
                env,
                &format!(
                    "the \"run\" command needs a shell{reason}; use the \
                     --shell_executable=<path> flag to specify the shell's path, e.g. \
                     --shell_executable=/bin/bash"
                ),
                Code::NoShellSpecified,
            ),
            stop_time,
        ));
    }
    Ok(sh_executable.get_path_string().to_owned())
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:965 RunCommand.reportAndCreateFailureResult
fn report_and_create_failure_result(
    env: &CommandEnvironment,
    message: &str,
    detailed_code: Code,
) -> BlazeCommandResult {
    env.get_reporter().handle(&Event::error(message));
    BlazeCommandResult::failure_detail(create_failure_detail(message, detailed_code))
}

/// `RunfilesException`: "Error creating runfiles: " and the failure.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1233 RunCommand.RunfilesException
struct RunfilesException {
    message: String,
    detailed_code: Code,
}

impl RunfilesException {
    fn new(message: String, detailed_code: Code) -> RunfilesException {
        RunfilesException {
            message: format!("Error creating runfiles: {message}"),
            detailed_code,
        }
    }

    fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1241 RunCommand.RunfilesException.createFailureDetail
    fn create_failure_detail(&self) -> FailureDetail {
        create_failure_detail(&self.message, self.detailed_code)
    }
}

/// Ensures that runfiles are built for the specified target. If they already are, does nothing,
/// otherwise builds them.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:976 RunCommand.ensureRunfilesBuilt
fn ensure_runfiles_built(
    env: &CommandEnvironment,
    runfiles_support: &RunfilesSupport,
    configuration: &Arc<BuildConfigurationValue>,
    runfiles_tree_updater: &RunfilesTreeUpdater,
) -> Result<Path, RunfilesException> {
    let runfiles_dir: PathFragment = runfiles_support.get_runfiles_tree().get_exec_path();
    let mut working_dir = env.get_exec_root().get_relative(&runfiles_dir);
    // On Windows, runfiles tree is disabled.
    // Workspace name directory doesn't exist, so don't add it.
    if configuration.runfiles_enabled() {
        working_dir = working_dir.get_relative_str(runfiles_support.get_runfiles().get_prefix());
    }

    // Return early if runfiles staging is managed by the output service.
    if OUTPUT_SERVICE_STAGES_TOP_LEVEL_RUNFILES {
        return Ok(working_dir);
    }

    // Always create runfiles directory and the workspace-named directory underneath, even if we
    // run with --enable_runfiles=no (which is the default on Windows as of 2020-01-24).
    // If the binary we run is in fact a test, it will expect to be able to chdir into the
    // runfiles directory. See https://github.com/bazelbuild/bazel/issues/10621
    if let Err(e) = runfiles_support
        .get_runfiles_directory()
        .get_relative_str(&runfiles_support.get_runfiles_tree().get_workspace_name())
        .create_directory_and_parents()
    {
        return Err(RunfilesException::new(
            format!("Failed to create runfiles directories: {e}"),
            Code::RunfilesDirectoriesCreationFailure,
        ));
    }

    let tree: Arc<dyn blaze_exec::actions::RunfilesTree> =
        Arc::new(RunfilesTreeAdapter(runfiles_support.get_runfiles_tree()));
    if let Err(e) = runfiles_tree_updater.update_runfiles(&[tree]) {
        let message = match &e {
            UpdateRunfilesError::Exec(e) => e.get_message(),
            UpdateRunfilesError::Io(e) => e.to_string(),
        };
        return Err(RunfilesException::new(
            format!("Failed to create runfiles symlinks: {message}"),
            Code::RunfilesSymlinksCreationFailure,
        ));
    }
    Ok(working_dir)
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1022 RunCommand.writeScript
fn write_script(
    env: &CommandEnvironment,
    script_path_frag: &PathFragment,
    script_content: &str,
) -> std::io::Result<()> {
    let script_path = env.get_working_directory().get_relative(script_path_frag);
    // FileSystemUtils.writeContent(scriptPath, ISO_8859_1, scriptContent)
    {
        let mut out = script_path.get_output_stream()?;
        out.write_all(&latin1_encode_lossy(script_content))?;
        out.flush()?;
    }
    script_path.set_executable(true)
}

/// `validateTargets`, the `TargetValidator` that `runBuild` passes to `BuildTool`: make sure we
/// are building exactly 1 binary target. If keepGoing, we'll build all the targets even if they
/// are non-binary.
struct RunTargetValidator {
    reporter: Arc<blaze_events::Reporter>,
    /// `request.getTargets()`.
    target_pattern_strings: Vec<String>,
    run_under: Option<RunUnder>,
}

impl blaze_sky_top::build_tool::TargetValidator for RunTargetValidator {
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1031 RunCommand.validateTargets
    fn validate_targets(
        &self,
        targets: &[Target],
        keep_going: bool,
    ) -> Result<(), LoadingFailedException> {
        let reporter: &dyn EventHandler = self.reporter.as_ref();
        let mut target_to_run: Option<&Target> = None;
        let mut run_under_target: Option<&Target> = None;
        let label_run_under = match &self.run_under {
            Some(RunUnder::LabelRunUnder { label, .. }) => Some(label),
            _ => None,
        };

        let mut single_target_warning_was_output = false;
        let max_targets = if label_run_under.is_some() { 2 } else { 1 };
        let target_names = || -> Vec<String> {
            targets
                .iter()
                .map(|t| t.get_label().to_string())
                .collect::<Vec<_>>()
        };
        if targets.len() > max_targets {
            warning_or_exception(
                reporter,
                &make_error_message_for_not_having_a_single_target(
                    &self.target_pattern_strings[0],
                    &target_names(),
                ),
                keep_going,
                Code::TooManyTargetsSpecified,
            )?;
            single_target_warning_was_output = true;
        }
        for target in targets {
            if !is_executable(target) {
                warning_or_exception(
                    reporter,
                    &not_executable_error(target),
                    keep_going,
                    Code::TargetNotExecutable,
                )?;
            }

            if let Some(label) = label_run_under
                && target.get_label() == label
            {
                // It's impossible to have two targets with the same label.
                check_state!(run_under_target.is_none());
                run_under_target = Some(target);
            } else if target_to_run.is_none() {
                target_to_run = Some(target);
            } else {
                if !single_target_warning_was_output {
                    warning_or_exception(
                        reporter,
                        &make_error_message_for_not_having_a_single_target(
                            &self.target_pattern_strings[0],
                            &target_names(),
                        ),
                        keep_going,
                        Code::TooManyTargetsSpecified,
                    )?;
                }
                return Ok(());
            }
        }
        // Handle target & run_under referring to the same target.
        if target_to_run.is_none() && run_under_target.is_some() {
            target_to_run = run_under_target;
        }
        if target_to_run.is_none() {
            warning_or_exception(
                reporter,
                NO_TARGET_MESSAGE,
                keep_going,
                Code::NoTargetSpecified,
            )?;
        }
        Ok(())
    }
}

/// If keepGoing, print a warning and return the given collection. Otherwise, throw
/// InvalidTargetException.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1091 RunCommand.warningOrException
fn warning_or_exception(
    reporter: &dyn EventHandler,
    message: &str,
    keep_going: bool,
    detailed_code: Code,
) -> Result<(), LoadingFailedException> {
    if keep_going {
        reporter.handle(&Event::warn(format!("{message}. Will continue anyway")));
        Ok(())
    } else {
        let failure_detail = create_failure_detail(message, detailed_code);
        Err(LoadingFailedException {
            message: message.to_owned(),
            exit_code: blaze_sky_top::build_tool::exit_code_of(&failure_detail),
            failure_detail,
        })
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1102 RunCommand.notExecutableError
fn not_executable_error(target: &Target) -> String {
    format!("Cannot run target {}: Not executable", target.get_label())
}

/// Performs all available validation checks on an individual target. Returns success if all
/// checks succeeded, otherwise a result describing the failure.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1115 RunCommand.fullyValidateTarget
fn fully_validate_target(
    env: &CommandEnvironment,
    skyframe_executor: &SkyframeExecutor,
    configured_target: &ConfiguredTarget,
) -> BlazeCommandResult {
    let target = match skyframe_executor
        .get_package_manager()
        .get_target(env.get_reporter().clone(), &configured_target.get_label())
    {
        Ok(target) => target,
        Err(NoSuchThingOrInterrupted::Interrupted) => {
            let message = "run command interrupted";
            env.get_reporter().handle(&Event::error(message));
            return BlazeCommandResult::detailed_exit_code(interrupted_detailed_exit_code(message));
        }
        Err(e) => {
            env.get_reporter().handle(&Event::error(format!(
                "Failed to find a target to validate. {e}"
            )));
            panic!("IllegalStateException: Failed to find a target to validate: {e}");
        }
    };

    if !is_executable(&target) {
        return report_and_create_failure_result(
            env,
            &not_executable_error(&target),
            Code::TargetNotExecutable,
        );
    }

    let executable = configured_target
        .get_provider::<FilesToRunProvider>()
        .unwrap_or_else(|| panic!("NullPointerException: {}", configured_target.get_label()))
        .get_executable()
        .cloned();
    let Some(executable) = executable else {
        return report_and_create_failure_result(
            env,
            &not_executable_error(&target),
            Code::TargetNotExecutable,
        );
    };

    let executable_path = executable.get_path();
    let check = if !executable_path.exists() {
        Ok(false)
    } else {
        executable_path.is_executable()
    };
    match check {
        Ok(true) => {}
        Ok(false) => {
            return report_and_create_failure_result(
                env,
                &format!("Non-existent or non-executable {}", executable_path),
                Code::TargetBuiltButPathNotExecutable,
            );
        }
        Err(e) => {
            return report_and_create_failure_result(
                env,
                &format!(
                    "Error checking {}: {}",
                    executable_path.get_path_string(),
                    e
                ),
                Code::TargetBuiltButPathValidationFailed,
            );
        }
    }

    BlazeCommandResult::success()
}

/// Return true iff it is possible that `target` is a rule that has an executable file. This
/// *_test rules, *_binary rules, aliases, generated outputs, and inputs.
///
/// Determining definitively whether a rule produces an executable can only be done after
/// analysis. This is only an early check to quickly catch most mistakes.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1176 RunCommand.isExecutable
fn is_executable(target: &Target) -> bool {
    is_plain_file(target)
        || is_executable_non_test_rule(target)
        || target_utils::is_test_rule(target)
        || AliasProvider::may_be_alias(target)
}

/// Return true iff `target` is a rule that generates an executable file and is user-executed
/// code.
// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1187 RunCommand.isExecutableNonTestRule
fn is_executable_non_test_rule(target: &Target) -> bool {
    match target.as_rule() {
        None => false,
        Some(rule) => rule.is_executable(),
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1194 RunCommand.isPlainFile
fn is_plain_file(target: &Target) -> bool {
    matches!(target, Target::OutputFile(_) | Target::InputFile(_))
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1198 RunCommand.makeErrorMessageForNotHavingASingleTarget
fn make_error_message_for_not_having_a_single_target(
    target_pattern_string: &str,
    expanded_target_names: &[String],
) -> String {
    const MAX_NUM_EXPANDED_TARGETS_TO_INCLUDE_IN_ERROR_MESSAGE: usize = 5;
    let truncate_target_name_list = expanded_target_names.len() > 5;
    let target_names_to_include_in_error_message: &[String] = if truncate_target_name_list {
        &expanded_target_names[..MAX_NUM_EXPANDED_TARGETS_TO_INCLUDE_IN_ERROR_MESSAGE]
    } else {
        expanded_target_names
    };
    // ImmutableSortedSet.copyOf: natural (String) order, duplicates dropped.
    let sorted: BTreeSet<&String> = target_names_to_include_in_error_message.iter().collect();
    format!(
        "Only a single target can be run. Your target pattern {} expanded to the targets {}{}",
        target_pattern_string,
        sorted
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        if truncate_target_name_list {
            "[TRUNCATED]"
        } else {
            ""
        }
    )
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommand.java:1214 RunCommand.createFailureDetail
fn create_failure_detail(message: &str, detailed_code: Code) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::RunCommand(failure_details::RunCommand {
            code: detailed_code as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(message: &str) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_owned(),
        category: Some(Category::Interrupted(failure_details::Interrupted {
            code: failure_details::interrupted::Code::Interrupted as i32,
        })),
    })
}

// -------------------------------------------------------------------------------------------
// RunCommandLine

/// Encapsulates information for launching the command specified by a run invocation.
///
/// Notably, this class handles per-platform command-line formatting (windows vs unix).
// Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:37 RunCommandLine
#[derive(Debug)]
struct RunCommandLine {
    args: Vec<String>,
    pretty_args: Vec<String>,
    residue: Vec<String>,
    run_under_prefix: Option<String>,
    pretty_run_under_prefix: Option<String>,
    run_environment: BTreeMap<String, String>,
    environment_variables_to_clear: BTreeSet<String>,
    working_dir: Path,
    is_test_target: bool,
}

impl RunCommandLine {
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:70 RunCommandLine.getWorkingDir
    fn get_working_dir(&self) -> &Path {
        &self.working_dir
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:74 RunCommandLine.getEnvironment
    fn get_environment(&self) -> &BTreeMap<String, String> {
        &self.run_environment
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:78 RunCommandLine.getEnvironmentVariablesToClear
    fn get_environment_variables_to_clear(&self) -> &BTreeSet<String> {
        &self.environment_variables_to_clear
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:82 RunCommandLine.isTestTarget
    fn is_test_target(&self) -> bool {
        self.is_test_target
    }

    /// Returns a console-friendly (including relative paths) representation of the command line.
    ///
    /// Arguments from the `run` command line are omitted as to avoid possibly leaking sensitive
    /// user-provided information in logging, BEP, etc.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:92 RunCommandLine.getPrettyArgs
    fn get_pretty_args(&self, run_omit_run_args: bool) -> String {
        let mut result = String::new();
        if let Some(prefix) = &self.pretty_run_under_prefix {
            result.push_str(prefix);
            result.push(' ');
        }
        for (i, arg) in self.pretty_args.iter().enumerate() {
            if i > 0 {
                result.push(' ');
            }
            result.push_str(&ShellEscaper::escape_string(arg));
        }
        if !self.residue.is_empty() {
            if run_omit_run_args {
                result.push_str(" <args omitted>");
            } else {
                for arg in &self.residue {
                    // `if (i < residue.size())`: always true.
                    result.push(' ');
                    result.push_str(&ShellEscaper::escape_string(arg));
                }
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:119 RunCommandLine.requiresShExecutable
    fn requires_sh_executable(&self) -> bool {
        // OS.getCurrent() != OS.WINDOWS || runUnderPrefix != null
        true
    }

    /// Returns the command arguments including residue.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:124 RunCommandLine.getArgs
    fn get_args(&self, sh_executable: Option<&str>) -> Vec<String> {
        let mut all = self.args.clone();
        all.extend(self.residue.iter().cloned());
        LinuxFormatter.format_argv(sh_executable, self.run_under_prefix.as_deref(), &all)
    }

    /// Returns the command arguments without residue (extra arguments from the run invocation's
    /// command line). This is intended to be used in places where we don't want to include the
    /// residue in case it contains sensitive information.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:137 RunCommandLine.getArgsWithoutResidue
    fn get_args_without_residue(&self, sh_executable: Option<&str>) -> Vec<String> {
        LinuxFormatter.format_argv(sh_executable, self.run_under_prefix.as_deref(), &self.args)
    }

    /// Returns the script form of the command, to be used as the contents of output file in
    /// --script_path mode.
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:145 RunCommandLine.getScriptForm
    fn get_script_form(&self, sh_executable: &str) -> String {
        let mut all = self.args.clone();
        all.extend(self.residue.iter().cloned());
        LinuxFormatter.get_script_form(
            sh_executable,
            self.working_dir.get_path_string(),
            &self.environment_variables_to_clear,
            &self.run_environment,
            self.run_under_prefix.as_deref(),
            &all,
        )
    }
    // PORT: formatter() returns the LinuxFormatter; the WindowsFormatter is skipped, Windows only.
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:174 RunCommandLine.LinuxFormatter
#[derive(Debug, Clone, Copy)]
struct LinuxFormatter;

impl LinuxFormatter {
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:176 RunCommandLine.LinuxFormatter.formatArgv
    fn format_argv(
        &self,
        sh_executable: Option<&str>,
        run_under_prefix: Option<&str>,
        args: &[String],
    ) -> Vec<String> {
        check_argument!(sh_executable.is_some(), "shExecutable must be non-null");
        let mut command = String::new();
        if let Some(prefix) = run_under_prefix {
            command.push_str(prefix);
            command.push(' ');
        }
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                command.push(' ');
            }
            command.push_str(&ShellEscaper::escape_string(arg));
        }
        vec![
            sh_executable.expect("checked").to_owned(),
            "-c".to_owned(),
            command,
        ]
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:195 RunCommandLine.LinuxFormatter.getScriptForm
    fn get_script_form(
        &self,
        sh_executable: &str,
        working_dir: &str,
        environment_vars_to_unset: &BTreeSet<String>,
        environment: &BTreeMap<String, String>,
        run_under_prefix: Option<&str>,
        args: &[String],
    ) -> String {
        let unset_env = environment_vars_to_unset
            .iter()
            .map(|v| format!("-u {v}"))
            .collect::<Vec<_>>()
            .join(" \\\n    ");
        let set_env = environment
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    ShellEscaper::escape_string(k),
                    ShellEscaper::escape_string(v)
                )
            })
            .collect::<Vec<_>>()
            .join(" \\\n    ");
        let command_line = Self::get_command_line(sh_executable, run_under_prefix, args);

        let mut result = String::new();
        result.push_str("#!");
        result.push_str(sh_executable);
        result.push('\n');
        result.push_str("cd ");
        result.push_str(&ShellEscaper::escape_string(working_dir));
        result.push_str(" && \\\n");
        result.push_str("  exec env \\\n");
        result.push_str("    ");
        result.push_str(&unset_env);
        result.push_str(" \\\n");
        result.push_str("    ");
        result.push_str(&set_env);
        result.push_str(" \\\n");
        result.push_str("  ");
        result.push_str(&command_line);
        result.push_str(" \"$@\"");
        result
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:226 RunCommandLine.LinuxFormatter.getCommandLine
    fn get_command_line(
        sh_executable: &str,
        run_under_prefix: Option<&str>,
        args: &[String],
    ) -> String {
        let mut command = String::new();
        if let Some(prefix) = run_under_prefix {
            command.push_str(prefix);
            command.push(' ');
        }
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                command.push(' ');
            }
            command.push_str(&ShellEscaper::escape_string(arg));
        }

        match run_under_prefix {
            None => command,
            Some(_) => format!(
                "{} -c {}",
                sh_executable,
                ShellEscaper::escape_string(&command)
            ),
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:316 RunCommandLine.Builder
struct RunCommandLineBuilder {
    run_environment: BTreeMap<String, String>,
    environment_variables_to_clear: BTreeSet<String>,
    working_dir: Path,
    is_test_target: bool,
    run_under_prefix: Option<String>,
    pretty_run_under_prefix: Option<String>,
    args: Vec<String>,
    pretty_print_args: Vec<String>,
    residue_args: Vec<String>,
}

impl RunCommandLineBuilder {
    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:330 RunCommandLine.Builder.<init>
    fn new(
        run_environment: BTreeMap<String, String>,
        environment_variables_to_clear: BTreeSet<String>,
        working_dir: Path,
        is_test_target: bool,
    ) -> RunCommandLineBuilder {
        RunCommandLineBuilder {
            run_environment,
            environment_variables_to_clear,
            working_dir,
            is_test_target,
            run_under_prefix: None,
            pretty_run_under_prefix: None,
            args: Vec::new(),
            pretty_print_args: Vec::new(),
            residue_args: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:342 RunCommandLine.Builder.setRunUnderPrefix
    fn set_run_under_prefix(&mut self, run_under_prefix: &str) -> &mut Self {
        self.run_under_prefix = Some(run_under_prefix.to_owned());
        self.pretty_run_under_prefix = Some(run_under_prefix.to_owned());
        self
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:349 RunCommandLine.Builder.setRunUnderTarget
    fn set_run_under_target(
        &mut self,
        run_under_binary: &Path,
        args: &[String],
        path_pretty_printer: &PathPrettyPrinter,
    ) -> &mut Self {
        let mut run_under = String::new();
        let mut pretty_run_under = String::new();
        run_under.push_str(&ShellEscaper::escape_string(
            run_under_binary.get_path_string(),
        ));
        pretty_run_under.push_str(&ShellEscaper::escape_string(
            path_pretty_printer
                .get_pretty_path(run_under_binary.as_fragment())
                .get_path_string(),
        ));
        for arg in args {
            let escaped_arg = ShellEscaper::escape_string(arg);
            run_under.push(' ');
            run_under.push_str(&escaped_arg);
            pretty_run_under.push(' ');
            pretty_run_under.push_str(&escaped_arg);
        }
        self.run_under_prefix = Some(run_under);
        self.pretty_run_under_prefix = Some(pretty_run_under);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:368 RunCommandLine.Builder.addArg
    fn add_arg(&mut self, arg: String) -> &mut Self {
        self.add_arg_internal(arg.clone(), arg)
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:373 RunCommandLine.Builder.addArg
    fn add_arg_path(&mut self, path: &Path, path_pretty_printer: &PathPrettyPrinter) -> &mut Self {
        self.add_arg_internal(
            path.get_path_string().to_owned(),
            path_pretty_printer
                .get_pretty_path(path.as_fragment())
                .get_path_string()
                .to_owned(),
        )
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:379 RunCommandLine.Builder.addArgs
    fn add_args(&mut self, args: Vec<String>) -> &mut Self {
        for arg in args {
            self.add_arg(arg);
        }
        self
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:387 RunCommandLine.Builder.addArgsFromResidue
    fn add_args_from_residue(&mut self, args: Vec<String>) -> &mut Self {
        self.residue_args.extend(args);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:393 RunCommandLine.Builder.addArgInternal
    fn add_arg_internal(&mut self, arg: String, pretty_print_arg: String) -> &mut Self {
        self.args.push(arg);
        self.pretty_print_args.push(pretty_print_arg);
        self
    }

    // Java: com/google/devtools/build/lib/runtime/commands/RunCommandLine.java:399 RunCommandLine.Builder.build
    fn build(self) -> RunCommandLine {
        RunCommandLine {
            args: self.args,
            pretty_args: self.pretty_print_args,
            residue: self.residue_args,
            run_under_prefix: self.run_under_prefix,
            pretty_run_under_prefix: self.pretty_run_under_prefix,
            run_environment: self.run_environment,
            environment_variables_to_clear: self.environment_variables_to_clear,
            working_dir: self.working_dir,
            is_test_target: self.is_test_target,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Port of `RunCommandLineTest` (src/test/java/com/google/devtools/build/lib/runtime/commands).

    use super::*;

    fn list(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn sorted_set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    fn sorted_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn linux_formatter_format_argv_requires_sh_executable() {
        LinuxFormatter.format_argv(
            /* sh_executable= */ None,
            Some("run under prefix"),
            &list(&["argv"]),
        );
    }

    #[test]
    #[ignore = "WindowsFormatter: PORT skipped, Windows only"]
    fn windows_formatter_format_argv_run_under_requires_sh_executable() {}

    #[test]
    fn linux_formatter_format_argv_returns_escaped_command_line() {
        let result = LinuxFormatter.format_argv(
            Some("/bin/bash"),
            /* run_under_prefix= */ None,
            &list(&["executable", "argv1", "arg w spaces"]),
        );
        assert_eq!(
            result,
            list(&["/bin/bash", "-c", "executable argv1 'arg w spaces'"])
        );
    }

    #[test]
    #[ignore = "WindowsFormatter: PORT skipped, Windows only"]
    fn windows_formatter_format_argv_returns_escaped_command_line() {}

    #[test]
    fn linux_formatter_format_argv_run_under_prefix_prepended_to_escaped_command_line() {
        let result = LinuxFormatter.format_argv(
            Some("/bin/bash"),
            Some("unescaped run-under prefix &&"),
            &list(&["executable", "argv1", "arg w spaces"]),
        );
        assert_eq!(
            result,
            list(&[
                "/bin/bash",
                "-c",
                "unescaped run-under prefix && executable argv1 'arg w spaces'"
            ])
        );
    }

    #[test]
    #[ignore = "WindowsFormatter: PORT skipped, Windows only"]
    fn windows_formatter_format_argv_run_under_prefix_prepended_to_escaped_command_line() {}

    #[test]
    fn linux_formatter_format_script_path_command_line_returns_concatenated_escaped_command() {
        let result = LinuxFormatter.get_script_form(
            "/bin/bash",
            "workingDir",
            &sorted_set(&["UNSET_ME", "UNSET_ME_TOO"]),
            &sorted_map(&[("ENV_VAR", "val"), ("ENV_VAR_WITH_SPACES", "foo bar")]),
            /* run_under_prefix= */ None,
            &list(&["executable", "argv1", "arg w spaces"]),
        );
        assert_eq!(
            result,
            "#!/bin/bash\n\
             cd workingDir && \\\n\
             \x20 exec env \\\n\
             \x20   -u UNSET_ME \\\n\
             \x20   -u UNSET_ME_TOO \\\n\
             \x20   ENV_VAR=val \\\n\
             \x20   ENV_VAR_WITH_SPACES='foo bar' \\\n\
             \x20 executable argv1 'arg w spaces' \"$@\""
        );
    }

    #[test]
    fn linux_formatter_format_script_path_command_line_run_under_prefix_prepended_to_escaped_command()
     {
        let result = LinuxFormatter.get_script_form(
            "/bin/bash",
            "workingDir",
            &sorted_set(&["UNSET_ME", "UNSET_ME_TOO"]),
            &sorted_map(&[("ENV_VAR", "val"), ("ENV_VAR_WITH_SPACES", "foo bar")]),
            Some("unescaped run-under prefix &&"),
            &list(&["executable", "argv1", "arg w spaces"]),
        );
        assert_eq!(
            result,
            "#!/bin/bash\n\
             cd workingDir && \\\n\
             \x20 exec env \\\n\
             \x20   -u UNSET_ME \\\n\
             \x20   -u UNSET_ME_TOO \\\n\
             \x20   ENV_VAR=val \\\n\
             \x20   ENV_VAR_WITH_SPACES='foo bar' \\\n\
             \x20 /bin/bash -c 'unescaped run-under prefix && executable argv1 '\\''arg w spaces'\\''' \"$@\""
        );
    }

    #[test]
    #[ignore = "WindowsFormatter: PORT skipped, Windows only"]
    fn windows_formatter_format_script_path_command_line_run_under_prefix_prepended_to_escaped_command()
     {
    }

    #[test]
    #[ignore = "WindowsFormatter: PORT skipped, Windows only"]
    fn windows_formatter_format_script_path_command_line_returns_concatenated_escaped_command() {}

    // Not a Java test: RunCommand.makeErrorMessageForNotHavingASingleTarget (sorted, truncated
    // after five names), checked against the Java expression.
    #[test]
    fn error_message_for_not_having_a_single_target_sorts_and_truncates() {
        assert_eq!(
            make_error_message_for_not_having_a_single_target("//...", &list(&["//:b", "//:a"]),),
            "Only a single target can be run. Your target pattern //... expanded to the targets \
             //:a, //:b"
        );
        assert_eq!(
            make_error_message_for_not_having_a_single_target(
                "//...",
                &list(&["//:f", "//:e", "//:d", "//:c", "//:b", "//:a"]),
            ),
            "Only a single target can be run. Your target pattern //... expanded to the targets \
             //:b, //:c, //:d, //:e, //:f[TRUNCATED]"
        );
    }
}
