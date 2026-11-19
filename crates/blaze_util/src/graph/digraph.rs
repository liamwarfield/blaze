//! Port of `lib/graph/Digraph.java`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use crate::graph::collecting_visitor::CollectingVisitor;
use crate::graph::dfs::{Dfs, DfsOrder};
use crate::graph::graph_visitor::{AbstractGraphVisitor, GraphVisitor};
use crate::graph::java_priority_queue::JavaPriorityQueue;
use crate::graph::node::{Node, NodeId};

/// Java `Comparator<? super T>`.
pub type Comparator<'a, T> = &'a dyn Fn(&T, &T) -> Ordering;

/// The label of a node of an image graph under a partition (Java `ImmutableSet<Node<T>>`): the
/// nodes of one part, in the part's order.
pub type NodeSet = Arc<[NodeId]>;

/// `Digraph` a generic directed graph or "digraph", suitable for modeling asymmetric binary
/// relations.
///
/// An instance `G = <V,E>` consists of a set of nodes or vertices `V`, and a set of directed
/// edges `E`, which is a subset of `V × V`. This permits self-edges but does not represent
/// multiple edges between the same pair of nodes.
///
/// Nodes may be labeled with values of any type (type parameter T). All nodes within a graph
/// have distinct labels. The null pointer is not a valid label.
///
/// The package supports various operations for modeling partial order relations, and supports
/// input/output in AT&T's 'dot' format. See <http://www.research.att.com/sw/tools/graphviz/>.
///
/// Some invariants:
/// - Each graph instances "owns" the nodes is creates. The behaviour of operations on nodes a
///   graph does not own is undefined.
/// - `Digraph` assumes immutability of node labels, much like `HashMap` assumes it for keys.
/// - Mutating the underlying graph invalidates any sets and iterators backed by it.
///
/// PORT: java-hash-order. Java keeps the nodes in a `ConcurrentHashMap<T, Node<T>>`, so
/// `getNodes()`, `getLabels()` and every algorithm that starts from them iterate in the keys'
/// hash order. The labels query puts in a graph (`Target`s, sets of nodes) hash by object
/// identity, so that order differs from one Bazel server to the next (the oracle prints
/// `query --output=graph` in a different order after each restart). Blaze iterates the nodes in
/// creation order (the arena order); the orders that are deterministic in Java (anything sorted
/// with a comparator, successor lists, DFS visitation from a given order) are reproduced
/// exactly.
// Java: com/google/devtools/build/lib/graph/Digraph.java:69 Digraph
pub struct Digraph<T> {
    /// Maps labels to their nodes (Java `nodes`); the arena holds the nodes by id.
    index: HashMap<T, NodeId>,
    arena: Vec<Option<Node<T>>>,
}

impl<T: Clone + Eq + Hash + fmt::Debug> Default for Digraph<T> {
    fn default() -> Self {
        Digraph::new()
    }
}

impl<T> fmt::Display for Digraph<T> {
    // Java: com/google/devtools/build/lib/graph/Digraph.java:132 Digraph.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digraph[{} nodes]", self.index.len())
    }
}

impl<T: Clone + Eq + Hash + fmt::Debug> Digraph<T> {
    /// Construct an empty Digraph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:80 Digraph.<init>
    pub fn new() -> Self {
        Digraph {
            index: HashMap::new(),
            arena: Vec::new(),
        }
    }

    /// Sanity-check: assert that a node is indeed a member of this graph and not another one.
    /// Perhaps this should go in a subclass, "DebuggingDigraph".
    // Java: com/google/devtools/build/lib/graph/Digraph.java:86 Digraph.checkNode
    fn check_node(&self, node: NodeId) {
        let ok = self
            .arena
            .get(node.0)
            .and_then(|n| n.as_ref())
            .is_some_and(|n| self.index.get(n.get_label()) == Some(&node));
        if !ok {
            panic!(
                "IllegalArgumentException: node {:?} is not a member of this graph",
                node
            );
        }
    }

    /// The node with the given id (Java: the `Node<T>` reference itself).
    pub fn node(&self, node: NodeId) -> &Node<T> {
        self.arena[node.0]
            .as_ref()
            .expect("node is not a member of this graph")
    }

    fn node_mut(&mut self, node: NodeId) -> &mut Node<T> {
        self.arena[node.0]
            .as_mut()
            .expect("node is not a member of this graph")
    }

