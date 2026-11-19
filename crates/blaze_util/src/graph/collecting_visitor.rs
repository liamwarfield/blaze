//! Port of `lib/graph/CollectingVisitor.java`.

use crate::graph::digraph::Digraph;
use crate::graph::graph_visitor::GraphVisitor;
use crate::graph::node::NodeId;

/// A graph visitor that collects the visited nodes in the order in which they were visited, and
/// allows them to be accessed as a list.
// Java: com/google/devtools/build/lib/graph/CollectingVisitor.java:24 CollectingVisitor
#[derive(Default)]
pub struct CollectingVisitor {
    order: Vec<NodeId>,
}

impl CollectingVisitor {
    pub fn new() -> Self {
        CollectingVisitor { order: Vec::new() }
    }

    /// Returns a reference to (not a copy of) the list of visited nodes in the order they were
    /// visited.
    // Java: com/google/devtools/build/lib/graph/CollectingVisitor.java:36 CollectingVisitor.getVisitedNodes
    pub fn get_visited_nodes(&self) -> &[NodeId] {
        &self.order
    }

    pub fn into_visited_nodes(self) -> Vec<NodeId> {
        self.order
    }
}

impl<T> GraphVisitor<T> for CollectingVisitor {
    // Java: com/google/devtools/build/lib/graph/CollectingVisitor.java:28 CollectingVisitor.visitNode
    fn visit_node(&mut self, _graph: &Digraph<T>, node: NodeId) {
        self.order.push(node);
    }
}
