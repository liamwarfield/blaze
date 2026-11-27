//! `SkyFunction.Reset` (the sentinel value a function returns to start its evaluation over and
//! rewind other nodes) and the rewind graph it carries (Guava's `MutableGraph<SkyKey>` /
//! `ImmutableGraph<SkyKey>`, directed, without self loops).

use crate::SkyKey;
use blaze_util::check_argument;
use indexmap::{IndexMap, IndexSet};
use std::collections::HashSet;
use std::fmt;

/// A directed graph of keys without self loops: Guava's `GraphBuilder.directed()
/// .allowsSelfLoops(false).build()`, as the rewind graph of a [`Reset`] uses it.
///
/// PORT: nodes iterate in insertion order (Guava's default `ElementOrder.insertion()`); the
/// successors of a node iterate in insertion order too, where Guava's default incident edge
/// order is unordered (hash order). Only the order in which a reset dirties the nodes depends on
/// it, which nothing reports.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RewindGraph {
    successors: IndexMap<SkyKey, IndexSet<SkyKey>>,
}

impl RewindGraph {
    /// `MutableGraph.addNode`: adds `node` if it is not already present. Returns whether the
    /// graph was modified.
    pub fn add_node(&mut self, node: SkyKey) -> bool {
        if self.successors.contains_key(&node) {
            return false;
        }
        self.successors.insert(node, IndexSet::new());
        true
    }

    /// `MutableGraph.putEdge`: adds an edge from `node_u` to `node_v`, adding the nodes that are
    /// not present. Returns whether the graph was modified.
    pub fn put_edge(&mut self, node_u: SkyKey, node_v: SkyKey) -> bool {
        // Guava: ConfigurableMutableValueGraph.putEdgeValue.
        check_argument!(
            node_u != node_v,
            "Cannot add self-loop edge on node {:?}, as self-loops are not allowed. To construct \
             a graph that allows self-loops, call allowsSelfLoops(true) on the Builder.",
            node_u
        );
        self.add_node(node_v.clone());
        self.successors.entry(node_u).or_default().insert(node_v)
    }

    /// `Graph.nodes()`.
    pub fn nodes(&self) -> impl Iterator<Item = &SkyKey> {
        self.successors.keys()
    }

    /// `graph.nodes().contains(node)`.
    pub fn contains(&self, node: &SkyKey) -> bool {
        self.successors.contains_key(node)
    }

    /// `graph.nodes().size()`.
    pub fn node_count(&self) -> usize {
        self.successors.len()
    }

    /// `Graph.successors(node)`.
    pub fn successors(&self, node: &SkyKey) -> impl Iterator<Item = &SkyKey> {
        self.successors
            .get(node)
            .unwrap_or_else(|| {
                panic!("IllegalArgumentException: Node {node:?} is not an element of this graph.")
            })
            .iter()
    }

    /// `graph.hasEdgeConnecting(nodeU, nodeV)`.
    pub fn has_edge_connecting(&self, node_u: &SkyKey, node_v: &SkyKey) -> bool {
        self.successors
            .get(node_u)
            .is_some_and(|s| s.contains(node_v))
    }

    /// `Traverser.forGraph(graph).depthFirstPostOrder(startNode)`: the nodes reachable from
    /// `start`, each after all the nodes reachable from it that were not visited before.
    pub fn depth_first_post_order(&self, start: &SkyKey) -> Vec<SkyKey> {
        // Iterative, as Guava's traverser: a stack of (node, successor iterator position).
        let mut result = Vec::with_capacity(self.successors.len());
        let mut visited: HashSet<&SkyKey> = HashSet::new();
        let mut stack: Vec<(&SkyKey, usize)> = Vec::new();
        let (start, _) = self.successors.get_key_value(start).unwrap_or_else(|| {
            panic!("IllegalArgumentException: Node {start:?} is not an element of this graph.")
        });
        visited.insert(start);
        stack.push((start, 0));
        while let Some((node, next)) = stack.last_mut() {
            let successors = &self.successors[*node];
            if let Some(successor) = successors.get_index(*next) {
                *next += 1;
                if visited.insert(successor) {
                    stack.push((successor, 0));
                }
            } else {
                result.push((*node).clone());
                stack.pop();
            }
        }
        result
    }
}

