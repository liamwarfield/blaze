//! Ports of `InvalidatingNodeVisitor` (with `DirtyingNodeVisitor`, `DeletingNodeVisitor` and the
//! invalidation states) and `EagerInvalidator`.
//!
//! PORT: Java runs the visitors on a fork-join pool and lets an interrupt stop a visitation, the
//! pending set (`InvalidationState.pendingValues`) keeping what was not done for the next run.
//! Here a visitation runs on the calling thread, depth first, and is never interrupted (the
//! evaluator checks its interrupt flag after invalidation); every effect and every check is
//! Java's. The pending set is kept as in Java (it is empty after every run).

use crate::FunctionHermeticity;
use crate::SkyKey;
use crate::in_memory_graph::InMemoryGraphImpl;
use crate::node_entry::{DirtyType, NodeEntry};
use crate::progress::{DirtyAndInflightTrackingProgressReceiver, EvaluationProgressReceiver};
use crate::reverse_deps_utility::java_list;
use blaze_util::check_state;
use indexmap::IndexSet;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:155 InvalidatingNodeVisitor.InvalidationType
pub(crate) enum InvalidationType {
    /// The node is dirty and must be recomputed.
    Changed,
    /// The node is dirty, but may be marked clean later during change pruning.
    Dirtied,
    /// The node is deleted.
    Deleted,
}

/// Invalidation state object that keeps track of which nodes need to be invalidated, but have not
/// been dirtied/deleted yet. This supports interrupts - by only deleting a node from this set
/// when all its parents have been invalidated, we ensure that no information is lost when an
/// interrupt comes in.
// Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:170 InvalidatingNodeVisitor.InvalidationState
pub(crate) struct InvalidationState {
    pending_values: IndexSet<(SkyKey, InvalidationType)>,
    default_update_type: InvalidationType,
}

