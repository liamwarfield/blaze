//! `blaze cquery`: `runtime/commands/CqueryCommand.java` (the body is in
//! `blaze_cquery::commands::cquery_command`).

use blaze_cquery::commands::PostAnalysisCommandEnvironment;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::CQUERY_COMMAND;
use crate::commands::query::{
    build_options_with_module_flag_aliases, create_skyframe_executor, query_output_stream,
    reporter, to_runtime_detailed_exit_code,
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
        let build_options = match build_options_with_module_flag_aliases(env, options) {
            Ok(build_options) => build_options,
            Err(result) => return result,
        };
        let skyframe_executor = match create_skyframe_executor(env, Some(build_options)) {
            Ok(e) => e,
            Err(result) => return result,
        };
        let command_env = PostAnalysisCommandEnvironment {
            skyframe_executor: &skyframe_executor,
            reporter: reporter(env),
            output: query_output_stream(env),
            relative_working_directory: env.get_relative_working_directory().clone(),
            working_directory: env.get_working_directory().clone(),
            product_name: env.get_runtime().get_product_name().to_owned(),
            options,
        };
        let detailed_exit_code = blaze_cquery::commands::cquery_command::exec(&command_env);
        BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(detailed_exit_code))
    }
}
