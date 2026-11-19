//! Ports of `Differencer` (with `Diff` and `DiffWithDelta.Delta`), `ImmutableDiff`,
//! `RecordingDifferencer`, `SequencedRecordingDifferencer` and `Injectable`.

use crate::evaluator::ERROR_TRANSIENCE_KEY;
use crate::version::Version;
use crate::{SkyKey, SkyValue};
use indexmap::IndexMap;

/// Represents the delta between two values of the same key.
///
/// * `old_value`: the old value, if any.
/// * `new_value`: the new value.
/// * `new_max_transitive_source_version`: the max transitive source version of the new value.
// Java: com/google/devtools/build/skyframe/Differencer.java:62 Differencer.DiffWithDelta.Delta
#[derive(Clone)]
pub(crate) struct Delta {
    #[allow(dead_code)] // Java API; nothing reads the old value at the pin
    pub(crate) old_value: Option<SkyValue>,
    pub(crate) new_value: SkyValue,
    pub(crate) new_max_transitive_source_version: Option<Version>,
}

impl Delta {
    // Java: com/google/devtools/build/skyframe/Differencer.java:67 Delta.justNew
    pub(crate) fn just_new(new_value: SkyValue) -> Delta {
        Delta::changed(None, new_value, None)
    }

    // Java: com/google/devtools/build/skyframe/Differencer.java:76 Delta.changed
    pub(crate) fn changed(
        old_value: Option<SkyValue>,
        new_value: SkyValue,
        new_max_transitive_source_version: Option<Version>,
    ) -> Delta {
        Delta {
            old_value,
            new_value,
            new_max_transitive_source_version,
        }
    }
}

/// Represents a set of changed values.
// Java: com/google/devtools/build/skyframe/Differencer.java:29 Differencer.Diff
pub(crate) trait Diff {
    // Java: com/google/devtools/build/skyframe/Differencer.java:30 Diff.isEmpty
    fn is_empty(&self) -> bool {
        self.changed_keys_without_new_values().is_empty()
            && self.changed_keys_with_new_values().is_empty()
    }

    /// Returns the value keys whose values have changed, but for which we don't have the new
    /// values.
    // Java: com/google/devtools/build/skyframe/Differencer.java:37 Diff.changedKeysWithoutNewValues
    fn changed_keys_without_new_values(&self) -> &[SkyKey];

    /// Returns the value keys whose values have changed, along with their new values wrapped in a
    /// [`Delta`].
    ///
    /// The values in here cannot have any dependencies. This is required in order to prevent
    /// conflation of injected values and derived values.
    // Java: com/google/devtools/build/skyframe/Differencer.java:46 Diff.changedKeysWithNewValues
    fn changed_keys_with_new_values(&self) -> &IndexMap<SkyKey, Delta>;
}

/// Calculate set of changed values in a graph.
///
/// PORT: `getDiff`'s `fromGraph` (a `WalkableGraph`) and versions are not read by the one
/// implementation ported (`SequencedRecordingDifferencer`), so they are not passed.
// Java: com/google/devtools/build/skyframe/Differencer.java:24 Differencer
pub(crate) trait Differencer: Send {
    /// Returns the value keys that have changed between the two Versions.
    // Java: com/google/devtools/build/skyframe/Differencer.java:86 Differencer.getDiff
    fn get_diff(&mut self, from_version: Option<Version>, to_version: Version) -> ImmutableDiff;
}

/// Immutable implementation of [`Diff`].
// Java: com/google/devtools/build/skyframe/ImmutableDiff.java:23 ImmutableDiff
pub(crate) struct ImmutableDiff {
    values_to_invalidate: Vec<SkyKey>,
    values_to_inject: IndexMap<SkyKey, Delta>,
}

impl ImmutableDiff {
    // Java: com/google/devtools/build/skyframe/ImmutableDiff.java:28 ImmutableDiff.<init>
    pub(crate) fn new(
        values_to_invalidate: Vec<SkyKey>,
        values_to_inject: IndexMap<SkyKey, Delta>,
    ) -> Self {
        ImmutableDiff {
            values_to_invalidate,
            values_to_inject,
        }
    }
}

