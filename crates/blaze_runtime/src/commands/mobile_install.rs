//! `blaze mobile-install`: builds an `android_binary` with the mobile-install aspect and runs
//! the launcher the aspect produces.
//!
//! Port of `runtime/mobileinstall/MobileInstallCommand.java` (its options are
//! `blaze_optdefs::command::mobile_install_command` and `write_adb_args_action`). The Starlark
//! side (the aspect, `@rules_android//mobile_install:mi.bzl`, and the launcher it writes) is
//! rules_android's; this command only applies the aspect, checks the target and runs
//! `<bin>/<package>/<name>_mi/launcher` with the adb and test flags.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_events::{Event, EventHandler as _};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::mobile_install_command::MobileInstallCommandOptions;
use blaze_optdefs::command::write_adb_args_action::WriteAdbArgsActionOptions;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_protos::command_server::{EnvironmentVariable, ExecRequest};
use blaze_protos::failure_details::{
    FailureDetail, MobileInstall, failure_detail::Category, mobile_install::Code,
};
use blaze_shell::CommandExecutionError;
use blaze_shell::util::command_builder::CommandBuilder;
use blaze_vfs::Path;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::commands::build::{PostBuildContext, RunBuild, RunBuildOutcome, run_build};
use crate::commands::fetch::NoTargetValidator;
use crate::options_result::OptionsParsingResult;

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:166 MobileInstallCommand.SINGLE_TARGET_MESSAGE
const SINGLE_TARGET_MESSAGE: &str =
    "Can only run a single target. Do not use wildcards that match more than one target";
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:168 MobileInstallCommand.NO_TARGET_MESSAGE
const NO_TARGET_MESSAGE: &str = "No targets found to run";

/// Implementation of the 'mobile-install' command.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:79 MobileInstallCommand
pub struct MobileInstallCommand;

impl BlazeCommand for MobileInstallCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::MOBILE_INSTALL_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:375 MobileInstallCommand.editOptions
    fn edit_options(&self, options_parser: &mut blaze_options::OptionsParser) {
        let options: MobileInstallCommandOptions = options_parser
            .get_options()
            .expect("NullPointerException: MobileInstallCommand.Options");
        // OutputGroupInfo.INTERNAL_SUFFIX
        let internal_suffix = blaze_analysis::providers::output_group_info::INTERNAL_SUFFIX;
        let result = options_parser.parse_with_priority(
            blaze_options::PriorityCategory::COMMAND_LINE,
            Some("Options required by the Starlark implementation of mobile-install command"),
            vec![
                format!("--aspects={}%MIASPECT", options.mobile_install_aspect),
                format!("--output_groups=mobile_install{internal_suffix}"),
                format!("--output_groups=mobile_install_launcher{internal_suffix}"),
            ],
        );
        if let Err(e) = result {
            panic!("IllegalStateException: {}", e.get_message());
        }
    }

    // Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:172 MobileInstallCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // This list should look like: ["//executable:target", "arg1", "arg2"]
        let target_and_args: Vec<String> = options.get_residue();

        // The user must at least specify an executable target.
        if target_and_args.is_empty() {
            let message = "Must specify a target to run";
            env.get_reporter().handle(&Event::error(message));
            return BlazeCommandResult::failure_detail(create_failure_result(
                message,
                Code::NoTargetSpecified,
            ));
        }

        let targets = vec![target_and_args[0].clone()];
        let run_target_args: Vec<String> = target_and_args[1..].to_vec();

        // AtomicReference<ExecRequest> deployerRequestRef
        let deployer_request_ref: Rc<RefCell<Option<ExecRequest>>> = Rc::new(RefCell::new(None));
        let callback_request_ref = deployer_request_ref.clone();
        let validator = NoTargetValidator;
        run_build(
            env,
            options,
            /* run_tests= */ false,
            Some(RunBuild {
                targets,
                // processRequest(request, /* validator= */ null, ...)
                validator: &validator,
                repo_precomputed: Default::default(),
                post_build_callback: Some(Box::new(move |context: PostBuildContext<'_>| {
                    do_mobile_install(context, options, &run_target_args, &callback_request_ref)
                })),
                after_build: Box::new(move |outcome: RunBuildOutcome<'_>| {
                    if !outcome.build_result.get_success() {
                        outcome.env.get_reporter().handle(&Event::error(
                            "Build failed. Not running mobile-install on target.",
                        ));
                        return BlazeCommandResult::detailed_exit_code(outcome.detailed_exit_code);
                    }

                    match outcome
                        .build_result
                        .get_post_build_call_back_failure_detail()
                    {
                        None => match deployer_request_ref.borrow_mut().take() {
                            None => BlazeCommandResult::success(),
                            Some(request) => BlazeCommandResult::execute(request),
                        },
                        Some(failure_detail) => {
                            BlazeCommandResult::failure_detail(failure_detail.clone())
                        }
                    }
                }),
            }),
        )
    }
}

