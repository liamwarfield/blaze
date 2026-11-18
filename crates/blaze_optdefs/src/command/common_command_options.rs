//! Transcription of `com/google/devtools/build/lib/runtime/CommonCommandOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:44 CommonCommandOptions
options! {
    /// Options common to all commands.
    pub struct CommonCommandOptions {
        #[option(
            name = "enable_platform_specific_config",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If true, Bazel picks up host-OS-specific config lines from bazelrc files. For example, ",
                "if the host OS is Linux and you run bazel build, Bazel picks up lines starting ",
                "with build:linux. Supported OS identifiers are linux, macos, windows, freebsd, ",
                "and openbsd. Enabling this flag is equivalent to using --config=linux on Linux, ",
                "--config=windows on Windows, etc.",
            ),
        )]
        pub enable_platform_specific_config: bool,

        #[option(
            name = "config",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Selects additional config sections from the rc files; for every <command>, it ",
                "also pulls in the options from <command>:<config> if such a section exists; ",
                "if this section doesn't exist in any .rc file, Blaze fails with an error. ",
                "The config sections and flag combinations they are equivalent to are ",
                "located in the tools/*.blazerc config files.",
            ),
        )]
        pub configs: Vec<String>,

        // Level.INFO
        #[option(
            name = "logging",
            default_value = "3",
            converter = LogLevelConverter,
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "The logging level.",
        )]
        pub verbosity: LogLevel,

        #[option(
            name = "client_cwd",
            default_value = "",
            converter = PathFragmentConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [HIDDEN],
            help = "A system-generated parameter which specifies the client's working directory",
        )]
        pub client_cwd: Foreign<PathFragment>,

        #[option(
            name = "announce_rc",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS],
            help = "Whether to announce rc options.",
        )]
        pub announce_rc_options: bool,

        #[option(
            name = "always_profile_slow_operations",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_INTERNAL_CONFIGURATION],
            help = "Whether profiling slow operations is always turned on",
        )]
        pub always_profile_slow_operations: bool,

        #[option(
            name = "experimental_install_base_gc_max_age",
            default_value = "30d",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "How long an install base must go unused before it's eligible for garbage collection.",
                " If nonzero, the server will attempt to garbage collect other install bases when",
                " idle.",
            ),
        )]
        pub install_base_gc_max_age: Duration,

        #[option(
            name = "experimental_action_cache_gc_idle_delay",
            default_value = "5m",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "How long the server must remain idle before a garbage collection of the action cache is",
                " attempted. Ineffectual unless --experimental_action_cache_gc_max_age is nonzero.",
            ),
        )]
        pub action_cache_gc_idle_delay: Duration,

        #[option(
            name = "experimental_action_cache_gc_threshold",
            default_value = "10",
            converter = PercentageConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "The percentage of stale action cache entries required for garbage collection to be",
                " triggered. Ineffectual unless --experimental_action_cache_gc_max_age is nonzero.",
            ),
        )]
        pub action_cache_gc_threshold: i32,

        #[option(
            name = "experimental_action_cache_gc_max_age",
            default_value = "0",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "If set to a nonzero value, the action cache will be periodically garbage collected to",
                " remove entries older than this age. Garbage collection occurs in the background",
                " once the server has become idle, as determined by the",
                " --experimental_action_cache_gc_idle_delay and",
                " --experimental_action_cache_gc_threshold flags.",
            ),
        )]
        pub action_cache_gc_max_age: Duration,

        #[option(
            name = "experimental_enable_thread_dump",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Whether to enable thread dumps. If true, Bazel will dump the state of all threads",
                " (including virtual threads) to a file every --experimental_thread_dump_interval,",
                " or after action execution being inactive for",
                " --experimental_thread_dump_action_execution_inactivity_duration. The dumps will",
                " be written to the <output_base>/server/thread_dumps/ directory.",
            ),
        )]
        pub enable_thread_dump: bool,

        #[option(
            name = "experimental_thread_dump_interval",
            default_value = "0",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "How often to dump the threads periodically. If zero, no thread dumps are written",
                " periodically.",
            ),
        )]
        pub thread_dump_interval: Duration,

        #[option(
            name = "experimental_thread_dump_action_execution_inactivity_duration",
            default_value = "0",
            converter = DurationConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Dump the threads when action execution being inactive for this duration. If zero, no",
                " thread dumps are written for action execution being inactive.",
            ),
        )]
        pub thread_dump_action_execution_inactivity_duration: Duration,

        #[option(
            name = "invocation_id",
            default_value = "",
            converter = UUIDConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [BAZEL_MONITORING, BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "Unique identifier, in UUID format, for the command being run. If explicitly specified",
                " uniqueness must be ensured by the caller. The UUID is printed to stderr, the BEP",
                " and remote execution protocol.",
            ),
        )]
        pub invocation_id: Option<UUID>,

        #[option(
            name = "build_request_id",
            default_value = "",
            converter = PrefixedUUIDConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_MONITORING, BAZEL_INTERNAL_CONFIGURATION],
            metadata_tags = [HIDDEN],
            help = "Unique string identifier for the build being run.",
        )]
        pub build_request_id: Option<String>,

        #[option(
            name = "build_metadata",
            default_value = "null",
            converter = AssignmentConverter,
            allow_multiple = true,
            documentation_category = UNCATEGORIZED,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Custom key-value string pairs to supply in a build event.",
        )]
        pub build_metadata: Vec<(String, String)>,

        #[option(
            name = "oom_message",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [BAZEL_MONITORING, TERMINAL_OUTPUT],
            metadata_tags = [HIDDEN],
            help = "Custom message to be emitted on an out of memory failure.",
        )]
        pub oom_message: String,

        #[option(
            name = "generate_json_trace_profile",
            old_name = "experimental_generate_json_trace_profile",
            default_value = "auto",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If enabled, Bazel profiles the build and writes a JSON-format profile into a file in",
                " the output base. View profile by loading into chrome://tracing. By default Bazel",
                " writes the profile for all build-like commands and query.",
            ),
        )]
        pub enable_tracer: TriState,

        #[option(
            name = "experimental_profile_additional_tasks",
            default_value = "null",
            converter = ProfilerTaskConverter,
            allow_multiple = true,
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "Specifies additional profile tasks to be included in the profile.",
        )]
        pub additional_profile_tasks: Vec<ProfilerTask>,

        #[option(
            name = "slim_profile",
            old_name = "experimental_slim_json_profile",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Slims down the size of the JSON profile by merging events if the profile gets ",
                "too large.",
            ),
        )]
        pub slim_profile: bool,

        #[option(
            name = "experimental_profile_include_primary_output",
            old_name = "experimental_include_primary_output",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Includes the extra \"out\" attribute in action events that contains the exec path ",
                "to the action's primary output.",
            ),
        )]
        pub include_primary_output: bool,

        #[option(
            name = "experimental_profile_include_target_label",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "Includes target label in action events' JSON profile data.",
        )]
        pub profile_include_target_label: bool,

        #[option(
            name = "experimental_profile_include_target_configuration",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "Includes target configuration hash in action events' JSON profile data.",
        )]
        pub profile_include_target_configuration: bool,

        #[option(
            name = "profiles_to_retain",
            default_value = "5",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Number of profiles to retain in the output base. If there are more than this number of",
                " profiles in the output base, the oldest are deleted until the total is under the",
                " limit.",
            ),
        )]
        pub profiles_to_retain: i32,

        #[option(
            name = "profile",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If set, profile Bazel and write data to the specified file. See",
                " https://bazel.build/advanced/performance/json-trace-profile for more",
                " information.",
            ),
        )]
        pub profile_path: Option<Foreign<PathFragment>>,

        #[option(
            name = "starlark_cpu_profile",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "Writes into the specified file a pprof profile of CPU usage by all Starlark threads.",
        )]
        pub starlark_cpu_profile: String,

        #[option(
            name = "record_full_profiler_data",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "By default, Bazel profiler will record only aggregated data for fast but numerous ",
                "events (such as statting the file). If this option is enabled, profiler will ",
                "record each event - resulting in more precise profiling data but LARGE ",
                "performance hit. Option only has effect if --profile used as well.",
            ),
        )]
        pub record_full_profiler_data: bool,

        #[option(
            name = "experimental_collect_worker_data_in_profiler",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "If enabled, the profiler collects worker's aggregated resource data.",
        )]
        pub collect_worker_data_in_profiler: bool,

        #[option(
            name = "experimental_collect_load_average_in_profiler",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "If enabled, the profiler collects the system's overall load average.",
        )]
        pub collect_load_average_in_profiler: bool,

        #[option(
            name = "experimental_collect_system_network_usage",
            default_value = "true",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "If enabled, the profiler collects the system's network usage.",
        )]
        pub collect_system_network_usage: bool,

        #[option(
            name = "experimental_collect_resource_estimation",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "If enabled, the profiler collects CPU and memory usage estimation for local actions.",
        )]
        pub collect_resource_estimation: bool,

        #[option(
            name = "experimental_collect_pressure_stall_indicators",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "If enabled, the profiler collects the Linux PSI data.",
        )]
        pub collect_pressure_stall_indicators: bool,

        #[option(
            name = "experimental_collect_skyframe_counts_in_profiler",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If enabled, the profiler collects SkyFunction counts in the Skyframe graph over time for",
                " key function types, like configured targets and action executions. May have a",
                " performance hit as this visits the ENTIRE Skyframe graph at every profiling time",
                " unit. Do not use this flag with performance-critical measurements.",
            ),
        )]
        pub collect_skyframe_counts: bool,

        #[option(
            name = "memory_profile",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If set, write memory usage data to the specified file at phase ends and stable heap to",
                " master log at end of build.",
            ),
        )]
        pub memory_profile_path: Option<Foreign<PathFragment>>,

        #[option(
            name = "memory_profile_stable_heap_parameters",
            default_value = "1,0",
            converter = MemoryProfileStableHeapParametersConverter,
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Tune memory profile's computation of stable heap at end of build. Should be and even",
                " number of  integers separated by commas. In each pair the first integer is the",
                " number of GCs to perform. The second integer in each pair is the number of",
                " seconds to wait between GCs. Ex: 2,4,4,0 would 2 GCs with a 4sec pause, followed",
                " by 4 GCs with zero second pause",
            ),
        )]
        pub memory_profile_stable_heap_parameters: MemoryProfileStableHeapParameters,

        #[option(
            name = "heap_dump_on_oom",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "Whether to manually output a heap dump if an OOM is thrown (including manual OOMs due to",
                " reaching --gc_thrashing_limits). The dump will be written to",
                " <output_base>/<invocation_id>.heapdump.hprof. This option effectively replaces",
                " -XX:+HeapDumpOnOutOfMemoryError, which has no effect for manual OOMs.",
            ),
        )]
        pub heap_dump_on_oom: bool,

        #[option(
            name = "startup_time",
            default_value = "0",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            metadata_tags = [HIDDEN],
            help = "The time in ms the launcher spends before sending the request to the bazel server.",
        )]
        pub startup_time: i64,

        #[option(
            name = "extract_data_time",
            default_value = "0",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            metadata_tags = [HIDDEN],
            help = "The time in ms spent on extracting the new bazel version.",
        )]
        pub extract_data_time: i64,

        #[option(
            name = "command_wait_time",
            default_value = "0",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            metadata_tags = [HIDDEN],
            help = "The time in ms a command had to wait on a busy Bazel server process.",
        )]
        pub wait_time: i64,

        #[option(
            name = "tool_tag",
            default_value = "",
            documentation_category = LOGGING,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            help = "A tool name to attribute this Bazel invocation to.",
        )]
        pub tool_tag: String,

        #[option(
            name = "restart_reason",
            default_value = "no_restart",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            metadata_tags = [HIDDEN],
            help = "The reason for the server restart.",
        )]
        pub restart_reason: String,

        #[option(
            name = "binary_path",
            default_value = "",
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS, BAZEL_MONITORING],
            metadata_tags = [HIDDEN],
            help = "The absolute path of the bazel binary.",
        )]
        pub binary_path: String,

        #[option(
            name = "experimental_allow_project_files",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [CHANGES_INPUTS],
            metadata_tags = [EXPERIMENTAL, HIDDEN],
            help = "Enable processing of +<file> parameters.",
        )]
        pub allow_project_files: bool,

        // In May 2018, this feature will have been out for 6 months. If the format we accept has not
        // changed in that time, we can remove the "experimental" prefix and tag.
        // Keep this flag HIDDEN so that it is not listed with our reported command lines, it being
        // reported separately.
        #[option(
            name = "experimental_tool_command_line",
            default_value = "",
            converter = ToolCommandLineEventConverter,
            documentation_category = UNDOCUMENTED,
            effect_tags = [AFFECTS_OUTPUTS],
            metadata_tags = [EXPERIMENTAL, HIDDEN],
            help = concat!(
                "An extra command line to report with this invocation's command line. Useful for tools ",
                "that invoke Bazel and want the original information that the tool received to be ",
                "logged with the rest of the Bazel invocation.",
            ),
        )]
        pub tool_command_line: ToolCommandLineEvent,

        #[option(
            name = "unconditional_warning",
            default_value = "null",
            allow_multiple = true,
            documentation_category = UNDOCUMENTED,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "A warning that will unconditionally get printed with build warnings and errors. This is",
                " useful to deprecate bazelrc files or --config definitions. If the intent is to",
                " effectively deprecate some flag or combination of flags, this is NOT sufficient.",
                " The flag or flags should use the deprecationWarning field in the option",
                " definition, or the bad combination should be checked for programmatically.",
            ),
        )]
        pub deprecation_warnings: Vec<String>,

        #[option(
            name = "track_incremental_state",
            old_name = "keep_incrementality_data",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "If false, Blaze will not persist data that allows for invalidation and re-evaluation ",
                "on incremental builds in order to save memory on this build. Subsequent builds ",
                "will not have any incrementality with respect to this one. Usually you will want ",
                "to specify --batch when setting this to false.",
            ),
        )]
        pub track_incremental_state: bool,

        #[option(
            name = "keep_state_after_build",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "If false, Blaze will discard the inmemory state from this build when the build ",
                "finishes. Subsequent builds will not have any incrementality with respect to this ",
                "one.",
            ),
        )]
        pub keep_state_after_build: bool,

        #[option(
            name = "repo_env",
            default_value = "null",
            converter = EnvVarsConverter,
            allow_multiple = true,
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [ACTION_COMMAND_LINES],
            help = "Specifies additional environment variables to be available only for repository rules. Note that repository rules see the full environment anyway, but in this way variables can be set via command-line flags and <code>.bazelrc</code> entries. The special syntax <code>=NAME</code> can be used to explicitly unset a variable. The string <code>%bazel_workspace%</code> in a value will be replaced with the absolute path of the workspace as printed by <code>bazel info workspace</code>.\n",
        )]
        pub repository_environment: Vec<EnvVar>,

        #[option(
            name = "incompatible_repo_env_ignores_action_env",
            default_value = "true",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = "If true, <code>--action_env=NAME=VALUE</code> will no longer affect repository rule and module extension environments.\n",
        )]
        pub repo_env_ignores_action_env: bool,

        #[option(
            name = "experimental_strict_repo_env",
            default_value = "false",
            documentation_category = OUTPUT_PARAMETERS,
            effect_tags = [LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = "If true, repository rules and module extensions will only inherit `PATH`, `PATHEXT`\n(on Windows), and environment variables explicitly specified by `--repo_env`.\n\nNote that unless `--incompatible_repo_env_ignores_action_env` is true,\n`--action_env=NAME=VALUE` will also be included.\n",
        )]
        pub use_strict_repo_env: bool,

        #[option(
            name = "heuristically_drop_nodes",
            old_name = "experimental_heuristically_drop_nodes",
            old_name_warning = false,
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOSES_INCREMENTAL_STATE],
            help = concat!(
                "If true, Blaze will remove FileState and DirectoryListingState nodes after related File",
                " and DirectoryListing node is done to save memory. We expect that it is less",
                " likely that these nodes will be needed again. If so, the program will re-evaluate",
                " them.",
            ),
        )]
        pub heuristically_drop_nodes: bool,

        #[option(
            name = "http_timeout_scaling",
            default_value = "1.0",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "Scale all timeouts related to http downloads by the given factor",
        )]
        pub http_timeout_scaling: f64,

        #[option(
            name = "http_connector_attempts",
            default_value = "8",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "The maximum number of attempts for http downloads.",
        )]
        pub http_connector_attempts: i32,

        #[option(
            name = "http_connector_retry_max_timeout",
            default_value = "0s",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = concat!(
                "The maximum timeout for http download retries. With a value of 0, no timeout maximum is",
                " defined.",
            ),
        )]
        pub http_connector_retry_max_timeout: Duration,

        #[option(
            name = "http_max_parallel_downloads",
            default_value = "8",
            documentation_category = BAZEL_CLIENT_OPTIONS,
            effect_tags = [BAZEL_INTERNAL_CONFIGURATION],
            help = "The maximum number parallel http downloads.",
        )]
        pub http_max_parallel_downloads: i32,

        #[option(
            name = "redirect_local_instrumentation_output_writes",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = concat!(
                "If true and supported, instrumentation output is redirected to be written locally on a",
                " different machine than where bazel is running on.",
            ),
        )]
        pub redirect_local_instrumentation_output_writes: bool,

        #[option(
            name = "write_command_log",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [BAZEL_MONITORING],
            help = "Whether or not to write the command.log file",
        )]
        pub write_command_log: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const COMMON_COMMAND_OPTIONS_OPTION_NAMES: &[&str] = &[
        "enable_platform_specific_config",
        "config",
        "logging",
        "client_cwd",
        "announce_rc",
        "always_profile_slow_operations",
        "experimental_install_base_gc_max_age",
        "experimental_action_cache_gc_idle_delay",
        "experimental_action_cache_gc_threshold",
        "experimental_action_cache_gc_max_age",
        "experimental_enable_thread_dump",
        "experimental_thread_dump_interval",
        "experimental_thread_dump_action_execution_inactivity_duration",
        "invocation_id",
        "build_request_id",
        "build_metadata",
        "oom_message",
        "generate_json_trace_profile",
        "experimental_profile_additional_tasks",
        "slim_profile",
        "experimental_profile_include_primary_output",
        "experimental_profile_include_target_label",
        "experimental_profile_include_target_configuration",
        "profiles_to_retain",
        "profile",
        "starlark_cpu_profile",
        "record_full_profiler_data",
        "experimental_collect_worker_data_in_profiler",
        "experimental_collect_load_average_in_profiler",
        "experimental_collect_system_network_usage",
        "experimental_collect_resource_estimation",
        "experimental_collect_pressure_stall_indicators",
        "experimental_collect_skyframe_counts_in_profiler",
        "memory_profile",
        "memory_profile_stable_heap_parameters",
        "heap_dump_on_oom",
        "startup_time",
        "extract_data_time",
        "command_wait_time",
        "tool_tag",
        "restart_reason",
        "binary_path",
        "experimental_allow_project_files",
        "experimental_tool_command_line",
        "unconditional_warning",
        "track_incremental_state",
        "keep_state_after_build",
        "repo_env",
        "incompatible_repo_env_ignores_action_env",
        "experimental_strict_repo_env",
        "heuristically_drop_nodes",
        "http_timeout_scaling",
        "http_connector_attempts",
        "http_connector_retry_max_timeout",
        "http_max_parallel_downloads",
        "redirect_local_instrumentation_output_writes",
        "write_command_log",
    ];

    #[test]
    fn common_command_options_option_names() {
        super::super::test_util::check_option_names(
            CommonCommandOptions::class(),
            COMMON_COMMAND_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn common_command_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<CommonCommandOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

/// `java.util.UUID`.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct UUID {
    pub most_sig_bits: i64,
    pub least_sig_bits: i64,
}

impl UUID {
    /// `UUID.fromString` (JDK); `None` is Java's `IllegalArgumentException`.
    // PORT: Long.parseLong accepts any Unicode digit (Character.digit); only ASCII hex digits are
    // accepted here.
    pub fn from_string(name: &str) -> Option<UUID> {
        if name.chars().count() > 36 {
            return None;
        }
        let dashes: Vec<usize> = name.match_indices('-').map(|(i, _)| i).collect();
        if dashes.len() != 4 {
            return None;
        }
        let part = |from: usize, to: usize| i64::from_str_radix(&name[from..to], 16).ok();
        let (d1, d2, d3, d4) = (dashes[0], dashes[1], dashes[2], dashes[3]);
        let mut msb = part(0, d1)? & 0xffff_ffff;
        msb <<= 16;
        msb |= part(d1 + 1, d2)? & 0xffff;
        msb <<= 16;
        msb |= part(d2 + 1, d3)? & 0xffff;
        let mut lsb = part(d3 + 1, d4)? & 0xffff;
        lsb <<= 48;
        lsb |= part(d4 + 1, name.len())? & 0xffff_ffff_ffff;
        Some(UUID {
            most_sig_bits: msb,
            least_sig_bits: lsb,
        })
    }
}

// Java UUID.toString: 8-4-4-4-12 lower-case hex digits.
impl OptionScalar for UUID {
    fn java_to_string(&self) -> String {
        let m = self.most_sig_bits as u64;
        let l = self.least_sig_bits as u64;
        format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            m >> 32,
            (m >> 16) & 0xffff,
            m & 0xffff,
            l >> 48,
            l & 0xffff_ffff_ffff
        )
    }
}
option_field_type!(UUID, "java.util.UUID");

// Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:194 CommonCommandOptions.UUIDConverter
/// Converter for UUID. Accepts values as specified by `UUID#fromString(String)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct UUIDConverter;

impl Converter for UUIDConverter {
    // Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:198 CommonCommandOptions.UUIDConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(None);
        }
        match UUID::from_string(input) {
            Some(u) => Ok(Some(Value::new(u))),
            None => Err(OptionsParsingException::new(format!(
                "Value '{}' is not a value UUID.",
                input
            ))),
        }
    }
    fn get_type_description(&self) -> String {
        "a UUID".into()
    }
}

// Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:220 CommonCommandOptions.PrefixedUUIDConverter
/// Converter for options (--build_request_id) that accept prefixed UUIDs. Since we do not care
/// about the structure of this value after validation, we store it as a string.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrefixedUUIDConverter;

impl Converter for PrefixedUUIDConverter {
    // Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:224 CommonCommandOptions.PrefixedUUIDConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(None);
        }
        // UUIDs that are accepted by UUID#fromString have 36 characters. Interpret the last 36
        // characters as an UUID and the rest as a prefix. We do not check anything about the
        // contents of the prefix.
        let chars: Vec<(usize, char)> = input.char_indices().collect();
        let valid = chars.len() >= 36 && {
            let uuid_start_index = chars[chars.len() - 36].0;
            UUID::from_string(&input[uuid_start_index..]).is_some()
        };
        if !valid {
            return Err(OptionsParsingException::new(format!(
                "Value '{}' does not end in a valid UUID.",
                input
            )));
        }
        Ok(Some(Value::new(input.to_string())))
    }
    fn get_type_description(&self) -> String {
        "An optionally prefixed UUID. The last 36 characters will be verified as a UUID.".into()
    }
}

