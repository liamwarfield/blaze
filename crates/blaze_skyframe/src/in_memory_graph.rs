//! Port of `InMemoryGraphImpl` (and `EdgelessInMemoryGraphImpl`): the evaluator's graph of
//! [`InMemoryNodeEntry`]s, each under a lock, plus what the blocking engine needs to let other
//! threads wait for a node that one thread evaluates.

use crate::error_info::ErrorInfo;
use crate::in_memory_node_entry::InMemoryNodeEntry;
use crate::node_entry::NodeEntry;
use crate::value_with_metadata::{StoredReportable, ValueWithMetadata};
use crate::version::Version;
use crate::{SkyKey, SkyValue};
use blaze_collect::NestedSet;
use parking_lot::{Condvar, Mutex, MutexGuard};
use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::thread::ThreadId;

/// How a node finished in one evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DoneKind {
    /// Java: the node is done in the graph (value and/or error).
    Committed,
    /// Java: an entry of `bubbleErrorInfo`, the error `ParallelEvaluator.bubbleErrorUp` built for
    /// a node on the bubbling path. It is never written to the graph: such entries live only in
    /// the map that `bubble_error_up` returns.
    Bubbled,
    /// Java: the node is still in flight because the evaluation aborted (no keep_going error,
    /// catastrophe, crash), or it found a dep in error without keep_going and so cannot finish.
    /// Parents see it as not done.
    Aborted,
    /// Java: the node is still in flight because the evaluation was interrupted.
    Interrupted,
    /// Java: the node is in or above a cycle and never finished. The cycle detector commits it
    /// (keep_going) or it stays in flight.
    InCycle,
}

/// What a requester sees of a node: Java's `NodeEntry.getValueMaybeWithMetadata` of a done node
/// (a "snapshot" taken when the node became done), or, for a node that did not finish in this
/// evaluation, how it ended (the blocking engine's stand-in for "still in flight").
///
/// PORT: Java has only the entry; a waiting thread here needs the outcome of the evaluation of
/// the node it waited for, and a done node is read thousands of times per evaluation, so the
/// done state is kept as an immutable snapshot that a reader clones with one `Arc` increment.
pub(crate) struct DoneEntry {
    pub(crate) kind: DoneKind,
    pub(crate) value: Option<SkyValue>,
    pub(crate) error: Option<Arc<ErrorInfo>>,
    /// Java: `ValueWithMetadata.getEvents`. `None` stands for `NO_EVENTS` (a bare value, or an
    /// entry that is not done).
    pub(crate) events: Option<NestedSet<StoredReportable>>,
    /// Java: `NodeEntry.getVersion()` of the done node (`Version::minimal()` if not done).
    pub(crate) version: Version,
    /// For a node that is not done: its function returned null because of a dep that was not
    /// available (or it waits in a dirty check). Java then registers the node as a reverse dep of
    /// the deps it declared (`enqueueChild`, or the edge to the child in error) and the node
    /// waits. False when the function threw an exception that was ignored (not the first error),
    /// was interrupted, or never ran: Java registers nothing for those.
    pub(crate) awaits_deps: bool,
}

impl DoneEntry {
    pub(crate) fn undone(kind: DoneKind) -> DoneEntry {
        DoneEntry {
            kind,
            value: None,
            error: None,
            events: None,
            version: Version::minimal(),
            awaits_deps: false,
        }
    }

    /// An undone node whose function returned null (see `awaits_deps`).
    pub(crate) fn awaiting_deps(kind: DoneKind) -> DoneEntry {
        DoneEntry {
            awaits_deps: true,
            ..DoneEntry::undone(kind)
        }
    }

    /// The snapshot of a done node's value.
    pub(crate) fn from_value(value: &ValueWithMetadata, version: Version) -> DoneEntry {
        let (error, events) = match value {
            ValueWithMetadata::Bare(_) => (None, None),
            ValueWithMetadata::ValueWithEvents {
                transitive_events, ..
            } => (None, Some(transitive_events.clone())),
            ValueWithMetadata::ErrorInfoValue {
                error_info,
                transitive_events,
                ..
            } => (Some(error_info.clone()), Some(transitive_events.clone())),
        };
        DoneEntry {
            kind: DoneKind::Committed,
            value: value.get_value().cloned(),
            error,
            events,
            version,
            awaits_deps: false,
        }
    }

    /// Java: `NodeEntry.isDone()` (committed to the graph).
    pub(crate) fn is_done(&self) -> bool {
        self.kind == DoneKind::Committed
    }

    /// Java: the dependency was not done when requested (`NULL_MARKER`).
    pub(crate) fn is_undone_view(&self) -> bool {
        matches!(
            self.kind,
            DoneKind::Aborted | DoneKind::Interrupted | DoneKind::InCycle
        )
    }

    /// Java: `ValueWithMetadata.getEvents(valueMaybeWithMetadata)`.
    pub(crate) fn get_events(&self) -> NestedSet<StoredReportable> {
        match &self.events {
            Some(events) => events.clone(),
            None => crate::value_with_metadata::no_events(),
        }
    }
}

