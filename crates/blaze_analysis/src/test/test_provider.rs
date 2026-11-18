//! Ports of `analysis/test/TestProvider.java` (with `TestParams`) and
//! `analysis/test/TestTagsProvider.java`.

use std::sync::Arc;

use blaze_actions::{ActionOwner, Artifact};
use blaze_optdefs::fragment::test_timeout::TestTimeout;

use crate::configured_target::ConfiguredTarget;
use crate::provider_map::TransitiveInfoProvider;
use crate::providers::file_provider::FilesToRunProvider;

/// A `TransitiveInfoProvider` for configured targets to implement test-specific behavior.
// Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:32 TestProvider
#[derive(Clone, Debug)]
pub struct TestProvider {
    test_params: Arc<TestParams>,
}

impl TestProvider {
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:35 TestProvider.<init>
    pub fn new(test_params: TestParams) -> TestProvider {
        TestProvider {
            test_params: Arc::new(test_params),
        }
    }

    /// Returns the [`TestParams`] object for the test represented by the corresponding
    /// configured target.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:43 TestProvider.getTestParams
    pub fn get_test_params(&self) -> &TestParams {
        &self.test_params
    }

    /// Returns the test status artifacts for a specified configured target.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:53 TestProvider.getTestStatusArtifacts
    pub fn get_test_status_artifacts(target: &ConfiguredTarget) -> Vec<Artifact> {
        crate::configuredtargets::get_provider::<TestProvider>(target)
            .expect("TestProvider")
            .get_test_params()
            .get_test_status_artifacts()
            .to_vec()
    }
}

impl TransitiveInfoProvider for TestProvider {
    fn java_class_name(&self) -> &'static str {
        "TestProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The coverage parameters of a test (Java `TestParams.CoverageParams`).
// Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:60 TestProvider.TestParams.CoverageParams
#[derive(Clone, Debug)]
pub struct CoverageParams {
    pub coverage_artifacts: Vec<Artifact>,
    pub coverage_report_generator: Arc<FilesToRunProvider>,
    pub action_owner: ActionOwner,
}

/// A value class describing the properties of a test.
// Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:58 TestProvider.TestParams
#[derive(Clone, Debug)]
pub struct TestParams {
    runs: i32,
    shards: i32,
    runs_detects_flakes: bool,
    timeout: TestTimeout,
    test_rule_class: String,
    test_status_artifacts: Vec<Artifact>,
    /// Java `ImmutableList<ActionInput>`: the spawn outputs (paths) and action outputs.
    // PORT: non-artifact ActionInputs (paths from ActionInputHelper.fromPath) are kept as exec
    // path strings.
    outputs: Vec<TestOutput>,
    coverage_params: Option<CoverageParams>,
}

/// One test output (Java `ActionInput`): an artifact or a bare exec path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestOutput {
    Artifact(Artifact),
    Path(blaze_vfs::PathFragment),
}

impl TestParams {
    /// Don't call this directly. Instead use `TestActionBuilder`.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:83 TestProvider.TestParams.<init>
    pub fn new(
        runs: i32,
        shards: i32,
        runs_detects_flakes: bool,
        timeout: TestTimeout,
        test_rule_class: String,
        test_status_artifacts: Vec<Artifact>,
        outputs: Vec<TestOutput>,
        coverage_params: Option<CoverageParams>,
    ) -> TestParams {
        TestParams {
            runs,
            shards,
            runs_detects_flakes,
            timeout,
            test_rule_class,
            test_status_artifacts,
            outputs,
            coverage_params,
        }
    }

    /// Returns the number of times this test should be run.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:104 TestProvider.TestParams.getRuns
    pub fn get_runs(&self) -> i32 {
        self.runs
    }

    /// Returns the number of shards for this test.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:109 TestProvider.TestParams.getShards
    pub fn get_shards(&self) -> i32 {
        self.shards
    }

    /// Returns true iff --runs_per_test_detects_flakes is set.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:114 TestProvider.TestParams.runsDetectsFlakes
    pub fn runs_detects_flakes(&self) -> bool {
        self.runs_detects_flakes
    }

    /// Returns the timeout of this test.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:119 TestProvider.TestParams.getTimeout
    pub fn get_timeout(&self) -> TestTimeout {
        self.timeout
    }

    /// Returns the test rule class.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:124 TestProvider.TestParams.getTestRuleClass
    pub fn get_test_rule_class(&self) -> &str {
        &self.test_rule_class
    }

    /// Returns a list of test status artifacts that represent serialized test status protobuffers
    /// produced by testing this target.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:133 TestProvider.TestParams.getTestStatusArtifacts
    pub fn get_test_status_artifacts(&self) -> &[Artifact] {
        &self.test_status_artifacts
    }

    /// Returns the list of outputs this test will produce when run.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:138 TestProvider.TestParams.getOutputs
    pub fn get_outputs(&self) -> &[TestOutput] {
        &self.outputs
    }

    /// Returns the coverageArtifacts.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:143 TestProvider.TestParams.getCoverageArtifacts
    pub fn get_coverage_artifacts(&self) -> Vec<Artifact> {
        self.coverage_params
            .as_ref()
            .map(|c| c.coverage_artifacts.clone())
            .unwrap_or_default()
    }

    /// Returns the coverage report generator tool.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:148 TestProvider.TestParams.getCoverageReportGenerator
    pub fn get_coverage_report_generator(&self) -> Option<&Arc<FilesToRunProvider>> {
        self.coverage_params
            .as_ref()
            .map(|c| &c.coverage_report_generator)
    }

    /// Returns the action owner for the coverage actions.
    // Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:154 TestProvider.TestParams.getActionOwnerForCoverage
    pub fn get_action_owner_for_coverage(&self) -> Option<&ActionOwner> {
        self.coverage_params.as_ref().map(|c| &c.action_owner)
    }
}

/// A `TransitiveInfoProvider` for configured targets to provide test tags.
// Java: com/google/devtools/build/lib/analysis/test/TestTagsProvider.java:24 TestTagsProvider
#[derive(Clone, Debug)]
pub struct TestTagsProvider {
    test_tags: Vec<String>,
}

impl TestTagsProvider {
    // Java: com/google/devtools/build/lib/analysis/test/TestTagsProvider.java:28 TestTagsProvider.<init>
    pub fn new(test_tags: Vec<String>) -> TestTagsProvider {
        TestTagsProvider { test_tags }
    }

    /// Returns the test tags.
    // Java: com/google/devtools/build/lib/analysis/test/TestTagsProvider.java:35 TestTagsProvider.getTestTags
    pub fn get_test_tags(&self) -> &[String] {
        &self.test_tags
    }
}

impl TransitiveInfoProvider for TestTagsProvider {
    fn java_class_name(&self) -> &'static str {
        "TestTagsProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Java `TestActionBuilder.createEmptyTestParams()`.
// Java: com/google/devtools/build/lib/analysis/test/TestActionBuilder.java:95 TestActionBuilder.createEmptyTestParams
pub fn create_empty_test_params() -> TestParams {
    TestParams::new(
        0,
        0,
        false,
        TestTimeout::ETERNAL,
        "invalid".to_owned(),
        Vec::new(),
        Vec::new(),
        None,
    )
}