// Java: com/google/devtools/build/lib/runtime/CommonCommandOptions.java:702 CommonCommandOptions.ProfilerTaskConverter
enum_converter!(pub ProfilerTaskConverter, ProfilerTask, "profiler task");

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn uuids() {
        let o = parse::<CommonCommandOptions>(&[
            "--invocation_id=123E4567-e89b-12d3-a456-426614174000",
            "--build_request_id=prefix-123e4567-e89b-12d3-a456-426614174000",
        ]);
        assert_eq!(
            OptionScalar::java_to_string(&o.invocation_id.unwrap()),
            "123e4567-e89b-12d3-a456-426614174000"
        );
        assert_eq!(
            o.build_request_id.as_deref(),
            Some("prefix-123e4567-e89b-12d3-a456-426614174000")
        );
        assert_eq!(
            parse_err::<CommonCommandOptions>(&["--invocation_id=nope"]),
            "While parsing option --invocation_id=nope: Value 'nope' is not a value UUID."
        );
        assert_eq!(
            parse_err::<CommonCommandOptions>(&["--build_request_id=short"]),
            "While parsing option --build_request_id=short: Value 'short' does not end in a valid UUID."
        );
        assert!(
            parse::<CommonCommandOptions>(&["--invocation_id="])
                .invocation_id
                .is_none()
        );
    }

    #[test]
    fn profiler_tasks_and_configs() {
        let o = parse::<CommonCommandOptions>(&[
            "--experimental_profile_additional_tasks=vfs_stat",
            "--config=a",
            "--config=b",
        ]);
        assert_eq!(o.additional_profile_tasks, vec![ProfilerTask::VFS_STAT]);
        assert_eq!(o.configs, vec!["a", "b"]);
        let o = parse::<CommonCommandOptions>(&["--experimental_tool_command_line=my tool"]);
        assert_eq!(
            o.tool_command_line.command_line.sections,
            vec![vec!["my tool".to_string()]]
        );
    }
}
