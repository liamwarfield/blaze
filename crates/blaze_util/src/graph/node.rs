//! Port of `lib/graph/Node.java`.

use std::fmt;

use crate::graph::concurrent_collection_wrapper::ConcurrentCollectionWrapper;

/// The identity of a node of a [`crate::graph::Digraph`] (Java: the `Node<T>` object; nodes are
/// equal only to themselves). Read the node through the graph that owns it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    /// The node's index in its graph's arena (creation order).
    pub fn index(self) -> usize {
        self.0
    }
}

/// A generic directed-graph Node class. Type parameter T is the type of the node's label.
///
/// Each node is identified by a label, which is unique within the graph owning the node.
///
/// Nodes are immutable, that is, their labels cannot be changed. However, their
/// predecessor/successor lists are mutable.
///
/// Nodes cannot be created directly by clients.
///
/// Clients should not confuse nodes belonging to two different graphs! (Use
/// `Digraph.check_node` to catch such errors.) There is no way to find the graph to which a
/// node belongs; it is intentionally not represented in the node.
// Java: com/google/devtools/build/lib/graph/Node.java:36 Node
#[derive(Clone)]
pub struct Node<T> {
    label: T,
    pub(crate) succs: ConcurrentCollectionWrapper<NodeId>,
    pub(crate) preds: ConcurrentCollectionWrapper<NodeId>,
}

impl<T> Node<T> {
    /// Only Digraph.createNode() can call this!
    // Java: com/google/devtools/build/lib/graph/Node.java:49 Node.<init>
    pub(crate) fn new(label: T) -> Self {
        Node {
            label,
            succs: ConcurrentCollectionWrapper::new(),
            preds: ConcurrentCollectionWrapper::new(),
        }
    }

    /// Returns the label for this node.
    // Java: com/google/devtools/build/lib/graph/Node.java:56 Node.getLabel
    pub fn get_label(&self) -> &T {
        &self.label
    }

    /// Returns the successors of this node (Java: an unmodifiable view).
    // Java: com/google/devtools/build/lib/graph/Node.java:63 Node.getSuccessors
    pub fn get_successors(&self) -> &[NodeId] {
        self.succs.get()
    }

    /// Equivalent to `get_successors().len()` but possibly more efficient.
    // Java: com/google/devtools/build/lib/graph/Node.java:87 Node.numSuccessors
    pub fn num_successors(&self) -> usize {
        self.succs.size()
    }

    /// Returns the predecessors of this node (Java: an unmodifiable view).
    // Java: com/google/devtools/build/lib/graph/Node.java:95 Node.getPredecessors
    pub fn get_predecessors(&self) -> &[NodeId] {
        self.preds.get()
    }

    /// Returns true iff this node has predecessors.
    // Java: com/google/devtools/build/lib/graph/Node.java:120 Node.hasPredecessors
    pub fn has_predecessors(&self) -> bool {
        !self.preds.get().is_empty()
    }

    /// Equivalent to `get_predecessors().len()` but possibly more efficient.
    // Java: com/google/devtools/build/lib/graph/Node.java:125 Node.numPredecessors
    pub fn num_predecessors(&self) -> usize {
        self.preds.size()
    }
}

impl<T: fmt::Display> fmt::Display for Node<T> {
    // Java: com/google/devtools/build/lib/graph/Node.java:193 Node.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "node:{}", self.label)
    }
}
