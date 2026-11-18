//! Ports of `skyframe/PatternExpandingError.java` and `pkgcache/ParsingFailedEvent.java`: the
//! events posted when a target pattern of the command line fails to parse or to expand.
//!
//! PORT: `PatternExpandingError` is a `BuildEvent` in Java; its build event protocol half
//! (`getEventId`, `getChildrenEvents`, `asStreamProto`: an `Aborted` with reason
//! `LOADING_FAILURE` and the message as description) belongs to `blaze_bep`, which wraps this
//! event.

use blaze_events::Postable;

/// Event reporting about failure to expand a target pattern properly.
// Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:27 PatternExpandingError
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PatternExpandingError {
    pattern: Vec<String>,
    message: String,
    skipped: bool,
}

impl PatternExpandingError {
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:39 PatternExpandingError.getPattern
    pub fn get_pattern(&self) -> &[String] {
        &self.pattern
    }

    /// The description of the failure (`asStreamProto`'s `Aborted.description`).
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Whether the pattern was skipped (`getEventId`: `targetPatternSkipped` rather than
    /// `targetPatternExpanded`).
    pub fn is_skipped(&self) -> bool {
        self.skipped
    }

    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:43 PatternExpandingError.failed
    pub fn failed(pattern: Vec<String>, message: impl Into<String>) -> Self {
        PatternExpandingError {
            pattern,
            message: message.into(),
            skipped: false,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:47 PatternExpandingError.failed
    pub fn failed_term(term: impl Into<String>, message: impl Into<String>) -> Self {
        PatternExpandingError {
            pattern: vec![term.into()],
            message: message.into(),
            skipped: false,
        }
    }

    // This is unused right now - when we generate the error, we don't know if we're in keep_going
    // mode or not.
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:53 PatternExpandingError.skipped
    pub fn skipped(term: impl Into<String>, message: impl Into<String>) -> Self {
        PatternExpandingError {
            pattern: vec![term.into()],
            message: message.into(),
            skipped: true,
        }
    }
}

impl Postable for PatternExpandingError {
    // Java: com/google/devtools/build/lib/skyframe/PatternExpandingError.java:77 PatternExpandingError.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}

/// This event is fired when a target or target pattern fails to parse. In some cases (not all)
/// this happens before targets are created, and thus in these cases there are no status lines.
/// Therefore, the parse failure is reported separately.
// Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:25 ParsingFailedEvent
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParsingFailedEvent {
    target_pattern: String,
    message: String,
}

impl ParsingFailedEvent {
    /// Creates a new parsing failed event with the given pattern and message.
    // Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:32 ParsingFailedEvent.<init>
    pub fn new(target_pattern: impl Into<String>, message: impl Into<String>) -> Self {
        ParsingFailedEvent {
            target_pattern: target_pattern.into(),
            message: message.into(),
        }
    }

    // Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:37 ParsingFailedEvent.getPattern
    pub fn get_pattern(&self) -> &str {
        &self.target_pattern
    }

    // Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:41 ParsingFailedEvent.getMessage
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl Postable for ParsingFailedEvent {
    // Java: com/google/devtools/build/lib/pkgcache/ParsingFailedEvent.java:46 ParsingFailedEvent.storeForReplay
    fn store_for_replay(&self) -> bool {
        true
    }
}
