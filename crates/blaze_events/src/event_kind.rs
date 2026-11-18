//! Port of `com.google.devtools.build.lib.events.EventKind`.

use std::fmt;

/// Indicates the kind of an [`crate::Event`]. Port of `EventKind`.
// Java: com/google/devtools/build/lib/events/EventKind.java:21 EventKind
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventKind {
    /// An unrecoverable error resulting in abrupt termination of the server.
    ///
    /// Events of this kind may be crash bugs (i.e. `RuntimeException`) or other errors such
    /// `OutOfMemoryError`. The only time such an event should be emitted is just prior to
    /// crashing. It should be the final event presented to the user before terminating.
    Fatal,
    /// For errors that will prevent a successful, correct build. In general, the build tool will
    /// not attempt to start or continue a build if an error is encountered (though the behavior
    /// specified by `--keep_going` is a counterexample).
    Error,
    /// For warnings of minor problems that do not affect the integrity of a build.
    Warning,
    /// For atemporal information that is true throughout the entire duration of a build (e.g. the
    /// number of targets found).
    Info,
    /// For temporal information that changes during the duration of a build (e.g. what action is
    /// executing now).
    Progress,
    /// For debug messages.
    Debug,
    // For progress messages (temporal information) relating to the start and end of particular
    // tasks (e.g. "Loading package foo", "Compiling bar", etc.).
    /// Beginning of a temporal task.
    Start,
    /// End of a temporal task.
    Finish,
    /// For command lines of subcommands executed by the build tool (like make-dbg "-v").
    Subcommand,
    // Output to stdout/stderr from subprocesses.
    /// Output to stdout from subprocess.
    Stdout,
    /// Output to stderr from subprocess.
    Stderr,
    // Test result messages (similar to the INFO and ERROR, but test-specific)
    /// Test passed.
    Pass,
    /// Test failed.
    Fail,
    /// Test timed out.
    Timeout,
    /// Test cancelled.
    Cancelled,
    /// For the reasoning of the dependency checker (like GNU Make "-d").
    Depchecker,
}

impl EventKind {
    /// All kinds in declaration (ordinal) order, as Java's `values()`.
    pub const VALUES: [EventKind; 16] = [
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Warning,
        EventKind::Info,
        EventKind::Progress,
        EventKind::Debug,
        EventKind::Start,
        EventKind::Finish,
        EventKind::Subcommand,
        EventKind::Stdout,
        EventKind::Stderr,
        EventKind::Pass,
        EventKind::Fail,
        EventKind::Timeout,
        EventKind::Cancelled,
        EventKind::Depchecker,
    ];

    // Java: com/google/devtools/build/lib/events/EventKind.java:89 EventKind.ALL_EVENTS
    pub const ALL_EVENTS: EventKindSet = EventKindSet::of(&EventKind::VALUES);

    // Java: com/google/devtools/build/lib/events/EventKind.java:92 EventKind.OUTPUT
    pub const OUTPUT: EventKindSet = EventKindSet::of(&[EventKind::Stdout, EventKind::Stderr]);

    // Java: com/google/devtools/build/lib/events/EventKind.java:95 EventKind.ERRORS
    pub const ERRORS: EventKindSet = EventKindSet::of(&[
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Fail,
        EventKind::Timeout,
    ]);

    // Java: com/google/devtools/build/lib/events/EventKind.java:98 EventKind.ERRORS_AND_WARNINGS
    pub const ERRORS_AND_WARNINGS: EventKindSet = EventKindSet::of(&[
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Warning,
        EventKind::Debug,
        EventKind::Fail,
        EventKind::Timeout,
    ]);

    // Java: com/google/devtools/build/lib/events/EventKind.java:107 EventKind.ERRORS_WARNINGS_AND_INFO
    pub const ERRORS_WARNINGS_AND_INFO: EventKindSet = EventKindSet::of(&[
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Warning,
        EventKind::Debug,
        EventKind::Pass,
        EventKind::Fail,
        EventKind::Timeout,
        EventKind::Info,
    ]);