impl InvalidationState {
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:176 InvalidationState.<init>
    fn new(default_update_type: InvalidationType) -> Self {
        InvalidationState {
            pending_values: IndexSet::new(),
            default_update_type,
        }
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:180 InvalidationState.update
    pub(crate) fn update(&mut self, diff: impl IntoIterator<Item = SkyKey>) {
        let t = self.default_update_type;
        self.pending_values.extend(diff.into_iter().map(|k| (k, t)));
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:186 InvalidationState.isEmpty
    pub(crate) fn is_empty(&self) -> bool {
        self.pending_values.is_empty()
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:191 InvalidationState.getInvalidationsForTesting
    #[cfg(test)]
    pub(crate) fn get_invalidations_for_testing(&self) -> HashSet<(SkyKey, InvalidationType)> {
        self.pending_values.iter().cloned().collect()
    }

    /// Java: `new DirtyingInvalidationState()`.
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:197 DirtyingInvalidationState.<init>
    pub(crate) fn dirtying() -> Self {
        InvalidationState::new(InvalidationType::Changed)
    }
}

// Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:202 InvalidatingNodeVisitor.DeletingInvalidationState
pub(crate) struct DeletingInvalidationState {
    base: InvalidationState,
    done_keys_with_rdeps_to_remove: IndexSet<SkyKey>,
    visited_keys_across_interruptions: IndexSet<SkyKey>,
}

impl DeletingInvalidationState {
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:206 DeletingInvalidationState.<init>
    pub(crate) fn new() -> Self {
        DeletingInvalidationState {
            base: InvalidationState::new(InvalidationType::Deleted),
            done_keys_with_rdeps_to_remove: IndexSet::new(),
            visited_keys_across_interruptions: IndexSet::new(),
        }
    }

    pub(crate) fn update(&mut self, diff: impl IntoIterator<Item = SkyKey>) {
        self.base.update(diff);
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:219 DeletingInvalidationState.isEmpty
    pub(crate) fn is_empty(&self) -> bool {
        self.base.is_empty() && self.done_keys_with_rdeps_to_remove.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn get_invalidations_for_testing(&self) -> HashSet<(SkyKey, InvalidationType)> {
        self.base.get_invalidations_for_testing()
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:223 DeletingInvalidationState.clear
    fn clear(&mut self) {
        self.done_keys_with_rdeps_to_remove = IndexSet::new();
        self.visited_keys_across_interruptions = IndexSet::new();
    }
}

/// A node-deleting implementation.
// Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:229 InvalidatingNodeVisitor.DeletingNodeVisitor
pub(crate) struct DeletingNodeVisitor<'a> {
    graph: &'a InMemoryGraphImpl,
    progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
    visited: HashSet<SkyKey>,
    traverse_graph: bool,
    state: &'a mut DeletingInvalidationState,
}

impl<'a> DeletingNodeVisitor<'a> {
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:234 DeletingNodeVisitor.<init>
    pub(crate) fn new(
        graph: &'a InMemoryGraphImpl,
        progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
        state: &'a mut DeletingInvalidationState,
        traverse_graph: bool,
    ) -> Self {
        DeletingNodeVisitor {
            graph,
            progress_receiver,
            visited: HashSet::new(),
            traverse_graph,
            state,
        }
    }

    /// Initiates visitation and waits for completion.
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:112 InvalidatingNodeVisitor.run
    pub(crate) fn run(&mut self) {
        // Make a copy to avoid concurrent modification confusing us as to which nodes were passed
        // by the caller, and which are added by other threads during the run.
        let pending_list: Vec<(SkyKey, InvalidationType)> =
            self.state.base.pending_values.iter().cloned().collect();
        self.run_internal(pending_list);
        check_state!(
            self.state.base.pending_values.is_empty(),
            "All dirty nodes should have been processed: {}",
            java_list(self.state.base.pending_values.iter().map(PairDebug))
        );
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:249 DeletingNodeVisitor.runInternal
    fn run_internal(&mut self, pending_list: Vec<(SkyKey, InvalidationType)>) {
        // PORT: Java starts DEFAULT_THREAD_COUNT jobs, each visiting a chunk of the list.
        self.visit(pending_list.into_iter().map(|(k, _)| k).collect());
        let deleted_keys: HashSet<SkyKey> = self
            .state
            .visited_keys_across_interruptions
            .iter()
            .cloned()
            .collect();
        for key in &self.state.done_keys_with_rdeps_to_remove {
            if let Some(node) = self.graph.get_if_present(key) {
                node.lock()
                    .entry
                    .remove_reverse_deps_from_done_entry_due_to_deletion(&deleted_keys);
            }
        }
        self.state.clear();
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:302 DeletingNodeVisitor.visit
    fn visit(&mut self, keys: Vec<SkyKey>) {
        // PORT: Java's recursive task submission is a work list.
        let mut work: Vec<SkyKey> = vec![];
        self.enqueue(keys, &mut work);
        while let Some(key) = work.pop() {
            self.delete(key, &mut work);
        }
    }

    /// The first half of Java's `visit`: marks unvisited keys pending and queues them.
    fn enqueue(&mut self, keys: Vec<SkyKey>, work: &mut Vec<SkyKey>) {
        let mut unvisited_keys = vec![];
        for key in keys {
            if self.visited.insert(key.clone()) {
                unvisited_keys.push(key);
            }
        }
        for key in &unvisited_keys {
            self.state
                .base
                .pending_values
                .insert((key.clone(), InvalidationType::Deleted));
        }
        // Java executes one task per key; the work list takes them last first.
        work.extend(unvisited_keys.into_iter().rev());
    }

    /// The task Java's `visit` executes for one key.
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:315 DeletingNodeVisitor.visit
    fn delete(&mut self, key: SkyKey, work: &mut Vec<SkyKey>) {
        let invalidation_pair = (key.clone(), InvalidationType::Deleted);
        let Some(node) = self.graph.get_if_present(&key) else {
            self.state
                .base
                .pending_values
                .shift_remove(&invalidation_pair);
            return;
        };

        if self.traverse_graph {
            // Propagate deletion upwards.
            let (rdeps, direct_deps) = {
                let mut state = node.lock();
                let rdeps = state.entry.get_all_reverse_deps_for_node_being_deleted();
                // Unregister this node as an rdep from its direct deps, since reverse dep edges
                // cannot point to non-existent nodes. To know whether the child has this node as
                // an "in-progress" rdep to be signaled, or just as a known rdep, we look at the
                // deps that this node declared during its last (presumably interrupted)
                // evaluation. If a dep is in this set, then it was notified to signal this node,
                // and so the rdep will be an in-progress rdep, if the dep itself isn't done.
                // Otherwise it will be a normal rdep. That information is used to remove this
                // node as an rdep from the correct list of rdeps in the child -- because of our
                // compact storage of rdeps, checking which list contains this parent could be
                // expensive.
                let direct_deps: Vec<SkyKey> = if state.entry.is_done() {
                    state.entry.get_direct_deps()
                } else {
                    state
                        .entry
                        .get_all_direct_deps_for_incomplete_node()
                        .into_iter()
                        .collect()
                };
                (rdeps, direct_deps)
            };
            self.enqueue(rdeps, work);

            // No need to do reverse dep surgery on nodes that are deleted/about to be deleted
            // anyway.
            let filtered: Vec<SkyKey> = direct_deps
                .into_iter()
                .filter(|k| {
                    !self.state.visited_keys_across_interruptions.contains(k)
                        && !self
                            .state
                            .base
                            .pending_values
                            .contains(&(k.clone(), InvalidationType::Deleted))
                })
                .collect();
            for (dep_key, dep) in self.graph.get_batch_map(&filtered) {
                let mut dep_state = dep.lock();
                if dep_state.entry.is_done() {
                    self.state.done_keys_with_rdeps_to_remove.insert(dep_key);
                    continue;
                }
                dep_state.entry.remove_reverse_dep(key.clone());
            }
        }

        // Allow custom key-specific logic to update dirtiness status.
        self.progress_receiver.deleted(&key);
        // Actually remove the node.
        self.graph.remove(&key);

        // Remove the node from the set and add it to global visited as the last operation.
        self.state
            .visited_keys_across_interruptions
            .insert(key.clone());
        self.state
            .base
            .pending_values
            .shift_remove(&invalidation_pair);
    }
}

/// A node-dirtying implementation.
// Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:402 InvalidatingNodeVisitor.DirtyingNodeVisitor
pub(crate) struct DirtyingNodeVisitor<'a> {
    graph: &'a InMemoryGraphImpl,
    progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
    pending_visitations: &'a mut InvalidationState,
    changed: HashSet<SkyKey>,
    dirtied: HashSet<SkyKey>,
}

/// One pending call of Java's recursive `visit`.
struct VisitTask {
    keys: Vec<SkyKey>,
    invalidation_type: InvalidationType,
    enqueueing_key_for_existence_check: Option<SkyKey>,
}

impl<'a> DirtyingNodeVisitor<'a> {
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:412 DirtyingNodeVisitor.<init>
    pub(crate) fn new(
        graph: &'a InMemoryGraphImpl,
        progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
        state: &'a mut InvalidationState,
    ) -> Self {
        DirtyingNodeVisitor {
            graph,
            progress_receiver,
            pending_visitations: state,
            changed: HashSet::new(),
            dirtied: HashSet::new(),
        }
    }

    /// Initiates visitation and waits for completion.
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:112 InvalidatingNodeVisitor.run
    pub(crate) fn run(&mut self) {
        let pending_list: Vec<(SkyKey, InvalidationType)> = self
            .pending_visitations
            .pending_values
            .iter()
            .cloned()
            .collect();
        // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:128 InvalidatingNodeVisitor.runInternal
        let mut work: Vec<VisitTask> = vec![];
        for (key, invalidation_type) in pending_list.into_iter().rev() {
            check_state!(invalidation_type != InvalidationType::Deleted, "{:?}", key);
            work.push(VisitTask {
                keys: vec![key],
                invalidation_type,
                enqueueing_key_for_existence_check: None,
            });
        }
        while let Some(task) = work.pop() {
            self.visit(task, &mut work);
        }
        check_state!(
            self.pending_visitations.pending_values.is_empty(),
            "All dirty nodes should have been processed: {}",
            java_list(
                self.pending_visitations
                    .pending_values
                    .iter()
                    .map(PairDebug)
            )
        );
    }

    /// Queues a task to dirty the nodes named by `keys`. It is possible that the same node is
    /// enqueued many times. However, we require that a node is only actually marked
    /// dirty/changed once, with two exceptions:
    ///
    /// (1) If a node is marked dirty, it can subsequently be marked changed. This can occur if,
    /// for instance, FileValue workspace/foo/foo.cc is marked dirty because FileValue
    /// workspace/foo is marked changed (and every FileValue depends on its parent). Then
    /// FileValue workspace/foo/foo.cc is itself changed (this can even happen on the same
    /// build).
    ///
    /// (2) If a node is going to be marked both dirty and changed, as, for example, in the
    /// previous case if both workspace/foo/foo.cc and workspace/foo have been changed in the
    /// same build, the thread marking workspace/foo/foo.cc dirty may race with the one marking
    /// it changed, and so try to mark it dirty after it has already been marked changed. In that
    /// case, the `NodeEntry` ignores the second marking.
    ///
    /// The invariant that we do not process a (SkyKey, InvalidationType) pair twice is enforced
    /// by the `changed` and `dirtied` sets.
    ///
    /// The "invariant" is also enforced across builds by checking to see if the entry is already
    /// marked changed, or if it is already marked dirty and we are just going to mark it dirty
    /// again.
    ///
    /// If either of the above tests shows that we have already started a task to mark this
    /// entry dirty/changed, or that it is already marked dirty/changed, we do not continue this
    /// task.
    ///
    /// PORT: the "hermetic functions cannot be marked 'changed'" check applies only to keys that
    /// declare their hermeticity (`Key::HERMETICITY`); an undeclared one is not checked
    /// (docs/skyframe-incremental.md section 3).
    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:456 DirtyingNodeVisitor.visit
    fn visit(&mut self, task: VisitTask, work: &mut Vec<VisitTask>) {
        let VisitTask {
            keys,
            invalidation_type,
            enqueueing_key_for_existence_check,
        } = task;
        let is_changed = invalidation_type == InvalidationType::Changed;
        let mut keys_to_get: Vec<SkyKey> = Vec::with_capacity(keys.len());
        for key in keys {
            let set_to_check = if is_changed {
                &mut self.changed
            } else {
                &mut self.dirtied
            };
            if set_to_check.insert(key.clone()) {
                if is_changed && key.hermeticity() == Some(FunctionHermeticity::Hermetic) {
                    panic!(
                        "IllegalStateException: Nodes with hermetic functions cannot be marked \
                         'changed': {:?} function:{} hermeticity:{}",
                        key,
                        key.function_name(),
                        hermeticity_name(key.hermeticity())
                    );
                }
                keys_to_get.push(key);
            }
        }
        for key in &keys_to_get {
            self.pending_visitations
                .pending_values
                .insert((key.clone(), invalidation_type));
        }
        let entries = self.graph.get_batch_map(&keys_to_get);
        if let Some(enqueueing_key) = &enqueueing_key_for_existence_check
            && entries.len() != keys_to_get.len()
        {
            let present: HashSet<&SkyKey> = entries.iter().map(|(k, _)| k).collect();
            let missing_keys: Vec<&SkyKey> = keys_to_get
                .iter()
                .filter(|k| !present.contains(k))
                .take(10)
                .collect();
            panic!(
                "IllegalStateException: key(s) {} not in the graph, but enqueued for dirtying by \
                 {:?}",
                java_list(missing_keys),
                enqueueing_key
            );
        }
        // PORT: Java dirties the last key on this thread and submits the others; all are queued
        // here, in key order.
        for key in keys_to_get.into_iter().rev() {
            self.dirty_key_and_visit_parents(key, &entries, invalidation_type, work);
        }
    }

    // Java: com/google/devtools/build/skyframe/InvalidatingNodeVisitor.java:519 DirtyingNodeVisitor.dirtyKeyAndVisitParents
    fn dirty_key_and_visit_parents(
        &mut self,
        key: SkyKey,
        entries: &[(SkyKey, std::sync::Arc<crate::in_memory_graph::Node>)],
        invalidation_type: InvalidationType,
        work: &mut Vec<VisitTask>,
    ) {
        let pair = (key.clone(), invalidation_type);
        let Some((_, node)) = entries.iter().find(|(k, _)| *k == key) else {
            self.pending_visitations.pending_values.shift_remove(&pair);
            return;
        };

        let is_changed = invalidation_type == InvalidationType::Changed;
        // The entry's calls are separate steps, as Java's `synchronized` methods are (a node
        // another visitor dirties in between makes `markDirty` return null, handled below).
        notify_graph_listener!(
            self.graph,
            &key,
            IsChanged,
            Before,
            crate::graph_listener::Context::Entry
        );
        let entry_is_changed = node.lock().entry.is_changed();
        let already_marked = entry_is_changed
            || (!is_changed && {
                notify_graph_listener!(
                    self.graph,
                    &key,
                    IsDirty,
                    Before,
                    crate::graph_listener::Context::Entry
                );
                node.lock().entry.is_dirty()
            });
        if already_marked {
            // If this node is already marked changed, or we are only marking this node dirty,
            // and it already is, move along.
            self.pending_visitations.pending_values.shift_remove(&pair);
            return;
        }

        let dirty_type = if is_changed {
            DirtyType::Change
        } else {
            DirtyType::Dirty
        };

        // This entry remains in the graph in this dirty state until it is re-evaluated.
        notify_graph_listener!(
            self.graph,
            &key,
            MarkDirty,
            Before,
            crate::graph_listener::Context::DirtyType(dirty_type)
        );
        let marked_dirty_result = {
            let mut state = node.lock();
            let r = state.entry.mark_dirty(dirty_type);
            state.refresh_snapshot();
            r.map(|r| (r, dirty_type))
        };
        notify_graph_listener!(
            self.graph,
            &key,
            MarkDirty,
            After,
            crate::graph_listener::Context::MarkDirtyAfter(
                crate::graph_listener::MarkDirtyAfterContext {
                    dirty_type,
                    actually_dirtied: marked_dirty_result.is_some(),
                }
            )
        );
        let Some((marked_dirty_result, dirty_type)) = marked_dirty_result else {
            // Another thread has already dirtied this node. Don't do anything in this thread.
            self.pending_visitations.pending_values.shift_remove(&pair);
            return;
        };

        self.progress_receiver.dirtied(&key, dirty_type);
        self.pending_visitations.pending_values.shift_remove(&pair);

        // Propagate dirtiness upwards and mark this node dirty/changed. Reverse deps should only
        // be marked dirty (because only a dependency of theirs has changed).
        work.push(VisitTask {
            keys: marked_dirty_result.get_reverse_deps_unsafe().to_vec(),
            invalidation_type: InvalidationType::Dirtied,
            enqueueing_key_for_existence_check: Some(key),
        });
    }
}

/// Java's `FunctionHermeticity.toString()` (the enum constant's name).
fn hermeticity_name(h: Option<FunctionHermeticity>) -> &'static str {
    match h {
        Some(FunctionHermeticity::Hermetic) => "HERMETIC",
        Some(FunctionHermeticity::SemiHermetic) => "SEMI_HERMETIC",
        Some(FunctionHermeticity::Nonhermetic) => "NONHERMETIC",
        None => "null",
    }
}

/// Java's `Pair.toString()`: `(first, second)`.
struct PairDebug<'a>(&'a (SkyKey, InvalidationType));

impl std::fmt::Debug for PairDebug<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let t = match self.0.1 {
            InvalidationType::Changed => "CHANGED",
            InvalidationType::Dirtied => "DIRTIED",
            InvalidationType::Deleted => "DELETED",
        };
        write!(f, "({:?}, {t})", self.0.0)
    }
}

/// Utility class for performing eager invalidation on Skyframe graphs.
// Java: com/google/devtools/build/skyframe/EagerInvalidator.java:27 EagerInvalidator
pub(crate) struct EagerInvalidator;

impl EagerInvalidator {
    /// Deletes given values. The `traverse_graph` parameter controls whether this method deletes
    /// (transitive) dependents of these nodes and relevant graph edges, or just the nodes
    /// themselves. Deleting just the nodes is inconsistent unless the graph will not be used for
    /// incremental builds in the future, but unfortunately there is a case where we delete nodes
    /// intra-build. As long as the full upward transitive closure of the nodes is specified for
    /// deletion, the graph remains consistent.
    // Java: com/google/devtools/build/skyframe/EagerInvalidator.java:39 EagerInvalidator.delete
    pub(crate) fn delete(
        graph: &InMemoryGraphImpl,
        diff: impl IntoIterator<Item = SkyKey>,
        progress_receiver: &DirtyAndInflightTrackingProgressReceiver,
        state: &mut DeletingInvalidationState,
        traverse_graph: bool,
    ) {
        if let Some(mut visitor) = EagerInvalidator::create_deleting_visitor_if_needed(
            graph,
            diff,
            progress_receiver,
            state,
            traverse_graph,
        ) {
            visitor.run();
        }
    }

    // Java: com/google/devtools/build/skyframe/EagerInvalidator.java:54 EagerInvalidator.createDeletingVisitorIfNeeded
    pub(crate) fn create_deleting_visitor_if_needed<'a>(
        graph: &'a InMemoryGraphImpl,
        diff: impl IntoIterator<Item = SkyKey>,
        progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
        state: &'a mut DeletingInvalidationState,
        traverse_graph: bool,
    ) -> Option<DeletingNodeVisitor<'a>> {
        state.update(diff);
        if state.is_empty() {
            None
        } else {
            Some(DeletingNodeVisitor::new(
                graph,
                progress_receiver,
                state,
                traverse_graph,
            ))
        }
    }