    /// Java `node.getLabel()`.
    pub fn get_label(&self, node: NodeId) -> &T {
        self.node(node).get_label()
    }

    /// Java `node.getSuccessors()`.
    pub fn get_successors(&self, node: NodeId) -> &[NodeId] {
        self.node(node).get_successors()
    }

    /// Java `node.getPredecessors()`.
    pub fn get_predecessors(&self, node: NodeId) -> &[NodeId] {
        self.node(node).get_predecessors()
    }

    /// Adds a directed edge between the nodes labelled 'from' and 'to', creating them if
    /// necessary.
    ///
    /// Returns true iff the edge was not already present.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:99 Digraph.addEdge
    pub fn add_edge(&mut self, from: T, to: T) -> bool {
        let from_node = self.create_node(from);
        let to_node = self.create_node(to);
        self.add_edge_nodes(from_node, to_node)
    }

    /// Adds a directed edge between the specified nodes, which must exist and belong to this
    /// graph.
    ///
    /// Returns true iff the edge was not already present.
    ///
    /// Note: multi-edges are ignored. Self-edges are permitted.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:113 Digraph.addEdge
    pub fn add_edge_nodes(&mut self, from_node: NodeId, to_node: NodeId) -> bool {
        self.check_node(from_node);
        self.check_node(to_node);
        self.node_add_edge(from_node, to_node)
    }

    /// Removes the edge between the specified nodes. Idempotent: attempts to remove non-existent
    /// edges have no effect.
    ///
    /// Returns true iff graph changed.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:125 Digraph.removeEdge
    pub fn remove_edge(&mut self, from_node: NodeId, to_node: NodeId) -> bool {
        self.check_node(from_node);
        self.check_node(to_node);
        self.node_remove_edge(from_node, to_node)
    }

    /// Adds 'value' to either the predecessor or successor set, updating the appropriate field
    /// as necessary.
    // Java: com/google/devtools/build/lib/graph/Node.java:138 Node.addEdge
    fn node_add_edge(&mut self, this: NodeId, target: NodeId) -> bool {
        let is_new_successor = self.node_mut(this).succs.add(target);
        // Java: com/google/devtools/build/lib/graph/Node.java:176 Node.addPredecessor
        let is_new_predecessor = self.node_mut(target).preds.add(this);
        if is_new_predecessor != is_new_successor {
            panic!("IllegalStateException: inconsistent graph state");
        }
        is_new_successor
    }

    /// Removes edge from 'this' to 'target'.
    // Java: com/google/devtools/build/lib/graph/Node.java:158 Node.removeEdge
    fn node_remove_edge(&mut self, this: NodeId, target: NodeId) -> bool {
        let is_successor_removed = self.node_mut(this).succs.remove(target);
        if is_successor_removed {
            // Java: com/google/devtools/build/lib/graph/Node.java:184 Node.removePredecessor
            let is_predecessor_removed = self.node_mut(target).preds.remove(this);
            if !is_predecessor_removed {
                panic!("IllegalStateException: inconsistent graph state");
            }
            return true;
        }
        false
    }

    /// Removes all edges to/from this node. Private: should only be called by Digraph.
    // Java: com/google/devtools/build/lib/graph/Node.java:73 Node.removeAllSuccessors
    fn node_remove_all_successors(&mut self, this: NodeId) -> Vec<NodeId> {
        self.node_remove_edge(this, this); // remove self edge
        let successors = self.node_mut(this).succs.clear();
        for s in &successors {
            // Java: com/google/devtools/build/lib/graph/Node.java:184 Node.removePredecessor
            if !self.node_mut(*s).preds.remove(this) {
                panic!("IllegalStateException: inconsistent graph state");
            }
        }
        successors
    }

    /// Removes all edges to/from this node. Private: should only be called by Digraph.
    // Java: com/google/devtools/build/lib/graph/Node.java:105 Node.removeAllPredecessors
    fn node_remove_all_predecessors(&mut self, this: NodeId) -> Vec<NodeId> {
        self.node_remove_edge(this, this); // remove self edge
        let predecessors = self.node_mut(this).preds.clear();
        for p in &predecessors {
            // Java: com/google/devtools/build/lib/graph/Node.java:188 Node.removeSuccessor
            if !self.node_mut(*p).succs.remove(this) {
                panic!("IllegalStateException: inconsistent graph state");
            }
        }
        predecessors
    }

