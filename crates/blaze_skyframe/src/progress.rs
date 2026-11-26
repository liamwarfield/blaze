//! Ports of `EvaluationProgressReceiver` and the receivers built on it:
//! `InflightTrackingProgressReceiver`, `InflightOnlyTrackingProgressReceiver`,
//! `DirtyAndInflightTrackingProgressReceiver`, `CompoundEvaluationProgressReceiverBase` and
//! `CompoundEvaluationProgressReceiver`.

use crate::node_entry::DirtyType;
use crate::{ErrorInfo, GroupedDeps, SkyKey, SkyValue};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

/// Receiver for various stages of the lifetime of a skyframe node evaluation.
///
/// Thread safe: every method may be called concurrently from evaluator threads.
// Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:22 EvaluationProgressReceiver
pub trait EvaluationProgressReceiver: Send + Sync {
    /// Notifies that the node for `sky_key` has been marked dirty (`NodeEntry.markDirty`) with
    /// the given [`DirtyType`].
    ///
    /// May be called concurrently from multiple threads.
    ///
    /// Only called after a successful `NodeEntry.markDirty` call: a call that returns a non-null
    /// value.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:97 EvaluationProgressReceiver.dirtied
    fn dirtied(&self, _sky_key: &SkyKey, _dirty_type: DirtyType) {}

    /// Notifies that the node for `sky_key` was deleted.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:100 EvaluationProgressReceiver.deleted
    fn deleted(&self, _sky_key: &SkyKey) {}

    /// Notifies that `sky_key` is about to get queued for evaluation.
    ///
    /// Note that we don't guarantee that it actually got enqueued or will, only that if
    /// everything "goes well" (e.g. no interrupts happen) it will.
    ///
    /// This guarantee is intentionally vague to encourage writing robust implementations.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:110 EvaluationProgressReceiver.enqueueing
    fn enqueueing(&self, _sky_key: &SkyKey) {}

    /// Notifies that the node for `sky_key` is about to enter the given `node_state`.
    ///
    /// Notably, this includes `SkyFunction.compute` calls due to Skyframe restarts, but also
    /// dirtiness checking and node completion.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:118 EvaluationProgressReceiver.stateStarting
    fn state_starting(&self, _sky_key: &SkyKey, _node_state: NodeState) {}

    /// Notifies that the node for `sky_key` is about to complete the given `node_state`.
    ///
    /// Always called symmetrically with [`EvaluationProgressReceiver::state_starting`].
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:125 EvaluationProgressReceiver.stateEnding
    fn state_ending(&self, _sky_key: &SkyKey, _node_state: NodeState) {}

    /// Notifies that the node for `sky_key` has been evaluated.
    ///
    /// * `state`: the current state of the node for `sky_key`
    /// * `new_value`: the node's value if [`EvaluationState::version_changed`] and
    ///   [`EvaluationState::succeeded`], otherwise `None`
    /// * `new_error`: the node's error if it has one and [`EvaluationState::version_changed`]
    /// * `direct_deps`: direct dependencies of `sky_key` if the node was just built, otherwise
    ///   `None`
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:137 EvaluationProgressReceiver.evaluated
    fn evaluated(
        &self,
        _sky_key: &SkyKey,
        _state: EvaluationState,
        _new_value: Option<&SkyValue>,
        _new_error: Option<&ErrorInfo>,
        _direct_deps: Option<&GroupedDeps>,
    ) {
    }

    /// Notifies that the node for `sky_key` has been change pruned.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:145 EvaluationProgressReceiver.changePruned
    fn change_pruned(&self, _sky_key: &SkyKey) {}
}

