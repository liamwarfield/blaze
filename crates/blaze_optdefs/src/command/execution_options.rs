//! Transcription of `com/google/devtools/build/lib/exec/ExecutionOptions.java` (the `OptionsBase` subclasses it declares).

// ---- transcribed mechanically from the Java @Option annotations; hand-written code follows the marker below ----
#[allow(unused_imports)]
use super::prelude::*;

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:59 ExecutionOptions
options! {
    /// Options affecting the execution phase of a build.
    ///
    /// <p>These options are interpreted by the BuildTool to choose an Executor to be used for the build.
    ///
    /// <p>Note: from the user's point of view, the characteristic function of this set of options is
    /// indistinguishable from that of the BuildRequestOptions: they are all per-request. The difference
    /// is only apparent in the implementation: these options are used only by the lib.exec machinery,
    /// which affects how C++ and Java compilation occur. (The BuildRequestOptions contain a mixture of
    /// "semantic" options affecting the choice of targets to build, and "non-semantic" options affecting
    /// the lib.actions machinery.) Ideally, the user would be unaware of the difference. For now, the
    /// usage strings are identical modulo "part 1", "part 2".
    pub struct ExecutionOptions {
        #[option(
            name = "spawn_strategy",
            default_value = "",
            converter = CommaSeparatedNonEmptyOptionListConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "Specify how spawn actions are executed by default. Accepts a comma-separated list of",
                " strategies from highest to lowest priority. For each action Bazel picks the",
                " strategy with the highest priority that can execute the action. The default",
                " value is \"remote,worker,sandboxed,local\". See",
                " https://blog.bazel.build/2019/06/19/list-strategy.html for details.",
            ),
        )]
        pub spawn_strategy: Vec<String>,

        #[option(
            name = "genrule_strategy",
            default_value = "",
            converter = CommaSeparatedNonEmptyOptionListConverter,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "Specify how to execute genrules. This flag will be phased out. Instead, use ",
                "--spawn_strategy=<value> to control all actions or --strategy=Genrule=<value> ",
                "to control genrules only.",
            ),
        )]
        pub genrule_strategy: Vec<String>,

        #[option(
            name = "strategy",
            default_value = "null",
            converter = StringToStringListConverter,
            allow_multiple = true,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "Specify how to distribute compilation of other spawn actions. Accepts a comma-separated",
                " list of strategies from highest to lowest priority. For each action Bazel picks",
                " the strategy with the highest priority that can execute the action. The default",
                " value is \"remote,worker,sandboxed,local\". This flag overrides the values set",
                " by --spawn_strategy (and --genrule_strategy if used with mnemonic Genrule). See",
                " https://blog.bazel.build/2019/06/19/list-strategy.html for details.",
            ),
        )]
        pub strategy: Vec<(String, Vec<String>)>,

        #[option(
            name = "strategy_regexp",
            default_value = "null",
            converter = RegexFilterAssignmentConverter,
            allow_multiple = true,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "Override which spawn strategy should be used to execute spawn actions that have ",
                "descriptions matching a certain regex_filter. See --per_file_copt for details on",
                "regex_filter matching. ",
                "The last regex_filter that matches the description is used. ",
                "This option overrides other flags for specifying strategy. ",
                "Example: --strategy_regexp=//foo.*\\.cc,-//foo/bar=local means to run actions ",
                "using local strategy if their descriptions match //foo.*.cc but not //foo/bar. ",
                "Example: --strategy_regexp='Compiling.*/bar=local ",
                " --strategy_regexp=Compiling=sandboxed will run 'Compiling //foo/bar/baz' with ",
                "the 'local' strategy, but reversing the order would run it with 'sandboxed'. ",
            ),
        )]
        pub strategy_by_regexp: Vec<RegexFilterToStringList>,

        #[option(
            name = "allowed_strategies_by_exec_platform",
            default_value = "null",
            converter = LabelToStringListConverter,
            allow_multiple = true,
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = "Filters spawn strategies by the execution platform without affecting order.\nFor example:\n```\ncommon --spawn_strategy=remote,sandboxed,worker,local\ncommon --strategy=Genrule=local\ncommon --allowed_strategies_by_exec_platform=@platforms//host:host=local,sandboxed,worker\ncommon --allowed_strategies_by_exec_platform=//:linux_amd64=remote\n```\nWith the above options;\n- Actions configured for the host platform will be given `remote,sandboxed,worker`.\n- Actions configured for the `//:linux_amd64` platform will be given `remote`.\n- Actions configured for the `//:linux_amd64` platform with mnemonic `Genrule` will be\n  given no strategies and fail to spawn.\n",
        )]
        pub allowed_strategies_by_exec_platform: Vec<LabelToStringList>,

        #[option(
            name = "materialize_param_files",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [EXECUTION],
            help = concat!(
                "Writes intermediate parameter files to output tree even when using remote action ",
                "execution or caching. Useful when debugging actions. This is implied by ",
                "--subcommands and --verbose_failures.",
            ),
        )]
        pub materialize_param_files: bool,

        #[option(
            name = "experimental_materialize_param_files_directly",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [EXECUTION],
            help = "If materializing param files, do so with direct writes to disk.",
        )]
        pub materialize_param_files_directly: bool,

        #[option(
            name = "verbose_failures",
            default_value = "false",
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = "If a command fails, print out the full command line.",
        )]
        pub verbose_failures: bool,

        #[option(
            name = "subcommands",
            abbrev = 's',
            default_value = "false",
            converter = ShowSubcommandsConverter,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = concat!(
                "Display the subcommands executed during a build. Related flags:",
                " --execution_log_json_file, --execution_log_binary_file (for logging subcommands",
                " to a file in a tool-friendly format).",
            ),
        )]
        pub show_subcommands: ShowSubcommands,

        #[option(
            name = "check_up_to_date",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "Don't perform the build, just check if it is up-to-date.  If all targets are ",
                "up-to-date, the build completes successfully.  If any step needs to be executed ",
                "an error is reported and the build fails.",
            ),
        )]
        pub check_up_to_date: bool,

        #[option(
            name = "check_tests_up_to_date",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [EXECUTION],
            implicit_requirements = ["--check_up_to_date"],
            help = concat!(
                "Don't run tests, just check if they are up-to-date.  If all tests results are ",
                "up-to-date, the testing completes successfully.  If any test needs to be built or ",
                "executed, an error is reported and the testing fails.  This option implies ",
                "--check_up_to_date behavior.",
            ),
        )]
        pub test_check_up_to_date: bool,

        #[option(
            name = "test_strategy",
            default_value = "",
            documentation_category = TESTING,
            effect_tags = [EXECUTION],
            help = "Specifies which strategy to use when running tests.",
        )]
        pub test_strategy: String,

        #[option(
            name = "test_keep_going",
            default_value = "true",
            documentation_category = TESTING,
            effect_tags = [EXECUTION],
            help = concat!(
                "When disabled, any non-passing test will cause the entire build to stop. By default ",
                "all tests are run, even if some do not pass.",
            ),
        )]
        pub test_keep_going: bool,

        #[option(
            name = "flaky_test_attempts",
            default_value = "default",
            converter = TestAttemptsConverter,
            allow_multiple = true,
            documentation_category = TESTING,
            effect_tags = [EXECUTION],
            help = concat!(
                "Each test will be retried up to the specified number of times in case of any test",
                " failure. Tests that required more than one attempt to pass are marked as 'FLAKY'",
                " in the test summary. Normally the value specified is just an integer or the",
                " string 'default'. If an integer, then all tests will be run up to N times. If",
                " 'default', then only a single test attempt will be made for regular tests and",
                " three for tests marked explicitly as flaky by their rule (flaky=1 attribute).",
                " Alternate syntax: regex_filter@flaky_test_attempts. Where flaky_test_attempts is",
                " as above and regex_filter stands for a list of include and exclude regular",
                " expression patterns (Also see --runs_per_test). Example:",
                " --flaky_test_attempts=//foo/.*,-//foo/bar/.*@3 deflakes all tests in //foo/",
                " except those under foo/bar three times. This option can be passed multiple",
                " times. The most recently passed argument that matches takes precedence. If",
                " nothing matches, behavior is as if 'default' above.",
            ),
        )]
        pub test_attempts: Vec<PerLabelOptions>,

        #[option(
            name = "test_tmpdir",
            default_value = "null",
            converter = PathFragmentConverter,
            documentation_category = TESTING,
            effect_tags = [UNKNOWN],
            help = "Specifies the base temporary directory for 'bazel test' to use.",
        )]
        pub test_tmp_dir: Option<Foreign<PathFragment>>,

        #[option(
            name = "test_output",
            default_value = "summary",
            converter = TestOutputFormatConverter,
            documentation_category = LOGGING,
            effect_tags = [TEST_RUNNER, TERMINAL_OUTPUT, EXECUTION],
            help = "Specifies desired output mode. Not to be confused with `--test_summary` which controls\nthe test summary printed on command completion.\n\nValid values are;\n- `summary` (default) to print summaries for failed tests,\n- `errors` to also print test logs for failed tests,\n- `all` to print summaries and logs for all tests and\n- `streamed` to output logs for all tests in real time (this will force tests to be\n  executed locally one at a time regardless of `--test_strategy` value).\n",
        )]
        pub test_output: TestOutputFormat,

        #[option(
            name = "max_test_output_bytes",
            default_value = "-1",
            documentation_category = LOGGING,
            effect_tags = [TEST_RUNNER, TERMINAL_OUTPUT, EXECUTION],
            help = concat!(
                "Specifies maximum per-test-log size that can be emitted when --test_output is 'errors' ",
                "or 'all'. Useful for avoiding overwhelming the output with excessively noisy test ",
                "output. The test header is included in the log size. Negative values imply no ",
                "limit. Output is all or nothing.",
            ),
        )]
        pub max_test_output_bytes: i32,

        #[option(
            name = "test_summary",
            default_value = "short",
            converter = TestSummaryFormatConverter,
            documentation_category = LOGGING,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Specifies the desired format of the test summary. Valid values are;\n- `short` to list all tests that ran to completion.\n- `short_uncached` to list tests that ran to completion, omitting cached tests.\n- `terse` to list only failed and flaky tests.\n- `detailed` to list tests that ran to completion and their test cases.\n- `detailed_uncached` to list tests that ran to completion and their test cases,\n  omitting cached tests.\n- `testcase` to print summary in test case resolution without detailed information about\n  failed test cases.\n- `none` to omit the summary.\n",
        )]
        pub test_summary: TestSummaryFormat,

        #[option(
            name = "local_resources",
            default_value = "null",
            converter = ResourceConverterAssignmentConverter,
            allow_multiple = true,
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [HOST_MACHINE_RESOURCE_OPTIMIZATIONS],
            help = concat!(
                "Set the number of resources available to Bazel. ",
                "Takes in an assignment to a float or ",
                "HOST_RAM",
                "/",
                "HOST_CPUS",
                ", optionally ",
                "followed by [-|*]<float> (eg. memory=",
                "HOST_RAM",
                "*.5 to use half the available RAM). ",
                "Can be used multiple times to specify multiple ",
                "types of resources. Bazel will limit concurrently running actions ",
                "based on the available resources and the resources required. ",
                "Tests can declare the amount of resources they need ",
                "by using a tag of the \"resources:<resource name>:<amount>\" format. ",
            ),
        )]
        pub local_resources: Vec<(String, f64)>,

        #[option(
            name = "experimental_cpu_load_scheduling",
            default_value = "false",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [EXECUTION],
            help = concat!(
                "Enables the experimental local execution scheduling based on CPU load, not estimation of",
                " actions one by one.  Experimental scheduling have showed the large benefit on a",
                " large local builds on a powerful machines with the large number of cores.",
                " Reccommended to use with --local_resources=cpu=HOST_CPUS",
            ),
        )]
        pub experimental_cpu_load_scheduling: bool,

        #[option(
            name = "experimental_cpu_load_scheduling_window_size",
            default_value = "5000ms",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = concat!(
                "The size of window during experimental scheduling of action based on CPU load. Make",
                " sense to define only when flag --experimental_cpu_load_scheduling is enabled.",
            ),
        )]
        pub experimental_cpu_load_scheduling_window_size: Duration,

        #[option(
            name = "local_test_jobs",
            default_value = "auto",
            converter = LocalTestJobsConverter,
            documentation_category = TESTING,
            effect_tags = [EXECUTION],
            help = concat!(
                "The max number of local test jobs to run concurrently. ",
                "Takes ",
                "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"",
                ". 0 means local resources will limit the number of local test jobs to run ",
                "concurrently instead. Setting this greater than the value for --jobs ",
                "is ineffectual.",
            ),
        )]
        pub local_test_jobs: i32,

        #[option(
            name = "cache_computed_file_digests",
            default_value = "50000",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If greater than 0, configures Bazel to cache file digests in memory based on their ",
                "metadata instead of recomputing the digests from disk every time they are needed. ",
                "Setting this to 0 ensures correctness because not all file changes can be noted ",
                "from file metadata. When not 0, the number indicates the size of the cache as the ",
                "number of file digests to be cached.",
            ),
        )]
        pub cache_size_for_computed_file_digests: i64,

        #[option(
            name = "experimental_enable_critical_path_profiling",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set (the default), critical path profiling is enabled for the execution phase. This",
                " has a slight overhead in RAM and CPU, and may prevent Bazel from making certain",
                " aggressive RAM optimizations in some cases.",
            ),
        )]
        pub enable_critical_path_profiling: bool,

        #[option(
            name = "experimental_stats_summary",
            default_value = "false",
            documentation_category = UNDOCUMENTED,
            effect_tags = [TERMINAL_OUTPUT],
            help = "Enable a modernized summary of the build stats.",
        )]
        pub stats_summary: bool,

        #[option(
            name = "execution_log_binary_file",
            default_value = "null",
            converter = EmptyToNullPathFragmentConverter,
            category = "verbosity",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Log the executed spawns into this file as length-delimited SpawnExec protos, according",
                " to src/main/protobuf/spawn.proto. Prefer --execution_log_compact_file, which is",
                " significantly smaller and cheaper to produce. Related flags:",
                " --execution_log_compact_file (compact format; mutually exclusive),",
                " --execution_log_json_file (text JSON format; mutually exclusive),",
                " --execution_log_sort (whether to sort the execution log),",
                " --subcommands (for displaying subcommands in terminal output).",
            ),
        )]
        pub execution_log_binary_file: Option<Foreign<PathFragment>>,

        #[option(
            name = "execution_log_json_file",
            default_value = "null",
            converter = EmptyToNullPathFragmentConverter,
            category = "verbosity",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Log the executed spawns into this file as newline-delimited JSON representations of",
                " SpawnExec protos, according to src/main/protobuf/spawn.proto. Prefer",
                " --execution_log_compact_file, which is significantly smaller and cheaper to",
                " produce. Related flags:",
                " --execution_log_compact_file (compact format; mutually exclusive),",
                " --execution_log_binary_file (binary protobuf format; mutually exclusive),",
                " --execution_log_sort (whether to sort the execution log),",
                " --subcommands (for displaying subcommands in terminal output).",
            ),
        )]
        pub execution_log_json_file: Option<Foreign<PathFragment>>,

        #[option(
            name = "execution_log_compact_file",
            old_name = "experimental_execution_log_compact_file",
            default_value = "null",
            converter = EmptyToNullPathFragmentConverter,
            category = "verbosity",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Log the executed spawns into this file as length-delimited ExecLogEntry protos,",
                " according to src/main/protobuf/spawn.proto. The entire file is zstd compressed.",
                " Related flags:",
                " --execution_log_binary_file (binary protobuf format; mutually exclusive),",
                " --execution_log_json_file (text JSON format; mutually exclusive),",
                " --subcommands (for displaying subcommands in terminal output).",
            ),
        )]
        pub execution_log_compact_file: Option<Foreign<PathFragment>>,

        #[option(
            name = "execution_log_sort",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Whether to sort the execution log, making it easier to compare logs across invocations.",
                " Set to false to avoid potentially significant CPU and memory usage at the end of",
                " the invocation, at the cost of producing the log in nondeterministic execution",
                " order. Only applies to the binary and JSON formats; the compact format is never",
                " sorted.",
            ),
        )]
        pub execution_log_sort: bool,

        // TODO: when this flag is moved to non-experimental, rename it to a more general name
        // to reflect the new logic - it's not only about cache evictions.
        #[option(
            name = "experimental_remote_cache_eviction_retries",
            default_value = "5",
            documentation_category = REMOTE,
            effect_tags = [EXECUTION],
            help = concat!(
                "The maximum number of attempts to retry if the build encountered a transient remote",
                " cache error that would otherwise fail the build. Applies for example when",
                " artifacts are evicted from the remote cache, or in certain cache failure",
                " conditions. A new invocation id will be generated for each attempt.",
            ),
        )]
        pub remote_retry_on_transient_cache_error: i32,

        // TODO: b/405364605 - Flip internally and change the default to false.
        #[option(
            name = "allow_one_action_on_resource_unavailable",
            default_value = "true",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            help = concat!(
                "If set, allow at least one action to run even if the resource is not enough or",
                " unavailable.",
            ),
        )]
        pub allow_one_action_on_resource_unavailable: bool,
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;

    /// Option names in Java declaration order (extracted from the Java source).
    const EXECUTION_OPTIONS_OPTION_NAMES: &[&str] = &[
        "spawn_strategy",
        "genrule_strategy",
        "strategy",
        "strategy_regexp",
        "allowed_strategies_by_exec_platform",
        "materialize_param_files",
        "experimental_materialize_param_files_directly",
        "verbose_failures",
        "subcommands",
        "check_up_to_date",
        "check_tests_up_to_date",
        "test_strategy",
        "test_keep_going",
        "flaky_test_attempts",
        "test_tmpdir",
        "test_output",
        "max_test_output_bytes",
        "test_summary",
        "local_resources",
        "experimental_cpu_load_scheduling",
        "experimental_cpu_load_scheduling_window_size",
        "local_test_jobs",
        "cache_computed_file_digests",
        "experimental_enable_critical_path_profiling",
        "experimental_stats_summary",
        "execution_log_binary_file",
        "execution_log_json_file",
        "execution_log_compact_file",
        "execution_log_sort",
        "experimental_remote_cache_eviction_retries",
        "allow_one_action_on_resource_unavailable",
    ];

    #[test]
    fn execution_options_option_names() {
        super::super::test_util::check_option_names(
            ExecutionOptions::class(),
            EXECUTION_OPTIONS_OPTION_NAMES,
        );
    }

    #[test]
    fn execution_options_defaults_round_trip() {
        super::super::test_util::check_defaults_round_trip::<ExecutionOptions>(SKIP_ROUND_TRIP);
    }
}

