//! Port of `analysis/test/TestConfiguration.java` (the fragment only; its options and
//! `RunsPerTestConverter` are `blaze_optdefs::fragment::test_configuration`).
//!
//! PORT: `SHOULD_INVALIDATE_FOR_OPTION_DIFF` (an `OptionsDiffPredicate` for incremental
//! invalidation) is skipped, incremental.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::test_configuration::{
    TestConfigurationTestOptions, TestConfigurationTestOptionsCancelConcurrentTests,
};
use blaze_optdefs::fragment::test_sharding_strategy::TestShardingStrategy;
use blaze_optdefs::fragment::test_size::TestSize;
use blaze_optdefs::fragment::test_timeout::TestTimeout;
use blaze_options::TriState;
use indexmap::IndexMap;

use crate::build_options::BuildOptions;
use crate::fragment::{Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException};
use crate::fragment_options_class::TEST_OPTIONS;

// Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:56 TestConfiguration
/// Test-related options.
#[derive(Debug)]
pub struct TestConfiguration {
    options: Option<TestConfigurationTestOptions>,
    /// `ImmutableMap<TestTimeout, Duration>` copied from an `EnumMap` (declaration order).
    test_timeout: Option<BTreeMap<TestTimeout, Duration>>,
    should_include: bool,
    test_resources: IndexMap<TestSize, IndexMap<String, f64>>,
}

/// `TestConfiguration.class`.
pub static TEST_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.analysis.test.TestConfiguration",
    starlark_name: None,
    required_options: || vec![&TEST_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(TestConfiguration::new(options));
    Ok(fragment)
}

impl TestConfiguration {
    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:419 TestConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> TestConfiguration {
        let options = build_options.get::<TestConfigurationTestOptions>().cloned();
        match options {
            Some(options) => {
                let test_timeout = options.test_timeout.0.clone();
                let mut test_resources: IndexMap<TestSize, IndexMap<String, f64>> =
                    IndexMap::with_capacity(TestSize::VALUES.len());
                for size in TestSize::VALUES {
                    // ImmutableMap.Builder.buildKeepingLast: the last value of a key wins, at the
                    // position of its first occurrence.
                    let mut resources: IndexMap<String, f64> = IndexMap::new();
                    for resource in &options.test_resources {
                        let value = *resource.second.get(&size).unwrap_or_else(|| {
                            panic!(
                                "NullPointerException: null value in entry: {}=null",
                                resource.first
                            )
                        });
                        resources.insert(resource.first.clone(), value);
                    }
                    test_resources.insert(size, resources);
                }
                TestConfiguration {
                    options: Some(options),
                    should_include: true,
                    test_timeout: Some(test_timeout),
                    test_resources,
                }
            }
            None => TestConfiguration {
                options: None,
                should_include: false,
                test_timeout: None,
                test_resources: IndexMap::new(),
            },
        }
    }