    // Java: com/google/devtools/build/skyframe/EagerInvalidator.java:67 EagerInvalidator.createInvalidatingVisitorIfNeeded
    pub(crate) fn create_invalidating_visitor_if_needed<'a>(
        graph: &'a InMemoryGraphImpl,
        diff: impl IntoIterator<Item = SkyKey>,
        progress_receiver: &'a DirtyAndInflightTrackingProgressReceiver,
        state: &'a mut InvalidationState,
    ) -> Option<DirtyingNodeVisitor<'a>> {
        state.update(diff);
        if state.is_empty() {
            None
        } else {
            Some(DirtyingNodeVisitor::new(graph, progress_receiver, state))
        }
    }

    /// Invalidates given values and their upward transitive closure in the graph if necessary.
    // Java: com/google/devtools/build/skyframe/EagerInvalidator.java:78 EagerInvalidator.invalidate
    pub(crate) fn invalidate(
        graph: &InMemoryGraphImpl,
        diff: impl IntoIterator<Item = SkyKey>,
        progress_receiver: &DirtyAndInflightTrackingProgressReceiver,
        state: &mut InvalidationState,
    ) {
        if let Some(mut visitor) = EagerInvalidator::create_invalidating_visitor_if_needed(
            graph,
            diff,
            progress_receiver,
            state,
        ) {
            visitor.run();
        }
    }
}
