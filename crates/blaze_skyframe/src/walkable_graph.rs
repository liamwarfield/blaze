//! Port of `WalkableGraph` (`DelegatingWalkableGraph` over the evaluator's graph).

use crate::node_entry::NodeEntry;
use crate::{Evaluator, SkyError, SkyKey, SkyValue};
use blaze_util::check_state;

/// Read-only graph that exposes the dependents, dependencies (reverse dependents), and value and
/// exception (if any) of a given node. Only nodes committed to the graph ("done") are visible.
///
/// PORT: Java's maps keyed by SkyKey are `HashMap`s; here they are vectors in the order of the
/// requested keys.
// Java: com/google/devtools/build/skyframe/WalkableGraph.java:30 WalkableGraph
pub trait WalkableGraph: Send + Sync {
    /// Returns the value of the given key, or `None` if it has no value due to an error during
    /// its computation or it is not done in the graph.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:44 WalkableGraph.getValue
    fn get_value(&self, key: &SkyKey) -> Option<SkyValue>;

    /// Returns the values of the given keys for done keys that were successfully computed.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:51 WalkableGraph.getSuccessfulValues
    fn get_successful_values(&self, keys: &[SkyKey]) -> Vec<(SkyKey, SkyValue)>;

    /// Returns an entry for each key that is missing, not done, or in error, with the exception
    /// if there is one.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:61 WalkableGraph.getMissingAndExceptions
    fn get_missing_and_exceptions(&self, keys: &[SkyKey]) -> Vec<(SkyKey, Option<SkyError>)>;

    /// Returns the exception thrown when computing the node with the given key, if any.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:69 WalkableGraph.getException
    fn get_exception(&self, key: &SkyKey) -> Option<SkyError>;

    /// Returns true if the node with the given `key` depends on a cycle.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:75 WalkableGraph.isCycle
    fn is_cycle(&self, key: &SkyKey) -> bool;

    /// Returns the direct dependencies of the node with the given key, in request order. A node
    /// for that key must exist in the graph and be done.
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:87 WalkableGraph.getDirectDeps
    fn get_direct_deps(&self, key: &SkyKey) -> Vec<SkyKey>;

    /// Returns the reverse dependencies of the nodes with the given keys (done nodes only).
    // Java: com/google/devtools/build/skyframe/WalkableGraph.java:93 WalkableGraph.getReverseDeps
    fn get_reverse_deps(&self, keys: &[SkyKey]) -> Vec<(SkyKey, Vec<SkyKey>)>;
}

impl WalkableGraph for Evaluator {
    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:41 DelegatingWalkableGraph.getValue
    fn get_value(&self, key: &SkyKey) -> Option<SkyValue> {
        self.committed_entry(key).and_then(|e| e.value.clone())
    }

    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:53 DelegatingWalkableGraph.getSuccessfulValues
    fn get_successful_values(&self, keys: &[SkyKey]) -> Vec<(SkyKey, SkyValue)> {
        keys.iter()
            .filter_map(|k| self.get_value(k).map(|v| (k.clone(), v)))
            .collect()
    }

    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:67 DelegatingWalkableGraph.getMissingAndExceptions
    fn get_missing_and_exceptions(&self, keys: &[SkyKey]) -> Vec<(SkyKey, Option<SkyError>)> {
        let mut result = vec![];
        for key in keys {
            match self.committed_entry(key) {
                None => result.push((key.clone(), None)),
                Some(e) => {
                    if let Some(error_info) = &e.error {
                        result.push((key.clone(), error_info.get_exception().cloned()));
                    }
                }
            }
        }
        result
    }

    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:98 DelegatingWalkableGraph.getException
    fn get_exception(&self, key: &SkyKey) -> Option<SkyError> {
        self.committed_entry(key)
            .and_then(|e| e.error.as_ref().and_then(|i| i.get_exception().cloned()))
    }

    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:88 DelegatingWalkableGraph.isCycle
    fn is_cycle(&self, key: &SkyKey) -> bool {
        self.committed_entry(key).is_some_and(|e| {
            e.error
                .as_ref()
                .is_some_and(|i| !i.get_cycle_info().is_empty())
        })
    }

    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:122 DelegatingWalkableGraph.getDirectDeps
    fn get_direct_deps(&self, key: &SkyKey) -> Vec<SkyKey> {
        let node = self.node(key);
        check_state!(node.is_some(), "{:?}", key);
        let node = node.expect("checked");
        let state = node.lock();
        // Note that the situation described in #getReverseDeps doesn't apply here. If the node
        // for `key` is done, then its direct deps must be done too.
        check_state!(
            state.entry.is_done(),
            "Node {:?} (with key {:?}) isn't done yet.",
            state.entry,
            key
        );
        state.entry.get_direct_deps()
    }

    /// PORT: the reverse deps are the node entry's (`getReverseDepsForDoneEntry`): every node
    /// that registered as a reverse dep, in registration order, compacted as the entry does.
    // Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:133 DelegatingWalkableGraph.getReverseDeps
    fn get_reverse_deps(&self, keys: &[SkyKey]) -> Vec<(SkyKey, Vec<SkyKey>)> {
        let mut result = vec![];
        for key in keys {
            let Some(node) = self.node(key) else {
                continue;
            };
            let mut state = node.lock();
            // SkyQuery may be operating on a Skyframe graph that contains more nodes and edges
            // than its universe. In this situation, Blaze's eager invalidation strategy may mean
            // here we can observe a rdep edge from a not-done node (because that node may have
            // been invalidated but not re-evaluated). Therefore, we tolerate this case
            // gracefully.
            if state.entry.is_done() {
                result.push((key.clone(), state.entry.get_reverse_deps_for_done_entry()));
            }
        }
        result
    }
}
