//! Port of `analysis/util/ConfigurationTestCase.java` (src/test/java): a test case that creates
//! configurations through the configuration SkyFunctions of the mock analysis environment.
//!
//! It runs on [`BuildViewTestCase`] (kind [`TestCaseKind::Configuration`]): the same scratch
//! workspace, mock client, event collection and [`SkyframeExecutor`] as the analysis tests.
//!
//! Differences from Java (each also marked `PORT:` where it applies):
//!
//! - **Baseline.** Java's `createConfiguration` calls `setBaselineConfiguration(targetOptions)`
//!   on its one executor; Blaze's baseline is fixed when the executor is built (one command per
//!   process), so `create` builds a new executor whose top-level options are the target options.
//!   `createExec` uses the executor of the last `create`, or, before any, one built from the
//!   default options of the test (the product flags). Java's initial baseline is the bare
//!   default options; the baseline only names output directories, which the exec tests do not
//!   look at.
//! - `ConfigurationTestCase.TestOptions` (`--multi_cpu`) is not in the parser: no ported test
//!   uses it.

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::{FragmentFactory, InvalidConfigurationException};
use blaze_build_config::output_directories::ConfigDirectories;
use blaze_options::Value;

use crate::skyframe_executor::SkyframeExecutor;
use crate::testing::build_view_test_case::{BuildViewTestCase, TestCaseKind, exec_options};

/// Java `ConfigurationTestCase`. Dereferences to the [`BuildViewTestCase`] it runs on
/// (`scratch`, `reporter`, `assert_contains_event`, ...).
// Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:64 ConfigurationTestCase
pub struct ConfigurationTestCase {
    base: BuildViewTestCase,
    /// Java `fragmentFactory`.
    fragment_factory: FragmentFactory,
}

impl std::ops::Deref for ConfigurationTestCase {
    type Target = BuildViewTestCase;
    fn deref(&self) -> &BuildViewTestCase {
        &self.base
    }
}

impl std::ops::DerefMut for ConfigurationTestCase {
    fn deref_mut(&mut self) -> &mut BuildViewTestCase {
        &mut self.base
    }
}

impl ConfigurationTestCase {
    /// `@Before` of `FoundationTestCase` and `ConfigurationTestCase.initializeSkyframeExecutor`.
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:88 ConfigurationTestCase.initializeSkyframeExecutor
    pub fn new() -> ConfigurationTestCase {
        ConfigurationTestCase {
            base: BuildViewTestCase::create(TestCaseKind::Configuration),
            fragment_factory: FragmentFactory,
        }
    }

    /// `checkError(expectedMessage, options...)`.
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:166 ConfigurationTestCase.checkError
    pub fn check_error(&mut self, expected_message: &str, options: &[&str]) {
        self.base.remove_fail_fast_handler();
        assert!(
            self.try_create_configuration(BTreeMap::new(), options)
                .is_err(),
            "expected InvalidConfigurationException"
        );
        self.base.assert_contains_event(expected_message);
    }

    /// `createConfiguration(args...)` / `create(args...)`: a target configuration with the given
    /// non-default options (`"--cpu=k8"`, ...).
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:233 ConfigurationTestCase.create
    pub fn create(&mut self, args: &[&str]) -> Arc<BuildConfigurationValue> {
        self.create_with_starlark(&[], args)
    }