impl Diff for ImmutableDiff {
    // Java: com/google/devtools/build/skyframe/ImmutableDiff.java:34 ImmutableDiff.changedKeysWithoutNewValues
    fn changed_keys_without_new_values(&self) -> &[SkyKey] {
        &self.values_to_invalidate
    }

    // Java: com/google/devtools/build/skyframe/ImmutableDiff.java:39 ImmutableDiff.changedKeysWithNewValues
    fn changed_keys_with_new_values(&self) -> &IndexMap<SkyKey, Delta> {
        &self.values_to_inject
    }
}

/// An object that accepts Skyframe key / [`Delta`] mapping.
// Java: com/google/devtools/build/skyframe/Injectable.java:20 Injectable
pub(crate) trait Injectable {
    // Java: com/google/devtools/build/skyframe/Injectable.java:21 Injectable.inject
    fn inject(&mut self, key: SkyKey, delta: Delta);

    // Java: com/google/devtools/build/skyframe/Injectable.java:23 Injectable.inject
    #[allow(dead_code)] // Java API; the evaluator injects one key at a time
    fn inject_all(&mut self, deltas: IndexMap<SkyKey, Delta>);
}

/// A simple [`Differencer`] that is manually informed of invalid/injected nodes.
// Java: com/google/devtools/build/skyframe/RecordingDifferencer.java:20 RecordingDifferencer
pub(crate) trait RecordingDifferencer: Differencer + Injectable {
    /// Stores the given values for invalidation.
    // Java: com/google/devtools/build/skyframe/RecordingDifferencer.java:25 RecordingDifferencer.invalidate
    fn invalidate(&mut self, values: impl IntoIterator<Item = SkyKey>);

    /// Invalidates the cached values of any values in error transiently.
    ///
    /// If a future call to `MemoizingEvaluator#evaluate` requests a value that transitively
    /// depends on any value that was in an error state (or is one of these), they will be
    /// re-computed.
    // Java: com/google/devtools/build/skyframe/RecordingDifferencer.java:33 RecordingDifferencer.invalidateTransientErrors
    fn invalidate_transient_errors(&mut self) {
        // All transient error values have a dependency on the single global ERROR_TRANSIENCE
        // value, so we only have to invalidate that one value to catch everything.
        self.invalidate([ERROR_TRANSIENCE_KEY.clone()]);
    }
}

/// A simple Differencer which just records the invalidated values it's been given.
///
/// PORT: java-hash-order. Java keeps the injections in a `HashMap`; the order in which they are
/// injected reaches only progress receivers (`injected`, `dirtied`), so insertion order is used.
// Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:25 SequencedRecordingDifferencer
#[derive(Default)]
pub(crate) struct SequencedRecordingDifferencer {
    values_to_invalidate: Vec<SkyKey>,
    values_to_inject: IndexMap<SkyKey, Delta>,
}

impl SequencedRecordingDifferencer {
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:30 SequencedRecordingDifferencer.<init>
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:34 SequencedRecordingDifferencer.clear
    fn clear(&mut self) {
        self.values_to_invalidate = vec![];
        self.values_to_inject = IndexMap::new();
    }
}

impl Differencer for SequencedRecordingDifferencer {
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:39 SequencedRecordingDifferencer.getDiff
    fn get_diff(&mut self, _from_version: Option<Version>, _to_version: Version) -> ImmutableDiff {
        let diff = ImmutableDiff::new(
            std::mem::take(&mut self.values_to_invalidate),
            std::mem::take(&mut self.values_to_inject),
        );
        self.clear();
        diff
    }
}

impl Injectable for SequencedRecordingDifferencer {
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:57 SequencedRecordingDifferencer.inject
    fn inject(&mut self, key: SkyKey, delta: Delta) {
        self.values_to_inject.insert(key, delta);
    }

    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:52 SequencedRecordingDifferencer.inject
    fn inject_all(&mut self, deltas: IndexMap<SkyKey, Delta>) {
        self.values_to_inject.extend(deltas);
    }
}

impl RecordingDifferencer for SequencedRecordingDifferencer {
    // Java: com/google/devtools/build/skyframe/SequencedRecordingDifferencer.java:47 SequencedRecordingDifferencer.invalidate
    fn invalidate(&mut self, values: impl IntoIterator<Item = SkyKey>) {
        self.values_to_invalidate.extend(values);
    }
}