    /// Returns true iff this graph is a subgraph of the argument. This means that this graph's
    /// nodes are a subset of those of the argument; moreover, for each node of this graph, its
    /// set of successors is a subset of those of the corresponding node in the argument graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:164 Digraph.isSubgraph
    pub fn is_subgraph(&self, that: &Digraph<T>) -> bool {
        if std::ptr::eq(self, that) {
            return true;
        }
        if self.get_node_count() > that.get_node_count() {
            return false;
        }
        for n1 in self.get_nodes() {
            let Some(n2) = that.get_node_maybe(self.get_label(n1)) else {
                return false; // 'that' is missing a node
            };
            let n1succs = self.get_successors(n1);
            let n2succs = that.get_successors(n2);
            if n1succs.len() > n2succs.len() {
                return false;
            }
            for succ1 in n1succs {
                let Some(succ2) = that.get_node_maybe(self.get_label(*succ1)) else {
                    return false;
                };
                if !n2succs.contains(&succ2) {
                    return false;
                }
            }
        }
        true
    }

    /// Returns true iff this graph is equal to `that` (both are subgraphs of the other).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:150 Digraph.equals
    pub fn equals(&self, that: &Digraph<T>) -> bool {
        // If this graph is a subgraph of thatObject, then we know that thatObject is of type
        // Digraph<?> and thatObject can be cast to this type.
        self.is_subgraph(that) && that.is_subgraph(self)
    }

    /// Returns a duplicate graph with the same set of node labels and edges.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:213 Digraph.clone
    pub fn clone_graph(&self) -> Digraph<T> {
        struct CloneVisitor<T> {
            that: Digraph<T>,
        }
        impl<T: Clone + Eq + Hash + fmt::Debug> GraphVisitor<T> for CloneVisitor<T> {
            fn visit_edge(&mut self, graph: &Digraph<T>, lhs: NodeId, rhs: NodeId) {
                self.that
                    .add_edge(graph.get_label(lhs).clone(), graph.get_label(rhs).clone());
            }

            fn visit_node(&mut self, graph: &Digraph<T>, node: NodeId) {
                self.that.create_node(graph.get_label(node).clone());
            }
        }
        let mut visitor = CloneVisitor {
            that: Digraph::new(),
        };
        self.visit_nodes_before_edges_from(&mut visitor, &self.get_nodes(), None);
        visitor.that
    }

    /// Returns a set of all nodes in the graph, sorted by `comparator` on their labels.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:233 Digraph.getNodes
    pub fn get_nodes_sorted(&self, comparator: Comparator<'_, T>) -> Vec<NodeId> {
        let mut nodes = self.get_nodes();
        nodes.sort_by(|a, b| comparator(self.get_label(*a), self.get_label(*b)));
        nodes
    }

