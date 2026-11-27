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
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:157 PostAnalysisQueryProcessor.getTransitiveConfigurations
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
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:168 PostAnalysisQueryProcessor.doPostAnalysisQuery
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
        // OutputFormatterCallback.process turns an IOException of processOutput into
        // `new InterruptedException("Interrupting due to a failure in the output formatter")`
        // and keeps the IOException for getIoException(). Unlike
        // OutputFormatterCallback.processAllTargets, doPostAnalysisQuery never asks for it, and
        // PostAnalysisQueryProcessor.process does not catch InterruptedException: BuildTool
        // reports "build interrupted" (exit code 8). That is what Bazel 9.3.2 prints for an
        // aquery action whose command line cannot be expanded
        // (scripts/w4/scenarios/query-map-each).
        // Java: com/google/devtools/build/lib/query2/engine/OutputFormatterCallback.java:52 OutputFormatterCallback.process
        Err(QueryError::Interrupted(_)) => return Err(PostAnalysisQueryError::Interrupted),
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

/// The message of `new ViewCreationFailedException(message, failureDetail, cause)`.
// Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:49 ViewCreationFailedException.combineMessages
fn combine_messages(message: &str, cause_message: &str) -> String {
    // isNullOrEmpty(cause.getMessage())
    if cause_message.is_empty() {
        message.to_owned()
    } else {
        format!("{}: {}", message, cause_message)
    }
}

