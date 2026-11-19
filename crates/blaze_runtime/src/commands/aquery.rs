//! `blaze aquery`: `runtime/commands/AqueryCommand.java` (the body is in
//! `blaze_cquery::commands::aquery_command`).

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::top_level::create_build_options;
use blaze_cquery::commands::PostAnalysisCommandEnvironment;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::AQUERY_COMMAND;
use crate::commands::cquery::construct_execution_tool;
use crate::commands::query::{
    AfterCommand, create_skyframe_executor, query_output_stream, reporter,
    to_runtime_detailed_exit_code,
};
use crate::options_result::OptionsParsingResult;

/// Handles the 'aquery' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:66 AqueryCommand
pub struct AqueryCommand;

impl BlazeCommand for AqueryCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &AQUERY_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:69 AqueryCommand.editOptions
    fn edit_options(&self, options_parser: &mut blaze_options::OptionsParser) {
        blaze_cquery::commands::aquery_command::edit_options(options_parser);
    }

    // Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:81 AqueryCommand.exec
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
        let execution_tool = || construct_execution_tool(env, options);
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
                crate::commands::build::stop_request_without_execution(env, detailed_exit_code)
            },
            options,
            execution_tool: &execution_tool,
        };
        let detailed_exit_code = blaze_cquery::commands::aquery_command::exec(&command_env);
        BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(detailed_exit_code))
    }
}
