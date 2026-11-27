//! Port of `query2/cquery/GraphOutputFormatterCallback.java`.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKeyRef;
use blaze_packages::target::LabelPrinter;
use blaze_query::query::output::graph_output_writer::{GraphOutputWriter, NodeReader};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, ThreadSafeOutputFormatterCallback,
};
use blaze_util::graph::Digraph;

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::cquery::cquery_options::CommonQueryOptionsView;
use crate::cquery::cquery_threadsafe_callback::{CqueryThreadsafeCallback, short_id};
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// A `CqueryNode` as a `Digraph` node label. Java keys the graph by the node objects, which
/// Skyframe shares per key; Blaze compares and hashes the lookup keys.
#[derive(Clone)]
pub struct CqueryGraphNode(pub CqueryNode, ActionLookupKeyRef);

impl CqueryGraphNode {
    fn new(node: CqueryNode) -> Self {
        let key = node.get_lookup_key();
        CqueryGraphNode(node, key)
    }
}

impl PartialEq for CqueryGraphNode {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

impl Eq for CqueryGraphNode {}

impl Hash for CqueryGraphNode {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.1.hash(state);
    }
}

impl fmt::Debug for CqueryGraphNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.1)
    }
}

/// cquery output formatter that prints the result as factored graph in AT&T GraphViz format.
// Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:32 GraphOutputFormatterCallback
pub struct GraphOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    label_printer: LabelPrinter,
}

/// The node reader of the callback (Java's anonymous `nodeReader`).
struct CqueryNodeReader<'b, 'a> {
    base: &'b CqueryThreadsafeCallback<'a>,
}

impl NodeReader<CqueryGraphNode> for CqueryNodeReader<'_, '_> {
    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:69 GraphOutputFormatterCallback.nodeReader.getLabel
    fn get_label(&self, node: &CqueryGraphNode, label_printer: &LabelPrinter) -> String {
        // Node payloads are ConfiguredTargets. Output node labels are target labels + config
        // hashes.
        let kct = &node.0;
        format!(
            "{} ({})",
            kct.get_description(label_printer),
            short_id(
                self.base
                    .get_configuration(kct.get_configuration_key().as_ref())
                    .as_ref()
            )
        )
    }

    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:49 GraphOutputFormatterCallback.nodeReader.configuredTargetOrdering
    fn comparator(&self, ct1: &CqueryGraphNode, ct2: &CqueryGraphNode) -> Ordering {
        // Order graph output first by target label, then by configuration hash.
        let label1 = ct1.0.get_original_label();
        let label2 = ct2.0.get_original_label();
        if label1 != label2 {
            return label1.cmp(&label2);
        }
        let checksum1 = ct1.0.get_configuration_checksum();
        let checksum2 = ct2.0.get_configuration_checksum();
        match (checksum1, checksum2) {
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            // String.compareTo over the (ASCII hex) checksums.
            (Some(c1), Some(c2)) => c1.cmp(&c2),
        }
    }
}

impl<'a> GraphOutputFormatterCallback<'a> {
    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:87 GraphOutputFormatterCallback.<init>
    pub fn new(base: CqueryThreadsafeCallback<'a>, label_printer: LabelPrinter) -> Self {
        GraphOutputFormatterCallback {
            base,
            label_printer,
        }
    }

    /// Java's `DepsRetriever`: `kct -> getFwdDeps(ImmutableList.of(kct))`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:236 ConfiguredTargetQueryEnvironment.getDefaultOutputFormatters
    fn get_direct_deps(&self, target: &CqueryNode) -> Vec<CqueryNode> {
        self.base
            .accessor
            .get_fwd_deps_set(std::slice::from_ref(target))
            .to_vec()
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for GraphOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:34 GraphOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "graph".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for GraphOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for GraphOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for GraphOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/GraphOutputFormatterCallback.java:101 GraphOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        // Transform the cquery-backed graph into a Digraph to make it suitable for
        // GraphOutputWriter. Note that this involves an extra iteration over the entire query
        // result subgraph. We could conceptually merge transformation and output writing into
        // the same iteration if needed.
        let mut graph: Digraph<CqueryGraphNode> = Digraph::new();
        let all_nodes: HashSet<CqueryGraphNode> = partial_result
            .iter()
            .map(|n| CqueryGraphNode::new(n.clone()))
            .collect();
        for configured_target in partial_result {
            let node = graph.create_node(CqueryGraphNode::new(configured_target.clone()));
            for dep in self.get_direct_deps(configured_target) {
                let dep = CqueryGraphNode::new(dep);
                if all_nodes.contains(&dep) {
                    let dep_node = graph.create_node(dep);
                    graph.add_edge_nodes(node, dep_node);
                }
            }
        }

        let reader = CqueryNodeReader { base: &self.base };
        let graph_writer = GraphOutputWriter::new(
            &reader,
            self.base.options.get_line_terminator(),
            /* sort_labels= */ true,
            self.base.options.graph_node_string_limit,
            // select() conditions don't matter for cquery because cquery operates post-analysis
            // phase, when select()s have been resolved and removed from the graph.
            /* max_conditional_edges= */
            0,
            self.base.options.graph_factored,
            self.label_printer.clone(),
        );
        let out = self.base.out.clone().expect("cquery output stream");
        graph_writer
            .write(&graph, /* conditional_edges= */ None, &out)
            .map_err(|e| OutputFormatterError::Io(e.into()))
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}
