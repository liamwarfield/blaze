//! Transcription of `com/google/devtools/build/lib/buildtool/BuildRequestOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:44 BuildRequestOptions
options! {
    /// Options interface for {@link BuildRequest}: can be used to parse command-line arguments.
    ///
    /// <p>See also {@code ExecutionOptions}; from the user's point of view, there's no qualitative
    /// difference between these two sets of options.
    pub struct BuildRequestOptions {
        #[option(
            name = "jobs",
            abbrev = 'j',
            default_value = "auto",
            converter = JobsConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = concat!(
                "The number of concurrent jobs to run. Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". Values must be between 1 and ",
                "5000",
                ". Values above ",
                "2500",
                " may cause memory issues. \"auto\" calculates a reasonable default based on",
                " host resources.",
            ),
        )]
        pub jobs: i32,

        #[option(
            name = "experimental_async_execution",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, Bazel is allowed to run action in a virtual thread. The number of\nactions in flight is still capped with `--jobs`.\n",
        )]
        pub use_async_execution: bool,

        #[option(
            name = "experimental_async_execution_max_concurrent_actions",
            default_value = "5000",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS, EXECUTION],
            help = "The number of maximum concurrent actions to run with async execution. If the value is\nless than `--jobs`, it is clamped to `--jobs`.\n",
        )]
        pub async_execution_max_concurrent_actions: i32,

        #[option(
            name = "progress_report_interval",
            default_value = "0",
            converter = ProgressReportIntervalConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "The number of seconds to wait between reports on still running jobs. The\ndefault value 0 means the first report will be printed after 10\nseconds, then 30 seconds and after that progress is reported once every minute.\nWhen `--curses` is enabled, progress is reported every second.\n",
        )]
        pub progress_report_interval: i32,

        #[option(
            name = "explain",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Causes the build system to explain each executed step of the ",
                "build. The explanation is written to the specified log file.",
            ),
        )]
        pub explanation_path: Option<Foreign<PathFragment>>,

        #[option(
            name = "output_filter",
            default_value = "null",
            converter = RegexPatternConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = concat!(
                "Only shows warnings and action outputs for rules with a name matching the provided ",
                "regular expression.",
            ),
        )]
        pub output_filter: Option<RegexPatternOption>,

        #[option(
            name = "analyze",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            help = "Execute the loading/analysis phase; this is the usual behaviour. Specifying `--noanalyze`\ncauses the build to stop before starting the loading/analysis phase, just doing\ntarget pattern parsing and returning zero if that completed successfully; this\nmode is useful for testing.\n",
        )]
        pub perform_analysis_phase: bool,

        #[option(
            name = "build",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS],
            help = "Execute the build; this is the usual behaviour.\nSpecifying `--nobuild` causes the build to stop before executing the build\nactions, returning zero if the package loading and analysis phases completed\nsuccessfully; this mode is useful for testing those phases.\n",
        )]
        pub perform_execution_phase: bool,

        #[option(
            name = "output_groups",
            default_value = "null",
            converter = CommaSeparatedOptionListConverter,
            allow_multiple = true,
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS],
            help = "A list of comma-separated output group names, each of which optionally prefixed by a `+`\nor a `-`. A group prefixed by `+` is added to the default set of output groups,\nwhile a group prefixed by `-` is removed from the default set. If at least one\ngroup is not prefixed, the default set of output groups is omitted. For example,\n`--output_groups=+foo,+bar` builds the union of the default set, foo, and bar,\nwhile `--output_groups=foo,bar` overrides the default set such that only foo and\nbar are built.\n",
        )]
        pub output_groups: Vec<String>,

        #[option(
            name = "run_validations",
            old_name = "experimental_run_validations",
            default_value = "true",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS],
            help = "Whether to run validation actions as part of the build. See [Validation Actions].\n\n[Validation Actions]: https://bazel.build/extending/rules#validation_actions\n",
        )]
        pub run_validation_actions: bool,

        #[option(
            name = "experimental_use_validation_aspect",
            default_value = "false",
            documentation_category = OUTPUT_SELECTION,
            effect_tags = [EXECUTION, AFFECTS_OUTPUTS],
            help = "Whether to run validation actions using aspect (for parallelism with tests).",
        )]
        pub use_validation_aspect: bool,

        #[option(
            name = "show_result",
            default_value = "1",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Show the results of the build. For each target, state whether or not it was brought\nup-to-date, and if so, a list of output files that were built.  The printed files\nare convenient strings for copy+pasting to the shell, to execute them.\n\nThis option requires an integer argument, which is the threshold number of targets\nabove which result information is not printed. Thus zero causes suppression of\nthe message and `MAX_INT` causes printing of the result to occur always. The\ndefault is one.\n\nIf nothing was built for a target its results may be omitted to keep the output\nunder the threshold.\n",
        )]
        pub max_result_targets: i32,

        #[option(
            name = "hide_aspect_results",
            default_value = "",
            converter = CommaSeparatedOptionListConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Comma-separated list of aspect names to not display in results (see `--show_result`).\nUseful for keeping aspects added by wrappers which are typically not interesting\nto end users out of console output.\n",
        )]
        pub hide_aspect_results: Vec<String>,

        #[option(
            name = "symlink_prefix",
            default_value = "null",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "The prefix that is prepended to any of the convenience symlinks that are created\nafter a build. If omitted, the default value is the name of the build tool\nfollowed by a hyphen. If `/` is passed, then no symlinks are created and no\nwarning is emitted. Warning: the special functionality for `/` will be deprecated\nsoon; use `--experimental_convenience_symlinks=ignore` instead.\n",
        )]
        pub symlink_prefix: Option<String>,

        #[option(
            name = "experimental_convenience_symlinks",
            default_value = "normal",
            converter = ConvenienceSymlinksConverter,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "This flag controls how the convenience symlinks (the symlinks that appear in the\nworkspace after the build) will be managed. Possible values:\n- `normal` (default): Each kind of convenience symlink will be created or deleted,\n  as determined by the build.\n- `clean`: All symlinks will be unconditionally deleted.\n- `ignore`: Symlinks will not be created or cleaned up.\n- `log_only`: Generate log messages as if `normal` were passed, but don't actually\n  perform any filesystem operations (useful for tools).\n\nNote that only symlinks whose names are generated by the current value of\n`--symlink_prefix` can be affected; if the prefix changes, any pre-existing\nsymlinks will be left alone.\n",
        )]
        pub experimental_convenience_symlinks: ConvenienceSymlinksMode,

        #[option(
            name = "experimental_convenience_symlinks_bep_event",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "This flag controls whether or not we will post the build event\n`ConvenienceSymlinksIdentified` to the Build Event Protocol. If the value is true,\nthe BEP will have an entry for `convenienceSymlinksIdentified`,\nlisting all of the convenience symlinks created in your workspace. If false, then\nthe `convenienceSymlinksIdentified` entry in the BEP will be empty.\n",
        )]
        pub experimental_convenience_symlinks_bep_event: bool,

        #[option(
            name = "output_tree_tracking",
            old_name = "experimental_output_tree_tracking",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "If set, tell the output service (if any) to track when files in the output ",
                "tree have been modified externally (not by the build system). ",
                "This should improve incremental build speed when an appropriate output service ",
                "is enabled.",
            ),
        )]
        pub finalize_actions: bool,

        #[option(
            name = "directory_creation_cache",
            default_value = "maximumSize=100000",
            converter = CaffeineSpecConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "Describes the cache used to store known regular directories as they're created. Parent",
                " directories of output files are created on-demand during action execution.",
            ),
        )]
        pub directory_creation_cache_spec: CaffeineSpec,

        #[option(
            name = "aspects",
            default_value = "null",
            converter = CommaSeparatedOptionListConverter,
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [UNKNOWN],
            help = "Comma-separated list of aspects to be applied to top-level targets. In the list, if\naspect `some_aspect` specifies required aspect providers via\n`required_aspect_providers`, `some_aspect` will run after\nevery aspect that was mentioned before it in the aspects list whose advertised\nproviders satisfy `some_aspect` required aspect providers. Moreover,\n`some_aspect` will run after all its required aspects specified by\n`requires` attribute.\n`some_aspect` will then have access to the values of those aspects'\nproviders.\n`{bzl-file-label}%{aspect_name}`, for example `//tools:my_def.bzl%my_aspect`, where\n`my_aspect` is a top-level value from a file `tools/my_def.bzl`.\n",
        )]
        pub aspects: Vec<String>,

        #[option(
            name = "aspects_parameters",
            default_value = "null",
            converter = AssignmentConverter,
            allow_multiple = true,
            documentation_category = GENERIC_INPUTS,
            effect_tags = [LOADING_AND_ANALYSIS],
            help = "Specifies the values of the command-line aspects parameters. Each parameter value is\nspecified via `<param_name>=<param_value>`, for example `my_param=my_val` where\n`my_param` is a parameter of some aspect in `--aspects` list or required by an\naspect in the list. This option can be used multiple times. However, it is not\nallowed to assign values to the same parameter more than once.\n",
        )]
        pub aspects_parameters: Vec<(String, String)>,

        #[option(
            name = "use_action_cache",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION, HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = "Whether to use the action cache",
        )]
        pub use_action_cache: bool,

        #[option(
            name = "rewind_lost_inputs",
            default_value = "true",
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            help = "Whether to use action rewinding to recover from lost inputs.",
        )]
        pub rewind_lost_inputs: bool,

        #[option(
            name = "incompatible_skip_genfiles_symlink",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If set to true, the genfiles symlink will not be created. For more information, see\n[GH-8651].\n\n[GH-8651]: https://github.com/bazelbuild/bazel/issues/8651\n",
        )]
        pub incompatible_skip_genfiles_symlink: bool,

        #[option(
            name = "target_pattern_file",
            default_value = "",
            documentation_category = GENERIC_INPUTS,
            effect_tags = [CHANGES_INPUTS],
            help = concat!(
                "If set, build will read patterns from the file named here, rather than on the command ",
                "line. It is an error to specify a file here as well as command-line patterns.",
            ),
        )]
        pub target_pattern_file: String,

        /// Do not use directly. Instead use {@link
        /// com.google.devtools.build.lib.runtime.CommandEnvironment#withMergedAnalysisAndExecutionSourceOfTruth()}.
        #[option(
            name = "experimental_merged_skyframe_analysis_execution",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "If this flag is set, the analysis and execution phases of Skyframe are merged.",
        )]
        pub merged_skyframe_analysis_execution_do_not_use_directly: bool,

        #[option(
            name = "experimental_skymeld_analysis_overlap_percentage",
            default_value = "100",
            converter = PercentageConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "The value represents the % of the analysis phase which will be overlapped with the",
                " execution phase. A value of x means Skyframe will queue up execution tasks and",
                " wait until there's x% of the top level target left to be analyzed before allowing",
                " them to launch. When the value is 0%, we'd wait for all analysis to finish before",
                " executing (no overlap). When it's 100%, the phases are free to overlap as much as",
                " they can.",
            ),
        )]
        pub skymeld_analysis_overlap_percentage: i32,

        #[option(
            name = "experimental_fsvc_threads",
            default_value = "200",
            converter = ThreadConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "The number of threads that are used by the `FileSystemValueChecker`.",
        )]
        pub fsvc_threads: i32,

        #[option(
            name = "experimental_skyframe_memory_dump",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Dump the memory use of individual nodes in the Skyframe graph after the build. This\noption takes a number of flags separated by commas:\n- `json` (no-op, that's the only format).\n- `notransient` (don't traverse transient fields).\n- `noconfig` (ignore objects related to configurations).\n- `noprecomputed` (ignore precomputed values).\n- `noworkspacestatus` (ignore objects related to the workspace status machinery).\n",
        )]
        pub skyframe_memory_dump: Option<String>,

        // TODO: b/341931019 - link to user documentation when available.
        #[option(
            name = "enforce_project_configs",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [LOADING_AND_ANALYSIS, AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, interactive builds may only pass the --scl_config build flag; they may not use",
                " any other build flags. --scl_config must be set to an officially suported project",
                " configuration. Supported configurations are defined in the target's PROJECT.scl,",
                " which can be found by walking up the target's packagge path. See b/324126745.",
            ),
        )]
        pub enforce_project_configs: bool,

        #[option(
            name = "experimental_skyframe_error_handling_refactor",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [NO_OP],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "Used solely for the safe rollout of simplifying Skyframe error handling. This will be",
                "removed once the rollout is complete (expected timeframe: 1 release)",
            ),
        )]
        pub skyframe_error_handling_refactor: bool,

        #[option(
            name = "experimental_aquery_dump_after_build_format",
            default_value = "null",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Writes the state of Skyframe (which includes previous invocations on this blaze instance\nas well) after a build. Output is streamed remotely unless local output is\nrequested with `--experimental_aquery_dump_after_build_output_file`.  Does not\nhonor aquery flags for `--include_*`, but uses the same defaults, except for\n`--include_commandline=false`. Possible output formats:\n`proto|streamed_proto|textproto|jsonproto`. Using this will disable Skymeld.\n",
        )]
        pub aquery_dump_after_build_format: Option<String>,

        // NOTE: This produces a very very large amount of data on large builds,
        // very useful for debugging, but may not be used regularly. When removing
        // unused flags based on usage this may not be a good candidate for
        // removal.
        #[option(
            name = "experimental_aquery_dump_after_build_output_file",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Specify the output file for the aquery dump after a build. Use in conjunction with\n`--experimental_aquery_dump_after_build_format`. The path provided is relative to\nBazel's output base, unless it's an absolute path. Using this will disable Skymeld.\n",
        )]
        pub aquery_dump_after_build_output_file: Option<Foreign<PathFragment>>,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const BUILD_REQUEST_OPTIONS_OPTION_NAMES: &[&str] = &[
        "jobs",
        "experimental_async_execution",
        "experimental_async_execution_max_concurrent_actions",
        "progress_report_interval",
        "explain",
        "output_filter",
        "analyze",
        "build",
        "output_groups",
        "run_validations",
        "experimental_use_validation_aspect",
        "show_result",
        "hide_aspect_results",
        "symlink_prefix",
        "experimental_convenience_symlinks",
        "experimental_convenience_symlinks_bep_event",
        "output_tree_tracking",
        "directory_creation_cache",
        "aspects",
        "aspects_parameters",
        "use_action_cache",
        "rewind_lost_inputs",
        "incompatible_skip_genfiles_symlink",
        "target_pattern_file",
        "experimental_merged_skyframe_analysis_execution",
        "experimental_skymeld_analysis_overlap_percentage",
        "experimental_fsvc_threads",
        "experimental_skyframe_memory_dump",
        "enforce_project_configs",
        "experimental_skyframe_error_handling_refactor",
        "experimental_aquery_dump_after_build_format",
        "experimental_aquery_dump_after_build_output_file",
    ];

    #[test]
    fn build_request_options_option_names() {
        super::super::test_util::check_option_names(
            BuildRequestOptions::class(),
            BUILD_REQUEST_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn build_request_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<BuildRequestOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

use std::sync::Arc;

use super::resource_converter::{
    ResourceConverter, host_cpus_supplier, resource_integer_converter,
};

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:47 BuildRequestOptions.MAX_JOBS
pub const MAX_JOBS: i32 = 5000;

resource_integer_converter!(
    // Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:435 BuildRequestOptions.ThreadConverter
    /// Converter for filesystem value checker threads.
    pub ThreadConverter => ResourceConverterIntegerConverter::new(Arc::new(host_cpus_supplier), 1, i32::MAX)
);

resource_integer_converter!(
    // Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:533 BuildRequestOptions.JobsConverter
    /// Converter for jobs: Takes keyword (FLAG_SYNTAX). Values must be between 1 and MAX_JOBS.
    pub JobsConverter => ResourceConverterIntegerConverter::new(Arc::new(host_cpus_supplier), 1, MAX_JOBS)
        .with_check_and_limit(jobs_check_and_limit)
);

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:539 BuildRequestOptions.JobsConverter.checkAndLimit
fn jobs_check_and_limit(
    c: &ResourceConverter<i32>,
    value: i32,
) -> Result<i32, OptionsParsingException> {
    if (value as f64) < c.min_value as f64 {
        return Err(OptionsParsingException::new(format!(
            "Value '({})' must be at least {}.",
            value, c.min_value
        )));
    }
    if (value as f64) > c.max_value as f64 {
        // PORT: Java logs a warning through GoogleLogger (Flag remoteWorker "jobs" ... was set too
        // high); logging is not ported.
        return Ok(c.max_value);
    }
    Ok(value)
}

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:557 BuildRequestOptions.ProgressReportIntervalConverter
/// Converter for progress_report_interval: [0, 3600].
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgressReportIntervalConverter;

impl Converter for ProgressReportIntervalConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        RangeConverter::new(0, 3600).convert(input, c)
    }
    fn get_type_description(&self) -> String {
        RangeConverter::new(0, 3600).get_type_description()
    }
}

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:579 BuildRequestOptions.ConvenienceSymlinksMode
option_enum! {
    /// Determines how the convenience symlinks are presented to the user
    pub enum ConvenienceSymlinksMode { NORMAL, CLEAN, IGNORE, LOG_ONLY }
}

// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:567 BuildRequestOptions.ConvenienceSymlinksConverter
bool_or_enum_converter!(
    pub ConvenienceSymlinksConverter,
    ConvenienceSymlinksMode,
    "convenience symlinks mode",
    ConvenienceSymlinksMode::NORMAL,
    ConvenienceSymlinksMode::IGNORE
);

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::local_host_capacity::{LocalHostResources, set_local_host_capacity};
    use super::super::test_util::{parse, parse_err};
    use super::*;

    fn capacity(cpu_usage: f64) {
        set_local_host_capacity(Some(LocalHostResources {
            memory_mb: 1.0,
            cpu_usage,
        }));
    }

    // Ported from src/test/java/com/google/devtools/build/lib/buildtool/JobsConverterTest.java.

    #[test]
    fn test_auto_jobs_uses_hardware_settings() {
        capacity(123.0);
        assert_eq!(JobsConverter::inner().convert_int("auto").unwrap(), 123);
        set_local_host_capacity(None);
    }

    #[test]
    fn test_auto_jobs_adjusts_if_hardware_detection_is_bogus() {
        capacity((MAX_JOBS + 1) as f64);
        assert_eq!(
            JobsConverter::inner().convert_int("auto").unwrap(),
            MAX_JOBS
        );
        set_local_host_capacity(None);
    }

    #[test]
    fn test_explicit_jobs_limited() {
        assert_eq!(
            JobsConverter::inner()
                .convert_int(&(MAX_JOBS + 1).to_string())
                .unwrap(),
            MAX_JOBS
        );
    }

    #[test]
    fn test_unbounded_jobs_deprecated() {
        let thrown = JobsConverter::inner().convert_int("-1").unwrap_err();
        assert!(thrown.get_message().contains("must be at least 1"));
    }

    #[test]
    fn jobs() {
        assert_eq!(parse::<BuildRequestOptions>(&["-j", "7"]).jobs, 7);
        assert_eq!(
            parse::<BuildRequestOptions>(&["--jobs=9999"]).jobs,
            MAX_JOBS
        );
        assert_eq!(
            parse_err::<BuildRequestOptions>(&["--jobs=0"]),
            "While parsing option --jobs=0: Value '(0)' must be at least 1."
        );
        assert!(parse::<BuildRequestOptions>(&["--jobs=HOST_CPUS*2"]).jobs >= 2);
    }

    #[test]
    fn convenience_symlinks() {
        let o = parse::<BuildRequestOptions>(&["--noexperimental_convenience_symlinks"]);
        assert_eq!(
            o.experimental_convenience_symlinks,
            ConvenienceSymlinksMode::IGNORE
        );
        let o = parse::<BuildRequestOptions>(&["--experimental_convenience_symlinks=clean"]);
        assert_eq!(
            o.experimental_convenience_symlinks,
            ConvenienceSymlinksMode::CLEAN
        );
    }

    #[test]
    fn directory_creation_cache_default() {
        assert_eq!(
            BuildRequestOptions::default()
                .directory_creation_cache_spec
                .maximum_size,
            Some(100000)
        );
    }
}
