//! Transcription of `com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/skyframe/LocalDiffAwareness.java:50 LocalDiffAwareness.Options
options! {
    /// Option to enable / disable local diff awareness.
    pub struct LocalDiffAwarenessOptions {
        #[option(
            name = "watchfs",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "On Linux/macOS: If true, %{product} tries to use the operating system's file watch ",
                "service for local changes instead of scanning every file for a change. On ",
                "Windows: this flag currently is a non-op but can be enabled in conjunction ",
                "with --experimental_windows_watchfs. On any OS: The behavior is undefined ",
                "if your workspace is on a network file system, and files are edited on a ",
                "remote machine.",
            ),
        )]
        pub watch_fs: bool,

        #[option(
            name = "experimental_windows_watchfs",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If true, experimental Windows support for --watchfs is enabled. Otherwise --watchfs",
                "is a non-op on Windows. Make sure to also enable --watchfs.",
            ),
        )]
        pub windows_watch_fs: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const LOCAL_DIFF_AWARENESS_OPTIONS_OPTION_NAMES: &[&str] =
        &["watchfs", "experimental_windows_watchfs"];

    #[test]
    fn local_diff_awareness_options_option_names() {
        super::super::test_util::check_option_names(
            LocalDiffAwarenessOptions::class(),
            LOCAL_DIFF_AWARENESS_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn local_diff_awareness_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<LocalDiffAwarenessOptions>(
            SKIP_ROUND_TRIP,
        );
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn watchfs() {
        assert!(parse::<LocalDiffAwarenessOptions>(&["--watchfs"]).watch_fs);
    }
}
