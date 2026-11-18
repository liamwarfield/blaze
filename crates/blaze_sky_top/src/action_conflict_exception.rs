//! `actions/ActionConflictException.java` and the identity-keyed map of bad actions
//! (`Map<ActionAnalysisMetadata, ActionConflictException>`).
//!
//! PORT: the port is `blaze_analysis::rule::action_conflict_exception` (the configured-target
//! builder throws the exception for a conflict inside one target, `Actions.java:219`); this
//! module re-exports it for the conflict code of this crate.

pub use blaze_analysis::rule::action_conflict_exception::{
    ActionConflictException, ActionConflicts,
};