/// The error handling of `PostAnalysisQueryProcessor.process` around `doPostAnalysisQuery`.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:72 PostAnalysisQueryProcessor.process
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
                // new ViewCreationFailedException(errorMessage, e.getFailureDetail(), e)
                return Err(BuildToolError::ViewCreationFailed {
                    message: combine_messages(error_message, e.get_message()),
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
                // new ViewCreationFailedException(errorMessage, failureDetail, e)
                return Err(BuildToolError::ViewCreationFailed {
                    message: combine_messages(error_message, &message),
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
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:72 PostAnalysisQueryProcessor.process
pub fn before_post_analysis_query(
    tracks_state_for_incrementality: bool,
    skyframe_executor: Option<(
        &blaze_sky_top::skyframe_executor::SkyframeExecutor,
        std::sync::Arc<dyn blaze_events::ExtendedEventHandler>,
    )>,
) -> Result<(), BuildToolError> {
    // This query will operate over the graph as constructed by analysis, but will also pick up
    // any nodes that are in the graph from prior builds, including dirty nodes. While they can't
    // be reached from the scope of the current build via deps, they can appear in rdeps, which
    // results in the processor doing unnecessary work or even expecting stale keys to be
    // present. Clean them up now, which requires ignoring the value of
    // --version_window_for_dirty_node_gc.
    if let Some((skyframe_executor, reporter)) = skyframe_executor {
        skyframe_executor.delete_old_nodes(/* version_window_for_dirty_gc= */ 0);
        if skyframe_executor.apply_invalidation(reporter).is_err() {
            return Err(BuildToolError::Interrupted);
        }
    }
    if !tracks_state_for_incrementality {
        return Err(BuildToolError::Exit(DetailedExitCode::of(FailureDetail {
            message: "Queries based on analysis results are not allowed if incrementality state \
                      is not being kept"
                .to_owned(),
            category: Some(Category::Query(failure_details::Query {
                code: query::Code::AnalysisQueryPrereqUnmet as i32,
            })),
        })));
    }
    Ok(())
}

/// `env.getRuntime().getQueryRuntimeHelperFactory().create(env, getQueryOptions(env))` of
/// `PostAnalysisQueryProcessor.process`; a `QueryRuntimeHelperException` is its
/// `ExitException(DetailedExitCode.of(e.getFailureDetail()))`.
// Java: com/google/devtools/build/lib/buildtool/PostAnalysisQueryProcessor.java:72 PostAnalysisQueryProcessor.process
pub fn create_query_runtime_helper(
    stdout: &blaze_query::query::output::output_formatter::OutputStream,
    working_directory: &blaze_vfs::Path,
    output_file: &str,
) -> Result<blaze_query::query::output::output_formatter::OutputStream, BuildToolError> {
    blaze_query::query::output::query_runtime_helper::create(stdout, working_directory, output_file)
        .map_err(|e| {
            let failure_detail = e.get_failure_detail();
            BuildToolError::Exit(blaze_query_engine::DetailedExitCode::of_exit_code(
                blaze_util::DetailedExitCode::get_numeric_exit_code(&failure_detail),
                failure_detail,
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_events::{EventKind, StoredEventHandler};
    use blaze_protos::failure_details::{ConfigurableQuery, configurable_query};

    /// The exception `evaluateQueryInternal` throws for `cquery "config(//:x, deadbeef)"`.
    fn unknown_configuration_exception() -> QueryException {
        QueryException::with_failure_detail(
            None,
            "Evaluation failed: Unknown configuration ID 'deadbeef'.",
            FailureDetail {
                message: "Unknown configuration ID 'deadbeef'.".to_owned(),
                category: Some(Category::ConfigurableQuery(ConfigurableQuery {
                    code: configurable_query::Code::IncorrectConfigArgumentError as i32,
                })),
            },
        )
    }

    // Expected text: scripts/w4/scenarios/query-errors/ORACLE.txt step 1 (Bazel 9.3.2):
    // "ERROR: Error doing post analysis query: Evaluation failed: Unknown configuration ID
    // 'deadbeef'." (ViewCreationFailedException.combineMessages keeps the cause).
    #[test]
    fn view_creation_failed_exception_keeps_the_cause() {
        let stored = Arc::new(StoredEventHandler::new());
        let reporter: Arc<dyn ExtendedEventHandler> = stored.clone();
        let result = handle_post_analysis_query_result(
            Err(PostAnalysisQueryError::Query(
                unknown_configuration_exception(),
            )),
            /* keep_going= */ false,
            &reporter,
        );
        match result {
            Err(BuildToolError::ViewCreationFailed {
                message,
                failure_detail,
            }) => {
                assert_eq!(
                    message,
                    "Error doing post analysis query: Evaluation failed: Unknown configuration \
                     ID 'deadbeef'."
                );
                assert_eq!(
                    failure_detail.message,
                    "Unknown configuration ID 'deadbeef'."
                );
            }
            other => panic!("expected ViewCreationFailed, got {:?}", other),
        }
        assert!(stored.get_events().is_empty());
    }

    // Expected text: scripts/w4/scenarios/query-errors/ORACLE.txt step 2 (--keep_going):
    // "ERROR: Error doing post analysis query: Unknown configuration ID 'deadbeef'.", exit 0.
    #[test]
    fn keep_going_reports_the_failure_detail_message() {
        let stored = Arc::new(StoredEventHandler::new());
        let reporter: Arc<dyn ExtendedEventHandler> = stored.clone();
        let result = handle_post_analysis_query_result(
            Err(PostAnalysisQueryError::Query(
                unknown_configuration_exception(),
            )),
            /* keep_going= */ true,
            &reporter,
        );
        assert!(result.is_ok());
        let events = stored.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].get_kind(), EventKind::Error);
        assert_eq!(
            events[0].get_message(),
            "Error doing post analysis query: Unknown configuration ID 'deadbeef'."
        );
    }

    // Expected text and exit code: scripts/w4/scenarios/query-errors/ORACLE.txt
    // (`cquery //:x --notrack_incremental_state`): "ERROR: Queries based on analysis results are
    // not allowed if incrementality state is not being kept", exit 2.
    #[test]
    fn queries_need_incrementality_state() {
        assert!(before_post_analysis_query(true, None).is_ok());
        match before_post_analysis_query(false, None) {
            Err(BuildToolError::Exit(detailed_exit_code)) => {
                assert_eq!(detailed_exit_code.get_exit_code(), 2);
                assert_eq!(
                    detailed_exit_code
                        .get_failure_detail()
                        .expect("failure detail")
                        .message,
                    "Queries based on analysis results are not allowed if incrementality state \
                     is not being kept"
                );
            }
            other => panic!("expected an ExitException, got {:?}", other),
        }
    }

    // scripts/w4/scenarios/query-map-each/ORACLE.txt: an output formatter failure surfaces as
    // the InterruptedException of OutputFormatterCallback.process ("ERROR: build interrupted",
    // exit 8), with and without --keep_going.
    #[test]
    fn interrupted_stays_interrupted() {
        for keep_going in [false, true] {
            let stored = Arc::new(StoredEventHandler::new());
            let reporter: Arc<dyn ExtendedEventHandler> = stored.clone();
            let result = handle_post_analysis_query_result(
                Err(PostAnalysisQueryError::Interrupted),
                keep_going,
                &reporter,
            );
            assert!(matches!(result, Err(BuildToolError::Interrupted)));
            assert!(stored.get_events().is_empty());
        }
    }
}
