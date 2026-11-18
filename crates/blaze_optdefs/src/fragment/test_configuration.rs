//! The options of `analysis/test/TestConfiguration.java` (`TestConfiguration.TestOptions` and
//! `TestConfiguration.RunsPerTestConverter`). The fragment itself is not ported here.

use blaze_options::{
    ConversionContext, Converter, EnvVar, EnvVarsConverter, OptionsParsingException, TriState,
    Value, converter::non_null, java_parse_int, options,
};

use super::converters::{ConvResult, LabelConverter, OptionLabel};
use super::fragment_options::{FragmentOptions, normalize_env_vars};
use super::per_label_options::{PerLabelOptions, PerLabelOptionsConverter};
use super::regex_filter::RegexFilter;
use super::test_resources_converter::{TestResourcesConverter, TestResourcesPair};
use super::test_sharding_strategy::{ShardingStrategyConverter, TestShardingStrategy};
use super::test_timeout::{TestTimeoutConverter, TestTimeoutMap};

// Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:83 TestConfiguration.TestOptions
options! {
    /// Command-line options.
    pub struct TestConfigurationTestOptions {
        #[option(
            name = "test_env",
            converter = EnvVarsConverter,
            allow_multiple = true,
            default_value = "null",
            documentation_category = TESTING,
            effect_tags = [TEST_RUNNER],
            help = concat!(
                "Specifies additional environment variables to be injected into the test runner\n",
                "environment. Variables can be either specified by `name`, in which\n",
                "case its value will be read from the Bazel client environment, or by the\n",
                "`name=value` pair.\n",
                "Previously set variables can be unset via `=name`.\n",
                "This option can be used multiple times to specify several variables.\n",
                "Used only by the 'bazel test' command.\n",
            ),
        )]
        pub test_environment: Vec<EnvVar>,

        #[option(
            name = "test_timeout",
            default_value = "-1",
            converter = TestTimeoutConverter,
            documentation_category = TESTING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Override the default test timeout values for test timeouts (in secs). If a single\n",
                "positive integer value is specified it will override all categories.  If 4\n",
                "comma-separated integers are specified, they will override the timeouts for\n",
                "`short`, `moderate`, `long` and `eternal` (in that order). In either form, a value of\n",
                "-1 tells blaze to use its default timeouts for that category.\n",
            ),
        )]
        pub test_timeout: TestTimeoutMap,

        #[option(
            name = "default_test_resources",
            default_value = "null",
            converter = TestResourcesConverter,
            allow_multiple = true,
            documentation_category = TESTING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Override the default resources amount for tests. The expected format is\n",
                "`{resource}={value}`. If a single positive number is specified as `{value}`\n",
                "it will override the default resources for all test sizes. If 4\n",
                "comma-separated numbers are specified, they will override the resource\n",
                "amount for respectively the `small`, `medium`, `large`, `enormous` test sizes.\n",
                "Values can also be `HOST_RAM`/`HOST_CPU`, optionally followed\n",
                "by `[-|*]{float}` (eg. `memory=HOST_RAM*.1,HOST_RAM*.2,HOST_RAM*.3,HOST_RAM*.4`).\n",
                "The default test resources specified by this flag are overridden by explicit\n",
                "resources specified in tags.\n",
            ),
        )]
        pub test_resources: Vec<TestResourcesPair>,

        #[option(
            name = "test_filter",
            allow_multiple = false,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies a filter to forward to the test framework.  Used to limit ",
                "the tests run. Note that this does not affect which targets are built.",
            ),
        )]
        pub test_filter: Option<String>,

        #[option(
            name = "test_runner_fail_fast",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Forwards fail fast option to the test runner. The test runner should stop execution",
                " upon first failure.",
            ),
        )]
        pub test_runner_fail_fast: bool,

        #[option(
            name = "cache_test_results",
            default_value = "auto",
            abbrev = 't',
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If set to `auto`, Bazel reruns a test if and only if:\n",
                "1. Bazel detects changes in the test or its dependencies,\n",
                "2. The test is marked as `external`,\n",
                "3. Multiple test runs were requested with `--runs_per_test`, or\n",
                "4. The test previously failed.\n",
                "If set to `yes`, Bazel caches all test results except for tests marked as\n",
                "`external`. If set to `no`, Bazel does not cache any test results.\n",
            ),
        )]
        pub cache_test_results: TriState,

        #[option(
            name = "test_result_expiration",
            default_value = "-1",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "This option is deprecated and has no effect.",
            deprecated = true,
        )]
        pub test_result_expiration: i32,

        #[option(
            name = "trim_test_configuration",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            help = concat!(
                "When enabled, test-related options will be cleared below the top level of the build.",
                " When this flag is active, tests cannot be built as dependencies of non-test",
                " rules, but changes to test-related options will not cause non-test rules to be",
                " re-analyzed.",
            ),
        )]
        pub trim_test_configuration: bool,

        #[option(
            name = "experimental_retain_test_configuration_across_testonly",
            default_value = "true",
            documentation_category = BUILD_TIME_OPTIMIZATION,
            effect_tags = [LOADING_AND_ANALYSIS, LOSES_INCREMENTAL_STATE],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "When enabled, `--trim_test_configuration` will not trim the test configuration for rules\n",
                "marked testonly=1. This is meant to reduce action conflict issues when non-test\n",
                "rules depend on `cc_test` rules. No effect if `--trim_test_configuration` is\n",
                "false.\n",
            ),
        )]
        pub experimental_retain_test_configuration_across_testonly: bool,

        #[option(
            name = "test_arg",
            allow_multiple = true,
            default_value = "null",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies additional options and arguments that should be passed to the test\n",
                "executable. Can be used multiple times to specify several arguments.\n",
                "If multiple tests are executed, each of them will receive identical arguments.\n",
                "Used only by the `bazel test` command.\n",
            ),
        )]
        pub test_arguments: Vec<String>,

        #[option(
            name = "test_sharding_strategy",
            default_value = "explicit",
            converter = ShardingStrategyConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specify strategy for test sharding:\n",
                "- `explicit` to only use sharding if the `shard_count` `BUILD` attribute is\n",
                "  present.\n",
                "- `disabled` to never use test sharding.\n",
                "- `forced=k` to enforce `k` shards for testing regardless of the `shard_count` `BUILD`\n",
                "  attribute.\n",
            ),
        )]
        pub test_sharding_strategy: TestShardingStrategy,

        #[option(
            name = "runs_per_test",
            allow_multiple = true,
            default_value = "1",
            converter = RunsPerTestConverter,
            documentation_category = TESTING,
            effect_tags = [UNKNOWN],
            help = concat!(
                "Specifies number of times to run each test. If any of those attempts fail for any\n",
                "reason, the whole test is considered failed. Normally the value specified is\n",
                "just an integer.\n",
                "\n",
                "Example: `--runs_per_test=3` will run all tests 3 times.\n",
                "\n",
                "Alternate syntax: `regex_filter@runs_per_test`. Where `runs_per_test` stands for\n",
                "an integer value and `regex_filter` stands for a list of include and exclude\n",
                "regular expression patterns (Also see --instrumentation_filter).\n",
                "\n",
                "Example: `--runs_per_test=//foo/.*,-//foo/bar/.*@3` runs all tests in `//foo/` except\n",
                "those under `//foo/bar` three times. This option can be passed multiple times. The most\n",
                "recently passed argument that matches takes precedence. If nothing matches,\n",
                "the test is only run once.\n",
            ),
        )]
        pub runs_per_test: Vec<PerLabelOptions>,

        #[option(
            name = "runs_per_test_detects_flakes",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = concat!(
                "If true, any shard in which at least one run/attempt passes and at least one ",
                "run/attempt fails gets a FLAKY status.",
            ),
        )]
        pub runs_per_test_detects_flakes: bool,

        #[option(
            name = "experimental_cancel_concurrent_tests",
            default_value = "never",
            converter = CancelConcurrentTestsConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If `on_failed` or `on_passed`, then Blaze will cancel concurrently running tests on the first\n",
                "successful run with that result. This is only useful in combination with\n",
                "`--runs_per_test_detects_flakes`.\n",
            ),
        )]
        pub cancel_concurrent_tests: TestConfigurationTestOptionsCancelConcurrentTests,

        #[option(
            name = "coverage_support",
            converter = LabelConverter,
            default_value = "@bazel_tools//tools/test:coverage_support",
            documentation_category = TOOLCHAIN,
            effect_tags = [CHANGES_INPUTS, AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            help = concat!(
                "Location of support files that are required on the inputs of every test action\n",
                "that collects code coverage. Defaults to `//tools/test:coverage_support`.\n",
            ),
        )]
        pub coverage_support: OptionLabel,

        #[option(
            name = "experimental_fetch_all_coverage_outputs",
            default_value = "false",
            documentation_category = UNCATEGORIZED,
            effect_tags = [AFFECTS_OUTPUTS, LOADING_AND_ANALYSIS],
            metadata_tags = [EXPERIMENTAL],
            help = concat!(
                "If true, then Bazel fetches the entire coverage data directory for each test during a ",
                "coverage run.",
            ),
        )]
        pub fetch_all_coverage_outputs: bool,

        #[option(
            name = "incompatible_exclusive_test_sandboxed",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            metadata_tags = [INCOMPATIBLE_CHANGE],
            help = concat!(
                "If true, exclusive tests will run with sandboxed strategy. Add `local` tag to force\n",
                "an exclusive test run locally\n",
            ),
        )]
        pub incompatible_exclusive_test_sandboxed: bool,

        #[option(
            name = "experimental_split_coverage_postprocessing",
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EXECUTION],
            metadata_tags = [EXPERIMENTAL],
            help = "If true, then Bazel will run coverage postprocessing for test in a new spawn.",
        )]
        pub split_coverage_post_processing: bool,

        #[option(
            name = "zip_undeclared_test_outputs",
            default_value = "false",
            documentation_category = TESTING,
            effect_tags = [TEST_RUNNER],
            help = "If true, undeclared test outputs will be archived in a zip file.",
        )]
        pub zip_undeclared_test_outputs: bool,

        #[option(
            name = "incompatible_check_sharding_support",
            default_value = "true",
            documentation_category = UNCATEGORIZED,
            metadata_tags = [INCOMPATIBLE_CHANGE],
            effect_tags = [UNKNOWN],
            help = concat!(
                "If true, Bazel will fail a sharded test if the test runner does not indicate that it\n",
                "supports sharding by touching the file at the path in `TEST_SHARD_STATUS_FILE`.\n",
                "If false, a test runner that does not support sharding will lead to all tests\n",
                "running in each shard.\n",
            ),
        )]
        pub check_sharding_support: bool,

        #[option(
            name = "allow_local_tests",
            default_value = "true",
            documentation_category = UNDOCUMENTED,
            effect_tags = [EXECUTION],
            help = "If true, Bazel will allow local tests to run.",
        )]
        pub allow_local_tests: bool,
    }
}

