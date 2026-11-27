//! Port of `runtime/commands/AqueryCommand.java` (`editOptions` and the body of `exec`) and
//! `QueryCommandUtils.getTopLevelTargets`.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_events::Event;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_options::{OptionsParser, PriorityCategory};
use blaze_protos::failure_details::{
    ActionQuery, FailureDetail, action_query, failure_detail::Category,
};
use blaze_query_engine::{
    DetailedExitCode, QueryException, QueryExpression, QueryFunction, QueryParser,
};
use blaze_sky_top::build_tool::BuildRequest;

use crate::aquery::action_graph_query_environment::{
    KeyedConfiguredTargetValue, aquery_functions, functions,
};
use crate::buildtool::aquery_processor::AqueryProcessor;
use crate::commands::{
    PostAnalysisCommandEnvironment, main_repo_target_parser, process_request, read_query,
};

/// Allows the command to provide command-specific option defaults and/or requirements.
// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:69 AqueryCommand.editOptions
pub fn edit_options(options_parser: &mut OptionsParser) {
    if let Err(e) = options_parser.parse_with_priority(
        PriorityCategory::COMPUTED_DEFAULT,
        Some("Option required by aquery"),
        vec!["--nobuild".to_owned()],
    ) {
        panic!(
            "IllegalStateException: Aquery's known options failed to parse: {}",
            e.get_message()
        );
    }
}

// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:185 AqueryCommand.getFunctionsMap
fn get_functions_map() -> HashMap<String, Arc<dyn QueryFunction<KeyedConfiguredTargetValue>>> {
    let mut functions_builder: HashMap<String, Arc<dyn QueryFunction<KeyedConfiguredTargetValue>>> =
        HashMap::new();
    for query_function in functions().into_iter().chain(aquery_functions()) {
        // ImmutableMap.Builder.buildOrThrow
        let name = query_function.get_name();
        if functions_builder
            .insert(name.clone(), query_function)
            .is_some()
        {
            panic!("IllegalArgumentException: Multiple entries with same key: {name}");
        }
    }
    // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
    functions_builder
}

// Java: com/google/devtools/build/lib/runtime/commands/QueryCommandUtils.java:42 QueryCommandUtils.getTopLevelTargets
fn get_top_level_targets(
    universe_scope: &[String],
    expr: Option<&QueryExpression>,
    query_current_skyframe_state: bool,
) -> Result<Vec<String>, QueryException> {
    let Some(expr) = expr else {
        return Ok(universe_scope.to_vec());
    };

    let top_level_targets: Vec<String> = if universe_scope.is_empty() {
        let mut target_pattern_set: indexmap::IndexSet<String> = indexmap::IndexSet::new();
        expr.collect_target_patterns(&mut |p| {
            target_pattern_set.insert(p);
        });
        target_pattern_set.into_iter().collect()
    } else {
        universe_scope.to_vec()
    };

    if query_current_skyframe_state && !top_level_targets.is_empty() {
        return Err(QueryException::with_action_query_code(
            None,
            format!(
                "Error while parsing '{}': Specifying build target(s) [{}] with --skyframe_state \
                 is currently not supported.",
                expr.to_trunctated_string(),
                top_level_targets.join(", ")
            ),
            action_query::Code::TopLevelTargetsWithSkyframeStateNotSupported,
        ));
    }

    Ok(top_level_targets)
}

/// Handles the 'aquery' command on the Blaze command line.
// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:81 AqueryCommand.exec
pub fn exec(env: &PostAnalysisCommandEnvironment<'_>) -> DetailedExitCode {
    // TODO(twerth): Reduce overlap with CqueryCommand.
    // SkyQueryEnvironment.DEFAULT_THREAD_COUNT (a static initializer in Java).
    crate::post_analysis_query_environment::set_default_thread_count(env.available_processors);
    let aquery_options: AqueryOptions = env
        .options
        .get_options::<AqueryOptions>()
        .expect("AqueryOptions");
    // PORT: skipped, incremental: QueryCommandUtils.resetDeserializedKeysFromRemoteAnalysisCache
    // deletes the values a previous command of the server deserialized from the remote analysis
    // cache.
    let query_current_skyframe_state = aquery_options.query_current_skyframe_state;

    let main_repo_target_parser = match main_repo_target_parser(env) {
        Ok(p) => p,
        Err(detailed_exit_code) => return detailed_exit_code,
    };

    let query = match read_query(&aquery_options, env, query_current_skyframe_state) {
        Ok(q) => q,
        Err(e) => return DetailedExitCode::of(e.get_failure_detail().clone()),
    };

    let functions = get_functions_map();

    // Query expression might be null in the case of --skyframe_state.
    let expr: Option<QueryExpression> = if query.is_empty() {
        None
    } else {
        match QueryParser::<KeyedConfiguredTargetValue>::parse_with_functions(&query, &functions) {
            Ok(expr) => Some(expr),
            Err(e) => {
                let message = format!(
                    "Error while parsing '{}': {}",
                    QueryExpression::truncate(&query),
                    e.get_message()
                );
                env.reporter.handle(&Event::error(message.clone()));
                return create_failure_result(&message, action_query::Code::ExpressionParseFailure);
            }
        }
    };

    let top_level_targets = match get_top_level_targets(
        &aquery_options.universe_scope,
        expr.as_ref(),
        query_current_skyframe_state,
    ) {
        Ok(t) => t,
        Err(e) => {
            env.reporter.handle(&Event::error(e.get_message()));
            return create_failure_result(
                e.get_message(),
                action_query::Code::SkyframeStateWithCommandLineExpression,
            );
        }
    };

    let mut request = BuildRequest::new(
        "aquery",
        env.options,
        top_level_targets,
        /* check_for_action_conflicts= */ true,
        /* report_incompatible_targets= */ true,
    );
    (env.configure_request)(&mut request);

    let expr_string = expr
        .as_ref()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let aquery_build_tool = match AqueryProcessor::new(
        expr,
        main_repo_target_parser,
        aquery_options,
        env.reporter.clone(),
        env.output.clone(),
        env.working_directory.clone(),
        env.tracks_state_for_incrementality,
    ) {
        Ok(p) => p,
        Err(e) => {
            let message = format!("{}\n{}", e.0, expr_string);
            env.reporter.handle(&Event::error(message.clone()));
            return create_failure_result(&message, action_query::Code::InvalidAqueryExpression);
        }
    };

    if query_current_skyframe_state {
        return aquery_build_tool.dump_action_graph_from_skyframe(env.skyframe_executor);
    }
    // PORT: StackOverflowError ("Aquery output was too large to handle") cannot be caught in
    // Rust.
    process_request(env, &request, &aquery_build_tool)
}

// Java: com/google/devtools/build/lib/runtime/commands/AqueryCommand.java:177 AqueryCommand.createFailureResult
fn create_failure_result(message: &str, detailed_code: action_query::Code) -> DetailedExitCode {
    DetailedExitCode::of(FailureDetail {
        message: message.to_owned(),
        category: Some(Category::ActionQuery(ActionQuery {
            code: detailed_code as i32,
        })),
    })
}