// ---- hand-written below this line (preserved by the generator) ----

use super::resource_converter::resource_integer_converter;

impl ExecutionOptions {
    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:169 ExecutionOptions.shouldMaterializeParamFiles
    pub fn should_materialize_param_files(&self) -> bool {
        // Implied by --subcommands and --verbose_failures
        self.materialize_param_files
            || self.show_subcommands != ShowSubcommands::FALSE
            || self.verbose_failures
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:402 ExecutionOptions.usingLocalTestJobs
    pub fn using_local_test_jobs(&self) -> bool {
        self.local_test_jobs != 0
    }
}

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:528 ExecutionOptions.TestOutputFormat
option_enum! {
    /// An enum for specifying different formats of test output.
    // SUMMARY: summary output only; ERRORS: print output from failed tests to the stderr after
    // the test failure; ALL: print output from all tests; STREAMED: stream output from tests as
    // they run (forces tests to be executed sequentially and locally).
    pub enum TestOutputFormat { SUMMARY, ERRORS, ALL, STREAMED }
}

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:544 ExecutionOptions.TestOutputFormat.Converter
enum_converter!(pub TestOutputFormatConverter, TestOutputFormat, "test output");

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:552 ExecutionOptions.TestSummaryFormat
option_enum! {
    /// An enum for specifying different formatting styles of test summaries.
    pub enum TestSummaryFormat {
        SHORT, SHORT_UNCACHED, TERSE, DETAILED, DETAILED_UNCACHED, NONE, TESTCASE,
    }
}

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:572 ExecutionOptions.TestSummaryFormat.Converter
enum_converter!(pub TestSummaryFormatConverter, TestSummaryFormat, "test summary");

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:580 ExecutionOptions.TestAttemptsConverter
/// Converter for the --flaky_test_attempts option.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestAttemptsConverter;

/// Java's `NumberFormatException` or `OptionsParsingException` out of `validateInput`.
enum ValidateError {
    NumberFormat,
    Options(OptionsParsingException),
}

impl TestAttemptsConverter {
    const MIN_VALUE: i32 = 1;
    const MAX_VALUE: i32 = 10;

    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:584 ExecutionOptions.TestAttemptsConverter.validateInput
    fn validate_input(input: &str) -> Result<(), ValidateError> {
        if input != "default" {
            let value = java_parse_int(input).ok_or(ValidateError::NumberFormat)?;
            if value < Self::MIN_VALUE {
                return Err(ValidateError::Options(OptionsParsingException::new(
                    format!("'{}' should be >= {}", input, Self::MIN_VALUE),
                )));
            } else if value > Self::MAX_VALUE {
                return Err(ValidateError::Options(OptionsParsingException::new(
                    format!("'{}' should be <= {}", input, Self::MAX_VALUE),
                )));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:596 ExecutionOptions.TestAttemptsConverter.convert
    pub fn convert_attempts(
        &self,
        input: &str,
    ) -> Result<PerLabelOptions, OptionsParsingException> {
        match Self::parse_as_integer(input) {
            Ok(p) => Ok(p),
            Err(ValidateError::Options(e)) => Err(e),
            Err(ValidateError::NumberFormat) => Self::parse_as_regex(input),
        }
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:604 ExecutionOptions.TestAttemptsConverter.parseAsInteger
    fn parse_as_integer(input: &str) -> Result<PerLabelOptions, ValidateError> {
        Self::validate_input(input)?;
        let catch_all =
            RegexFilter::new(&[".*".to_string()], &[]).expect("'.*' is a valid regular expression");
        Ok(PerLabelOptions::new(catch_all, vec![input.to_string()]))
    }

    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:612 ExecutionOptions.TestAttemptsConverter.parseAsRegex
    fn parse_as_regex(input: &str) -> Result<PerLabelOptions, OptionsParsingException> {
        let test_regexps = PerLabelOptionsConverter.convert_options(input)?;
        if test_regexps.get_options().len() != 1 {
            return Err(OptionsParsingException::new(format!(
                "'{}' has multiple runs for a single pattern",
                input
            )));
        }
        let runs_per_test = &test_regexps.get_options()[0];
        // Run this in order to catch errors.
        match Self::validate_input(runs_per_test) {
            Ok(()) => Ok(test_regexps),
            Err(ValidateError::Options(e)) => Err(e),
            Err(ValidateError::NumberFormat) => Err(OptionsParsingException::new(format!(
                "'{}' has a non-numeric value",
                input
            ))),
        }
    }
}

impl Converter for TestAttemptsConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_attempts(non_null(input))?)))
    }
    fn starlark_convertible(&self) -> bool {
        PerLabelOptionsConverter.starlark_convertible()
    }
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        PerLabelOptionsConverter.reverse_for_starlark(converted)
    }
    fn get_type_description(&self) -> String {
        "a positive integer, the string \"default\", or test_regex@attempts. This flag may be passed more than once"
            .into()
    }
}

