//! Port of `runtime/commands/CqueryCommand.java` (`editOptions` and the body of `exec`).

use std::collections::HashMap;
use std::sync::Arc;

use blaze_events::Event;
use blaze_optdefs::command::cquery_options::{CqueryOptions, Transitions};
use blaze_optdefs::fragment::core_options::CoreOptionsIncludeConfigFragmentsEnum;
use blaze_options::{OptionScalar, OptionsParser, PriorityCategory};
use blaze_protos::failure_details::{
    ConfigurableQuery, FailureDetail, configurable_query, failure_detail::Category,
};
use blaze_query_engine::{DetailedExitCode, QueryExpression, QueryFunction, QueryParser};
use blaze_sky_top::build_tool::BuildRequest;
use parking_lot::RwLock;

use crate::buildtool::cquery_processor::CqueryProcessor;
use crate::commands::{
    PostAnalysisCommandEnvironment, main_repo_target_parser, process_request, read_query,
};
use crate::cquery::config_function::ConfigFunctionSlot;
use crate::cquery::configured_target_query_environment::{CqueryNode, functions};

/// Allows the command to provide command-specific option defaults and/or requirements.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:79 CqueryCommand.editOptions
pub fn edit_options(options_parser: &mut OptionsParser) {
    let cquery_options: CqueryOptions = options_parser
        .get_options::<CqueryOptions>()
        .expect("CqueryOptions");
    let result = (|| {
        if cquery_options.transitions != Transitions::NONE {
            options_parser.parse_with_priority(
                PriorityCategory::COMPUTED_DEFAULT,
                Some("Option required by setting the --transitions flag"),
                vec!["--output=transitions".to_owned()],
            )?;
        }
        options_parser.parse_with_priority(
            PriorityCategory::COMPUTED_DEFAULT,
            Some("Options required by cquery"),
            vec!["--nobuild".to_owned()],
        )?;
        options_parser.parse_with_priority(
            PriorityCategory::COMPUTED_DEFAULT,
            Some("cquery should include 'tags = [\"manual\"]' targets by default"),
            vec!["--build_manual_tests".to_owned()],
        )?;
        options_parser.parse_with_priority(
            PriorityCategory::SOFTWARE_REQUIREMENT,
            // https://github.com/bazelbuild/bazel/issues/11078
            Some("cquery should not exclude test_suite rules"),
            vec!["--noexpand_test_suites".to_owned()],
        )?;
        if cquery_options.show_required_config_fragments
            != CoreOptionsIncludeConfigFragmentsEnum::OFF
        {
            options_parser.parse_with_priority(
                PriorityCategory::COMPUTED_DEFAULT,
                Some("Options required by cquery's --show_config_fragments flag"),
                vec![format!(
                    "--include_config_fragments_provider={}",
                    cquery_options
                        .show_required_config_fragments
                        .java_to_string()
                )],
            )?;
        }
        options_parser.parse_with_priority(
            PriorityCategory::SOFTWARE_REQUIREMENT,
            Some("cquery should not exclude tests"),
            vec!["--nobuild_tests_only".to_owned()],
        )
    })();
    if let Err(e) = result {
        panic!(
            "IllegalStateException: Cquery's known options failed to parse: {}",
            e.get_message()
        );
    }
}

/// The cquery `QueryFunction`s by name, with the slot `config()` reaches the environment through.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:149 CqueryCommand.exec
pub fn function_map(
    slot: &ConfigFunctionSlot,
) -> HashMap<String, Arc<dyn QueryFunction<CqueryNode>>> {
    let mut map: HashMap<String, Arc<dyn QueryFunction<CqueryNode>>> = HashMap::new();
    for query_function in functions(slot) {
        map.insert(query_function.get_name(), query_function);
    }
    // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
    map
}

/// Handles the 'cquery' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:118 CqueryCommand.exec
pub fn exec(env: &PostAnalysisCommandEnvironment<'_>) -> DetailedExitCode {
    // SkyQueryEnvironment.DEFAULT_THREAD_COUNT (a static initializer in Java).
    crate::post_analysis_query_environment::set_default_thread_count(env.available_processors);
    let main_repo_target_parser = match main_repo_target_parser(env) {
        Ok(p) => p,
        Err(detailed_exit_code) => return detailed_exit_code,
    };

    let cquery_options: CqueryOptions = env
        .options
        .get_options::<CqueryOptions>()
        .expect("CqueryOptions");
    let query = match read_query(&cquery_options, env, /* allow_empty_query= */ false) {
        Ok(q) => q,
        Err(e) => return DetailedExitCode::of(e.get_failure_detail().clone()),
    };

    let slot: ConfigFunctionSlot = Arc::new(RwLock::new(None));
    let functions = function_map(&slot);
    let expr: QueryExpression =
        match QueryParser::<CqueryNode>::parse_with_functions(&query, &functions) {
            Ok(expr) => expr,
            Err(e) => {
                let message = format!(
                    "Error while parsing '{}': {}",
                    QueryExpression::truncate(&query),
                    e.get_message()
                );
                env.reporter.handle(&Event::error(message.clone()));
                return create_failure_result(
                    &message,
                    configurable_query::Code::ExpressionParseFailure,
                );
            }
        };

    let mut top_level_targets: Vec<String> = cquery_options.universe_scope.clone();
    let mut target_pattern_set: indexmap::IndexSet<String> = indexmap::IndexSet::new();
    let mut targets_for_project_resolution: Option<Vec<String>> = None;
    if top_level_targets.is_empty() {
        expr.collect_target_patterns(&mut |p| {
            target_pattern_set.insert(p);
        });
        top_level_targets = target_pattern_set.iter().cloned().collect();
        // Java: functionExpr.getFunction() instanceof SomePathFunction / AllPathsFunction.
        // PORT: the builtin functions are told apart by their query-language name.
        if let blaze_query_engine::query_expression::QueryExpression::Function(function_expr) =
            &expr
            && matches!(
                function_expr.get_function().get_name().as_str(),
                "somepath" | "allpaths"
            )
        {
            targets_for_project_resolution = target_pattern_set.first().cloned().map(|p| vec![p]);
        }
    }

    let mut request = BuildRequest::new(
        "cquery",
        env.options,
        top_level_targets,
        /* check_for_action_conflicts= */ false,
        /* report_incompatible_targets= */ false,
    );
    request.targets_for_project_resolution = targets_for_project_resolution;
    // PORT: skipped, incremental: QueryCommandUtils.resetDeserializedKeysFromRemoteAnalysisCache
    // deletes the values a previous command of the server deserialized from the remote analysis
    // cache.
    let processor = CqueryProcessor::new(
        Some(expr),
        main_repo_target_parser,
        cquery_options,
        slot,
        env.reporter.clone(),
        env.output.clone(),
        env.working_directory.clone(),
        env.tracks_state_for_incrementality,
    );
    process_request(env, &request, &processor)
}

// Java: com/google/devtools/build/lib/runtime/commands/CqueryCommand.java:205 CqueryCommand.createFailureResult
fn create_failure_result(
    message: &str,
    detailed_code: configurable_query::Code,
) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_owned(),
        category: Some(Category::ConfigurableQuery(ConfigurableQuery {
            code: detailed_code as i32,
        })),
    })
}