/// A no-op [`EvaluationProgressReceiver`].
// Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:25 EvaluationProgressReceiver.NULL
pub static NULL: &(dyn EvaluationProgressReceiver + 'static) = &NullEvaluationProgressReceiver;

struct NullEvaluationProgressReceiver;

impl EvaluationProgressReceiver for NullEvaluationProgressReceiver {}

/// A static receiver (`NULL`) as an owned one.
impl EvaluationProgressReceiver for &'static (dyn EvaluationProgressReceiver + 'static) {
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        (**self).dirtied(sky_key, dirty_type)
    }
    fn deleted(&self, sky_key: &SkyKey) {
        (**self).deleted(sky_key)
    }
    fn enqueueing(&self, sky_key: &SkyKey) {
        (**self).enqueueing(sky_key)
    }
    fn state_starting(&self, sky_key: &SkyKey, node_state: NodeState) {
        (**self).state_starting(sky_key, node_state)
    }
    fn state_ending(&self, sky_key: &SkyKey, node_state: NodeState) {
        (**self).state_ending(sky_key, node_state)
    }
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        (**self).evaluated(sky_key, state, new_value, new_error, direct_deps)
    }
    fn change_pruned(&self, sky_key: &SkyKey) {
        (**self).change_pruned(sky_key)
    }
}

/// The state of a node after it was evaluated.
// Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:28 EvaluationProgressReceiver.EvaluationState
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvaluationState {
    SuccessVersionChanged,
    SuccessVersionUnchanged,
    FailVersionChanged,
    /// Note: Java constructs this constant with `versionChanged = true`
    /// (`FAIL_VERSION_UNCHANGED(false, true)`), so [`EvaluationState::version_changed`] returns
    /// true for it. Ported as written.
    FailVersionUnchanged,
}

impl EvaluationState {
    /// PORT: Java takes the value maybe wrapped in `ValueWithMetadata` and tests
    /// `ValueWithMetadata.justValue(valueMaybeWithMetadata) != null`; callers here pass the
    /// just-value (`None` when the node has only an error), which is what that test reads (the
    /// evaluator's callers have it at hand, a `SkyValue` has no wrapper type).
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:34 EvaluationProgressReceiver.EvaluationState.get
    pub(crate) fn get(just_value: Option<&SkyValue>, version_changed: bool) -> EvaluationState {
        let success = just_value.is_some();
        if version_changed {
            if success {
                EvaluationState::SuccessVersionChanged
            } else {
                EvaluationState::FailVersionChanged
            }
        } else if success {
            EvaluationState::SuccessVersionUnchanged
        } else {
            EvaluationState::FailVersionUnchanged
        }
    }

    /// Whether the node has a value.
    ///
    /// If `false`, the node has only an error and no value.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:56 EvaluationProgressReceiver.EvaluationState.succeeded
    pub fn succeeded(&self) -> bool {
        match self {
            EvaluationState::SuccessVersionChanged | EvaluationState::SuccessVersionUnchanged => {
                true
            }
            EvaluationState::FailVersionChanged | EvaluationState::FailVersionUnchanged => false,
        }
    }

    /// Whether the node's `NodeEntry.getVersion` changed as a result of this evaluation.
    ///
    /// If `true`, the node was built at the current version and its `NodeEntry.getVersion`
    /// changed, either because it was built incrementally and changed or was built as part of a
    /// clean build. Parents need to be rebuilt.
    ///
    /// If `false`, the node's `NodeEntry.getVersion` did not change, either because it was deemed
    /// up-to-date and not built or was built incrementally and evaluated to the same value as its
    /// prior evaluation. Parents do not necessarily need to be rebuilt.
    // Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:71 EvaluationProgressReceiver.EvaluationState.versionChanged
    pub fn version_changed(&self) -> bool {
        match self {
            EvaluationState::SuccessVersionChanged => true,
            EvaluationState::SuccessVersionUnchanged => false,
            EvaluationState::FailVersionChanged => true,
            // Java: FAIL_VERSION_UNCHANGED(false, true).
            EvaluationState::FailVersionUnchanged => true,
        }
    }
}

/// Overall state of the node while it is being evaluated.
// Java: com/google/devtools/build/skyframe/EvaluationProgressReceiver.java:77 EvaluationProgressReceiver.NodeState
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeState {
    /// The node is undergoing a dirtiness check and may be re-validated.
    CheckDirty,
    /// The node is prepping for evaluation.
    InitializingEnvironment,
    /// The node is in compute().
    Compute,
    /// The node is done evaluation and committing the result.
    Commit,
}

