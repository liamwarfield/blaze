//! Port of `query2/query/output/QueryOutputUtils.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::DigraphQueryEvalResult;
use blaze_query_engine::callback::{OutputFormatterError, process_all_targets};
use blaze_vfs::DigestHashFunction;

use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::blaze_query_environment::TargetNode;
use crate::query::output::output_formatter::{OutputFormatter, OutputStream};
use crate::query::output::output_formatters::is_streamed_formatter;

// Java: com/google/devtools/build/lib/query2/query/output/QueryOutputUtils.java:36 QueryOutputUtils.lexicographicallySortOutput
pub fn lexicographically_sort_output(
    query_options: &QueryOptions,
    formatter: &dyn OutputFormatter,
) -> bool {
    query_options.order_output == OrderOutput::AUTO
        && query_options.lexicographical_output
        && is_streamed_formatter(formatter)
}

// Java: com/google/devtools/build/lib/query2/query/output/QueryOutputUtils.java:43 QueryOutputUtils.shouldStreamUnorderedOutput
pub fn should_stream_unordered_output(
    query_options: &QueryOptions,
    formatter: &dyn OutputFormatter,
) -> bool {
    query_options.order_output == OrderOutput::NO && is_streamed_formatter(formatter)
}

/// Returns whether the query results should be streamed.
// Java: com/google/devtools/build/lib/query2/query/output/QueryOutputUtils.java:49 QueryOutputUtils.shouldStreamResults
pub fn should_stream_results(
    query_options: &QueryOptions,
    formatter: &dyn OutputFormatter,
) -> bool {
    should_stream_unordered_output(query_options, formatter)
        || lexicographically_sort_output(query_options, formatter)
}

/// PORT: Java casts the `QueryEvalResult` to `DigraphQueryEvalResult` for the non-streamed
/// branch; Blaze passes the result's graph (`AbstractBlazeQueryEnvironment::get_result_graph`)
/// as `result`.
// Java: com/google/devtools/build/lib/query2/query/output/QueryOutputUtils.java:54 QueryOutputUtils.output
pub fn output(
    query_options: &QueryOptions,
    result: Option<&DigraphQueryEvalResult<TargetNode>>,
    targets_result: &[Target],
    formatter: &dyn OutputFormatter,
    output_stream: OutputStream,
    aspect_resolver: Arc<dyn AspectResolver>,
    event_handler: Option<Arc<dyn EventHandler>>,
    hash_function: DigestHashFunction,
    label_printer: &LabelPrinter,
) -> Result<(), OutputFormatterError> {
    // This is not really streaming, but we are using the streaming interface for writing into
    // the output everything in one batch. This happens when the QueryEnvironment does not
    // support streaming but we don't care about ordered results.
    if should_stream_results(query_options, formatter) {
        let streamed_formatter = formatter
            .as_streamed_formatter()
            .expect("StreamedFormatter");
        streamed_formatter.set_options(query_options, aspect_resolver, hash_function);
        streamed_formatter.set_event_handler(event_handler);
        process_all_targets(
            &*streamed_formatter.create_post_facto_stream_callback(
                output_stream,
                query_options,
                label_printer,
            ),
            targets_result,
        )
    } else {
        let digraph_query_eval_result =
            result.expect("ClassCastException: not a DigraphQueryEvalResult");
        // PORT: Profiler "digraph.extractSubgraph" / "formatter.output" skipped (profiling).
        let targets: HashSet<TargetNode> = targets_result
            .iter()
            .map(|t| TargetNode(t.clone()))
            .collect();
        let subgraph = digraph_query_eval_result
            .get_graph()
            .extract_subgraph(&|t: &TargetNode| targets.contains(t));

        formatter.output(
            query_options,
            &subgraph,
            output_stream,
            aspect_resolver,
            event_handler,
            hash_function,
            label_printer,
        )
    }
}
