//! Port of the test utility `InvalidationProgressReceiver`.

use crate::{DirtyType, EvaluationProgressReceiver, SkyKey};

/// New state of the value entry after invalidation.
// Java: src/test/java/com/google/devtools/build/skyframe/InvalidationProgressReceiver.java:37 InvalidationProgressReceiver.InvalidationState
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum InvalidationState {
    /// The value is dirty, although it might get re-validated again.
    Dirty,
    /// The value is dirty and got deleted, cannot get re-validated again.
    Deleted,
}

/// Test utility that funnels both `dirtied` and `deleted` to a single `invalidated` method.
///
/// PORT: Java is an abstract class whose `dirtied` and `deleted` are final; here a type
/// implements `invalidated` and is wrapped in [`InvalidationProgressReceiverAdapter`], which
/// provides the final methods.
// Java: src/test/java/com/google/devtools/build/skyframe/InvalidationProgressReceiver.java:24 InvalidationProgressReceiver
pub(crate) trait InvalidationProgressReceiver: Send + Sync {
    // Java: src/test/java/com/google/devtools/build/skyframe/InvalidationProgressReceiver.java:45 InvalidationProgressReceiver.invalidated
    fn invalidated(&self, sky_key: &SkyKey, state: InvalidationState);
}

/// The [`EvaluationProgressReceiver`] of an [`InvalidationProgressReceiver`].
pub(crate) struct InvalidationProgressReceiverAdapter<R>(pub(crate) R);

impl<R: InvalidationProgressReceiver> EvaluationProgressReceiver
    for InvalidationProgressReceiverAdapter<R>
{
    // Java: src/test/java/com/google/devtools/build/skyframe/InvalidationProgressReceiver.java:27 InvalidationProgressReceiver.dirtied
    fn dirtied(&self, sky_key: &SkyKey, _dirty_type: DirtyType) {
        self.0.invalidated(sky_key, InvalidationState::Dirty);
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/InvalidationProgressReceiver.java:32 InvalidationProgressReceiver.deleted
    fn deleted(&self, sky_key: &SkyKey) {
        self.0.invalidated(sky_key, InvalidationState::Deleted);
    }
}