impl TestConfigurationTestOptions {
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:84 TestConfiguration.TestOptions.ALWAYS_INVALIDATE_WHEN_CHANGED
    /// Names of the options whose change always invalidates (used by `TestConfiguration`'s
    /// `shouldInvalidate` hook).
    pub const ALWAYS_INVALIDATE_WHEN_CHANGED: [&'static str; 2] = [
        "trim_test_configuration",
        "experimental_retain_test_configuration_across_testonly",
    ];
}

impl FragmentOptions for TestConfigurationTestOptions {
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:407 TestConfiguration.TestOptions.getNormalized
    fn get_normalized(&self) -> TestConfigurationTestOptions {
        let mut result = self.clone();
        result.test_environment = normalize_env_vars(&self.test_environment);
        result
    }
}

fragment_enum! {
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:297 TestConfiguration.TestOptions.CancelConcurrentTests
    /// When to cancel concurrently running tests.
    pub enum TestConfigurationTestOptionsCancelConcurrentTests { NEVER, ON_FAILED, ON_PASSED }
}

// Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:303 TestConfiguration.TestOptions.CancelConcurrentTests.Converter
blaze_options::bool_or_enum_converter!(
    pub CancelConcurrentTestsConverter,
    TestConfigurationTestOptionsCancelConcurrentTests,
    "when to cancel concurrent tests",
    TestConfigurationTestOptionsCancelConcurrentTests::ON_PASSED,
    TestConfigurationTestOptionsCancelConcurrentTests::NEVER
);

// Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:534 TestConfiguration.RunsPerTestConverter
/// Converter for `--runs_per_test`.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunsPerTestConverter;

impl RunsPerTestConverter {
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:536 TestConfiguration.RunsPerTestConverter.convert
    pub fn convert_options(&self, input: &str) -> Result<PerLabelOptions, OptionsParsingException> {
        match Self::parse_as_integer(input) {
            Some(result) => result,
            // NumberFormatException
            None => Self::parse_as_regex(input),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:544 TestConfiguration.RunsPerTestConverter.parseAsInteger
    /// `None` is Java's `NumberFormatException`.
    fn parse_as_integer(input: &str) -> Option<Result<PerLabelOptions, OptionsParsingException>> {
        let numeric_value = java_parse_int(input)?;
        if numeric_value <= 0 {
            Some(Err(OptionsParsingException::new(format!(
                "'{}' should be >= 1",
                input
            ))))
        } else {
            let catch_all = RegexFilter::new(&[".*".to_string()], &[])
                .expect("'.*' is a valid regular expression");
            Some(Ok(PerLabelOptions::new(catch_all, vec![input.to_string()])))
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:556 TestConfiguration.RunsPerTestConverter.parseAsRegex
    fn parse_as_regex(input: &str) -> Result<PerLabelOptions, OptionsParsingException> {
        let test_regexps = PerLabelOptionsConverter.convert_options(input)?;
        if test_regexps.get_options().len() != 1 {
            return Err(OptionsParsingException::new(format!(
                "'{}' has multiple runs for a single pattern",
                input
            )));
        }
        let runs_per_test = &test_regexps.get_options()[0];
        match java_parse_int(runs_per_test) {
            Some(numeric_runs_per_test) => {
                if numeric_runs_per_test <= 0 {
                    return Err(OptionsParsingException::new(format!(
                        "'{}' has a value < 1",
                        input
                    )));
                }
            }
            None => {
                return Err(OptionsParsingException::with_cause(
                    format!("'{}' has a non-numeric value", input),
                    format!("For input string: \"{}\"", runs_per_test),
                ));
            }
        }
        Ok(test_regexps)
    }
}

impl Converter for RunsPerTestConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_options(non_null(input))?)))
    }
    fn starlark_convertible(&self) -> bool {
        PerLabelOptionsConverter.starlark_convertible()
    }
    fn reverse_for_starlark(&self, converted: &Value) -> String {
        PerLabelOptionsConverter.reverse_for_starlark(converted)
    }
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:574 TestConfiguration.RunsPerTestConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a positive integer or test_regex@runs. This flag may be passed more than once".into()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{
        assert_defaults_round_trip, assert_option_names, assert_same, create_with_prefix, parse,
    };
    use super::*;

    #[test]
    fn option_names_match_java() {
        assert_option_names::<TestConfigurationTestOptions>(&[
            "test_env",
            "test_timeout",
            "default_test_resources",
            "test_filter",
            "test_runner_fail_fast",
            "cache_test_results",
            "test_result_expiration",
            "trim_test_configuration",
            "experimental_retain_test_configuration_across_testonly",
            "test_arg",
            "test_sharding_strategy",
            "runs_per_test",
            "runs_per_test_detects_flakes",
            "experimental_cancel_concurrent_tests",
            "coverage_support",
            "experimental_fetch_all_coverage_outputs",
            "incompatible_exclusive_test_sandboxed",
            "experimental_split_coverage_postprocessing",
            "zip_undeclared_test_outputs",
            "incompatible_check_sharding_support",
            "allow_local_tests",
        ]);
    }

    #[test]
    fn defaults_round_trip() {
        assert_defaults_round_trip::<TestConfigurationTestOptions>(&[]);
    }

    #[test]
    fn spot_check_parses() {
        let o = parse::<TestConfigurationTestOptions>(&[
            "--test_timeout=1,2,3,4",
            "--runs_per_test=3",
            "--runs_per_test=//foo@2",
            "--test_sharding_strategy=forced=4",
            "--experimental_cancel_concurrent_tests",
            "--cache_test_results=no",
        ])
        .unwrap();
        assert_eq!(
            blaze_options::OptionScalar::java_to_string(&o.test_timeout),
            "{short=PT1S, moderate=PT2S, long=PT3S, eternal=PT4S}"
        );
        assert_eq!(o.runs_per_test.len(), 2);
        assert_eq!(o.runs_per_test[0].get_options(), ["3".to_string()]);
        assert_eq!(o.test_sharding_strategy, TestShardingStrategy::Forced(4));
        assert_eq!(
            o.cancel_concurrent_tests,
            TestConfigurationTestOptionsCancelConcurrentTests::ON_PASSED
        );
        assert_eq!(o.cache_test_results, TriState::NO);
        assert_eq!(
            parse::<TestConfigurationTestOptions>(&["--runs_per_test=0"])
                .unwrap_err()
                .get_message(),
            "While parsing option --runs_per_test=0: '0' should be >= 1"
        );
        assert_eq!(
            parse::<TestConfigurationTestOptions>(&["--runs_per_test=//foo@x"])
                .unwrap_err()
                .get_message(),
            "While parsing option --runs_per_test=//foo@x: '//foo@x' has a non-numeric value"
        );
    }

    #[test]
    fn normalization_keeps_last_test_env() {
        assert_same(
            &create_with_prefix::<TestConfigurationTestOptions>("--test_env=", &["A=1", "A=2"]),
            &create_with_prefix::<TestConfigurationTestOptions>("--test_env=", &["A=2"]),
        );
    }
}
