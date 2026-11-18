//! Port of `buildtool/AqueryProcessor.java` (the post-analysis query path; `--skyframe_state`
//! is not ported).

use std::sync::Arc;

use blaze_cmdline::TargetPatternParser;
use blaze_events::ExtendedEventHandler;
use blaze_optdefs::command::aquery_options::AqueryOptions;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query_engine::regex_filter_expression::compile_java_pattern;
use blaze_query_engine::{ArgumentType, FunctionExpression, QueryExpression};
use blaze_sky_top::build_tool::{AnalysisPostProcessor, BuildRequest, BuildToolError};
use blaze_sky_top::build_view::AnalysisResult;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;

use crate::aquery;
use crate::aquery::action_graph_query_environment::{
    ActionGraphQueryEnvironment, aquery_functions, new_action_graph_query_environment,
};
use crate::aquery::aquery_action_filter::AqueryActionFilter;
use crate::buildtool::post_analysis_query_processor::{
    before_post_analysis_query, do_post_analysis_query, get_transitive_configurations,
    handle_post_analysis_query_result,
};
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::post_analysis_query_environment::TopLevelConfigurations;

/// Custom exception class for aquery filtering.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:249 AqueryProcessor.AqueryActionFilterException
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct AqueryActionFilterException(pub String);

/// Performs `aquery` processing.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:62 AqueryProcessor
pub struct AqueryProcessor {
    query_expression: Option<QueryExpression>,
    main_repo_target_parser: TargetPatternParser,
    action_filters: AqueryActionFilter,
    aquery_options: AqueryOptions,
    reporter: Arc<dyn ExtendedEventHandler>,
    output: OutputStream,
}

impl AqueryProcessor {
    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:65 AqueryProcessor.<init>
    pub fn new(
        query_expression: Option<QueryExpression>,
        main_repo_target_parser: TargetPatternParser,
        aquery_options: AqueryOptions,
        reporter: Arc<dyn ExtendedEventHandler>,
        output: OutputStream,
    ) -> Result<Self, AqueryActionFilterException> {
        let action_filters = build_action_filters(query_expression.as_ref())?;
        Ok(AqueryProcessor {
            query_expression,
            main_repo_target_parser,
            action_filters,
            aquery_options,
            reporter,
            output,
        })
    }

    /// Outputs the current action graph from Skyframe.
    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:78 AqueryProcessor.dumpActionGraphFromSkyframe
    pub fn dump_action_graph_from_skyframe(&self) -> ! {
        blaze_util::unported!(
            "AqueryProcessor.dumpActionGraphFromSkyframe",
            "--skyframe_state"
        )
    }

    // Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:149 AqueryProcessor.getQueryEnvironment
    fn get_query_environment(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        top_level_configurations: TopLevelConfigurations,
        transitive_configurations: indexmap::IndexMap<
            String,
            Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
        >,
        walkable_graph: Arc<dyn blaze_skyframe::WalkableGraph>,
    ) -> ActionGraphQueryEnvironment {
        // PORT: env.getRuntime().getQueryFunctions() is empty for Bazel.
        let extra_functions = aquery_functions();
        let starlark_semantics = skyframe_executor.get_effective_starlark_semantics();
        // queryEnvironment.setActionFilters(actionFilters): the filters are handed to the output
        // callbacks directly (get_default_output_formatters).
        new_action_graph_query_environment(
            request.keep_going,
            self.reporter.clone(),
            extra_functions,
            top_level_configurations,
            transitive_configurations,
            self.main_repo_target_parser.clone(),
            skyframe_executor.get_package_path().clone(),
            walkable_graph,
            &self.aquery_options,
            self.aquery_options.get_label_printer(
                starlark_semantics,
                self.main_repo_target_parser.get_repo_mapping().clone(),
            ),
        )
    }
}