    /// `create(starlarkOptions, args...)`: also sets Starlark options (label-like name to value).
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:244 ConfigurationTestCase.create
    pub fn create_with_starlark(
        &mut self,
        starlark_options: &[(&str, &str)],
        args: &[&str],
    ) -> Arc<BuildConfigurationValue> {
        self.try_create_configuration(starlark_options_map(starlark_options), args)
            .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()))
    }

    /// `createConfiguration(starlarkOptions, args...)`.
    ///
    /// PORT: `handleDiffsForTesting` + `setBaselineConfiguration` + `createConfiguration` on one
    /// executor become a new executor whose baseline is the target options (module
    /// documentation).
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:183 ConfigurationTestCase.createConfiguration
    pub fn try_create_configuration(
        &mut self,
        starlark_options: BTreeMap<String, Value>,
        args: &[&str],
    ) -> Result<Arc<BuildConfigurationValue>, InvalidConfigurationException> {
        self.base.set_configuration_state(args, starlark_options);
        self.base.try_ensure_executor()?;
        let result = self.base.get_target_configuration();
        self.base.check_fail_fast();
        Ok(result)
    }

    /// `parseBuildOptions(starlarkOptions, args...)`: the build options of the given command
    /// line (with the product flags first).
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:207 ConfigurationTestCase.parseBuildOptions
    pub fn parse_build_options_with_starlark(
        &mut self,
        starlark_options: &[(&str, &str)],
        args: &[&str],
    ) -> BuildOptions {
        let parser = self
            .base
            .parse_options_with(args, starlark_options_map(starlark_options));
        blaze_build_config::top_level::create_build_options(
            &BazelRuleClassProvider::create(),
            &parser,
        )
    }

    /// `parseBuildOptions(args...)`.
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:213 ConfigurationTestCase.parseBuildOptions
    pub fn parse_build_options(&mut self, args: &[&str]) -> BuildOptions {
        self.parse_build_options_with_starlark(&[], args)
    }

    /// `createRaw(buildOptions, mnemonic, siblingRepositoryLayout)`.
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:218 ConfigurationTestCase.createRaw
    pub fn create_raw(
        &mut self,
        build_options: BuildOptions,
        mnemonic: &str,
        sibling_repository_layout: bool,
    ) -> BuildConfigurationValue {
        // Java: skyframeExecutor.getBlazeDirectoriesForTesting() /
        // getRuleClassProviderForTesting(): the directories and provider of the harness's
        // executor (see SkyframeExecutor::create_with_builtin_modules).
        let directories = ConfigDirectories {
            exec_root_base: self.base.output_base.get_child("execroot"),
            product_name: self.base.analysis_mock.get_product_name().to_string(),
        };
        BuildConfigurationValue::create_for_testing(
            Arc::new(build_options),
            mnemonic,
            sibling_repository_layout,
            &directories,
            &BazelRuleClassProvider::create(),
            &self.fragment_factory,
        )
        .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()))
    }

    /// `createExec(starlarkOptions, args...)`: an exec configuration derived from a target
    /// configuration with the given non-default options.
    ///
    /// PORT: runs in the executor of the last `create` (module documentation).
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:265 ConfigurationTestCase.createExec
    pub fn create_exec_with_starlark(
        &mut self,
        starlark_options: &[(&str, &str)],
        args: &[&str],
    ) -> Arc<BuildConfigurationValue> {
        let target_options =
            Arc::new(self.parse_build_options_with_starlark(starlark_options, args));
        let exec = self.exec_options(&target_options);
        let handler = self.base.event_handler();
        let result = self
            .skyframe_executor()
            .get_configuration(handler, Arc::new(exec), false)
            .unwrap_or_else(|e| panic!("InvalidConfigurationException: {}", e.get_message()));
        self.base.check_fail_fast();
        result
    }

    /// `createExec(args...)`.
    // Java: com/google/devtools/build/lib/analysis/util/ConfigurationTestCase.java:281 ConfigurationTestCase.createExec
    pub fn create_exec(&mut self, args: &[&str]) -> Arc<BuildConfigurationValue> {
        self.create_exec_with_starlark(&[], args)
    }

    /// `AnalysisTestUtil.execOptions(targetOptions, skyframeExecutor, reporter)`.
    pub fn exec_options(&mut self, target_options: &Arc<BuildOptions>) -> BuildOptions {
        let handler = self.base.event_handler();
        let result = exec_options(self.skyframe_executor(), target_options, handler);
        self.base.check_fail_fast();
        result
    }

    /// `skyframeExecutor`: the executor of the last `create`, or one with the default options.
    pub fn skyframe_executor(&mut self) -> &SkyframeExecutor {
        self.base.skyframe_executor()
    }
}

impl Default for ConfigurationTestCase {
    fn default() -> Self {
        Self::new()
    }
}

/// Java `ImmutableMap<String, Object>` of Starlark options with string values.
fn starlark_options_map(options: &[(&str, &str)]) -> BTreeMap<String, Value> {
    options
        .iter()
        .map(|(k, v)| (k.to_string(), Value::new(v.to_string())))
        .collect()
}
