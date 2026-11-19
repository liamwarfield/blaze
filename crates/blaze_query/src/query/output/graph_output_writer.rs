//! Port of `query2/query/output/GraphOutputWriter.java`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

use blaze_cmdline::Label;
use blaze_packages::target::LabelPrinter;
use blaze_util::graph::{Digraph, DotOutputVisitor, GraphVisitor, NodeId, NodeSet};

use crate::query::output::conditional_edges::ConditionalEdges;
use crate::query::output::output_formatter::OutputStream;

/// Interface for reading the contents of a `Digraph` `Node`.
// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:55 GraphOutputWriter.NodeReader
pub trait NodeReader<T> {
    /// Returns the label to associate with a GraphViz node.
    ///
    /// This is not the same as a build `Label`. This is just the text associated with a node in
    /// a GraphViz graph.
    fn get_label(&self, node: &T, label_printer: &LabelPrinter) -> String;

    /// Returns a comparator for the build graph nodes that form the payloads of GraphViz nodes.
    fn comparator(&self, a: &T, b: &T) -> Ordering;

    /// Java `((Target) node.getLabel()).getLabel()` in `getConditionsGraphLabel`: only reached
    /// when conditional edges are given, which only `query` (over `Target`s) does.
    fn get_build_label(&self, _node: &T) -> Label {
        panic!("ClassCastException: graph node payload is not a Target")
    }
}

/// Generic logic for writing query expression results to GraphViz format
/// (<http://graphviz.org/doc/info/lang.html>).
///
/// This can be used by any query implementation that can provide results as a `Digraph`.
// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:53 GraphOutputWriter
pub struct GraphOutputWriter<'r, T> {
    node_reader: &'r dyn NodeReader<T>,
    line_terminator: String,
    sort_labels: bool,
    max_label_size: i32,
    max_conditional_edges: i32,
    merge_equivalent_nodes: bool,
    label_printer: LabelPrinter,
}

// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:77 GraphOutputWriter.RESERVED_LABEL_CHARS
const RESERVED_LABEL_CHARS: i32 = "\\n...and 9999999 more items".len() as i32;

impl<'r, T: Clone + Eq + Hash + fmt::Debug> GraphOutputWriter<'r, T> {
    /// Constructors a new writer.
    ///
    /// - `node_reader`: for reading node content
    /// - `line_terminator`: line string terminator
    /// - `sort_labels`: if true, output nodes in sorted order with `NodeReader::comparator`
    /// - `max_label_size`: maximum characters in label output. Longer labels are truncated. -1
    ///   means no limit.
    /// - `max_conditional_edges`: maximum number of `select()` conditional labels to show on
    ///   each edge. -1 means no limit. 0 means no labels.
    /// - `merge_equivalent_nodes`: if true, topologically equivalent nodes are merged together as
    ///   multiple labels in the same node. This condenses the graph. For example, given graph
    ///   `(nodes=[A, B, C], edges=[A->B, A->C])`, the output has two nodes: "A" and "B,C".
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:93 GraphOutputWriter.<init>
    pub fn new(
        node_reader: &'r dyn NodeReader<T>,
        line_terminator: &str,
        sort_labels: bool,
        max_label_size: i32,
        max_conditional_edges: i32,
        merge_equivalent_nodes: bool,
        label_printer: LabelPrinter,
    ) -> Self {
        GraphOutputWriter {
            node_reader,
            line_terminator: line_terminator.to_owned(),
            sort_labels,
            max_label_size,
            max_conditional_edges,
            merge_equivalent_nodes,
            label_printer,
        }
    }

    /// `nodeComparator`: `Ordering.from(nodeReader.comparator()).onResultOf(Node::getLabel)`.
    fn node_comparator(&self, graph: &Digraph<T>, a: NodeId, b: NodeId) -> Ordering {
        self.node_reader
            .comparator(graph.get_label(a), graph.get_label(b))
    }

    /// `nodeComparator.lexicographical()` over sets of nodes.
    fn node_set_comparator(&self, graph: &Digraph<T>, a: &NodeSet, b: &NodeSet) -> Ordering {
        let mut i = 0;
        loop {
            match (a.get(i), b.get(i)) {
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
                (Some(x), Some(y)) => {
                    let c = self.node_comparator(graph, *x, *y);
                    if c != Ordering::Equal {
                        return c;
                    }
                }
            }
            i += 1;
        }
    }