/// An [`EvaluationProgressReceiver`] that also tracks the state of inflight keys.
// Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:19 InflightTrackingProgressReceiver
pub(crate) trait InflightTrackingProgressReceiver: EvaluationProgressReceiver {
    /// Called when a node is injected into the graph, and not evaluated.
    // Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:22 InflightTrackingProgressReceiver.injected
    fn injected(&self, sky_key: &SkyKey);

    /// Called when a node was requested to be enqueued but wasn't because either an interrupt or
    /// an error (in `--nokeep_going` mode) had occurred.
    // Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:28 InflightTrackingProgressReceiver.enqueueAfterError
    fn enqueue_after_error(&self, sky_key: &SkyKey);

    /// Returns whether the given key is enqueued for evaluation.
    // Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:31 InflightTrackingProgressReceiver.isInflight
    fn is_inflight(&self, sky_key: &SkyKey) -> bool;

    /// Removes the given key from the set of inflight keys.
    // Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:34 InflightTrackingProgressReceiver.removeFromInflight
    #[allow(dead_code)] // Java calls it for partial reevaluation only (not ported)
    fn remove_from_inflight(&self, sky_key: &SkyKey);

    /// Returns the set of all keys that are enqueued for evaluation, and resets the set to empty.
    // Java: com/google/devtools/build/skyframe/InflightTrackingProgressReceiver.java:37 InflightTrackingProgressReceiver.getAndClearInflightKeys
    fn get_and_clear_inflight_keys(&self) -> HashSet<SkyKey>;
}

/// A delegating [`InflightTrackingProgressReceiver`] that tracks inflight keys but not dirty
/// keys.
///
/// Suitable for non-incremental evaluations or evaluators that do not support deletion of dirty
/// nodes.
// Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:31 InflightOnlyTrackingProgressReceiver
#[allow(dead_code)] // Java uses it for evaluators without dirty tracking; this evaluator always tracks
pub(crate) struct InflightOnlyTrackingProgressReceiver {
    progress_receiver: Arc<dyn EvaluationProgressReceiver>,
    inflight_keys: Mutex<HashSet<SkyKey>>,
}

#[allow(dead_code)]
impl InflightOnlyTrackingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:36 InflightOnlyTrackingProgressReceiver.<init>
    pub(crate) fn new(progress_receiver: Arc<dyn EvaluationProgressReceiver>) -> Self {
        InflightOnlyTrackingProgressReceiver {
            progress_receiver,
            inflight_keys: Mutex::new(HashSet::new()),
        }
    }
}

