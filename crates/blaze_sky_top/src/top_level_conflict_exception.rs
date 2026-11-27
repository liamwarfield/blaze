//! Port of `skyframe/TopLevelConflictException.java`.

use crate::action_conflict_exception::ActionConflicts;

/// Encapsulates a collection of action conflicts of the transitive closure of a top-level
/// ActionLookupKey.
// Java: com/google/devtools/build/lib/skyframe/TopLevelConflictException.java:24 TopLevelConflictException
#[derive(Clone, thiserror::Error)]
#[error("{message}")]
pub struct TopLevelConflictException {
    message: String,
    transitive_action_conflicts: ActionConflicts,
}

impl TopLevelConflictException {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelConflictException.java:29 TopLevelConflictException.<init>
    pub fn new(message: String, action_conflicts: ActionConflicts) -> TopLevelConflictException {
        TopLevelConflictException {
            message,
            transitive_action_conflicts: action_conflicts,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TopLevelConflictException.java:36 TopLevelConflictException.getTransitiveActionConflicts
    pub fn get_transitive_action_conflicts(&self) -> &ActionConflicts {
        &self.transitive_action_conflicts
    }
}

impl std::fmt::Debug for TopLevelConflictException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TopLevelConflictException")
            .field("message", &self.message)
            .field("conflicts", &self.transitive_action_conflicts.len())
            .finish()
    }
}
