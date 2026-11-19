//! Ports of `lib/graph/GraphVisitor.java` and `lib/graph/AbstractGraphVisitor.java`.

use crate::graph::digraph::Digraph;
use crate::graph::node::NodeId;

/// An graph visitor interface; particularly useful for allowing subclasses to specify how to
/// output a graph. The order in which node and edge callbacks are made (DFS, BFS, etc) is
/// defined by the choice of Digraph visitation method used.
///
/// PORT: the default methods are `AbstractGraphVisitor`'s (do nothing). Each callback also
/// receives the graph, through which the visitor reads the node.
// Java: com/google/devtools/build/lib/graph/GraphVisitor.java:24 GraphVisitor
pub trait GraphVisitor<T> {
    /// Called before visitation commences.
    // Java: com/google/devtools/build/lib/graph/AbstractGraphVisitor.java:24 AbstractGraphVisitor.beginVisit
    fn begin_visit(&mut self) {}

    /// Called after visitation is complete.
    // Java: com/google/devtools/build/lib/graph/AbstractGraphVisitor.java:26 AbstractGraphVisitor.endVisit
    fn end_visit(&mut self) {}

    /// Called for each edge.
    // Java: com/google/devtools/build/lib/graph/AbstractGraphVisitor.java:28 AbstractGraphVisitor.visitEdge
    fn visit_edge(&mut self, _graph: &Digraph<T>, _lhs: NodeId, _rhs: NodeId) {}

    /// Called for each node.
    // Java: com/google/devtools/build/lib/graph/AbstractGraphVisitor.java:30 AbstractGraphVisitor.visitNode
    fn visit_node(&mut self, _graph: &Digraph<T>, _node: NodeId) {}
}

/// A stub implementation of GraphVisitor providing default behaviour (do nothing) for all its
/// methods.
// Java: com/google/devtools/build/lib/graph/AbstractGraphVisitor.java:22 AbstractGraphVisitor
pub struct AbstractGraphVisitor;

impl<T> GraphVisitor<T> for AbstractGraphVisitor {}
