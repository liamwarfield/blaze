//! Port of `lib/util/TimeUtilities.java`.

use crate::java_format::java_format_fixed;

// PORT: skipped, a private constructor of a static-only class: TimeUtilities.TimeUtilities

/// Converts time to the user-friendly string representation; `time_in_ns` is the length of time
/// in nanoseconds.
// Java: com/google/devtools/build/lib/util/TimeUtilities.java:26 TimeUtilities.prettyTime
pub fn pretty_time(time_in_ns: f64) -> String {
    let ms = time_in_ns / 1000000.0;
    if ms < 10.0 {
        return format!("{} ms", java_format_fixed(ms, 2));
    } else if ms < 100.0 {
        return format!("{} ms", java_format_fixed(ms, 1));
    } else if ms < 1000.0 {
        return format!("{} ms", java_format_fixed(ms, 0));
    }
    format!("{} s", java_format_fixed(ms / 1000.0, 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a Java test (Bazel has no `TimeUtilitiesTest`): the expected strings are the oracle
    /// JDK's output of the same Java code (embedded `jdk` of the oracle's install base).
    #[test]
    fn pretty_time_matches_java() {
        for (ns, expected) in [
            (0.0, "0.00 ms"),
            (1234567.0, "1.23 ms"),
            (9995000.0, "10.00 ms"),
            (12345678.0, "12.3 ms"),
            (123456789.0, "123 ms"),
            (1234567890.0, "1.235 s"),
            (999999999.9, "1000 ms"),
            (5000000.0, "5.00 ms"),
        ] {
            assert_eq!(pretty_time(ns), expected, "{ns}");
        }
    }
}
