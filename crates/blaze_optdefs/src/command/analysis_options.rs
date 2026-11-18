//! Transcription of `com/google/devtools/build/lib/analysis/AnalysisOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/analysis/AnalysisOptions.java:31 AnalysisOptions
options! {
    /// Options that affect the <i>mechanism</i> of analysis. These are distinct from {@link
    /// com.google.devtools.build.lib.analysis.config.BuildOptions}, which affect the <i>value</i> of a
    /// BuildConfigurationValue.
    pub struct AnalysisOptions {
        #[option(
            name = "discard_analysis_cache",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Discard the analysis cache immediately after the analysis phase completes.",
                " Reduces memory usage by ~10%, but makes further incremental builds slower.",
            ),
        )]
        pub discard_analysis_cache: bool,

        #[option(
            name = "allow_analysis_cache_discard",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = "If discarding the analysis cache due to a change in the build system, setting this\noption to false will cause bazel to exit, rather than continuing with the build.\nThis option has no effect when `--discard_analysis_cache` is also set.\n",
        )]
        pub allow_analysis_cache_discards: bool,

        #[option(
            name = "max_config_changes_to_show",
            default_value = "3",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "When discarding the analysis cache due to a change in the build options, ",
                "displays up to the given number of changed option names. ",
                "If the number given is -1, all changed options will be displayed.",
            ),
        )]
        pub max_config_changes_to_show: i32,

        #[option(
            name = "skip_incompatible_explicit_targets",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "Skip incompatible targets that are explicitly listed on the command line.\nBy default, building such targets results in an error but they are\nsilently skipped when this option is enabled. See: [Skipping incompatible targets]\n\n[Skipping incompatible targets]: https://bazel.build/extending/platforms#skipping-incompatible-targets\n",
        )]
        pub skip_incompatible_explicit_targets: bool,

        #[option(
            name = "experimental_extra_action_filter",
            default_value = "",
            converter = RegexFilterConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Deprecated in favor of aspects. Filters set of targets to schedule extra_actions for.",
        )]
        pub extra_action_filter: RegexFilter,

        #[option(
            name = "experimental_extra_action_top_level_only",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Deprecated in favor of aspects. Only schedules extra_actions for top level targets.",
        )]
        pub extra_action_top_level_only: bool,

        #[option(
            name = "version_window_for_dirty_node_gc",
            default_value = "0",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Nodes that have been dirty for more than this many versions will be deleted",
                " from the graph upon the next update. Values must be non-negative long integers,",
                " or -1 indicating the maximum possible window.",
            ),
        )]
        pub version_window_for_dirty_node_gc: i64,

        #[option(
            name = "experimental_skyframe_cpu_heavy_skykeys_thread_pool_size",
            default_value = "HOST_CPUS",
            converter = CpuResourceConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If set to a positive value (e.g. `",
                "HOST_CPUS",
                "*1.5`),",
                " Skyframe will run the loading/analysis phase with 2 separate thread pools:",
                " 1 with `<value>` threads (ideally close to `",
                "HOST_CPUS",
                "`) reserved for CPU-heavy SkyKeys, and 1 \"standard\" thread pool (whose size is",
                " controlled by `--loading_phase_threads`) for the rest.",
            ),
        )]
        pub cpu_heavy_sky_keys_thread_pool_size: i32,

        #[option(
            name = "experimental_oom_sensitive_skyfunctions_semaphore_size",
            default_value = "HOST_CPUS",
            converter = CpuResourceConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Sets the size of the semaphore used to prevent SkyFunctions with large peak memory",
                " requirement from OOM-ing blaze. A value of 0 indicates that no semaphore should",
                " be used. Example value: `",
                "HOST_CPUS",
                "*0.5`.",
            ),
        )]
        pub oom_sensitive_sky_functions_semaphore_size: i32,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const ANALYSIS_OPTIONS_OPTION_NAMES: &[&str] = &[
        "discard_analysis_cache",
        "allow_analysis_cache_discard",
        "max_config_changes_to_show",
        "skip_incompatible_explicit_targets",
        "experimental_extra_action_filter",
        "experimental_extra_action_top_level_only",
        "version_window_for_dirty_node_gc",
        "experimental_skyframe_cpu_heavy_skykeys_thread_pool_size",
        "experimental_oom_sensitive_skyfunctions_semaphore_size",
    ];

    #[test]
    fn analysis_options_option_names() {
        super::super::test_util::check_option_names(
            AnalysisOptions::class(),
            ANALYSIS_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn analysis_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<AnalysisOptions>(SKIP_ROUND_TRIP);
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
    fn parses() {
        let o = parse::<AnalysisOptions>(&[
            "--discard_analysis_cache",
            "--experimental_extra_action_filter=//foo,-//foo/bar",
        ]);
        assert!(o.discard_analysis_cache);
        assert!(o.extra_action_filter.is_included("//foo:x"));
        assert!(!o.extra_action_filter.is_included("//foo/bar:x"));
        assert!(
            AnalysisOptions::default()
                .extra_action_filter
                .is_included("anything")
        );
    }
}