/// The post-build callback: checks the target and runs (or hands to the client) the launcher.
/// Returns `None` in case of success.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:228 MobileInstallCommand.doMobileInstall
fn do_mobile_install(
    context: PostBuildContext<'_>,
    options: &OptionsParsingResult,
    run_target_args: &[String],
    deployer_request_ref: &RefCell<Option<ExecRequest>>,
) -> Option<FailureDetail> {
    let PostBuildContext {
        env,
        successful_targets,
        skyframe_executor,
    } = context;
    let Some(successful_targets) = successful_targets else {
        env.get_reporter().handle(&Event::warn(NO_TARGET_MESSAGE));
        return None;
    };
    if successful_targets.len() != 1 {
        env.get_reporter()
            .handle(&Event::error(SINGLE_TARGET_MESSAGE));
        return Some(create_failure_result(
            SINGLE_TARGET_MESSAGE,
            Code::MultipleTargetsSpecified,
        ));
    }
    let target_to_run = &successful_targets[0];
    let mobile_install_options: MobileInstallCommandOptions = options
        .get_options()
        .expect("NullPointerException: MobileInstallCommand.Options");
    let adb_options: WriteAdbArgsActionOptions = options
        .get_options()
        .expect("NullPointerException: WriteAdbArgsAction.Options");

    if !mobile_install_options
        .mobile_install_supported_rules
        .is_empty()
        && let Some(message) = error_message_if_not_supported(
            target_to_run,
            &mobile_install_options.mobile_install_supported_rules,
        )
    {
        env.get_reporter().handle(&Event::error(message.clone()));
        return Some(create_failure_result(&message, Code::TargetTypeInvalid));
    }

    let mut cmd_line: Vec<String> = Vec::new();
    // TODO(bazel-team): Get the executable path from the filesToRun provider from the aspect.
    let configuration = skyframe_executor
        .get_configuration_for_key(
            env.get_reporter().clone(),
            &target_to_run
                .get_configuration_key()
                .expect("NullPointerException: the target's configuration key"),
        )
        .expect("the configuration of a built target");
    let label = target_to_run.get_label();
    cmd_line.push(format!(
        "{}/{}_mi/launcher",
        configuration
            .get_bin_fragment(label.get_repository())
            .get_path_string(),
        label.to_path_fragment().get_path_string()
    ));
    cmd_line.extend(run_target_args.iter().cloned());

    cmd_line.push(format!("--build_id={}", env.get_command_id()));

    // Collect relevant common command options.
    let common_command_options: CommonCommandOptions = options
        .get_options()
        .expect("NullPointerException: CommonCommandOptions");
    if !common_command_options.tool_tag.is_empty() {
        cmd_line.push(format!("--tool_tag={}", common_command_options.tool_tag));
    }

    // Collect relevant adb options.
    cmd_line.push(format!("--start={}", adb_options.start));
    if !adb_options.adb.is_empty() {
        cmd_line.push(format!("--adb={}", adb_options.adb));
    }
    for adb_arg in &adb_options.adb_args {
        if !adb_arg.is_empty() {
            cmd_line.push(format!("--adb_arg={adb_arg}"));
        }
    }
    if !adb_options.device.is_empty() {
        cmd_line.push(format!("--device={}", adb_options.device));
    }

    // Collect relevant test options.
    let test_options: TestConfigurationTestOptions = options
        .get_options()
        .expect("NullPointerException: TestConfiguration.TestOptions");
    // Default value of testFilter is null.
    if let Some(test_filter) = &test_options.test_filter
        && !test_filter.is_empty()
    {
        cmd_line.push(format!("--test_filter={test_filter}"));
    }
    for arg in &test_options.test_arguments {
        if !arg.is_empty() {
            cmd_line.push(format!("--test_arg={arg}"));
        }
    }

    let working_dir = env
        .get_directories()
        .get_output_path(env.get_workspace_name())
        .get_parent_directory()
        .expect("the output path has a parent");

    if mobile_install_options.run_in_client {
        *deployer_request_ref.borrow_mut() =
            Some(create_exec_request(env, &working_dir, &cmd_line));
        None
    } else {
        execute_as_child(env, &working_dir, &cmd_line)
    }
}

/// An `OutputStream` of the command's `OutErr` as the `java.io.OutputStream` the child's
/// output is copied to.
struct OutErrStream(Arc<dyn blaze_events::io::OutputStream>);

