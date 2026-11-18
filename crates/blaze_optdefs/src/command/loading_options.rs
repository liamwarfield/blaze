//! Transcription of `com/google/devtools/build/lib/pkgcache/LoadingOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/pkgcache/LoadingOptions.java:29 LoadingOptions
options! {
    /// Options that affect how command-line target patterns are resolved to individual targets.
    pub struct LoadingOptions {
        #[option(
            name = "build_tests_only",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If specified, only *_test and test_suite rules will be built and other targets specified ",
                "on the command line will be ignored. By default everything that was requested ",
                "will be built.",
            ),
        )]
        pub build_tests_only: bool,

        #[option(
            name = "compile_one_dependency",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Compile a single dependency of the argument files. This is useful for syntax checking ",
                "source files in IDEs, for example, by rebuilding a single target that depends on ",
                "the source file to detect errors as early as possible in the edit/build/test cycle. ",
                "This argument affects the way all non-flag arguments are interpreted; instead of ",
                "being targets to build they are source filenames.  For each source filename ",
                "an arbitrary target that depends on it will be built.",
            ),
        )]
        pub compile_one_dependency: bool,

        #[option(
            name = "build_tag_filters",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a comma-separated list of tags. Each tag can be optionally ",
                "preceded with '-' to specify excluded tags. Only those targets will be built that ",
                "contain at least one included tag and do not contain any excluded tags. This option ",
                "does not affect the set of tests executed with the 'test' command; those are be ",
                "governed by the test filtering options, for example '--test_tag_filters'",
            ),
        )]
        pub build_tag_filter_list: Vec<String>,

        #[option(
            name = "test_tag_filters",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a comma-separated list of test tags. Each tag can be optionally ",
                "preceded with '-' to specify excluded tags. Only those test targets will be ",
                "found that contain at least one included tag and do not contain any excluded ",
                "tags. This option affects --build_tests_only behavior and the test command.",
            ),
        )]
        pub test_tag_filter_list: Vec<String>,

        #[option(
            name = "test_size_filters",
            default_value = "",
            converter = TestSizeFilterConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a comma-separated list of test sizes. Each size can be optionally ",
                "preceded with '-' to specify excluded sizes. Only those test targets will be ",
                "found that contain at least one included size and do not contain any excluded ",
                "sizes. This option affects --build_tests_only behavior and the test command.",
            ),
        )]
        pub test_size_filter_set: EnumSet<TestSize>,

        #[option(
            name = "test_timeout_filters",
            default_value = "",
            converter = TestTimeoutFilterConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a comma-separated list of test timeouts. Each timeout can be ",
                "optionally preceded with '-' to specify excluded timeouts. Only those test ",
                "targets will be found that contain at least one included timeout and do not ",
                "contain any excluded timeouts. This option affects --build_tests_only behavior ",
                "and the test command.",
            ),
        )]
        pub test_timeout_filter_set: EnumSet<TestTimeout>,

        #[option(
            name = "test_lang_filters",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a comma-separated list of test languages. Each language can be ",
                "optionally preceded with '-' to specify excluded languages. Only those ",
                "test targets will be found that are written in the specified languages. ",
                "The name used for each language should be the same as the language prefix in the ",
                "*_test rule, e.g. one of 'cc', 'java', 'py', etc. ",
                "This option affects --build_tests_only behavior and the test command.",
            ),
        )]
        pub test_lang_filter_list: Vec<String>,

        #[option(
            name = "build_manual_tests",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Forces test targets tagged 'manual' to be built. 'manual' tests are excluded from ",
                "processing. This option forces them to be built (but not executed).",
            ),
        )]
        pub build_manual_tests: bool,

        #[option(
            name = "experimental_skyframe_target_pattern_evaluator",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            deprecated = true,
            help = concat!(
                "Use the Skyframe-based target pattern evaluator; implies ",
                "--experimental_interleave_loading_and_analysis.",
            ),
        )]
        pub use_skyframe_target_pattern_evaluator: bool,

        #[option(
            name = "expand_test_suites",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = concat!(
                "Expand test_suite targets into their constituent tests before analysis. ",
                "When this flag is turned on (the default), negative target patterns will apply ",
                "to the tests belonging to the test suite, otherwise they will not. ",
                "Turning off this flag is useful when top-level aspects are applied at command line: ",
                "then they can analyze test_suite targets.",
            ),
        )]
        pub expand_test_suites: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const LOADING_OPTIONS_OPTION_NAMES: &[&str] = &[
        "build_tests_only",
        "compile_one_dependency",
        "build_tag_filters",
        "test_tag_filters",
        "test_size_filters",
        "test_timeout_filters",
        "test_lang_filters",
        "build_manual_tests",
        "experimental_skyframe_target_pattern_evaluator",
        "expand_test_suites",
    ];

    #[test]
    fn loading_options_option_names() {
        super::super::test_util::check_option_names(
            LoadingOptions::class(),
            LOADING_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn loading_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<LoadingOptions>(SKIP_ROUND_TRIP);
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
    fn filters() {
        let o = parse::<LoadingOptions>(&[
            "--test_size_filters=small,medium",
            "--test_timeout_filters=-eternal",
            "--build_tag_filters=a,-b",
        ]);
        assert!(o.test_size_filter_set.contains(&TestSize::SMALL));
        assert!(!o.test_size_filter_set.contains(&TestSize::LARGE));
        assert!(o.test_timeout_filter_set.contains(&TestTimeout::LONG));
        assert!(!o.test_timeout_filter_set.contains(&TestTimeout::ETERNAL));
        assert_eq!(o.build_tag_filter_list, vec!["a", "-b"]);
    }
}