    /// Writes the given graph.
    ///
    /// - `graph`: build graph to write
    /// - `conditional_edges`: edges corresponding to select()s (see `ConditionalEdges`)
    /// - `out`: output stream to write to
    ///
    /// PORT: Java prints through `PrintWriter(OutputStreamWriter(out, UTF_8))`, so each char of
    /// the (internal) label strings is written as UTF-8: the string's `as_bytes()`.
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:118 GraphOutputWriter.write
    pub fn write(
        &self,
        graph: &Digraph<T>,
        conditional_edges: Option<&ConditionalEdges>,
        out: &OutputStream,
    ) -> std::io::Result<()> {
        let text = if self.merge_equivalent_nodes {
            self.output_factored(graph, conditional_edges)
        } else {
            self.output_unfactored(graph, conditional_edges)
        };
        // PrintWriter swallows I/O errors (checkError is never called).
        let _ = out.borrow_mut().write_all(text.as_bytes());
        let _ = out.borrow_mut().flush();
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:128 GraphOutputWriter.outputUnfactored
    fn output_unfactored(
        &self,
        graph: &Digraph<T>,
        conditional_edges: Option<&ConditionalEdges>,
    ) -> String {
        struct Visitor<'a, 'r, T> {
            dot: DotOutputVisitor<'a, T>,
            writer: &'a GraphOutputWriter<'r, T>,
            conditional_edges: Option<&'a ConditionalEdges>,
        }
        impl<T: Clone + Eq + Hash + fmt::Debug> GraphVisitor<T> for Visitor<'_, '_, T> {
            fn begin_visit(&mut self) {
                self.dot.begin_visit();
                // TODO(bazel-team): (2009) make this the default in Digraph.
                let line = format!("  node [shape=box];{}", self.dot.line_terminator);
                self.dot.out.push_str(&line);
            }

            fn end_visit(&mut self) {
                self.dot.end_visit();
            }

            fn visit_edge(&mut self, graph: &Digraph<T>, lhs: NodeId, rhs: NodeId) {
                self.dot.visit_edge(graph, lhs, rhs);
                let output_label = self.writer.get_conditions_graph_label(
                    graph,
                    &[lhs],
                    &[rhs],
                    self.conditional_edges,
                );
                if !output_label.is_empty() {
                    let line = format!(
                        "  [label=\"{}\"];{}",
                        output_label, self.dot.line_terminator
                    );
                    self.dot.out.push_str(&line);
                }
            }

            fn visit_node(&mut self, graph: &Digraph<T>, node: NodeId) {
                self.dot.visit_node(graph, node);
            }
        }
        let mut visitor = Visitor {
            dot: DotOutputVisitor::new(
                &self.line_terminator,
                Box::new(|g: &Digraph<T>, n: NodeId| {
                    self.node_reader
                        .get_label(g.get_label(n), &self.label_printer)
                }),
            ),
            writer: self,
            conditional_edges,
        };
        let comparator = |a: &T, b: &T| self.node_reader.comparator(a, b);
        graph.visit_nodes_before_edges(
            &mut visitor,
            if self.sort_labels {
                Some(&comparator)
            } else {
                None
            },
        );
        visitor.dot.out
    }

    /// Given `collection_of_unordered_sets`, a collection of sets of nodes, returns a collection
    /// of sets with the same elements as `collection_of_unordered_sets` but with a stable
    /// iteration order within each set given by the target ordering, and the collection ordered
    /// by the same induced order.
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:160 GraphOutputWriter.orderPartition
    fn order_partition(
        &self,
        graph: &Digraph<T>,
        collection_of_unordered_sets: Vec<Vec<NodeId>>,
    ) -> Vec<Vec<NodeId>> {
        let mut result: Vec<Vec<NodeId>> = Vec::new();
        for part in collection_of_unordered_sets {
            let mut to_sort = part;
            to_sort.sort_by(|a, b| self.node_comparator(graph, *a, *b));
            result.push(to_sort);
        }
        result.sort_by(|a, b| {
            let a: NodeSet = a.clone().into();
            let b: NodeSet = b.clone().into();
            self.node_set_comparator(graph, &a, &b)
        });
        result
    }

    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:172 GraphOutputWriter.outputFactored
    fn output_factored(
        &self,
        graph: &Digraph<T>,
        conditional_edges: Option<&ConditionalEdges>,
    ) -> String {
        let mut partition = self.partition_factored(graph);
        if self.sort_labels {
            partition = self.order_partition(graph, partition);
        }

        let factored_graph: Digraph<NodeSet> = graph.create_image_under_partition(&partition);

        // Concatenate the labels of all topologically-equivalent nodes.
        let label_serializer = |fg: &Digraph<NodeSet>, node: NodeId| -> String {
            let actual_limit = self.max_label_size - RESERVED_LABEL_CHARS;
            let mut first_item = true;
            let mut buf = String::new();
            let mut buf_len: i32 = 0;
            let mut count: i32 = 0;
            let eq_nodes = fg.get_label(node);
            for eq_node in eq_nodes.iter() {
                let label_string = self
                    .node_reader
                    .get_label(graph.get_label(*eq_node), &self.label_printer);
                let label_len = label_string.chars().count() as i32;
                if !first_item {
                    buf.push_str("\\n");
                    buf_len += 2;

                    // Use -1 to denote no limit, as it is easier than trying to pass MAX_INT on
                    // the cmdline
                    if self.max_label_size != -1 && (buf_len + label_len > actual_limit) {
                        buf.push_str("...and ");
                        buf.push_str(&(eq_nodes.len() as i32 - count).to_string());
                        buf.push_str(" more items");
                        break;
                    }
                }

                buf.push_str(&label_string);
                buf_len += label_len;
                count += 1;
                first_item = false;
            }
            buf
        };

        struct Visitor<'a, 'r, T> {
            dot: DotOutputVisitor<'a, NodeSet>,
            writer: &'a GraphOutputWriter<'r, T>,
            graph: &'a Digraph<T>,
            conditional_edges: Option<&'a ConditionalEdges>,
        }
        impl<T: Clone + Eq + Hash + fmt::Debug> GraphVisitor<NodeSet> for Visitor<'_, '_, T> {
            fn begin_visit(&mut self) {
                self.dot.begin_visit();
                // TODO(bazel-team): (2009) make this the default in Digraph.
                let line = format!("  node [shape=box];{}", self.dot.line_terminator);
                self.dot.out.push_str(&line);
            }

            fn end_visit(&mut self) {
                self.dot.end_visit();
            }

            fn visit_edge(&mut self, fg: &Digraph<NodeSet>, lhs: NodeId, rhs: NodeId) {
                self.dot.visit_edge(fg, lhs, rhs);

                let output_label = self.writer.get_conditions_graph_label(
                    self.graph,
                    fg.get_label(lhs),
                    fg.get_label(rhs),
                    self.conditional_edges,
                );
                if !output_label.is_empty() {
                    let line = format!(
                        "  [label=\"{}\"];{}",
                        output_label, self.dot.line_terminator
                    );
                    self.dot.out.push_str(&line);
                }
            }

            fn visit_node(&mut self, fg: &Digraph<NodeSet>, node: NodeId) {
                self.dot.visit_node(fg, node);
            }
        }
        let mut visitor = Visitor {
            dot: DotOutputVisitor::new(&self.line_terminator, Box::new(label_serializer)),
            writer: self,
            graph,
            conditional_edges,
        };
        let comparator = |a: &NodeSet, b: &NodeSet| self.node_set_comparator(graph, a, b);
        factored_graph.visit_nodes_before_edges(
            &mut visitor,
            if self.sort_labels {
                Some(&comparator)
            } else {
                None
            },
        );
        visitor.dot.out
    }

    /// Partitions the graph into equivalence classes of topologically equivalent nodes.
    ///
    /// Algorithm: Visit each node, comparing children with each other based on the eq relation to
    /// put them into their eq classes. Compare top-level nodes as well as though they were
    /// children of a fake root node.
    ///
    /// Invariant: Two nodes are in the same equivalence class -> they have the same parents (and
    /// children).
    ///
    /// Contrapositive: If two nodes do not have the same parents (or children) -> they are not in
    /// the same equivalence class.
    ///
    /// Because of the contrapositive, we only need to compare children nodes of each parent node
    /// (rather than each node with every other node). This allows us to significantly reduce the
    /// number of comparisons between nodes.
    ///
    /// PORT: java-hash-order. Java collects the classes in a `HashMap<Node, Set<Node>>` and each
    /// class in a `HashSet<Node>` (identity hashes); Blaze keeps the classes in the order they
    /// were created and each class in insertion order.
    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:254 GraphOutputWriter.partitionFactored
    fn partition_factored(&self, graph: &Digraph<T>) -> Vec<Vec<NodeId>> {
        // Two nodes are equivalent iff they have the same successors and predecessors.
        let equivalence_relation = |x: NodeId, y: NodeId| -> i32 {
            if x == y {
                return 0;
            }
            let xn = graph.node(x);
            let yn = graph.node(y);
            if xn.num_predecessors() != yn.num_predecessors()
                || xn.num_successors() != yn.num_successors()
            {
                return -1;
            }

            let xpred: HashSet<NodeId> = xn.get_predecessors().iter().copied().collect();
            let ypred: HashSet<NodeId> = yn.get_predecessors().iter().copied().collect();
            if xpred != ypred {
                return -1;
            }

            let xsucc: HashSet<NodeId> = xn.get_successors().iter().copied().collect();
            let ysucc: HashSet<NodeId> = yn.get_successors().iter().copied().collect();
            if xsucc != ysucc {
                return -1;
            }

            0
        };

        // Keep a map of equivalence classes that each node belongs to, so that we know whether a
        // node already belongs to one.
        let mut eq_classes = EqClasses {
            class_of: HashMap::new(),
            classes: Vec::new(),
        };

        // Top-level nodes need to be compared amongst each other because they can form an
        // equivalence class amongst themselves too.
        process_successors(&graph.get_roots(), &mut eq_classes, &equivalence_relation);

        // For each node, compare its children with each other to put them into equivalence
        // classes.
        for node in graph.get_nodes() {
            process_successors(
                graph.get_successors(node),
                &mut eq_classes,
                &equivalence_relation,
            );
        }

        // eqClasses.values().stream().distinct()
        eq_classes.classes
    }

    // Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:344 GraphOutputWriter.getConditionsGraphLabel
    fn get_conditions_graph_label(
        &self,
        graph: &Digraph<T>,
        lhs: &[NodeId],
        rhs: &[NodeId],
        conditional_edges: Option<&ConditionalEdges>,
    ) -> String {
        let mut buf = String::new();
        let Some(conditional_edges) = conditional_edges else {
            return buf;
        };
        if self.max_conditional_edges == 0 {
            return buf;
        }

        let mut annotated_labels: HashSet<Label> = HashSet::new();
        for src in lhs {
            let src_label = self.node_reader.get_build_label(graph.get_label(*src));
            for dest in rhs {
                let dest_label = self.node_reader.get_build_label(graph.get_label(*dest));
                if let Some(conditions) = conditional_edges.get(&src_label, &dest_label) {
                    let mut first_item = true;

                    let limit: i32 = if self.max_conditional_edges == -1 {
                        conditions.len() as i32
                    } else {
                        self.max_conditional_edges - 1
                    };
                    if limit < 0 {
                        panic!("IllegalArgumentException: limit is negative");
                    }

                    for condition_label in conditions.iter().take(limit as usize) {
                        if !annotated_labels.insert(condition_label.clone()) {
                            // duplicate label; skip.
                            continue;
                        }

                        if !first_item {
                            buf.push_str("\\n");
                        }

                        buf.push_str(&condition_label.get_canonical_form());
                        first_item = false;
                    }
                    if conditions.len() as i32 > limit {
                        buf.push_str("...");
                    }
                }
            }
        }
        buf
    }
}

/// `HashMap<Node<T>, Set<Node<T>>> eqClasses` (see `partition_factored`).
struct EqClasses {
    class_of: HashMap<NodeId, usize>,
    classes: Vec<Vec<NodeId>>,
}

/// Compares a list of successors of a parent node amongst each other and adds them to their
/// equivalence classes.
// Java: com/google/devtools/build/lib/query2/query/output/GraphOutputWriter.java:308 GraphOutputWriter.processSuccessors
fn process_successors(
    successors: &[NodeId],
    eq_classes: &mut EqClasses,
    equivalence_relation: &dyn Fn(NodeId, NodeId) -> i32,
) {
    let num_successors = successors.len();

    for i in 0..num_successors {
        let child = successors[i];
        if eq_classes.class_of.contains_key(&child) {
            // This child has already been added to an equivalence class, there is no need to
            // compare because all members in that equivalence class would have already been
            // added.
            continue;
        }

        // Put the child in its own equivalence class and compare with its siblings.
        let class_index = eq_classes.classes.len();
        eq_classes.classes.push(vec![child]);
        eq_classes.class_of.insert(child, class_index);

        // Start at i+1, since j <= i has already been checked.
        for sibling in successors.iter().skip(i + 1) {
            if eq_classes.class_of.contains_key(sibling) {
                // The sibling has already been added to another equivalence class, no need to
                // compare.
                continue;
            }

            // This is expensive, so we want to minimize this as much as possible.
            if equivalence_relation(child, *sibling) == 0 {
                eq_classes.classes[class_index].push(*sibling);
                eq_classes.class_of.insert(*sibling, class_index);
            }
        }
    }
}