    // Java: com/google/devtools/build/lib/events/EventKind.java:118 EventKind.ERRORS_AND_OUTPUT
    pub const ERRORS_AND_OUTPUT: EventKindSet = EventKindSet::of(&[
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Fail,
        EventKind::Timeout,
        EventKind::Stdout,
        EventKind::Stderr,
    ]);

    // Java: com/google/devtools/build/lib/events/EventKind.java:127 EventKind.ERRORS_AND_WARNINGS_AND_OUTPUT
    pub const ERRORS_AND_WARNINGS_AND_OUTPUT: EventKindSet = EventKindSet::of(&[
        EventKind::Fatal,
        EventKind::Error,
        EventKind::Warning,
        EventKind::Debug,
        EventKind::Fail,
        EventKind::Timeout,
        EventKind::Stdout,
        EventKind::Stderr,
    ]);

    /// Java `Enum.name()`: the constant's name, which is also its `toString()`.
    pub const fn name(self) -> &'static str {
        match self {
            EventKind::Fatal => "FATAL",
            EventKind::Error => "ERROR",
            EventKind::Warning => "WARNING",
            EventKind::Info => "INFO",
            EventKind::Progress => "PROGRESS",
            EventKind::Debug => "DEBUG",
            EventKind::Start => "START",
            EventKind::Finish => "FINISH",
            EventKind::Subcommand => "SUBCOMMAND",
            EventKind::Stdout => "STDOUT",
            EventKind::Stderr => "STDERR",
            EventKind::Pass => "PASS",
            EventKind::Fail => "FAIL",
            EventKind::Timeout => "TIMEOUT",
            EventKind::Cancelled => "CANCELLED",
            EventKind::Depchecker => "DEPCHECKER",
        }
    }

    /// Java `Enum.ordinal()`.
    pub const fn ordinal(self) -> i32 {
        self as i32
    }

    /// Java `Enum.valueOf(String)`: exact, case-sensitive constant name.
    pub fn value_of(name: &str) -> Option<EventKind> {
        EventKind::VALUES.into_iter().find(|k| k.name() == name)
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of [`EventKind`]s; the port of the `ImmutableSet<EventKind>` /
/// `Sets.immutableEnumSet` values Bazel passes around as event masks.
///
/// PORT: Java's enum sets iterate in ordinal order; so does this bit set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EventKindSet(u32);

impl EventKindSet {
    /// The empty set.
    pub const EMPTY: EventKindSet = EventKindSet(0);

    /// A set holding exactly `kinds`.
    pub const fn of(kinds: &[EventKind]) -> EventKindSet {
        let mut bits = 0u32;
        let mut i = 0;
        while i < kinds.len() {
            bits |= 1 << (kinds[i] as u32);
            i += 1;
        }
        EventKindSet(bits)
    }

    pub const fn contains(self, kind: EventKind) -> bool {
        self.0 & (1 << (kind as u32)) != 0
    }

    #[must_use]
    pub const fn with(self, kind: EventKind) -> EventKindSet {
        EventKindSet(self.0 | (1 << (kind as u32)))
    }

    #[must_use]
    pub const fn without(self, kind: EventKind) -> EventKindSet {
        EventKindSet(self.0 & !(1 << (kind as u32)))
    }

    #[must_use]
    pub const fn union(self, other: EventKindSet) -> EventKindSet {
        EventKindSet(self.0 | other.0)
    }

    #[must_use]
    pub const fn difference(self, other: EventKindSet) -> EventKindSet {
        EventKindSet(self.0 & !other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Iterates in ordinal order.
    pub fn iter(self) -> impl Iterator<Item = EventKind> {
        EventKind::VALUES
            .into_iter()
            .filter(move |k| self.contains(*k))
    }
}

impl FromIterator<EventKind> for EventKindSet {
    fn from_iter<I: IntoIterator<Item = EventKind>>(iter: I) -> Self {
        iter.into_iter()
            .fold(EventKindSet::EMPTY, |set, kind| set.with(kind))
    }
}

/// Formats like Java's `AbstractCollection.toString`: `[A, B]`.
impl fmt::Debug for EventKindSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for EventKindSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (i, kind) in self.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(kind.name())?;
        }
        f.write_str("]")
    }
}