impl fmt::Debug for DoneEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeEntry")
            .field("state", &self.kind)
            .field("hasValue", &self.value.is_some())
            .field("error", &self.error)
            .field("version", &self.version)
            .finish()
    }
}

/// The node of the graph: Java's `InMemoryNodeEntry` (whose `synchronized` is this lock) plus the
/// blocking engine's per-evaluation state.
pub(crate) struct Node {
    state: Mutex<NodeState>,
    /// Notified when [`NodeState::outcome`] is set.
    done: Condvar,
}

/// Everything under a node's lock.
pub(crate) struct NodeState {
    pub(crate) entry: InMemoryNodeEntry,
    /// PORT: blocking engine. The thread that evaluates the node in the current evaluation (Java:
    /// the node is enqueued or running). Used by the wait-for graph that detects cycles.
    pub(crate) owner: Option<ThreadId>,
    /// The snapshot of the entry while it is done (`None` while it is not).
    pub(crate) snapshot: Option<Arc<DoneEntry>>,
    /// PORT: blocking engine. How the evaluation of this node ended in the current evaluation:
    /// the done snapshot, or an undone outcome. Cleared when a thread claims the node
    /// (`NEEDS_SCHEDULING`); set by that thread when it is finished with the node, after the
    /// parents the node signals have been signalled. Threads that wait for the node wait for it.
    pub(crate) outcome: Option<Arc<DoneEntry>>,
    /// PORT: blocking engine. Requesters whose request closed a dependency cycle through this
    /// node (they are below it and got "not done" back at once).
    pub(crate) cycle_rdeps: Vec<SkyKey>,
    /// PORT: blocking engine. A dep this node registered was found undone when it registered it
    /// (it had been done when the node's function read it, and was rewound since): Java's
    /// `maybeHandleUndoneDepForDoneEntry` (`DEP_NOT_DONE`), after which the node is not
    /// committed and its function runs again when the dep signals it. The thread that
    /// registered the dep evaluated it; the node's own thread runs its function again
    /// ([`crate::evaluator`]'s `evaluate_node`).
    pub(crate) restart_requested: bool,
}

impl NodeState {
    /// Re-reads the done snapshot after an operation that may have changed the entry's
    /// doneness (`setValue`, `markClean`, `markDirty`).
    pub(crate) fn refresh_snapshot(&mut self) {
        self.snapshot = if self.entry.is_done() {
            let value = self
                .entry
                .get_value_maybe_with_metadata()
                .expect("done entry has a value");
            Some(Arc::new(DoneEntry::from_value(
                value,
                self.entry.get_version(),
            )))
        } else {
            // A dirtied node's outcome from an earlier evaluation is gone.
            self.outcome = None;
            None
        };
    }
}

impl Node {
    fn new(entry: InMemoryNodeEntry) -> Arc<Node> {
        Arc::new(Node {
            state: Mutex::new(NodeState {
                entry,
                owner: None,
                snapshot: None,
                outcome: None,
                cycle_rdeps: vec![],
                restart_requested: false,
            }),
            done: Condvar::new(),
        })
    }

    /// Java: entering a `synchronized` method of the entry.
    pub(crate) fn lock(&self) -> MutexGuard<'_, NodeState> {
        self.state.lock()
    }

    /// The snapshot of the node if it is done.
    pub(crate) fn snapshot(&self) -> Option<Arc<DoneEntry>> {
        self.state.lock().snapshot.clone()
    }

    /// The snapshot if the node is done, else this evaluation's outcome for it, if any.
    pub(crate) fn done_entry(&self) -> Option<Arc<DoneEntry>> {
        let state = self.state.lock();
        state.snapshot.clone().or_else(|| state.outcome.clone())
    }

    /// Publishes how the node ended in this evaluation and wakes the threads waiting for it.
    pub(crate) fn publish_outcome(&self, outcome: Arc<DoneEntry>) {
        let mut state = self.state.lock();
        state.outcome = Some(outcome);
        state.owner = None;
        drop(state);
        self.done.notify_all();
    }

    /// Blocks until the node has an outcome in this evaluation.
    pub(crate) fn wait_for_outcome(&self) -> Arc<DoneEntry> {
        let mut state = self.state.lock();
        loop {
            if let Some(outcome) = &state.outcome {
                return outcome.clone();
            }
            self.done.wait(&mut state);
        }
    }
}

const SHARDS: usize = 64;

/// An in-memory graph implementation. All operations are thread-safe with ConcurrentMap
/// semantics.
///
/// PORT: Java's `ConcurrentHashMap` is a fixed set of locked shards; `EdgelessInMemoryGraphImpl`
/// is the same type with `keep_edges == false` (it differs only in `newNodeEntry`). Key interning
/// (`SkyKeyInterner`, `usePooledInterning`) is a memory optimization and is not ported.
// Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:55 InMemoryGraphImpl
pub(crate) struct InMemoryGraphImpl {
    shards: Vec<Mutex<HashMap<SkyKey, Arc<Node>>>>,
    hasher: std::collections::hash_map::RandomState,
    keep_edges: bool,
    /// Test hook: Java's `NotifyingHelper` graph transformers (`graph_listener`); a new graph
    /// starts with the listeners pending for new graphs.
    pub(crate) listeners_for_testing: crate::graph_listener::GraphListeners,
}