impl EvaluationProgressReceiver for InflightOnlyTrackingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:48 InflightOnlyTrackingProgressReceiver.dirtied
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        self.progress_receiver.dirtied(sky_key, dirty_type);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:53 InflightOnlyTrackingProgressReceiver.deleted
    fn deleted(&self, sky_key: &SkyKey) {
        self.progress_receiver.deleted(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:58 InflightOnlyTrackingProgressReceiver.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        if self.inflight_keys.lock().insert(sky_key.clone()) {
            // Only tell the external listener the node was enqueued if no there was neither an
            // error nor interrupt.
            self.progress_receiver.enqueueing(sky_key);
        }
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:72 InflightOnlyTrackingProgressReceiver.stateStarting
    fn state_starting(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.progress_receiver.state_starting(sky_key, node_state);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:77 InflightOnlyTrackingProgressReceiver.stateEnding
    fn state_ending(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.progress_receiver.state_ending(sky_key, node_state);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:82 InflightOnlyTrackingProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        self.progress_receiver
            .evaluated(sky_key, state, new_value, new_error, direct_deps);

        // This key was either built or marked clean, so we can remove it from both the dirty and
        // inflight nodes.
        self.inflight_keys.lock().remove(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:96 InflightOnlyTrackingProgressReceiver.changePruned
    fn change_pruned(&self, sky_key: &SkyKey) {
        self.progress_receiver.change_pruned(sky_key);
    }
}

impl InflightTrackingProgressReceiver for InflightOnlyTrackingProgressReceiver {
    /// Called when a node is injected into the graph, and not evaluated.
    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:42 InflightOnlyTrackingProgressReceiver.injected
    fn injected(&self, sky_key: &SkyKey) {
        // This node was never evaluated, but is now clean and need not be re-evaluated.
        self.inflight_keys.lock().remove(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:67 InflightOnlyTrackingProgressReceiver.enqueueAfterError
    fn enqueue_after_error(&self, sky_key: &SkyKey) {
        self.inflight_keys.lock().insert(sky_key.clone());
    }

    /// Returns if the key is enqueued for evaluation.
    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:102 InflightOnlyTrackingProgressReceiver.isInflight
    fn is_inflight(&self, sky_key: &SkyKey) -> bool {
        self.inflight_keys.lock().contains(sky_key)
    }

    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:107 InflightOnlyTrackingProgressReceiver.removeFromInflight
    fn remove_from_inflight(&self, sky_key: &SkyKey) {
        self.inflight_keys.lock().remove(sky_key);
    }

    /// Returns the set of all keys that are enqueued for evaluation, and resets the set to empty.
    // Java: com/google/devtools/build/skyframe/InflightOnlyTrackingProgressReceiver.java:114 InflightOnlyTrackingProgressReceiver.getAndClearInflightKeys
    fn get_and_clear_inflight_keys(&self) -> HashSet<SkyKey> {
        std::mem::take(&mut *self.inflight_keys.lock())
    }
}

/// A delegating [`InflightTrackingProgressReceiver`] that tracks both inflight and dirty keys.
// Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:27 DirtyAndInflightTrackingProgressReceiver
pub(crate) struct DirtyAndInflightTrackingProgressReceiver {
    progress_receiver: Arc<dyn EvaluationProgressReceiver>,
    dirty_keys: ConcurrentSet,
    inflight_keys: ConcurrentSet,
    unsuccessfully_rewound_keys: ConcurrentSet,
}

/// Java: `Sets.newConcurrentHashSet()`, a set many evaluator threads update at once.
///
/// PORT: a fixed number of locked shards (Java's `ConcurrentHashMap` locks per bin); every
/// evaluated node goes through the in-flight set twice.
pub(crate) struct ConcurrentSet {
    shards: Vec<Mutex<HashSet<SkyKey>>>,
    hasher: std::collections::hash_map::RandomState,
}

impl ConcurrentSet {
    const SHARDS: usize = 64;

    pub(crate) fn new() -> Self {
        ConcurrentSet {
            shards: (0..Self::SHARDS)
                .map(|_| Mutex::new(HashSet::new()))
                .collect(),
            hasher: Default::default(),
        }
    }

    fn shard(&self, key: &SkyKey) -> &Mutex<HashSet<SkyKey>> {
        use std::hash::BuildHasher;
        &self.shards[(self.hasher.hash_one(key) as usize) % Self::SHARDS]
    }

    pub(crate) fn insert(&self, key: &SkyKey) -> bool {
        let mut shard = self.shard(key).lock();
        if shard.contains(key) {
            return false;
        }
        shard.insert(key.clone())
    }

    pub(crate) fn remove(&self, key: &SkyKey) -> bool {
        self.shard(key).lock().remove(key)
    }

    pub(crate) fn contains(&self, key: &SkyKey) -> bool {
        self.shard(key).lock().contains(key)
    }

    /// A copy of the set.
    pub(crate) fn snapshot(&self) -> HashSet<SkyKey> {
        self.shards
            .iter()
            .flat_map(|s| s.lock().iter().cloned().collect::<Vec<_>>())
            .collect()
    }

    /// Returns the set and empties it.
    pub(crate) fn take(&self) -> HashSet<SkyKey> {
        let mut all = HashSet::new();
        for s in &self.shards {
            all.extend(std::mem::take(&mut *s.lock()));
        }
        all
    }
}

impl DirtyAndInflightTrackingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:34 DirtyAndInflightTrackingProgressReceiver.<init>
    pub(crate) fn new(progress_receiver: Arc<dyn EvaluationProgressReceiver>) -> Self {
        DirtyAndInflightTrackingProgressReceiver {
            progress_receiver,
            dirty_keys: ConcurrentSet::new(),
            inflight_keys: ConcurrentSet::new(),
            unsuccessfully_rewound_keys: ConcurrentSet::new(),
        }
    }

    /// The same dirty and in-flight sets, forwarding to another receiver.
    ///
    /// PORT: Blaze-only. Java's evaluator keeps one receiver for its lifetime; Blaze hands the
    /// graph of one command's evaluator to the next command's evaluator, whose functions (and
    /// the receiver they report to) are the next command's
    /// ([`crate::EvaluatorBuilder::build_reusing`]). The sets belong to the graph.
    pub(crate) fn with_receiver(
        self,
        progress_receiver: Arc<dyn EvaluationProgressReceiver>,
    ) -> Self {
        DirtyAndInflightTrackingProgressReceiver {
            progress_receiver,
            dirty_keys: self.dirty_keys,
            inflight_keys: self.inflight_keys,
            unsuccessfully_rewound_keys: self.unsuccessfully_rewound_keys,
        }
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:63 DirtyAndInflightTrackingProgressReceiver.enqueueing
    fn enqueueing_after_error(&self, sky_key: &SkyKey, after_error: bool) {
        // We unconditionally add the key to the set of in-flight nodes even if evaluation is
        // never scheduled, because we still want to remove the previously created NodeEntry from
        // the graph. Otherwise we would leave the graph in a weird state (wasteful garbage in the
        // best case and inconsistent in the worst case).
        let newly_enqueued = self.inflight_keys.insert(sky_key);
        if newly_enqueued {
            // All nodes enqueued for evaluation will be either verified clean, re-evaluated, or
            // cleaned up after being in-flight when an error happens in nokeep_going mode or in
            // the event of an interrupt. In any of these cases, they won't be dirty anymore. Note
            // that we don't remove from unsuccessfullyRewoundKeys here - that is only done when
            // the key completes successfully.
            self.dirty_keys.remove(sky_key);
            if !after_error {
                // Only tell the external listener the node was enqueued if no there was neither
                // an error or interrupt.
                self.progress_receiver.enqueueing(sky_key);
            }
        }
    }

    /// Returns the set of all keys that were rewound (`DirtyType::Rewind`) but did not complete
    /// successfully, and resets the set to empty.
    ///
    /// The returned set includes keys that were rewound and were either:
    ///
    /// * not yet enqueued
    /// * enqueued but not evaluated
    /// * evaluated to an error
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:159 DirtyAndInflightTrackingProgressReceiver.getAndClearUnsuccessfullyRewoundKeys
    pub(crate) fn get_and_clear_unsuccessfully_rewound_keys(&self) -> HashSet<SkyKey> {
        self.unsuccessfully_rewound_keys.take()
    }

    /// Returns the set of all dirty keys that have not been enqueued. This is useful for garbage
    /// collection, where we would not want to remove dirty nodes that are needed for evaluation
    /// (in the downward transitive closure of the set of the evaluation's top level nodes).
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:170 DirtyAndInflightTrackingProgressReceiver.getUnenqueuedDirtyKeys
    pub(crate) fn get_unenqueued_dirty_keys(&self) -> HashSet<SkyKey> {
        self.dirty_keys.snapshot()
    }

    /// The keys in flight (enqueued and not done), without clearing them. Test access for the
    /// model driver's graph invariants.
    #[cfg(test)]
    pub(crate) fn inflight_keys_for_testing(&self) -> HashSet<SkyKey> {
        self.inflight_keys.snapshot()
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:174 DirtyAndInflightTrackingProgressReceiver.addToDirtySet
    fn add_to_dirty_set(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        if dirty_type == DirtyType::Rewind {
            self.unsuccessfully_rewound_keys.insert(sky_key);
        } else {
            self.dirty_keys.insert(sky_key);
        }
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:182 DirtyAndInflightTrackingProgressReceiver.removeFromDirtySet
    fn remove_from_dirty_set(&self, sky_key: &SkyKey) {
        // A key will never be present in both sets because EvaluationProgressReceiver#dirtied is
        // only called after successful NodeEntry#markDirty calls, i.e. a call that transitioned
        // the node from done to dirty.
        if !self.dirty_keys.remove(sky_key) {
            self.unsuccessfully_rewound_keys.remove(sky_key);
        }
    }
}

impl EvaluationProgressReceiver for DirtyAndInflightTrackingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:46 DirtyAndInflightTrackingProgressReceiver.dirtied
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        self.progress_receiver.dirtied(sky_key, dirty_type);
        self.add_to_dirty_set(sky_key, dirty_type);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:52 DirtyAndInflightTrackingProgressReceiver.deleted
    fn deleted(&self, sky_key: &SkyKey) {
        self.progress_receiver.deleted(sky_key);
        // This key was removed from the graph, so no longer needs to be marked as dirty.
        self.remove_from_dirty_set(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:59 DirtyAndInflightTrackingProgressReceiver.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        self.enqueueing_after_error(sky_key, false);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:85 DirtyAndInflightTrackingProgressReceiver.changePruned
    fn change_pruned(&self, sky_key: &SkyKey) {
        self.progress_receiver.change_pruned(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:99 DirtyAndInflightTrackingProgressReceiver.stateStarting
    fn state_starting(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.progress_receiver.state_starting(sky_key, node_state);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:104 DirtyAndInflightTrackingProgressReceiver.stateEnding
    fn state_ending(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.progress_receiver.state_ending(sky_key, node_state);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:109 DirtyAndInflightTrackingProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        self.progress_receiver
            .evaluated(sky_key, state, new_value, new_error, direct_deps);

        // This key was either built or marked clean, so we can remove it from both the dirty and
        // inflight nodes.
        self.inflight_keys.remove(sky_key);

        if state.succeeded() {
            self.remove_from_dirty_set(sky_key);
        } else {
            // Leave unsuccessful keys in unsuccessfullyRewoundKeys. Only remove them from
            // dirtyKeys.
            self.dirty_keys.remove(sky_key);
        }
    }
}

impl InflightTrackingProgressReceiver for DirtyAndInflightTrackingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:39 DirtyAndInflightTrackingProgressReceiver.injected
    fn injected(&self, sky_key: &SkyKey) {
        // This node was never evaluated, but is now clean and need not be re-evaluated.
        self.inflight_keys.remove(sky_key);
        self.remove_from_dirty_set(sky_key);
    }

    /// Called when a node was requested to be enqueued but wasn't because either an interrupt or
    /// an error (in nokeep_going mode) had occurred.
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:94 DirtyAndInflightTrackingProgressReceiver.enqueueAfterError
    fn enqueue_after_error(&self, sky_key: &SkyKey) {
        self.enqueueing_after_error(sky_key, true);
    }

    /// Returns if the key is enqueued for evaluation.
    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:131 DirtyAndInflightTrackingProgressReceiver.isInflight
    fn is_inflight(&self, sky_key: &SkyKey) -> bool {
        self.inflight_keys.contains(sky_key)
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:136 DirtyAndInflightTrackingProgressReceiver.removeFromInflight
    fn remove_from_inflight(&self, sky_key: &SkyKey) {
        self.inflight_keys.remove(sky_key);
    }

    // Java: com/google/devtools/build/skyframe/DirtyAndInflightTrackingProgressReceiver.java:141 DirtyAndInflightTrackingProgressReceiver.getAndClearInflightKeys
    fn get_and_clear_inflight_keys(&self) -> HashSet<SkyKey> {
        self.inflight_keys.take()
    }
}

/// Helper class to allow implementing [`EvaluationProgressReceiver`] implementations which
/// delegate to a bunch of other [`EvaluationProgressReceiver`]s.
///
/// PORT: Java's constructor is `protected` (subclasses only); Rust has no subclassing, so it is
/// `pub` and a "subclass" holds a `CompoundEvaluationProgressReceiverBase` and forwards to it.
// Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:24 CompoundEvaluationProgressReceiverBase
pub struct CompoundEvaluationProgressReceiverBase {
    pub receivers: Vec<Arc<dyn EvaluationProgressReceiver>>,
}

impl CompoundEvaluationProgressReceiverBase {
    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:27 CompoundEvaluationProgressReceiverBase.<init>
    pub fn new(receivers: Vec<Arc<dyn EvaluationProgressReceiver>>) -> Self {
        CompoundEvaluationProgressReceiverBase { receivers }
    }
}

impl EvaluationProgressReceiver for CompoundEvaluationProgressReceiverBase {
    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:33 CompoundEvaluationProgressReceiverBase.dirtied
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        for receiver in &self.receivers {
            receiver.dirtied(sky_key, dirty_type);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:40 CompoundEvaluationProgressReceiverBase.deleted
    fn deleted(&self, sky_key: &SkyKey) {
        for receiver in &self.receivers {
            receiver.deleted(sky_key);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:47 CompoundEvaluationProgressReceiverBase.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        for receiver in &self.receivers {
            receiver.enqueueing(sky_key);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:54 CompoundEvaluationProgressReceiverBase.stateStarting
    fn state_starting(&self, sky_key: &SkyKey, state: NodeState) {
        for receiver in &self.receivers {
            receiver.state_starting(sky_key, state);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:61 CompoundEvaluationProgressReceiverBase.stateEnding
    fn state_ending(&self, sky_key: &SkyKey, state: NodeState) {
        for receiver in &self.receivers {
            receiver.state_ending(sky_key, state);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:68 CompoundEvaluationProgressReceiverBase.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        for receiver in &self.receivers {
            receiver.evaluated(sky_key, state, new_value, new_error, direct_deps);
        }
    }

    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiverBase.java:80 CompoundEvaluationProgressReceiverBase.changePruned
    fn change_pruned(&self, sky_key: &SkyKey) {
        for receiver in &self.receivers {
            receiver.change_pruned(sky_key);
        }
    }
}

/// An [`EvaluationProgressReceiver`] that delegates to a bunch of other
/// [`EvaluationProgressReceiver`]s.
// Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiver.java:22 CompoundEvaluationProgressReceiver
pub struct CompoundEvaluationProgressReceiver(CompoundEvaluationProgressReceiverBase);

impl CompoundEvaluationProgressReceiver {
    // Java: com/google/devtools/build/skyframe/CompoundEvaluationProgressReceiver.java:24 CompoundEvaluationProgressReceiver.<init>
    pub fn new(receivers: Vec<Arc<dyn EvaluationProgressReceiver>>) -> Self {
        CompoundEvaluationProgressReceiver(CompoundEvaluationProgressReceiverBase::new(receivers))
    }
}

/// Java inherits every method from `CompoundEvaluationProgressReceiverBase`.
impl EvaluationProgressReceiver for CompoundEvaluationProgressReceiver {
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        self.0.dirtied(sky_key, dirty_type);
    }
    fn deleted(&self, sky_key: &SkyKey) {
        self.0.deleted(sky_key);
    }
    fn enqueueing(&self, sky_key: &SkyKey) {
        self.0.enqueueing(sky_key);
    }
    fn state_starting(&self, sky_key: &SkyKey, state: NodeState) {
        self.0.state_starting(sky_key, state);
    }
    fn state_ending(&self, sky_key: &SkyKey, state: NodeState) {
        self.0.state_ending(sky_key, state);
    }
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        self.0
            .evaluated(sky_key, state, new_value, new_error, direct_deps);
    }
    fn change_pruned(&self, sky_key: &SkyKey) {
        self.0.change_pruned(sky_key);
    }
}
