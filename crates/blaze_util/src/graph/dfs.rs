//! Port of `lib/graph/DFS.java`.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use crate::graph::digraph::{Comparator, Digraph};
use crate::graph::graph_visitor::GraphVisitor;
use crate::graph::node::NodeId;

/// Java `DFS.Order`.
// Java: com/google/devtools/build/lib/graph/DFS.java:45 DFS.Order
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DfsOrder {
    Preorder,
    Postorder,
}

/// The DFS class encapsulates a depth-first search visitation, including the order in which
/// nodes are to be visited relative to their successors (PREORDER/POSTORDER), whether the
/// forward or transposed graph is to be used, and which nodes have been seen already.
///
/// A variety of common uses of DFS are offered through methods of Digraph; however clients can
/// use this class directly for maximum flexibility.
///
/// Clients should not modify the enclosing Digraph instance of a DFS while a traversal is in
/// progress.
// Java: com/google/devtools/build/lib/graph/DFS.java:42 DFS
pub struct Dfs<'c, T> {
    order: DfsOrder,
    edge_order: Option<Comparator<'c, T>>,
    transpose: bool,
    /// Java `CompactHashSet` (iterates in insertion order).
    marked: Vec<NodeId>,
    marked_set: HashSet<NodeId>,
}

impl<'c, T: Clone + Eq + Hash + fmt::Debug> Dfs<'c, T> {
    /// Constructs a DFS instance for searching over the enclosing Digraph instance, using the
    /// specified visitation parameters.
    ///
    /// `edge_order`: an ordering in which the edges originating from the same node should be
    /// visited (if `None`, the order is the successor order).
    // Java: com/google/devtools/build/lib/graph/DFS.java:68 DFS.<init>
    pub fn new(order: DfsOrder, edge_order: Option<Comparator<'c, T>>, transpose: bool) -> Self {
        Dfs {
            order,
            edge_order,
            transpose,
            marked: Vec::new(),
            marked_set: HashSet::new(),
        }
    }

    /// Returns the (immutable) set of nodes visited so far.
    // Java: com/google/devtools/build/lib/graph/DFS.java:82 DFS.getMarked
    pub fn get_marked(&self) -> &[NodeId] {
        &self.marked
    }

    pub(crate) fn into_marked(self) -> Vec<NodeId> {
        self.marked
    }

    // Java: com/google/devtools/build/lib/graph/DFS.java:86 DFS.visit
    pub fn visit(&mut self, graph: &Digraph<T>, node: NodeId, visitor: &mut dyn GraphVisitor<T>) {
        if !self.marked_set.insert(node) {
            return;
        }
        self.marked.push(node);

        if self.order == DfsOrder::Preorder {
            visitor.visit_node(graph, node);
        }

        let mut edge_targets: Vec<NodeId> = if self.transpose {
            graph.get_predecessors(node).to_vec()
        } else {
            graph.get_successors(node).to_vec()
        };
        if let Some(edge_order) = self.edge_order {
            edge_targets.sort_by(|a, b| edge_order(graph.get_label(*a), graph.get_label(*b)));
        }

        for v in edge_targets {
            self.visit(graph, v, visitor);
        }

        if self.order == DfsOrder::Postorder {
            visitor.visit_node(graph, node);
        }
    }
}