impl InMemoryGraphImpl {
    // Java: com/google/devtools/build/skyframe/InMemoryGraph.java:27 InMemoryGraph.create
    // Java: com/google/devtools/build/skyframe/InMemoryGraph.java:39 InMemoryGraph.createEdgeless
    pub(crate) fn new(keep_edges: bool) -> Self {
        InMemoryGraphImpl {
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            hasher: Default::default(),
            keep_edges,
            listeners_for_testing: Default::default(),
        }
    }

    fn shard(&self, key: &SkyKey) -> &Mutex<HashMap<SkyKey, Arc<Node>>> {
        &self.shards[(self.hasher.hash_one(key) as usize) % SHARDS]
    }

    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:103 InMemoryGraphImpl.remove
    pub(crate) fn remove(&self, sky_key: &SkyKey) {
        self.shard(sky_key).lock().remove(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:145 InMemoryGraphImpl.get
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:299 InMemoryGraphImpl.getIfPresent
    pub(crate) fn get_if_present(&self, key: &SkyKey) -> Option<Arc<Node>> {
        self.shard(key).lock().get(key).cloned()
    }

    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:195 InMemoryGraphImpl.newNodeEntry
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:316 EdgelessInMemoryGraphImpl.newNodeEntry
    fn new_node_entry(&self, key: SkyKey) -> InMemoryNodeEntry {
        InMemoryNodeEntry::new(key, self.keep_edges)
    }

    /// Java: `createIfAbsentBatch` for one key.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:217 InMemoryGraphImpl.createIfAbsent
    pub(crate) fn create_if_absent(&self, sky_key: &SkyKey) -> Arc<Node> {
        // Java: NotifyingHelper.NotifyingProcessableGraph.createIfAbsentBatch
        notify_graph_listener!(
            self,
            sky_key,
            CreateIfAbsent,
            Before,
            crate::graph_listener::Context::Null
        );
        let mut shard = self.shard(sky_key).lock();
        if let Some(node) = shard.get(sky_key) {
            return node.clone();
        }
        let node = Node::new(self.new_node_entry(sky_key.clone()));
        shard.insert(sky_key.clone(), node.clone());
        node
    }

    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:203 InMemoryGraphImpl.createIfAbsentBatch
    pub(crate) fn create_if_absent_batch<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a SkyKey>,
    ) -> Vec<Arc<Node>> {
        keys.into_iter().map(|k| self.create_if_absent(k)).collect()
    }

    /// Java: `getBatchMap`: the nodes of `keys` that are present, in the order of `keys`.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:177 InMemoryGraphImpl.getBatchMap
    pub(crate) fn get_batch_map<'a>(
        &self,
        keys: impl IntoIterator<Item = &'a SkyKey>,
    ) -> Vec<(SkyKey, Arc<Node>)> {
        keys.into_iter()
            .filter_map(|k| self.get_if_present(k).map(|n| (k.clone(), n)))
            .collect()
    }

    /// Every node, with its key (Java: `getAllNodeEntries`, `parallelForEach`).
    ///
    /// PORT: a copy of the node map, so the caller may lock nodes and mutate the graph.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:261 InMemoryGraphImpl.getAllNodeEntries
    pub(crate) fn get_all_node_entries(&self) -> Vec<(SkyKey, Arc<Node>)> {
        self.shards
            .iter()
            .flat_map(|s| {
                s.lock()
                    .iter()
                    .map(|(k, n)| (k.clone(), n.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Returns the value of every node: the done value, or the last built value of a dirty node
    /// (`toValue`); nodes without one are left out.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:248 InMemoryGraphImpl.getValues
    pub(crate) fn get_values(&self) -> Vec<(SkyKey, SkyValue)> {
        self.get_all_node_entries()
            .into_iter()
            .filter_map(|(k, n)| n.lock().entry.to_value().map(|v| (k, v)))
            .collect()
    }

    /// Returns the value of every done node that has one.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:253 InMemoryGraphImpl.getDoneValues
    pub(crate) fn get_done_values(&self) -> Vec<(SkyKey, SkyValue)> {
        self.get_all_node_entries()
            .into_iter()
            .filter_map(|(k, n)| {
                let state = n.lock();
                if state.entry.is_done() {
                    state.entry.get_value().map(|v| (k, v))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Java: `parallelForEach`.
    ///
    /// PORT: sequential (Java's `forEachValue` with a parallelism threshold is a speed-up).
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:276 InMemoryGraphImpl.parallelForEach
    pub(crate) fn parallel_for_each(&self, mut consumer: impl FnMut(&SkyKey, &Arc<Node>)) {
        for (k, n) in self.get_all_node_entries() {
            consumer(&k, &n);
        }
    }
}
