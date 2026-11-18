//! Port of `com/google/devtools/build/lib/events/EventKind.java`, the enum constants and `ALL_EVENTS` (used by
//! `UiOptions.EventFiltersConverter`).
//!
//! PORT: `blaze_events` has its own `EventKind`; this crate does not depend on it, and the
//! options value type must be local (orphan rule). Convert by name at the use site.

use super::prelude::*;

// Java: com/google/devtools/build/lib/events/EventKind.java:21 EventKind
option_enum! {
    /// An EventKind is a distinguished category of message.
    pub enum EventKind {
        FATAL, ERROR, WARNING, INFO, PROGRESS, DEBUG, START, FINISH, SUBCOMMAND, STDOUT, STDERR,
        PASS, FAIL, TIMEOUT, CANCELLED, DEPCHECKER,
    }
}

// Java: com/google/devtools/build/lib/events/EventKind.java:88 EventKind.ALL_EVENTS
/// All event kinds, in declaration order (`EnumSet.allOf`).
pub const ALL_EVENTS: &[EventKind] = &[
    EventKind::FATAL,
    EventKind::ERROR,
    EventKind::WARNING,
    EventKind::INFO,
    EventKind::PROGRESS,
    EventKind::DEBUG,
    EventKind::START,
    EventKind::FINISH,
    EventKind::SUBCOMMAND,
    EventKind::STDOUT,
    EventKind::STDERR,
    EventKind::PASS,
    EventKind::FAIL,
    EventKind::TIMEOUT,
    EventKind::CANCELLED,
    EventKind::DEPCHECKER,
];
