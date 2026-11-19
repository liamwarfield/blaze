//! Port of `query2/query/output/MinrankOutputFormatter.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::callback::OutputFormatterError;
use blaze_util::graph::NodeId;
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_vfs::DigestHashFunction;

use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::blaze_query_environment::TargetDigraph;
use crate::query::output::output_formatter::{OutputFormatter, OutputStream};
use crate::query::output::query_options::get_line_terminator;
use crate::query::output::rank_and_label::RankAndLabel;

/// An output formatter that prints the labels in minimum rank order, preceded by their rank
/// number. "Roots" have rank 0, their direct prerequisites have rank 1, etc. All nodes in a
/// cycle are considered of equal rank. MINRANK shows the lowest rank for a given node, i.e. the
/// length of the shortest path from a zero-rank node to it.
///
/// If the result came from a `deps(x)` query, then the MINRANKs correspond to the shortest path
/// from x to each of its prerequisites.
// Java: com/google/devtools/build/lib/query2/query/output/MinrankOutputFormatter.java:47 MinrankOutputFormatter
pub struct MinrankOutputFormatter;

/// Java `new PrintStream(out)`: the default charset (ISO-8859-1 on Bazel's server).
pub(crate) fn print(buffer: &mut Vec<u8>, s: &str) {
    buffer.extend_from_slice(&latin1_encode_lossy(s));
}

// Java: com/google/devtools/build/lib/query2/query/output/MinrankOutputFormatter.java:54 MinrankOutputFormatter.outputToStreamOrSave
fn output_to_stream_or_save(
    rank: i32,
    label: &Label,
    out: &mut Vec<u8>,
    to_save: Option<&mut Vec<RankAndLabel>>,
    line_terminator: &str,
    label_printer: &LabelPrinter,
) {
    match to_save {
        Some(to_save) => to_save.push(RankAndLabel::new(rank, label.clone())),
        None => print(
            out,
            &format!(
                "{} {}{}",
                rank,
                label_printer.to_string(label),
                line_terminator
            ),
        ),
    }
}

/// Writes the printed bytes; `PrintStream` reports a failed write through `checkError`.
// Java: com/google/devtools/build/lib/query2/query/output/MinrankOutputFormatter.java:121 MinrankOutputFormatter.flushAndCheckError
pub(crate) fn flush_and_check_error(
    out: &OutputStream,
    bytes: &[u8],
) -> Result<(), OutputFormatterError> {
    let mut stream = out.borrow_mut();
    if stream
        .write_all(bytes)
        .and_then(|()| stream.flush())
        .is_err()
    {
        return Err(OutputFormatterError::Io(
            std::io::Error::other("PrintStream encountered an error").into(),
        ));
    }
    Ok(())
}

impl OutputFormatter for MinrankOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/MinrankOutputFormatter.java:50 MinrankOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "minrank"
    }

    // Java: com/google/devtools/build/lib/query2/query/output/MinrankOutputFormatter.java:69 MinrankOutputFormatter.output
    fn output(
        &self,
        options: &QueryOptions,
        result: &TargetDigraph,
        out: OutputStream,
        _aspect_resolver: Arc<dyn AspectResolver>,
        _event_handler: Option<Arc<dyn EventHandler>>,
        _hash_function: DigestHashFunction,
        label_printer: &LabelPrinter,
    ) -> Result<(), OutputFormatterError> {
        let mut print_stream: Vec<u8> = Vec::new();
        // getRoots() isn't defined for cyclic graphs, so in order to handle cycles correctly, we
        // need work on the strong component graph, as cycles should be treated a "clump" of
        // nodes all on the same rank. Graphs may contain cycles because there are errors in BUILD
        // files.

        let mut output_to_order: Option<Vec<RankAndLabel>> =
            if options.order_output == OrderOutput::FULL {
                Some(Vec::new())
            } else {
                None
            };
        let sc_graph = result.get_strong_component_graph();
        // PORT: java-hash-order. `getRoots()` and `seen` are `HashSet`s of nodes (identity hash);
        // Blaze keeps the roots in node order.
        let mut rank_nodes: Vec<NodeId> = sc_graph.get_roots();
        let mut seen: HashSet<NodeId> = HashSet::new();
        seen.extend(rank_nodes.iter().copied());
        let line_term = get_line_terminator(options);
        let mut rank = 0;
        while !rank_nodes.is_empty() {
            // Print out this rank:
            for x_scc in &rank_nodes {
                for x in sc_graph.get_label(*x_scc).iter() {
                    output_to_stream_or_save(
                        rank,
                        result.get_label(*x).0.get_label(),
                        &mut print_stream,
                        output_to_order.as_mut(),
                        line_term,
                        label_printer,
                    );
                }
            }

            // Find the next rank:
            let mut next_rank_nodes: Vec<NodeId> = Vec::new();
            for x in &rank_nodes {
                for y in sc_graph.get_successors(*x) {
                    if seen.insert(*y) {
                        next_rank_nodes.push(*y);
                    }
                }
            }
            rank_nodes = next_rank_nodes;
            rank += 1;
        }
        if let Some(mut output_to_order) = output_to_order {
            output_to_order.sort();
            for item in &output_to_order {
                print(
                    &mut print_stream,
                    &format!("{}{}", item.to_string(label_printer), line_term),
                );
            }
        }

        flush_and_check_error(&out, &print_stream)
    }
}