    fn options(&self) -> &TestConfigurationTestOptions {
        self.options
            .as_ref()
            .expect("NullPointerException: TestConfiguration.options")
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:447 TestConfiguration.getTestTimeout
    /// Returns test timeout mapping as set by --test_timeout options.
    pub fn get_test_timeout(&self) -> Option<&BTreeMap<TestTimeout, Duration>> {
        self.test_timeout.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:452 TestConfiguration.getTestResources
    /// Returns test resource mapping as set by --default_test_resources options.
    pub fn get_test_resources(&self, size: TestSize) -> IndexMap<String, f64> {
        self.test_resources.get(&size).cloned().unwrap_or_default()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:456 TestConfiguration.getTestFilter
    pub fn get_test_filter(&self) -> Option<&str> {
        self.options().test_filter.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:460 TestConfiguration.getTestRunnerFailFast
    pub fn get_test_runner_fail_fast(&self) -> bool {
        self.options().test_runner_fail_fast
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:464 TestConfiguration.cacheTestResults
    pub fn cache_test_results(&self) -> TriState {
        self.options().cache_test_results
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:468 TestConfiguration.getTestArguments
    pub fn get_test_arguments(&self) -> &[String] {
        &self.options().test_arguments
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:472 TestConfiguration.testShardingStrategy
    pub fn test_sharding_strategy(&self) -> TestShardingStrategy {
        self.options().test_sharding_strategy
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:476 TestConfiguration.getCoverageSupport
    pub fn get_coverage_support(&self) -> &Label {
        self.options().coverage_support.get()
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:484 TestConfiguration.getRunsPerTestForLabel
    /// Returns number of times the given test should run. If the test doesn't match any of the
    /// filters, runs it once.
    pub fn get_runs_per_test_for_label(&self, label: &Label) -> i32 {
        for per_label_runs in self.options().runs_per_test.iter().rev() {
            if per_label_runs.is_included(label) {
                let options = per_label_runs.get_options();
                if options.len() != 1 {
                    panic!(
                        "IllegalArgumentException: expected one element but was: <{}>",
                        options.join(", ")
                    );
                }
                return options[0].parse::<i32>().unwrap_or_else(|_| {
                    panic!(
                        "NumberFormatException: For input string: \"{}\"",
                        options[0]
                    )
                });
            }
        }
        1
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:493 TestConfiguration.runsPerTestDetectsFlakes
    pub fn runs_per_test_detects_flakes(&self) -> bool {
        self.options().runs_per_test_detects_flakes
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:497 TestConfiguration.cancelConcurrentTests
    pub fn cancel_concurrent_tests(&self) -> TestConfigurationTestOptionsCancelConcurrentTests {
        self.options().cancel_concurrent_tests
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:501 TestConfiguration.fetchAllCoverageOutputs
    pub fn fetch_all_coverage_outputs(&self) -> bool {
        self.options().fetch_all_coverage_outputs
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:505 TestConfiguration.incompatibleExclusiveTestSandboxed
    pub fn incompatible_exclusive_test_sandboxed(&self) -> bool {
        self.options().incompatible_exclusive_test_sandboxed
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:509 TestConfiguration.splitCoveragePostProcessing
    pub fn split_coverage_post_processing(&self) -> bool {
        self.options().split_coverage_post_processing
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:513 TestConfiguration.getZipUndeclaredTestOutputs
    pub fn get_zip_undeclared_test_outputs(&self) -> bool {
        self.options().zip_undeclared_test_outputs
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:517 TestConfiguration.checkShardingSupport
    pub fn check_sharding_support(&self) -> bool {
        self.options().check_sharding_support
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:521 TestConfiguration.allowLocalTests
    pub fn allow_local_tests(&self) -> bool {
        self.options().allow_local_tests
    }
}

impl Fragment for TestConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &TEST_CONFIGURATION
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:442 TestConfiguration.shouldInclude
    fn should_include(&self) -> bool {
        self.should_include
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fragment_options_class::all_known_fragment_options_classes;

    fn create(args: &[&str]) -> TestConfiguration {
        let options = BuildOptions::of_args(&all_known_fragment_options_classes(), args)
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        TestConfiguration::new(&options)
    }

    #[test]
    fn runs_per_test_last_matching_filter_wins() {
        let config = create(&["--runs_per_test=3", "--runs_per_test=//foo@5"]);
        assert_eq!(
            config.get_runs_per_test_for_label(&Label::parse_canonical_unchecked("//foo:bar")),
            5
        );
        assert_eq!(
            config.get_runs_per_test_for_label(&Label::parse_canonical_unchecked("//baz:bar")),
            3
        );
    }

    #[test]
    fn runs_per_test_defaults_to_one() {
        let config = create(&[]);
        assert!(config.should_include());
        assert_eq!(
            config.get_runs_per_test_for_label(&Label::parse_canonical_unchecked("//foo:bar")),
            1
        );
    }

    #[test]
    fn missing_test_options_excludes_fragment() {
        let options = BuildOptions::builder().build();
        let config = TestConfiguration::new(&options);
        assert!(!config.should_include());
        assert!(config.get_test_timeout().is_none());
        assert!(config.get_test_resources(TestSize::SMALL).is_empty());
    }

    // TrimTestConfigurationTest exercises the trimming transition and needs full analysis.
    #[test]
    #[ignore = "needs analysis (TrimTestConfigurationTest)"]
    fn trim_test_configuration_tests() {}
}
