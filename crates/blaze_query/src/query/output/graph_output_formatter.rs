//! Port of `query2/query/output/GraphOutputFormatter.java`.

use std::cmp::Ordering;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::callback::OutputFormatterError;
use blaze_vfs::DigestHashFunction;

use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::blaze_query_environment::{TargetDigraph, TargetNode};
use crate::query::output::conditional_edges::ConditionalEdges;
use crate::query::output::format_utils;
use crate::query::output::graph_output_writer::{GraphOutputWriter, NodeReader};
use crate::query::output::output_formatter::{OutputFormatter, OutputStream};
use crate::query::output::query_options::get_line_terminator;

/// An output formatter that prints the result as factored graph in AT&T GraphViz format.
// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:33 GraphOutputFormatter
pub struct GraphOutputFormatter;

/// `GraphOutputFormatter.NODE_READER`.
// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:39 GraphOutputFormatter.NODE_READER
struct TargetNodeReader;

impl NodeReader<TargetNode> for TargetNodeReader {
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:44 GraphOutputFormatter.NODE_READER.getLabel
    fn get_label(&self, node: &TargetNode, label_printer: &LabelPrinter) -> String {
        // Node payloads are Targets. Output node labels are target labels.
        label_printer.to_string(node.0.get_label())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:50 GraphOutputFormatter.NODE_READER.comparator
    fn comparator(&self, a: &TargetNode, b: &TargetNode) -> Ordering {
        format_utils::target_ordering(&a.0, &b.0)
    }

    fn get_build_label(&self, node: &TargetNode) -> blaze_cmdline::Label {
        node.0.get_label().clone()
    }
}

impl OutputFormatter for GraphOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:35 GraphOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "graph"
    }

    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputFormatter.java:56 GraphOutputFormatter.output
    fn output(
        &self,
        options: &QueryOptions,
        result: &TargetDigraph,
        out: OutputStream,
        _aspect_provider: Arc<dyn AspectResolver>,
        _event_handler: Option<Arc<dyn EventHandler>>,
        _hash_function: DigestHashFunction,
        label_printer: &LabelPrinter,
    ) -> Result<(), OutputFormatterError> {
        let sort_labels = options.order_output == OrderOutput::FULL;
        let reader = TargetNodeReader;
        let graph_writer = GraphOutputWriter::new(
            &reader,
            get_line_terminator(options),
            sort_labels,
            options.graph_node_string_limit,
            options.graph_conditional_edges_limit,
            options.graph_factored,
            label_printer.clone(),
        );
        graph_writer
            .write(result, Some(&ConditionalEdges::from_graph(result)), &out)
            .map_err(|e| OutputFormatterError::Io(e.into()))
    }
}