    /// Returns the set of nodes in this graph (see the type's PORT note on the order).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:243 Digraph.getNodes
    pub fn get_nodes(&self) -> Vec<NodeId> {
        self.arena
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_some())
            .map(|(i, _)| NodeId(i))
            .collect()
    }

    /// Returns the set of root nodes: those with no predecessors.
    ///
    /// NOTE: in a cyclic graph, there may be nodes that are not reachable from any "root".
    ///
    /// PORT: java-hash-order. Java returns a `HashSet` of nodes (identity hash); Blaze returns
    /// them in node order.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:253 Digraph.getRoots
    pub fn get_roots(&self) -> Vec<NodeId> {
        let mut roots = Vec::new();
        for node in self.get_nodes() {
            if !self.node(node).has_predecessors() {
                roots.push(node);
            }
        }
        roots
    }

    /// Returns the set of labels of nodes in this graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:266 Digraph.getLabels
    pub fn get_labels(&self) -> Vec<&T> {
        self.arena
            .iter()
            .filter_map(|n| n.as_ref().map(|n| n.get_label()))
            .collect()
    }

    /// Finds and returns the node with the specified label. If there is no such node, an
    /// exception is thrown. The null pointer is not a valid label.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:278 Digraph.getNode
    pub fn get_node(&self, label: &T) -> NodeId {
        match self.index.get(label) {
            Some(node) => *node,
            None => panic!("IllegalArgumentException: No such node label: {label:?}"),
        }
    }

    /// Find the node with the specified label. Returns `None` if it doesn't exist. The null
    /// pointer is not a valid label.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:295 Digraph.getNodeMaybe
    pub fn get_node_maybe(&self, label: &T) -> Option<NodeId> {
        self.index.get(label).copied()
    }

    /// Returns the number of nodes in the graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:305 Digraph.getNodeCount
    pub fn get_node_count(&self) -> usize {
        self.index.len()
    }

    /// Returns the number of edges in the graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:314 Digraph.getEdgeCount
    pub fn get_edge_count(&self) -> usize {
        let mut edges = 0;
        for node in self.get_nodes() {
            edges += self.get_successors(node).len();
        }
        edges
    }

    /// Find or create a node with the specified label. This is the only factory of Nodes. The
    /// null pointer is not a valid label.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:326 Digraph.createNode
    pub fn create_node(&mut self, label: T) -> NodeId {
        if let Some(node) = self.index.get(&label) {
            return *node;
        }
        // Java: com/google/devtools/build/lib/graph/Digraph.java:330 Digraph.createNodeNative
        let id = NodeId(self.arena.len());
        self.arena.push(Some(Node::new(label.clone())));
        self.index.insert(label, id);
        id
    }

    /// Returns the strong component graph of "this". That is, returns a new acyclic graph in
    /// which all strongly-connected components in the original graph have been "fused" into a
    /// single node.
    ///
    /// Returns a new graph, which is the strong-component graph of this.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:344 Digraph.getStrongComponentGraph
    pub fn get_strong_component_graph(&self) -> Digraph<NodeSet> {
        let sccs = self.get_strongly_connected_components();
        let mut sc_graph = self.create_image_under_partition(&sccs);
        sc_graph.remove_self_edges(); // scGraph should be acyclic: no self-edges
        sc_graph
    }

    /// Returns a partition of the nodes of this graph into sets, each set being one
    /// strongly-connected component of the graph.
    ///
    /// PORT: java-hash-order. Each component is a `HashSet` of nodes in Java; Blaze keeps the
    /// order in which the nodes leave the DFS stack.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:355 Digraph.getStronglyConnectedComponents
    pub fn get_strongly_connected_components(&self) -> Vec<Vec<NodeId>> {
        let mut sccs: Vec<Vec<NodeId>> = Vec::new();
        let mut v = SccVisitor::new();
        for node in self.get_nodes() {
            v.visit(self, &mut |scc| sccs.push(scc), node);
        }
        sccs
    }

    /// Returns the image of this graph in a given function, expressed as a mapping from labels
    /// to some other domain.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:380 Digraph.createImageUnderPartition
    pub fn create_image_under_partition(&self, partition: &[Vec<NodeId>]) -> Digraph<NodeSet> {
        let mut label_to_image: HashMap<T, NodeSet> = HashMap::new();
        for set in partition {
            // ImmutableSet.copyOf(set)
            let mut image: Vec<NodeId> = Vec::with_capacity(set.len());
            for node in set {
                if !image.contains(node) {
                    image.push(*node);
                }
            }
            let image_set: NodeSet = image.into();
            for node in image_set.iter() {
                label_to_image.insert(self.get_label(*node).clone(), image_set.clone());
            }
        }

        if label_to_image.len() != self.get_node_count() {
            panic!(
                "IllegalArgumentException: createImageUnderPartition(): argument is not a partition"
            );
        }

        self.create_image_under_mapping(&label_to_image)
    }

    /// Returns the image of this graph in a given function, expressed as a mapping from labels
    /// to some other domain.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:405 Digraph.createImageUnderMapping
    pub fn create_image_under_mapping<ImageT: Clone + Eq + Hash + fmt::Debug>(
        &self,
        map: &HashMap<T, ImageT>,
    ) -> Digraph<ImageT> {
        let mut image_graph: Digraph<ImageT> = Digraph::new();

        for from_node in self.get_nodes() {
            let from_label = self.get_label(from_node);

            let Some(from_image) = map.get(from_label) else {
                panic!(
                    "IllegalArgumentException: Incomplete function: undefined for {from_label:?}"
                );
            };
            image_graph.create_node(from_image.clone());

            for to_node in self.get_successors(from_node) {
                let to_label = self.get_label(*to_node);

                let Some(to_image) = map.get(to_label) else {
                    panic!(
                        "IllegalArgumentException: Incomplete function: undefined for {to_label:?}"
                    );
                };
                image_graph.add_edge(from_image.clone(), to_image.clone());
            }
        }

        image_graph
    }

    /// Removes any self-edges (x,x) in this graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:436 Digraph.removeSelfEdges
    pub fn remove_self_edges(&mut self) {
        for node in self.get_nodes() {
            self.remove_edge(node, node);
        }
    }

    /// Finds the shortest directed path from "fromNode" to "toNode". The path is returned as an
    /// ordered list of nodes, including both endpoints. Returns `None` if there is no path. Uses
    /// breadth-first search. Running time is O(n).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:448 Digraph.getShortestPath
    pub fn get_shortest_path(&self, from_node: NodeId, to_node: NodeId) -> Option<Vec<NodeId>> {
        self.check_node(from_node);
        self.check_node(to_node);

        if from_node == to_node {
            return Some(vec![from_node]);
        }

        let mut path_predecessor: HashMap<NodeId, NodeId> = HashMap::new();

        let mut marked: HashSet<NodeId> = HashSet::new();

        let mut queue: VecDeque<NodeId> = VecDeque::new();
        queue.push_back(from_node);
        marked.insert(from_node);

        while let Some(u) = queue.pop_front() {
            for v in self.get_successors(u) {
                if marked.insert(*v) {
                    path_predecessor.insert(*v, u);
                    if *v == to_node {
                        return Some(get_path_to_tree_node(&path_predecessor, *v)); // found a path
                    }
                    queue.push_back(*v);
                }
            }
        }
        None // no path
    }

    /// Returns the nodes of an acyclic graph in topological order (the reverse of postorder).
    ///
    /// There may be many possible topological orders consistent with a given graph; the one
    /// returned depends on the order of the nodes and of each node's successors.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:504 Digraph.getTopologicalOrder
    pub fn get_topological_order(&self) -> Vec<NodeId> {
        let mut order = self.get_postorder();
        order.reverse();
        order
    }

    /// Returns an immutable topological ordering of the graph, using the specified ordering of
    /// edges (and the same ordering of the nodes from which the DFS starts).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:527 Digraph.getTopologicalOrder
    pub fn get_topological_order_by(&self, edge_order: Comparator<'_, T>) -> Vec<NodeId> {
        let mut visitor = CollectingVisitor::new();
        let mut visitation = Dfs::new(DfsOrder::Postorder, Some(edge_order), false);
        GraphVisitor::<T>::begin_visit(&mut visitor);
        for node in self.get_nodes_sorted(edge_order) {
            visitation.visit(self, node, &mut visitor);
        }
        GraphVisitor::<T>::end_visit(&mut visitor);

        let mut order = visitor.into_visited_nodes();
        order.reverse();
        order
    }

    /// Returns the nodes of this graph in postorder (from each node in node order).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:544 Digraph.getPostorder
    pub fn get_postorder(&self) -> Vec<NodeId> {
        let mut collecting_visitor = CollectingVisitor::new();
        self.visit_postorder(&mut collecting_visitor);
        collecting_visitor.into_visited_nodes()
    }

    /// Returns the set of nodes reachable from the given start nodes (including them), in the
    /// order of first visit (Java's `CompactHashSet` iterates in insertion order).
    // Java: com/google/devtools/build/lib/graph/Digraph.java:554 Digraph.getFwdReachable
    pub fn get_fwd_reachable(&self, start_nodes: &[NodeId]) -> Vec<NodeId> {
        let mut dfs = Dfs::new(DfsOrder::Preorder, None, false);
        for n in start_nodes {
            dfs.visit(self, *n, &mut AbstractGraphVisitor);
        }
        dfs.into_marked()
    }

    /// Removes the node in the graph specified by the given label. Optionally, preserves the
    /// graph order (by connecting up the broken edges) or drop them all. If the specified label
    /// is not the label of any node in the graph, does nothing.
    ///
    /// Returns the neighbours of the node if `preserve_order`, otherwise an empty list.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:588 Digraph.removeNode
    pub fn remove_node(&mut self, node: NodeId, preserve_order: bool) -> Vec<NodeId> {
        self.check_node(node);

        let predecessors = self.node_remove_all_predecessors(node);
        let successors = self.node_remove_all_successors(node);

        let mut neighbours: Vec<NodeId> = Vec::new();

        if preserve_order {
            neighbours.reserve(successors.len() + predecessors.len());
            neighbours.extend_from_slice(&successors);
            neighbours.extend_from_slice(&predecessors);

            for p in &predecessors {
                for s in &successors {
                    self.node_add_edge(*p, *s);
                }
            }
        }

        let label = self.get_label(node).clone();
        let del = self.index.remove(&label);
        if del != Some(node) {
            panic!("IllegalStateException: {del:?} {node:?}");
        }
        self.arena[node.0] = None;

        neighbours
    }

    /// Extracts the subgraph G' of this graph G, containing exactly the nodes specified by the
    /// labels in V', and preserving the original <i>transitive</i> graph relation among those
    /// nodes.
    ///
    /// `keep` decides whether a label is in `subset`, a subset of the labels of this graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:624 Digraph.extractSubgraph
    pub fn extract_subgraph(&self, keep: &dyn Fn(&T) -> bool) -> Digraph<T> {
        let mut subgraph = self.clone_graph();
        subgraph.subgraph(keep);
        subgraph
    }

    /// Removes all nodes from this graph except those whose label is in keepLabels. The graph
    /// relation is preserved, i.e. if two nodes in keepLabels are connected by a path through
    /// removed nodes, they remain connected.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:637 Digraph.subgraph
    fn subgraph(&mut self, keep_labels: &dyn Fn(&T) -> bool) {
        // This algorithm does the following:
        // Initially, keepNeighbors contains all nodes that are reachable from (and to) the
        // keepLabels nodes, in one step. It removes the nodes in that set in an order sorted by
        // the number of edges that the removal adds (approximated by
        // numPredecessors * numSuccessors) and adds the neighbours of each removed node to the
        // set. It then removes the remaining nodes that are not in keepLabels in any order.
        //
        // PORT: java-hash-order. `toRemove` and `keepNeighbors` are `HashSet`s of nodes
        // (identity hash); Blaze keeps them in node order.
        let mut to_remove: Vec<NodeId> = Vec::new();
        let mut to_remove_set: HashSet<NodeId> = HashSet::new();
        let mut keep_neighbors: Vec<NodeId> = Vec::new();
        let mut keep_neighbors_set: HashSet<NodeId> = HashSet::new();

        for node in self.get_nodes() {
            if keep_labels(self.get_label(node)) {
                for n in self
                    .get_predecessors(node)
                    .iter()
                    .chain(self.get_successors(node))
                {
                    if keep_neighbors_set.insert(*n) {
                        keep_neighbors.push(*n);
                    }
                }
            } else {
                to_remove.push(node);
                to_remove_set.insert(node);
            }
        }

        if to_remove.is_empty() {
            return;
        }

        // keepNeighbors.retainAll(toRemove)
        keep_neighbors.retain(|n| to_remove_set.contains(n));
        // The comparator reads the live edge counts, as Java's does.
        let mut reachables: JavaPriorityQueue<NodeId> = JavaPriorityQueue::new();
        // reachables.addAll(keepNeighbors)
        for n in &keep_neighbors {
            reachables.offer(*n, &|a, b| self.compare_removal_cost(a, b));
        }
        // toRemove.removeAll(reachables)
        for n in &keep_neighbors {
            to_remove_set.remove(n);
        }

        while let Some(node) = {
            let graph = &*self;
            reachables.poll(&|a, b| graph.compare_removal_cost(a, b))
        } {
            let neighbours = self.remove_node(node, /* preserve_order= */ true);

            for neighbour in neighbours {
                if to_remove_set.remove(&neighbour) {
                    let graph = &*self;
                    reachables.offer(neighbour, &|a, b| graph.compare_removal_cost(a, b));
                }
            }
        }

        for node in to_remove {
            if to_remove_set.contains(&node) {
                self.remove_node(node, false);
            }
        }
    }

    /// `comparingLong(arg -> (long) arg.numPredecessors() * (long) arg.numSuccessors())`.
    fn compare_removal_cost(&self, a: &NodeId, b: &NodeId) -> Ordering {
        let cost = |n: &NodeId| {
            let node = self.node(*n);
            (node.num_predecessors() as i64) * (node.num_successors() as i64)
        };
        cost(a).cmp(&cost(b))
    }

    /// Perform a postorder depth-first search, invoking the visitor for each node, starting from
    /// the specified nodes.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:819 Digraph.visitPostorder
    pub fn visit_postorder_from(&self, visitor: &mut dyn GraphVisitor<T>, start_nodes: &[NodeId]) {
        self.visit_depth_first(visitor, DfsOrder::Postorder, false, start_nodes);
    }

    /// Equivalent to `visit_postorder_from(visitor, get_nodes())`.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:827 Digraph.visitPostorder
    pub fn visit_postorder(&self, visitor: &mut dyn GraphVisitor<T>) {
        self.visit_postorder_from(visitor, &self.get_nodes());
    }

    /// Performs a depth-first search of this graph.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:837 Digraph.visitDepthFirst
    pub fn visit_depth_first(
        &self,
        visitor: &mut dyn GraphVisitor<T>,
        order: DfsOrder,
        transpose: bool,
        start_nodes: &[NodeId],
    ) {
        let mut visitation = Dfs::new(order, None, transpose);
        visitor.begin_visit();
        for node in start_nodes {
            visitation.visit(self, *node, visitor);
        }
        visitor.end_visit();
    }

    // Java: com/google/devtools/build/lib/graph/Digraph.java:859 Digraph.maybeOrderCollection
    fn maybe_order_collection(
        &self,
        unordered: &[NodeId],
        comparator: Option<Comparator<'_, T>>,
    ) -> Vec<NodeId> {
        let mut nodes = unordered.to_vec();
        if let Some(comparator) = comparator {
            // Java: com/google/devtools/build/lib/graph/Digraph.java:849 Digraph.makeNodeComparator
            nodes.sort_by(|a, b| comparator(self.get_label(*a), self.get_label(*b)));
        }
        nodes
    }

    // Java: com/google/devtools/build/lib/graph/Digraph.java:866 Digraph.visitNodesBeforeEdges
    fn visit_nodes_before_edges_from(
        &self,
        visitor: &mut dyn GraphVisitor<T>,
        start_nodes: &[NodeId],
        comparator: Option<Comparator<'_, T>>,
    ) {
        visitor.begin_visit();
        for from_node in start_nodes {
            visitor.visit_node(self, *from_node);
            for to_node in self.maybe_order_collection(self.get_successors(*from_node), comparator)
            {
                visitor.visit_edge(self, *from_node, to_node);
            }
        }
        visitor.end_visit();
    }

    /// A visitation over the graph that visits all nodes and edges in topological order such
    /// that each node is visited before any edge coming out of that node; ties among nodes are
    /// broken using the provided `comparator` if not `None`; edges are visited in order specified
    /// by the comparator, **not** topological order of the target nodes.
    // Java: com/google/devtools/build/lib/graph/Digraph.java:886 Digraph.visitNodesBeforeEdges
    pub fn visit_nodes_before_edges(
        &self,
        visitor: &mut dyn GraphVisitor<T>,
        comparator: Option<Comparator<'_, T>>,
    ) {
        let start_nodes = match comparator {
            None => self.get_topological_order(),
            Some(comparator) => self.get_topological_order_by(comparator),
        };
        self.visit_nodes_before_edges_from(visitor, &start_nodes, comparator);
    }
}

