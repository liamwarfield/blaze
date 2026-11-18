//! Port of `lib/actions/CommandLineLimits.java`.

/// Command line OS limitations, such as the max length.
// Java: com/google/devtools/build/lib/actions/CommandLineLimits.java:17 CommandLineLimits
// PORT: Java's `maxLength` is an `int`; the field is `i64` (API contract), holding `int` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandLineLimits {
    pub max_length: i64,
}

impl CommandLineLimits {
    /// "Unlimited" command line limits.
    ///
    /// Use these limits when you want to prohibit param files, or you don't use param files so you
    /// don't care what the limit is.
    // Java: com/google/devtools/build/lib/actions/CommandLineLimits.java:24 CommandLineLimits.UNLIMITED
    pub const UNLIMITED: CommandLineLimits = CommandLineLimits {
        max_length: i32::MAX as i64,
    };

    // Java: com/google/devtools/build/lib/actions/CommandLineLimits.java:28 CommandLineLimits.<init>
    pub const fn new(max_length: i64) -> CommandLineLimits {
        CommandLineLimits { max_length }
    }
}
