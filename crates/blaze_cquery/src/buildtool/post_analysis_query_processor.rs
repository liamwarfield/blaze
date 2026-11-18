//! Port of `buildtool/PostAnalysisQueryProcessor.java`: runs a post-analysis query (cquery or
//! aquery) over the graph the analysis phase built.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category, query};
use blaze_query::common::abstract_blaze_query_environment::EvaluateQueryError;
use blaze_query_engine::query_util;
use blaze_query_engine::{
    DetailedExitCode, QueryError, QueryException, QueryExpression, TargetAccessor,
};
use blaze_sky_top::build_tool::BuildToolError;
use blaze_skyframe::{SkyKey, WalkableGraph};
use indexmap::IndexMap;

use crate::post_analysis_query_environment::{
    NamedThreadSafeOutputFormatterCallback, PostAnalysisNode, PostAnalysisQueryEnvironment,
    callback_names, select_callback,
};

/// `ExitCode.COMMAND_LINE_ERROR`.
const COMMAND_LINE_ERROR: i32 = 2;
/// `Query.Code.OUTPUT_FORMATTER_IO_EXCEPTION` exit code.
const OUTPUT_FORMATTER_IO_EXCEPTION_EXIT_CODE: i32 = 36;

/// What `doPostAnalysisQuery` throws.
#[derive(Debug)]
pub enum PostAnalysisQueryError {
    Query(QueryException),
    Io(String),
    Interrupted,
    /// `OptionsParsingException` (message).
    OptionsParsing(String),
}

impl From<EvaluateQueryError> for PostAnalysisQueryError {
    fn from(e: EvaluateQueryError) -> Self {
        match e {
            EvaluateQueryError::Query(e) => PostAnalysisQueryError::Query(e),
            EvaluateQueryError::Interrupted(_) => PostAnalysisQueryError::Interrupted,
            EvaluateQueryError::Io(e) => PostAnalysisQueryError::Io(e.to_string()),
        }
    }
}

/// The configurations of the graph keyed by checksum, sorted by checksum.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:150 PostAnalysisQueryProcessor.getTransitiveConfigurations
pub fn get_transitive_configurations(
    transitive_configuration_keys: &[SkyKey],
    graph: &dyn WalkableGraph,
) -> IndexMap<String, Arc<BuildConfigurationValue>> {
    // BuildConfigurationKey and BuildConfigurationValue should be 1:1 so merge function
    // intentionally omitted
    let mut values: Vec<Arc<BuildConfigurationValue>> = graph
        .get_successful_values(transitive_configuration_keys)
        .into_iter()
        .map(|(_, v)| {
            v.downcast::<BuildConfigurationValue>()
                .unwrap_or_else(|_| panic!("BUILD_CONFIGURATION value"))
        })
        .collect();
    values.sort_by(|a, b| a.checksum().cmp(b.checksum()));
    let mut result: IndexMap<String, Arc<BuildConfigurationValue>> = IndexMap::new();
    for v in values {
        if result.insert(v.checksum().to_owned(), v.clone()).is_some() {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                v.checksum()
            );
        }
    }
    result
}