resource_integer_converter!(
    // Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:635 ExecutionOptions.LocalTestJobsConverter
    /// Converter for --local_test_jobs, which takes FLAG_SYNTAX
    pub LocalTestJobsConverter => ResourceIntegerConverter::new(|| 0, 0, i32::MAX)
);

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:642 ExecutionOptions.ShowSubcommandsConverter
bool_or_enum_converter!(
    pub ShowSubcommandsConverter,
    ShowSubcommands,
    "subcommand option",
    ShowSubcommands::TRUE,
    ShowSubcommands::FALSE
);

/// `Map.Entry<Label, List<String>>`, the result of `LabelToStringListConverter`.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelToStringList {
    pub key: Label,
    pub value: Vec<String>,
}

// Java Map.Entry.toString: key=value.
impl OptionScalar for LabelToStringList {
    fn java_to_string(&self) -> String {
        format!(
            "{}={}",
            self.key.get_canonical_form(),
            blaze_options::value::java_list_to_string(self.value.iter().cloned())
        )
    }
}
option_field_type!(
    LabelToStringList,
    "java.util.Map$Entry",
    raw = LabelToStringList {
        key: Label::parse_canonical_unchecked("//:raw"),
        value: vec![],
    }
);

/// `CoreOptionConverters.LabelConverter` as the key converter of an assignment.
struct LabelKeyConverter;