/// Return the action filters in the form { inputs: <pattern>, outputs: <pattern>, ... }.
///
/// Fails if an aquery filter function is preceded by any other function types.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:191 AqueryProcessor.buildActionFilters
fn build_action_filters(
    query_expression: Option<&QueryExpression>,
) -> Result<AqueryActionFilter, AqueryActionFilterException> {
    let mut action_filters_builder = AqueryActionFilter::builder();

    let Some(QueryExpression::Function(function_expression)) = query_expression else {
        return Ok(action_filters_builder.build());
    };

    let mut function_expression_optional: Option<Arc<FunctionExpression>> =
        Some(function_expression.clone());

    let mut non_aquery_filter_function_expression: Option<Arc<FunctionExpression>> = None;

    // Unwrap the function layers
    // Validate that aquery filter functions (inputs, outputs, mnemonics) are not preceded by any
    // other function types
    while let Some(function_expression) = function_expression_optional {
        let function_name = function_expression.get_function().get_name();
        // PORT: Java tests `getFunction() instanceof ActionFilterFunction`; the aquery functions
        // map holds exactly the three ActionFilterFunctions under these names.
        if is_action_filter_function(&function_name) {
            if let Some(non_aquery) = &non_aquery_filter_function_expression {
                return Err(AqueryActionFilterException(format!(
                    "aquery filter functions (inputs, outputs, mnemonic) produce actions, and \
                     therefore can't be the input of other function types: {}",
                    non_aquery.get_function().get_name()
                )));
            }

            let pattern_string = function_expression.get_args()[0].get_word();
            match compile_java_pattern(pattern_string) {
                Ok(pattern) => {
                    action_filters_builder.put(&function_name, pattern);
                }
                Err(e) => {
                    return Err(AqueryActionFilterException(format!(
                        "Wrong query syntax: {}",
                        e
                    )));
                }
            }
        } else {
            non_aquery_filter_function_expression = Some(function_expression.clone());
        }

        function_expression_optional = get_next_function_expression(&function_expression);
    }

    Ok(action_filters_builder.build())
}

fn is_action_filter_function(name: &str) -> bool {
    matches!(name, "inputs" | "outputs" | "mnemonic")
}

/// Unwrap input `function_expression` to get the next FunctionExpression in the query.
// Java: com/google/devtools/build/lib/buildtool/AqueryProcessor.java:239 AqueryProcessor.getNextFunctionExpression
fn get_next_function_expression(
    function_expression: &FunctionExpression,
) -> Option<Arc<FunctionExpression>> {
    for arg in function_expression.get_args() {
        if arg.get_type() == ArgumentType::Expression {
            if let QueryExpression::Function(f) = arg.get_expression() {
                return Some(f.clone());
            }
        }
    }
    None
}

impl AnalysisPostProcessor for AqueryProcessor {
    // Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:71 PostAnalysisQueryProcessor.process
    fn process(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError> {
        let Some(query_expression) = &self.query_expression else {
            return Ok(());
        };
        before_post_analysis_query()?;

        // doPostAnalysisQuery
        let walkable_graph = skyframe_executor.get_walkable_graph();
        let transitive_configurations = get_transitive_configurations(
            &skyframe_executor.get_transitive_configuration_keys(),
            &*walkable_graph,
        );
        let env = self.get_query_environment(
            request,
            skyframe_executor,
            TopLevelConfigurations::new(analysis_result.get_top_level_targets_with_configs()),
            transitive_configurations,
            walkable_graph,
        );
        let callbacks = aquery::get_default_output_formatters(
            &env,
            &self.aquery_options,
            self.reporter.clone(),
            self.output.clone(),
            &self.action_filters,
        );
        let result = do_post_analysis_query(
            &env,
            &callbacks,
            &self.aquery_options.output_format,
            query_expression,
            &self.reporter,
        );
        handle_post_analysis_query_result(result, request.keep_going, &self.reporter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_expression_has_no_filters() {
        let filters = build_action_filters(None).unwrap();
        assert!(!filters.has_filter_for_function("mnemonic"));
    }
}
