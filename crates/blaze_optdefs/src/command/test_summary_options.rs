//! Transcription of `com/google/devtools/build/lib/runtime/TestSummaryOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/TestSummaryOptions.java:23 TestSummaryOptions
options! {
    /// Flags specific to test summary reporting.
    pub struct TestSummaryOptions {
        #[option(
            name = "verbose_test_summary",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, print additional information (timing, number of failed runs, etc) in the",
                " test summary.",
            ),
        )]
        pub verbose_summary: bool,

        #[option(
            name = "test_verbose_timeout_warnings",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, print additional warnings when the actual test execution time does not ",
                "match the timeout defined by the test (whether implied or explicit).",
            ),
        )]
        pub test_verbose_timeout_warnings: bool,

        #[option(
            name = "print_relative_test_log_paths",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "If true, when printing the path to a test log, use relative path that makes use of ",
                "the 'testlogs' convenience symlink. N.B. - A subsequent 'build'/'test'/etc ",
                "invocation with a different configuration can cause the target of this symlink ",
                "to change, making the path printed previously no longer useful.",
            ),
        )]
        pub print_relative_test_log_paths: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const TEST_SUMMARY_OPTIONS_OPTION_NAMES: &[&str] = &[
        "verbose_test_summary",
        "test_verbose_timeout_warnings",
        "print_relative_test_log_paths",
    ];

    #[test]
    fn test_summary_options_option_names() {
        super::super::test_util::check_option_names(
            TestSummaryOptions::class(),
            TEST_SUMMARY_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn test_summary_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<TestSummaryOptions>(SKIP_ROUND_TRIP);
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
    fn verbose_summary() {
        assert!(!parse::<TestSummaryOptions>(&["--noverbose_test_summary"]).verbose_summary);
    }
}
