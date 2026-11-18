//! Transcription of `com/google/devtools/build/lib/skyframe/SkyfocusOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/skyframe/SkyfocusOptions.java:25 SkyfocusOptions
options! {
    /// The set of options for the Skyfocus feature.
    pub struct SkyfocusOptions {
        #[option(
            name = "experimental_enable_skyfocus",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If true, enable the use of --experimental_active_directories to reduce Bazel's memory",
                " footprint for incremental builds. This feature is known as Skyfocus.",
            ),
        )]
        pub skyfocus_enabled: bool,

        #[option(
            name = "experimental_active_directories",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Active directories for Skyfocus and remote analysis caching. Specify as comma-separated",
                " workspace root-relative paths. This is a stateful flag. Defining one persists it",
                " for subsequent invocations, until it is redefined with a new set.",
            ),
        )]
        pub active_directories: Vec<String>,

        #[option(
            name = "experimental_skyfocus_dump_keys",
            default_value = "none",
            converter = SkyfocusDumpEnumConverter,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "For debugging Skyfocus. Dump the focused SkyKeys (roots, leafs, focused deps, focused",
                " rdeps).",
            ),
        )]
        pub dump_keys: SkyfocusDumpOption,

        #[option(
            name = "experimental_skyfocus_dump_post_gc_stats",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "For debugging Skyfocus. If enabled, trigger manual GC before/after focusing to report",
                " heap sizes reductions. This will increase the Skyfocus latency.",
            ),
        )]
        pub dump_post_gc_stats: bool,

        #[option(
            name = "experimental_frontier_violation_check",
            default_value = "strict",
            converter = FrontierViolationCheckConverter,
            documentation_category = LOGGING,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = concat!(
                "Strategies to handle potential incorrectness from changes beyond the frontier (i.e.",
                " outside the active directories)",
            ),
        )]
        pub frontier_violation_check: FrontierViolationCheck,

        #[option(
            name = "experimental_frontier_violation_verbose",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = "If true, Bazel will print instructions for fixing Skycache violations",
        )]
        pub frontier_violation_verbose: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const SKYFOCUS_OPTIONS_OPTION_NAMES: &[&str] = &[
        "experimental_enable_skyfocus",
        "experimental_active_directories",
        "experimental_skyfocus_dump_keys",
        "experimental_skyfocus_dump_post_gc_stats",
        "experimental_frontier_violation_check",
        "experimental_frontier_violation_verbose",
    ];

    #[test]
    fn skyfocus_options_option_names() {
        super::super::test_util::check_option_names(
            SkyfocusOptions::class(),
            SKYFOCUS_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn skyfocus_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<SkyfocusOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

// Java: com/google/devtools/build/lib/skyframe/SkyfocusOptions.java:71 SkyfocusOptions.SkyfocusDumpOption
option_enum! {
    /// Options to dump the state of the Skyframe graph before/after Skyfocus.
    pub enum SkyfocusDumpOption { NONE, COUNT, VERBOSE }
}

// Java: com/google/devtools/build/lib/skyframe/SkyfocusOptions.java:85 SkyfocusOptions.SkyfocusDumpEnumConverter
enum_converter!(pub SkyfocusDumpEnumConverter, SkyfocusDumpOption, "Skyframe Dump option");

// Java: com/google/devtools/build/lib/skyframe/SkyfocusOptions.java:115 SkyfocusOptions.FrontierViolationCheck
option_enum! {
    /// Strategies for handing the "sad path" in Skyfocus and analysis caching, where it needs to
    /// handle changes outside of the active directories. This usually requires some reanalysis
    /// to rebuild the dropped Skyframe nodes.
    pub enum FrontierViolationCheck { STRICT, WARN, DISABLED_FOR_TESTING }
}

// Java: com/google/devtools/build/lib/skyframe/SkyfocusOptions.java:140 SkyfocusOptions.FrontierViolationCheckConverter
enum_converter!(
    pub FrontierViolationCheckConverter,
    FrontierViolationCheck,
    "Skyfocus handling strategy option"
);

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::parse;
    use super::*;

    #[test]
    fn skyfocus() {
        let o = parse::<SkyfocusOptions>(&[
            "--experimental_active_directories=a/b,c",
            "--experimental_frontier_violation_check=warn",
        ]);
        assert_eq!(o.active_directories, vec!["a/b", "c"]);
        assert_eq!(o.frontier_violation_check, FrontierViolationCheck::WARN);
        assert_eq!(
            SkyfocusOptions::default().dump_keys,
            SkyfocusDumpOption::NONE
        );
    }
}
