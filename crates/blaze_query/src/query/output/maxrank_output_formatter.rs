//! Port of `query2/query/output/MaxrankOutputFormatter.java`.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_events::EventHandler;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_packages::target::LabelPrinter;
use blaze_query_engine::callback::OutputFormatterError;
use blaze_util::graph::{Digraph, NodeId, NodeSet};
use blaze_vfs::DigestHashFunction;

use crate::query::aspectresolvers::aspect_resolver::AspectResolver;
use crate::query::blaze_query_environment::TargetDigraph;
use crate::query::output::minrank_output_formatter::{flush_and_check_error, print};
use crate::query::output::output_formatter::{OutputFormatter, OutputStream};
use crate::query::output::query_options::get_line_terminator;
use crate::query::output::rank_and_label::RankAndLabel;

/// An output formatter that prints the labels in maximum rank order, preceded by their rank
/// number. "Roots" have rank 0, all other nodes have a rank which is 1 greater than the maximum
/// rank of their predecessors. All nodes in a cycle are considered of equal rank. MAXRANK shows
/// the highest rank for a given node, i.e. the length of the longest non-cyclic path from a
/// zero-rank node to it.
///
/// If the result came from a `deps(x)` query, then the MAXRANKs correspond to the longest path
/// from x to each of its prerequisites.
// Java: com/google/devtools/build/lib/query2/query/output/MaxrankOutputFormatter.java:48 MaxrankOutputFormatter
pub struct MaxrankOutputFormatter;

/// Dynamic programming algorithm: rank(x) = max(rank(p)) + 1 foreach p in preds(x).
// Java: com/google/devtools/build/lib/query2/query/output/MaxrankOutputFormatter.java:72 MaxrankOutputFormatter.output.DP
struct Dp<'a> {
    graph: &'a Digraph<NodeSet>,
    ranks: HashMap<NodeId, i32>,
}

impl Dp<'_> {
    // Java: com/google/devtools/build/lib/query2/query/output/MaxrankOutputFormatter.java:75 MaxrankOutputFormatter.output.DP.rank
    fn rank(&mut self, node: NodeId) -> i32 {
        if let Some(rank) = self.ranks.get(&node) {
            return *rank;
        }
        let mut max_pred_rank = -1;
        for p in self.graph.get_predecessors(node).to_vec() {
            max_pred_rank = max_pred_rank.max(self.rank(p));
        }
        let rank = max_pred_rank + 1;
        self.ranks.insert(node, rank);
        rank
    }
}

impl OutputFormatter for MaxrankOutputFormatter {
    // Java: com/google/devtools/build/lib/query2/query/output/MaxrankOutputFormatter.java:51 MaxrankOutputFormatter.getName
    fn get_name(&self) -> &'static str {
        "maxrank"
    }

    // Java: com/google/devtools/build/lib/query2/query/output/MaxrankOutputFormatter.java:56 MaxrankOutputFormatter.output
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
        // In order to handle cycles correctly, we need work on the strong component graph, as
        // cycles should be treated a "clump" of nodes all on the same rank. Graphs may contain
        // cycles because there are errors in BUILD files.
        let sc_graph = result.get_strong_component_graph();
        let mut dp = Dp {
            graph: &sc_graph,
            ranks: HashMap::new(),
        };

        // Now sort by rank...
        let mut output: Vec<RankAndLabel> = Vec::new();
        for x in sc_graph.get_nodes() {
            let rank = dp.rank(x);
            for y in sc_graph.get_label(x).iter() {
                output.push(RankAndLabel::new(
                    rank,
                    result.get_label(*y).0.get_label().clone(),
                ));
            }
        }
        if options.order_output == OrderOutput::FULL {
            // Use the natural order for RankAndLabels, which breaks ties alphabetically.
            output.sort();
        } else {
            output.sort_by_key(|r| r.get_rank());
        }
        let line_term = get_line_terminator(options);
        let mut print_stream: Vec<u8> = Vec::new();
        for item in &output {
            print(
                &mut print_stream,
                &format!("{}{}", item.to_string(label_printer), line_term),
            );
        }
        flush_and_check_error(&out, &print_stream)
    }
}