impl std::io::Write for OutErrStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Executes the mobile-install deployer as a child process on this machine.
///
/// PORT: Java wraps the run in `GoogleAutoProfilerUtils.profiledAndLogged("mobile install",
/// ProfilerTask.INFO)` (a profiler task and a server-log line); not written.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:314 MobileInstallCommand.executeAsChild
fn execute_as_child(
    env: &CommandEnvironment,
    working_dir: &Path,
    cmd_line: &[String],
) -> Option<FailureDetail> {
    let mut builder = CommandBuilder::new(env.get_client_env());
    builder
        .add_args(cmd_line)
        .set_env(env.get_client_env())
        .set_working_dir(working_dir);
    let command = builder.build();

    // Restore a raw EventHandler if it is registered. This allows for blaze run to produce the
    // actual output of the command being run even if --color=no is specified.
    env.get_reporter().switch_to_ansi_allowing_handler();

    let out_err = env.get_reporter().get_out_err();
    let std_out: blaze_shell::OutputStream = Arc::new(parking_lot::Mutex::new(OutErrStream(
        out_err.get_output_stream().clone(),
    )));
    let std_err: blaze_shell::OutputStream = Arc::new(parking_lot::Mutex::new(OutErrStream(
        out_err.get_error_stream().clone(),
    )));
    // The command API is a little strange in that the following statement will return normally
    // only if the program exits with exit code 0. If it ends with any other code, we have to
    // catch BadExitStatusException.
    match command.execute_streaming(std_out, std_err) {
        Ok(_) => None,
        Err(CommandExecutionError::Command(blaze_shell::CommandException::BadExitStatus(e))) => {
            let message = format!(
                "Non-zero return code '{}' from command: {}",
                e.get_result().termination_status().get_exit_code(),
                e.get_message()
            );
            env.get_reporter().handle(&Event::error(message.clone()));
            Some(create_failure_result(&message, Code::NonZeroExit))
        }
        Err(CommandExecutionError::Command(e)) => {
            let message = format!("Error running program: {}", e.get_message());
            env.get_reporter().handle(&Event::error(message.clone()));
            Some(create_failure_result(&message, Code::ErrorRunningProgram))
        }
        // `throws InterruptedException`: doMobileInstall's caller reports the interruption.
        Err(CommandExecutionError::Interrupted(_)) => {
            blaze_util::unported!("MobileInstallCommand.executeAsChild (interrupted)")
        }
    }
}

/// `PathToReplaceUtils.bytes`: ISO-8859-1.
fn bytes(string: &str) -> Vec<u8> {
    blaze_util::string_encoding::latin1_encode_lossy(string).into_owned()
}

/// Returns an `ExecRequest` for running the mobile-install deployer in the client.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:356 MobileInstallCommand.createExecRequest
fn create_exec_request(
    env: &CommandEnvironment,
    working_dir: &Path,
    cmd_line: &[String],
) -> ExecRequest {
    ExecRequest {
        should_exec: true,
        working_directory: bytes(working_dir.get_path_string()),
        argv: cmd_line.iter().map(|a| bytes(a)).collect(),
        path_to_replace: crate::commands::run::path_to_replace_utils_get_paths_to_replace(env),
        // TODO: b/333695932 - Shim for client run-support, remove once no longer needed.
        environment_variable: vec![
            EnvironmentVariable {
                name: bytes("BUILD_WORKING_DIRECTORY"),
                value: bytes(env.get_working_directory().get_path_string()),
            },
            EnvironmentVariable {
                name: bytes("BUILD_WORKSPACE_DIRECTORY"),
                value: bytes(
                    env.get_workspace()
                        .expect("NullPointerException: the workspace")
                        .get_path_string(),
                ),
            },
        ],
        ..Default::default()
    }
}

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:391 MobileInstallCommand.errorMessageIfNotSupported
fn error_message_if_not_supported(
    target: &ConfiguredTarget,
    mobile_install_supported_rules: &[String],
) -> Option<String> {
    // Dereference any aliases that might be present.
    let target = target.get_actual();

    // `target instanceof AbstractConfiguredTarget`: every configured target class but
    // MergedConfiguredTarget (and the alias, dereferenced above).
    if !matches!(target, ConfiguredTarget::Merged(_)) {
        let rule_type = blaze_analysis::configuredtargets::get_rule_class_string(&target);
        if !mobile_install_supported_rules.contains(&rule_type) {
            // String.format("%s", List): `[a, b]`.
            return Some(format!(
                "mobile-install can only be run on [{}] targets. Got: {}",
                mobile_install_supported_rules.join(", "),
                rule_type
            ));
        } else {
            return None;
        }
    }
    Some("Invalid target".to_owned())
}

// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallCommand.java:410 MobileInstallCommand.createFailureResult
fn create_failure_result(message: &str, detailed_code: Code) -> FailureDetail {
    FailureDetail {
        message: message.to_owned(),
        category: Some(Category::MobileInstall(MobileInstall {
            code: detailed_code as i32,
        })),
    }
}
