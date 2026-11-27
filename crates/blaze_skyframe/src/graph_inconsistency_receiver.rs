//! `GraphInconsistencyReceiver` and the `Inconsistency` enum of `graph_inconsistency.proto`.

use crate::SkyKey;
use std::fmt;

/// `skyframe.Inconsistency` (`graph_inconsistency.proto`).
// Java: com/google/devtools/build/skyframe/graph_inconsistency.proto:22 Inconsistency
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Inconsistency {
    ResetRequested = 1,
    /// Deprecated: children should never be missing.
    DirtyParentHadMissingChild = 2,
    ParentForceRebuildOfChild = 3,
    /// Deprecated: children requested for rewinding should be created if absent.
    ParentForceRebuildOfMissingChild = 4,
    BuildingParentFoundUndoneChild = 5,
    AlreadyDeclaredChildMissing = 6,
}

impl fmt::Display for Inconsistency {
    /// The Java enum constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Inconsistency::ResetRequested => "RESET_REQUESTED",
            Inconsistency::DirtyParentHadMissingChild => "DIRTY_PARENT_HAD_MISSING_CHILD",
            Inconsistency::ParentForceRebuildOfChild => "PARENT_FORCE_REBUILD_OF_CHILD",
            Inconsistency::ParentForceRebuildOfMissingChild => {
                "PARENT_FORCE_REBUILD_OF_MISSING_CHILD"
            }
            Inconsistency::BuildingParentFoundUndoneChild => "BUILDING_PARENT_FOUND_UNDONE_CHILD",
            Inconsistency::AlreadyDeclaredChildMissing => "ALREADY_DECLARED_CHILD_MISSING",
        })
    }
}

/// A receiver that can be informed of inconsistencies detected in Skyframe. Such inconsistencies
/// are usually the result of external data loss (such as nodes in the graph, or the results of
/// external computations stored in a remote execution service).
///
/// The receiver can tolerate such inconsistencies, or throw hard if they are unexpected.
// Java: com/google/devtools/build/skyframe/GraphInconsistencyReceiver.java:29 GraphInconsistencyReceiver
pub trait GraphInconsistencyReceiver: Send + Sync {
    // Java: com/google/devtools/build/skyframe/GraphInconsistencyReceiver.java:31 GraphInconsistencyReceiver.noteInconsistencyAndMaybeThrow
    fn note_inconsistency_and_maybe_throw(
        &self,
        key: &SkyKey,
        other_keys: Option<&[SkyKey]>,
        inconsistency: Inconsistency,
    );

    // Java: com/google/devtools/build/skyframe/GraphInconsistencyReceiver.java:45 GraphInconsistencyReceiver.reset
    fn reset(&self) {}
}

/// A [`GraphInconsistencyReceiver`] that crashes on any inconsistency.
// Java: com/google/devtools/build/skyframe/GraphInconsistencyReceiver.java:35 GraphInconsistencyReceiver.THROWING
pub struct Throwing;

impl GraphInconsistencyReceiver for Throwing {
    fn note_inconsistency_and_maybe_throw(
        &self,
        key: &SkyKey,
        other_keys: Option<&[SkyKey]>,
        inconsistency: Inconsistency,
    ) {
        // Java: "Unexpected inconsistency: " + key + ", " + otherKey + ", " + inconsistency.
        let other = match other_keys {
            None => "null".to_owned(),
            Some(keys) => format!("{keys:?}"),
        };
        panic!(
            "IllegalStateException: Unexpected inconsistency: {key:?}, {other}, {inconsistency}"
        );
    }
}