/// The query part of `doPostAnalysisQuery`: selects the output callback, evaluates the query
/// into an aggregating callback and replays the full result to the output callback.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:162 PostAnalysisQueryProcessor.doPostAnalysisQuery
pub fn do_post_analysis_query<T: PostAnalysisNode>(
    post_analysis_query_environment: &PostAnalysisQueryEnvironment<T>,
    callbacks: &[Box<dyn NamedThreadSafeOutputFormatterCallback<T> + '_>],
    output_format: &str,
    query_expression: &QueryExpression,
    reporter: &Arc<dyn ExtendedEventHandler>,
) -> Result<(), PostAnalysisQueryError>
where
    PostAnalysisQueryEnvironment<T>: TargetAccessor<T>,
{
    let Some(callback_index) = select_callback(output_format, callbacks) else {
        return Err(PostAnalysisQueryError::OptionsParsing(format!(
            "Invalid output format '{}'. Valid values are: {}",
            output_format,
            callback_names(callbacks)
        )));
    };
    let callback = &callbacks[callback_index];

    // A certain subset of output formatters support "streaming" results - the formatter is
    // called multiple times where each call has only a some of the full query results (see
    // StreamedOutputFormatter for details). cquery and aquery don't do this. But the reason is
    // subtle and hard to follow. Post-analysis output formatters inherit from Callback, which
    // declares "void process(Iterable<T> partialResult)". Its javadoc says that the subinterface
    // BatchCallback may stream partial results. But post-analysis callbacks don't inherit
    // BatchCallback!
    //
    // To protect against accidental feature regression (like implementing a callback that
    // accidentally inherits BatchCallback), we explicitly disable streaming here. The
    // aggregating callback collects the entire query's results, even if the query was evaluated
    // in a streaming manner. Note that streaming query evaluation is a distinct concept from
    // streaming output formatting. Once the complete query finishes, we replay the full results
    // back to the original callback. That way callback implementations can safely assume they're
    // only called once and the results for that call are indeed complete.
    let aggregate_results_callback =
        query_util::new_ordered_aggregate_all_output_formatter_callback(
            post_analysis_query_environment,
        );
    let result = post_analysis_query_environment
        .evaluate_query(query_expression, &*aggregate_results_callback)?;
    if result.is_empty() {
        reporter.handle(&Event::info("Empty query results"));
    }
    callback
        .start()
        .map_err(|e| PostAnalysisQueryError::Io(e.to_string()))?;
    match callback.process(&aggregate_results_callback.get_result()) {
        Ok(()) => {}
        Err(QueryError::Interrupted(_)) => {
            return Err(match callback.get_io_exception() {
                Some(e) => PostAnalysisQueryError::Io(e.to_string()),
                None => PostAnalysisQueryError::Interrupted,
            });
        }
        Err(QueryError::Query(e)) => return Err(PostAnalysisQueryError::Query(e)),
        Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
    }
    callback
        .close(/* fail_fast= */ !result.get_success())
        .map_err(|e| match e {
            blaze_query_engine::OutputFormatterError::Io(e) => {
                PostAnalysisQueryError::Io(e.to_string())
            }
            blaze_query_engine::OutputFormatterError::Interrupted(_) => {
                PostAnalysisQueryError::Interrupted
            }
        })?;

    // queryRuntimeHelper.afterQueryOutputIsWritten(): nothing to do for stdout.
    Ok(())
}

/// The error handling of `PostAnalysisQueryProcessor.process` around `doPostAnalysisQuery`.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:71 PostAnalysisQueryProcessor.process
pub fn handle_post_analysis_query_result(
    result: Result<(), PostAnalysisQueryError>,
    keep_going: bool,
    reporter: &Arc<dyn ExtendedEventHandler>,
) -> Result<(), BuildToolError> {
    match result {
        Ok(()) => Ok(()),
        Err(PostAnalysisQueryError::Query(e)) => {
            let error_message = "Error doing post analysis query";
            if !keep_going {
                return Err(BuildToolError::ViewCreationFailed {
                    message: error_message.to_owned(),
                    failure_detail: e.get_failure_detail().clone(),
                });
            }
            reporter.handle(&Event::error(format!(
                "{}: {}",
                error_message,
                e.get_failure_detail().message
            )));
            Ok(())
        }
        Err(PostAnalysisQueryError::Io(message)) => {
            let error_message = "I/O error doing post analysis query";
            let failure_detail = FailureDetail {
                message: format!("{}: {}", error_message, message),
                category: Some(Category::Query(failure_details::Query {
                    code: query::Code::OutputFormatterIoException as i32,
                })),
            };
            if !keep_going {
                return Err(BuildToolError::ViewCreationFailed {
                    message: error_message.to_owned(),
                    failure_detail,
                });
            }
            reporter.handle(&Event::error(failure_detail.message.clone()));
            let _ = OUTPUT_FORMATTER_IO_EXCEPTION_EXIT_CODE;
            Ok(())
        }
        Err(PostAnalysisQueryError::Interrupted) => Err(BuildToolError::Interrupted),
        Err(PostAnalysisQueryError::OptionsParsing(message)) => {
            Err(BuildToolError::Exit(DetailedExitCode::of_exit_code(
                COMMAND_LINE_ERROR,
                FailureDetail {
                    message,
                    category: Some(Category::ActionQuery(failure_details::ActionQuery {
                        code: failure_details::action_query::Code::IncorrectArguments as i32,
                    })),
                },
            )))
        }
    }
}

/// The checks `process` makes before running the query.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:84 PostAnalysisQueryProcessor.process
pub fn before_post_analysis_query() -> Result<(), BuildToolError> {
    // This query will operate over the graph as constructed by analysis, but will also pick up
    // any nodes that are in the graph from prior builds, including dirty nodes.
    // PORT: deleteOldNodes / applyInvalidation are incremental-state plumbing (one evaluation
    // per process); the executor always tracks state for incrementality.
    Ok(())
}
