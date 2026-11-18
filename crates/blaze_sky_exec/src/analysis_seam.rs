//! The seam between configured-target analysis (blaze_analysis / blaze_sky_analysis) and the
//! execution phase.
//!
//! In Java the execution SkyFunctions request analysis keys directly
//! (`env.getValue(actionLookupKey)` → `ActionLookupValue`, `CONFIGURED_TARGET` →
//! `ConfiguredTargetValue`) and read providers off `ConfiguredTarget`. Those types live in crates
//! that sit below this one but are written concurrently, so the execution phase reads them through
//! the traits here; the analysis crates implement them and hand an [`AnalysisSeam`] to
//! [`crate::register_execution_functions`].

use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::{ActionLookupKeyRef, Artifact};
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_skyframe::{ComputeError, Environment};
use indexmap::IndexMap;

/// Base interface for things that own actions (configured targets, aspects, the workspace status
/// and build-info keys).
// Java: com/google/devtools/build/lib/actions/ActionLookupValue.java:27 ActionLookupValue
pub trait ActionLookupValue: Send + Sync {
    /// Returns a list of actions registered by this SkyValue.
    // Java: com/google/devtools/build/lib/actions/ActionLookupValue.java:32 ActionLookupValue.getActions
    fn get_actions(&self) -> &[ActionRef];

    /// Returns the action registered at `index`.
    // Java: com/google/devtools/build/lib/actions/ActionLookupValue.java:38 ActionLookupValue.getAction
    fn get_action(&self, index: i32) -> ActionRef {
        self.get_actions()[index as usize].clone()
    }
}

/// `TestProvider.TestParams`: what the execution phase reads from a test target.
// Java: com/google/devtools/build/lib/analysis/test/TestProvider.java:63 TestProvider.TestParams
#[derive(Clone, Debug)]
pub struct TestParams {
    /// `getRuns()`.
    pub runs: i32,
    /// `getShards()`.
    pub shards: i32,
    /// `runsDetectsFlakes()`.
    pub runs_detects_flakes: bool,
    /// `getTimeout()`.
    pub timeout: TestTimeout,
    /// `getTestRuleClass()`, e.g. `cc_test`.
    pub test_rule_class: String,
    /// `getTestStatusArtifacts()`: one `test.cache_status` artifact per run and shard; their
    /// generating actions are the `TestRunnerAction`s.
    pub test_status_artifacts: Vec<Artifact>,
}

/// The parts of a top-level `ConfiguredTarget` (with its `FileProvider`, `OutputGroupInfo`,
/// `FilesToRunProvider` and `TestProvider`) the execution phase reads.
pub trait ConfiguredTargetView: Send + Sync + fmt::Debug {
    /// `getLabel()`.
    fn get_label(&self) -> Label;

    /// `getOriginalLabel()` (differs from `getLabel()` for aliases).
    fn get_original_label(&self) -> Label {
        self.get_label()
    }

    /// The configuration checksum of `getConfigurationKey()`, `None` for source files and other
    /// targets without a configuration.
    fn get_configuration_checksum(&self) -> Option<String>;

    /// The `ActionLookupKey` of this target (its `ConfiguredTargetKey`).
    fn get_lookup_key(&self) -> ActionLookupKeyRef;

    /// The rule class name (`cc_binary`, `cc_test`, `test_suite`, ...) or the target kind for
    /// non-rules (`source file`, `generated file`).
    fn get_rule_class_or_kind(&self) -> String;

    /// `OutputGroupInfo` of this target in insertion order (`default`,
    /// `_hidden_top_level_INTERNAL_`, `_validation`, ...). Includes the `default` group computed
    /// from `FileProvider.getFilesToBuild()` as `TopLevelArtifactHelper` sees it.
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>>;

    /// `FilesToRunProvider.getExecutable()`.
    fn get_executable(&self) -> Option<Artifact>;

    /// `TestProvider.getTestParams()` for test rules, `None` otherwise.
    fn get_test_params(&self) -> Option<TestParams>;

    /// `ConfiguredTarget` of a `test_suite`: the expanded tests are handled by
    /// `TestsForTargetPatternFunction`, so the suite itself builds nothing.
    fn is_test_suite(&self) -> bool {
        self.get_rule_class_or_kind() == "test_suite"
    }
}

/// How the execution SkyFunctions reach analysis values.
pub trait AnalysisSeam: Send + Sync {
    /// `env.getValue(actionLookupKey)` as an `ActionLookupValue` (`CONFIGURED_TARGET`, `ASPECT`,
    /// `BUILD_INFO`, `WORKSPACE_STATUS`, ...). `Ok(None)` when the value is missing.
    fn get_action_lookup_value(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ActionLookupValue>>, ComputeError>;

    /// `env.getValue(configuredTargetKey)` as a top-level target view. `Ok(None)` when missing.
    fn get_configured_target(
        &self,
        key: &ActionLookupKeyRef,
        env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredTargetView>>, ComputeError>;
}
