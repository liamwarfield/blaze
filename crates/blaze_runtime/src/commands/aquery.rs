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
        // BuildTool.buildTargets: the project file's options go through the command's parser
        // (as for `build`, crate::commands::build).
        let config_flag_definitions = env.get_config_flag_definitions().clone();
        let project_options_applier: std::sync::Arc<
            dyn blaze_sky_top::build_tool::ProjectOptionsApplier,
        > = {
            let mut applier = crate::project_options::BuildProjectOptionsApplier::new(
                std::sync::Arc::new(options.clone()),
                env.get_relative_working_directory().clone(),
                reporter(env),
            );
            if let Some(bep) = env.get_bep() {
                let bep = bep.clone();
                let product_name = env.get_runtime().get_product_name().to_string();
                let startup_options = env.get_runtime().get_startup_options_provider_arc();
                applier = applier.with_canonical_command_line_poster(Box::new(move |parser| {
                    bep.post_project_canonical_command_line(
                        &product_name,
                        startup_options.clone(),
                        "aquery",
                        parser,
                    )
                }));
            }
            std::sync::Arc::new(applier)
        };
        let configure_request = move |request: &mut blaze_sky_top::build_tool::BuildRequest| {
            request.project_options_context.config_flag_definitions =
                config_flag_definitions.clone();
            request.project_options_applier = Some(project_options_applier.clone());
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
            configure_request: &configure_request,
        };
        let detailed_exit_code = blaze_cquery::commands::aquery_command::exec(&command_env);
        BlazeCommandResult::detailed_exit_code(to_runtime_detailed_exit_code(detailed_exit_code))
    }
}
