//! The statistics half of `InMemoryMemoizingEvaluator.ProgressReceiver` and
//! `SkyframeGraphStatsEvent` (posted by `InMemoryMemoizingEvaluator.postLoggingStats`).
//!
//! PORT: Java's `ProgressReceiver` extends `DirtyAndInflightTrackingProgressReceiver`; here the
//! statistics are a receiver of their own between the tracking receiver and the user's receiver
//! ([`StatsCollectingProgressReceiver`]): it counts, then forwards every call unchanged. The
//! counts do not depend on the order of the two.
//!
//! PORT: java-hash-order. Java keeps each count in a `ConcurrentHashMultiset<SkyFunctionName>`
//! (over a `ConcurrentHashMap` of initial capacity `availableProcessors()`), so the order of
//! the reported entries is that map's iteration order. Here each count keeps its function names
//! in first-insertion order; the consumer replays them into a `ConcurrentHashMap` model.

use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::node_entry::DirtyType;
use crate::progress::{EvaluationProgressReceiver, EvaluationState, NodeState};
use crate::{ErrorInfo, GroupedDeps, SkyKey, SkyValue};

/// Data about the Skyframe evaluations that happened during this command: per category, the
/// function names with their counts, in first-insertion order.
// Java: com/google/devtools/build/skyframe/SkyframeGraphStatsEvent.java:25 SkyframeGraphStatsEvent.EvaluationStats
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvaluationStats {
    pub dirtied: Vec<(&'static str, i32)>,
    pub changed: Vec<(&'static str, i32)>,
    pub built: Vec<(&'static str, i32)>,
    pub cleaned: Vec<(&'static str, i32)>,
    pub evaluated: Vec<(&'static str, i32)>,
}

/// `SkyframeGraphStatsEvent`: the graph size and the evaluation statistics.
// Java: com/google/devtools/build/skyframe/SkyframeGraphStatsEvent.java:23 SkyframeGraphStatsEvent
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkyframeGraphStatsEvent {
    // Java: com/google/devtools/build/skyframe/SkyframeGraphStatsEvent.java:40 SkyframeGraphStatsEvent.getGraphSize
    /// `getGraphSize()`: `InMemoryGraph.valuesSize()`.
    pub graph_size: i32,
    // Java: com/google/devtools/build/skyframe/SkyframeGraphStatsEvent.java:44 SkyframeGraphStatsEvent.getEvaluationStats
    pub evaluation_stats: EvaluationStats,
}

/// The five multisets of `InMemoryMemoizingEvaluator.ProgressReceiver`.
#[derive(Debug, Default)]
pub(crate) struct EvaluationStatsCounters {
    // Nodes that were dirtied because one of their transitive dependencies changed
    dirtied: Mutex<IndexMap<&'static str, i32>>,
    // Nodes that were dirtied because they themselves changed (for example, a leaf node that
    // represents a file and that changed between builds)
    changed: Mutex<IndexMap<&'static str, i32>>,
    // Nodes that were built and found different from the previous version
    built: Mutex<IndexMap<&'static str, i32>>,
    // Nodes that were built and found to be same as the previous version
    cleaned: Mutex<IndexMap<&'static str, i32>>,
    // Nodes that were computed during the build
    evaluated: Mutex<IndexMap<&'static str, i32>>,
}

fn add(multiset: &Mutex<IndexMap<&'static str, i32>>, key: &SkyKey) {
    *multiset.lock().entry(key.function_name().0).or_insert(0) += 1;
}

fn from_multiset(multiset: &Mutex<IndexMap<&'static str, i32>>) -> Vec<(&'static str, i32)> {
    let mut map = multiset.lock();
    let result = map.iter().map(|(k, v)| (*k, *v)).collect();
    map.clear();
    result
}

/// A copy of the five counts (see [`crate::Evaluator::evaluation_stats_snapshot`]).
#[derive(Clone, Debug)]
pub struct EvaluationStatsSnapshot([IndexMap<&'static str, i32>; 5]);

impl EvaluationStatsCounters {
    fn multisets(&self) -> [&Mutex<IndexMap<&'static str, i32>>; 5] {
        [
            &self.dirtied,
            &self.changed,
            &self.built,
            &self.cleaned,
            &self.evaluated,
        ]
    }

    pub(crate) fn snapshot(&self) -> EvaluationStatsSnapshot {
        EvaluationStatsSnapshot(self.multisets().map(|multiset| multiset.lock().clone()))
    }

    pub(crate) fn restore(&self, snapshot: EvaluationStatsSnapshot) {
        for (multiset, saved) in self.multisets().into_iter().zip(snapshot.0) {
            *multiset.lock() = saved;
        }
    }

    /// Removes the counts of `function` from every statistic.
    pub(crate) fn remove_function(&self, function: crate::SkyFunctionName) {
        for multiset in [
            &self.dirtied,
            &self.changed,
            &self.built,
            &self.cleaned,
            &self.evaluated,
        ] {
            multiset.lock().shift_remove(function.0);
        }
    }

    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:82 InMemoryMemoizingEvaluator.ProgressReceiver.aggregateAndReset
    pub(crate) fn aggregate_and_reset(&self) -> EvaluationStats {
        EvaluationStats {
            dirtied: from_multiset(&self.dirtied),
            changed: from_multiset(&self.changed),
            built: from_multiset(&self.built),
            cleaned: from_multiset(&self.cleaned),
            evaluated: from_multiset(&self.evaluated),
        }
    }
}

/// Counts the statistics, then forwards to the wrapped receiver.
pub(crate) struct StatsCollectingProgressReceiver {
    pub(crate) stats: Arc<EvaluationStatsCounters>,
    pub(crate) inner: Arc<dyn EvaluationProgressReceiver>,
}

impl EvaluationProgressReceiver for StatsCollectingProgressReceiver {
    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:101 InMemoryMemoizingEvaluator.ProgressReceiver.dirtied
    fn dirtied(&self, sky_key: &SkyKey, dirty_type: DirtyType) {
        self.inner.dirtied(sky_key, dirty_type);
        match dirty_type {
            DirtyType::Dirty => add(&self.stats.dirtied, sky_key),
            DirtyType::Change => add(&self.stats.changed, sky_key),
            DirtyType::Rewind => {} // Should not happen but let's not crash the server due to logging
        }
    }

    fn deleted(&self, sky_key: &SkyKey) {
        self.inner.deleted(sky_key);
    }

    fn enqueueing(&self, sky_key: &SkyKey) {
        self.inner.enqueueing(sky_key);
    }

    fn state_starting(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.inner.state_starting(sky_key, node_state);
    }

    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:131 InMemoryMemoizingEvaluator.ProgressReceiver.stateEnding
    fn state_ending(&self, sky_key: &SkyKey, node_state: NodeState) {
        self.inner.state_ending(sky_key, node_state);
        if node_state == NodeState::Compute {
            add(&self.stats.evaluated, sky_key);
        }
    }

    // Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:113 InMemoryMemoizingEvaluator.ProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        new_value: Option<&SkyValue>,
        new_error: Option<&ErrorInfo>,
        direct_deps: Option<&GroupedDeps>,
    ) {
        self.inner
            .evaluated(sky_key, state, new_value, new_error, direct_deps);
        // Without direct deps no actual evaluation work was done, so it is not recorded.
        if direct_deps.is_some() {
            if state.version_changed() {
                add(&self.stats.built, sky_key);
            } else {
                add(&self.stats.cleaned, sky_key);
            }
        }
    }

    fn change_pruned(&self, sky_key: &SkyKey) {
        self.inner.change_pruned(sky_key);
    }
}