impl fmt::Debug for RewindGraph {
    /// Guava's `AbstractGraph.toString`: `isDirected: true, allowsSelfLoops: false, nodes: [...],
    /// edges: [<u -> v>, ...]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "isDirected: true, allowsSelfLoops: false, nodes: [")?;
        for (i, node) in self.successors.keys().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{node:?}")?;
        }
        write!(f, "], edges: [")?;
        let mut first = true;
        for (u, vs) in &self.successors {
            for v in vs {
                if !first {
                    f.write_str(", ")?;
                }
                first = false;
                write!(f, "<{u:?} -> {v:?}>")?;
            }
        }
        f.write_str("]")
    }
}

/// Sentinel `SkyValue` type for `SkyFunction.compute` to return, indicating that the evaluation
/// should be started over (including calling `NodeEntry.resetEvaluationFromScratch`).
///
/// Returning a `Reset` from `compute` differs from returning `None`. A `None` return is expected
/// under normal circumstances when a dependency is requested but is not yet done. A `Reset`
/// signals a more complex issue that requires clearing the associated node's temporary direct
/// deps and rewinding (`DirtyType::Rewind`) nodes associated with other keys in
/// [`Reset::rewind_graph`] (whose directed edges should correspond to the nodes' direct
/// dependencies).
///
/// An intended cause for returning this is external data loss; e.g., if a dependency's
/// "done-ness" is intended to mean that certain data is available in an external system, but
/// during evaluation of a node that depends on that external data, that data has gone missing,
/// and reevaluation of the dependency is expected to repair the discrepancy.
///
/// Values of this type will *never* be returned by `Environment`'s `get_value` methods or from
/// `NodeEntry.getValue`.
///
/// A function returns it as its value (`Arc::new(reset)` as a `SkyValue`); the evaluator
/// recognizes it before committing anything.
// Java: com/google/devtools/build/skyframe/SkyFunction.java:132 SkyFunction.Reset
pub struct Reset {
    rewind_graph: RewindGraph,
}

impl Reset {
    /// Convenience method that creates a [`RewindGraph`] that fulfills the basic requirements of
    /// a [`Reset`]. Additional edges may be added to the graph before passing to [`Reset::of`].
    // Java: com/google/devtools/build/skyframe/SkyFunction.java:140 Reset.newRewindGraphFor
    pub fn new_rewind_graph_for(key_to_reset: SkyKey) -> RewindGraph {
        let mut rewind_graph = RewindGraph::default();
        rewind_graph.add_node(key_to_reset);
        rewind_graph
    }

    // Java: com/google/devtools/build/skyframe/SkyFunction.java:146 Reset.of
    pub fn of(rewind_graph: RewindGraph) -> Reset {
        // Directed and without self loops by construction (RewindGraph).
        check_argument!(
            rewind_graph.node_count() > 0,
            "Rewind graph must include key to reset"
        );
        Reset { rewind_graph }
    }

    /// Creates a [`Reset`] for a single key with no rewinding of dependencies.
    ///
    /// This can be used to clear out a node's temporary direct deps without any rewinding.
    // Java: com/google/devtools/build/skyframe/SkyFunction.java:158 Reset.selfOnly
    pub fn self_only(key: SkyKey) -> Reset {
        Reset::of(Reset::new_rewind_graph_for(key))
    }

    // Java: com/google/devtools/build/skyframe/SkyFunction.java:168 Reset.rewindGraph
    pub fn rewind_graph(&self) -> &RewindGraph {
        &self.rewind_graph
    }
}

impl fmt::Debug for Reset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reset")
            .field("rewind_graph", &self.rewind_graph)
            .finish()
    }
}