/// Given a tree (expressed as a map from each node to its parent), and a starting node, returns
/// the path from the root of the tree to 'node' as a list.
// Java: com/google/devtools/build/lib/graph/Digraph.java:484 Digraph.getPathToTreeNode
pub fn get_path_to_tree_node<X: Copy + Eq + Hash>(tree: &HashMap<X, X>, node: X) -> Vec<X> {
    let mut path = Vec::new();
    let mut node = Some(node);
    while let Some(n) = node {
        path.push(n);
        node = tree.get(&n).copied(); // get parent
    }
    path.reverse();
    path
}

/// An algorithm for computing the partition of a graph into strongly-connected components.
///
/// The idea is as follows: we perform a DFS over the graph, maintaining a stack of the nodes
/// visited so far and their preorder numbers. When a node finishes, if it is the root of a
/// strongly-connected component (no edge led back above it), it pops the component's nodes.
// Java: com/google/devtools/build/lib/graph/Digraph.java:733 Digraph.SccVisitor
struct SccVisitor {
    /// Set of nodes that have been assigned to a SCC.
    assigned: HashSet<NodeId>,
    /// Map from each visited node to its preorder number.
    preorder: HashMap<NodeId, i32>,
    /// Stack of visited nodes.
    stack: Vec<NodeId>,
    /// Stack of preorder numbers for the nodes on `stack` that may be roots of a component.
    preorder_stack: Vec<i32>,
    counter: i32,
}

