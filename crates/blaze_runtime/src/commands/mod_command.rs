//! `blaze mod`: `bazel/commands/ModCommand.java` (`exec` / `execInternal`). The subcommand logic
//! (argument validation, `ModExecutor` and the output formatters) is
//! `blaze_bzlmod::modcommand::mod_command`.

use blaze_bzlmod::modcommand::mod_command::{
    ModCommandResult, exec_after_evaluation, exec_before_evaluation, report_evaluation_error,
};
use blaze_bzlmod::skyframe::{BazelDepGraphKey, BazelModuleInspectorKey};
use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::misc::mod_options::ModOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_skyframe::SkyKey;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::MOD_COMMAND;
use crate::commands::query::{AfterCommand, create_skyframe_executor, reporter};
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

/// Queries the Bzlmod external dependency graph.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:119 ModCommand
pub struct ModCommand;

impl BlazeCommand for ModCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &MOD_COMMAND
    }

    // Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:200 ModCommand.execInternal
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        let reporter = reporter(env);
        let mod_options: ModOptions = options.get_options().expect("ModOptions");
        let residue = options.get_residue();
        let request = match exec_before_evaluation(&residue, &mod_options, reporter.as_ref()) {
            Ok(r) => r,
            Err(r) => return to_command_result(r),
        };
        if !request.repo_mapping_repo_names.is_empty() {
            // PORT: dump_repo_mapping evaluates RepositoryMappingValue keys; the output of that
            // subcommand is not ported (blaze_bzlmod exec_after_evaluation).
            blaze_util::unported!("ModCommand.execInternal (dump_repo_mapping)");
        }
        let threads = options
            .get_options::<LoadingPhaseThreadsOption>()
            .map(|o| o.threads.max(1) as usize)
            .unwrap_or(blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT);

        // env.syncPackageLoading(options)
        let skyframe_executor = match create_skyframe_executor(env, None) {
            Ok(e) => e,
            Err(result) => return result,
        };
        let _after_command = AfterCommand(&skyframe_executor);
        // PORT: TIDY evaluates BazelModTidyValue (not ported); exec_after_evaluation fails loudly.
        let keys = [
            SkyKey::new(BazelDepGraphKey),
            SkyKey::new(BazelModuleInspectorKey),
        ];
        let evaluation_result = match skyframe_executor.evaluate(
            &keys,
            /* keep_going= */ true,
            threads,
            reporter.clone(),
        ) {
            Ok(r) => r,
            Err(_) => {
                let error_message = "mod command interrupted: null";
                env.get_reporter().handle(&Event::error(error_message));
                return BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
                    crate::exit_code::ExitCode::INTERRUPTED,
                    FailureDetail {
                        message: error_message.to_owned(),
                        category: Some(Category::Interrupted(failure_details::Interrupted {
                            code: failure_details::interrupted::Code::Interrupted as i32,
                        })),
                    },
                ));
            }
        };
        if evaluation_result.has_error() {
            // PORT: the CyclesReporter (BzlmodRepoCycleReporter, BzlLoadCycleReporter) is not
            // ported; cycles report the generic message.
            let message = evaluation_result
                .get_first_error()
                .and_then(|e| e.get_exception())
                .map(|e| e.to_string());
            return to_command_result(report_evaluation_error(
                reporter.as_ref(),
                message.as_deref(),
            ));
        }
        let dep_graph = evaluation_result
            .get(&BazelDepGraphKey)
            .expect("BazelDepGraphValue without error");
        let inspector = evaluation_result
            .get(&BazelModuleInspectorKey)
            .expect("BazelModuleInspectorValue without error");
        let out_err = env.get_reporter_out_err().clone();
        let mut out = crate::commands::query::ReporterStdout(out_err.get_output_stream().clone());
        let result = exec_after_evaluation(
            request.subcommand,
            &request.args,
            &mod_options,
            &dep_graph,
            &inspector,
            reporter.as_ref(),
            &mut out,
        );
        let _ = std::io::Write::flush(&mut out);
        to_command_result(result)
    }
}

/// `ModCommand.reportAndCreateFailureResult`'s `BlazeCommandResult` (the message was reported).
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:806 ModCommand.createFailureResult
fn to_command_result(result: ModCommandResult) -> BlazeCommandResult {
    match result {
        ModCommandResult::Success => BlazeCommandResult::success(),
        ModCommandResult::Failure { message, code, .. } => {
            BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
                message,
                category: Some(Category::ModCommand(failure_details::ModCommand {
                    code: failure_details::mod_command::Code::from_str_name(code)
                        .unwrap_or(failure_details::mod_command::Code::ModCommandUnknown)
                        as i32,
                })),
            }))
        }
    }
}
