//! Shared utilities for Blaze: porting macros and small ports of `lib/util`.

pub mod build_arena;
pub mod exit_code;
pub mod fingerprint;
pub mod graph;
pub mod java_regex;
pub mod profiler;
pub mod string_encoding;
pub mod uncaught_exception;

pub use exit_code::DetailedExitCode;
pub use exit_code::ExitCode;
pub use fingerprint::Fingerprint;

/// Marks a point the port has not reached yet. Always fails loudly; never returns a guess.
/// Argument: the Java `Class.method` that would run here.
#[macro_export]
macro_rules! unported {
    ($what:expr) => {
        panic!("blaze: not ported yet: {}", $what)
    };
    ($what:expr, $($arg:tt)+) => {
        panic!("blaze: not ported yet: {}: {}", $what, format_args!($($arg)+))
    };
}

/// Port of Guava `Preconditions.checkState`: panics with the same message as Java would.
#[macro_export]
macro_rules! check_state {
    ($cond:expr) => {
        if !$cond {
            panic!("IllegalStateException");
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            panic!("IllegalStateException: {}", format_args!($($arg)+));
        }
    };
}

/// Port of Guava `Preconditions.checkArgument`.
#[macro_export]
macro_rules! check_argument {
    ($cond:expr) => {
        if !$cond {
            panic!("IllegalArgumentException");
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            panic!("IllegalArgumentException: {}", format_args!($($arg)+));
        }
    };
}
