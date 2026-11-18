//! Port of `query2/query/output/QueryOutputUtils.java`.

use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::{LabelPrinter, Target};
use blaze_query_engine::callback::{OutputFormatterError, process_all_targets};
use blaze_vfs::DigestHashFunction;

use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
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

/// PORT: the non-streamed branch needs `DigraphQueryEvalResult` (lib/graph `Digraph`), which is
/// not ported yet.
// Java: com/google/devtools/build/lib/query2/query/output/QueryOutputUtils.java:54 QueryOutputUtils.output
pub fn output(
    query_options: &QueryOptions,
    targets_result: &[Target],
    formatter: &dyn OutputFormatter,
    output_stream: OutputStream,
    aspect_resolver: Arc<dyn AspectResolver>,
    event_handler: Option<Arc<dyn EventHandler>>,
    hash_function: DigestHashFunction,
    label_printer: &LabelPrinter,
) -> Result<(), OutputFormatterError> {
    // Note that if this is a StreamedFormatter, the results are output by the formatter's
    // callbacks, not here.
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
        formatter.output_digraph();
        Ok(())
    }
}