impl Converter for LabelKeyConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(Foreign(
            crate::fragment::converters::convert_options_label(non_null(input), c)?,
        ))))
    }
    fn get_type_description(&self) -> String {
        "a build target label".into()
    }
}

// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:649 ExecutionOptions.LabelToStringListConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct LabelToStringListConverter;

impl Converter for LabelToStringListConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        let inner = AssignmentToListOfValuesConverter {
            key_converter: LabelKeyConverter,
            value_converter: StringConverter,
            allow_empty_keys: AllowEmptyKeys::NO,
        };
        let (key, values) = inner.convert_parts(non_null(input), c)?;
        Ok(Some(Value::new(LabelToStringList {
            key: key
                .expect("LabelConverter never returns null")
                .get::<Foreign<Label>>()
                .0,
            value: values
                .into_iter()
                .map(|v| v.expect("StringConverter never returns null").get())
                .collect(),
        })))
    }
    fn get_type_description(&self) -> String {
        "a '<Label>=value[,value]' assignment".into()
    }
}

/// Options whose default cannot be re-parsed in a test (none).
#[cfg(test)]
const SKIP_ROUND_TRIP: &[&str] = &[];

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, parse_err};
    use super::*;

    #[test]
    fn flaky_test_attempts() {
        let o = parse::<ExecutionOptions>(&[
            "--flaky_test_attempts=3",
            "--flaky_test_attempts=//foo@2",
            "--flaky_test_attempts=default",
        ]);
        assert_eq!(o.test_attempts.len(), 3);
        assert_eq!(o.test_attempts[0].get_options(), &["3"]);
        assert!(
            o.test_attempts[0]
                .get_regex_filter()
                .is_included("//anything")
        );
        assert_eq!(o.test_attempts[1].get_options(), &["2"]);
        assert_eq!(
            parse_err::<ExecutionOptions>(&["--flaky_test_attempts=11"]),
            "While parsing option --flaky_test_attempts=11: '11' should be <= 10"
        );
        assert_eq!(
            parse_err::<ExecutionOptions>(&["--flaky_test_attempts=//foo@x"]),
            "While parsing option --flaky_test_attempts=//foo@x: '//foo@x' has a non-numeric value"
        );
        assert_eq!(
            parse_err::<ExecutionOptions>(&["--flaky_test_attempts=//foo@1,2"]),
            "While parsing option --flaky_test_attempts=//foo@1,2: '//foo@1,2' has multiple runs for a single pattern"
        );
    }

    #[test]
    fn subcommands_and_strategies() {
        let o = parse::<ExecutionOptions>(&["-s"]);
        assert_eq!(o.show_subcommands, ShowSubcommands::TRUE);
        assert!(o.should_materialize_param_files());
        let o = parse::<ExecutionOptions>(&["--subcommands=pretty_print"]);
        assert_eq!(o.show_subcommands, ShowSubcommands::PRETTY_PRINT);
        let o = parse::<ExecutionOptions>(&[
            "--strategy=Genrule=local,remote",
            "--strategy_regexp=//foo.*\\.cc,-//foo/bar=local",
            "--allowed_strategies_by_exec_platform=//:linux=remote",
            "--local_resources=cpu=2",
            "--test_output=errors",
            "--test_summary=terse",
        ]);
        assert_eq!(
            o.strategy,
            vec![(
                "Genrule".to_string(),
                vec!["local".to_string(), "remote".to_string()]
            )]
        );
        assert_eq!(o.strategy_by_regexp[0].value, vec!["local"]);
        assert!(o.strategy_by_regexp[0].key.is_included("//foo/x.cc"));
        assert!(!o.strategy_by_regexp[0].key.is_included("//foo/bar.cc"));
        assert_eq!(
            OptionScalar::java_to_string(&o.allowed_strategies_by_exec_platform[0]),
            "//:linux=[remote]"
        );
        assert_eq!(o.local_resources, vec![("cpu".to_string(), 2.0)]);
        assert_eq!(o.test_output, TestOutputFormat::ERRORS);
        assert_eq!(o.test_summary, TestSummaryFormat::TERSE);
        assert_eq!(ExecutionOptions::default().local_test_jobs, 0);
        assert!(!ExecutionOptions::default().using_local_test_jobs());
    }

    // Ported from src/test/java/com/google/devtools/build/lib/exec/LocalTestJobsTest.java.

    #[test]
    fn test_local_test_jobs_must_be_positive() {
        let thrown = LocalTestJobsConverter::inner()
            .convert_int("-1")
            .unwrap_err();
        assert!(thrown.get_message().contains("must be at least 0"));
    }

    #[test]
    fn test_local_test_jobs_auto_is_zero() {
        assert_eq!(
            LocalTestJobsConverter::inner().convert_int("auto").unwrap(),
            0
        );
    }

    #[test]
    fn implicit_requirements() {
        let o = parse::<ExecutionOptions>(&["--check_tests_up_to_date"]);
        assert!(o.test_check_up_to_date);
        assert!(o.check_up_to_date);
    }
}