impl SccVisitor {
    fn new() -> Self {
        SccVisitor {
            assigned: HashSet::new(),
            preorder: HashMap::new(),
            stack: Vec::new(),
            preorder_stack: Vec::new(),
            counter: 0,
        }
    }

    // Java: com/google/devtools/build/lib/graph/Digraph.java:762 Digraph.SccVisitor.visit
    fn visit<T: Clone + Eq + Hash + fmt::Debug>(
        &mut self,
        graph: &Digraph<T>,
        visitor: &mut dyn FnMut(Vec<NodeId>),
        node: NodeId,
    ) {
        if self.preorder.contains_key(&node) {
            return;
        }
        self.preorder.insert(node, self.counter);
        self.stack.push(node);
        self.preorder_stack.push(self.counter);
        self.counter += 1;
        let preorder_length = self.preorder_stack.len();
        for succ in graph.get_successors(node) {
            match self.preorder.get(succ).copied() {
                None => self.visit(graph, visitor, *succ),
                Some(succ_preorder) => {
                    if !self.assigned.contains(succ) {
                        while *self.preorder_stack.last().unwrap() > succ_preorder {
                            self.preorder_stack.pop();
                        }
                    }
                }
            }
        }
        if preorder_length == self.preorder_stack.len() {
            self.preorder_stack.pop();
            let mut scc: Vec<NodeId> = Vec::new();
            loop {
                let comp_node = self.stack.pop().unwrap();
                self.assigned.insert(comp_node);
                scc.push(comp_node);
                if comp_node == node {
                    break;
                }
            }
            visitor(scc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/graph/DigraphTest.java testStableOrdering
    #[test]
    fn test_stable_ordering() {
        let mut digraph: Digraph<String> = Digraph::new();
        let [a, b, c, d, e, f, g] =
            ["a", "b", "c", "d", "e", "f", "g"].map(|n| format!("//pkg:{n}"));
        //    f
        // / | | \
        // c g e d
        //      / \
        //      a  b
        digraph.add_edge(f.clone(), c.clone());
        digraph.add_edge(f.clone(), g.clone());
        digraph.add_edge(d.clone(), a.clone());
        digraph.add_edge(d.clone(), b.clone());
        digraph.add_edge(f.clone(), e.clone());
        digraph.add_edge(f.clone(), d.clone());

        // Get them back in topological and, within a valid topological ordering, alphabetical
        // order.
        let comparator = |o1: &String, o2: &String| o1.cmp(o2).reverse();
        let nodes: Vec<String> = digraph
            .get_topological_order_by(&comparator)
            .into_iter()
            .map(|n| digraph.get_label(n).clone())
            .collect();
        // containsExactlyElementsIn: same elements, any order.
        let mut sorted = nodes.clone();
        sorted.sort();
        assert_eq!(sorted, vec![a, b, c, d, e, f, g]);
    }

    #[test]
    fn subgraph_preserves_transitive_edges() {
        let mut g: Digraph<&str> = Digraph::new();
        g.add_edge("a", "b");
        g.add_edge("b", "c");
        g.add_edge("c", "d");
        g.add_edge("a", "e");
        let sub = g.extract_subgraph(&|l: &&str| matches!(*l, "a" | "d" | "e"));
        assert_eq!(sub.get_node_count(), 3);
        let a = sub.get_node(&"a");
        let labels: Vec<&str> = sub
            .get_successors(a)
            .iter()
            .map(|n| *sub.get_label(*n))
            .collect();
        assert_eq!(labels, vec!["e", "d"]);
    }

    #[test]
    fn strong_component_graph_fuses_cycles() {
        let mut g: Digraph<&str> = Digraph::new();
        g.add_edge("a", "b");
        g.add_edge("b", "a");
        g.add_edge("b", "c");
        let sc = g.get_strong_component_graph();
        assert_eq!(sc.get_node_count(), 2);
        assert_eq!(sc.get_edge_count(), 1);
    }

    #[test]
    fn shortest_path() {
        let mut g: Digraph<&str> = Digraph::new();
        g.add_edge("a", "b");
        g.add_edge("b", "c");
        g.add_edge("a", "c");
        let path = g
            .get_shortest_path(g.get_node(&"a"), g.get_node(&"c"))
            .unwrap();
        assert_eq!(path.len(), 2);
    }
}
