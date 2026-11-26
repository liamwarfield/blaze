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

    /// `ConfiguredTargetAndData.getLocation()`: the target's location.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java ConfiguredTargetAndData.getLocation
    fn get_location(&self) -> blaze_events::location::Location {
        blaze_util::unported!("ConfiguredTargetAndData.getLocation (view without its target)")
    }

    /// `getMnemonic()` of the target's `BuildConfigurationValue` when the view knows it (the
    /// `TestAnalyzedEvent` the build driver posts carries it).
    fn get_configuration_mnemonic(&self) -> Option<String> {
        None
    }

    /// `ConfiguredTarget` of a `test_suite`: the expanded tests are handled by
    /// `TestsForTargetPatternFunction`, so the suite itself builds nothing.
    fn is_test_suite(&self) -> bool {
        self.get_rule_class_or_kind() == "test_suite"
    }

    /// `getProvider(ExtraActionArtifactsProvider.class)`: its extra action artifacts and its
    /// transitive extra action artifacts; `None` without the provider (no
    /// `--experimental_action_listener`).
    fn get_extra_action_artifacts_provider(
        &self,
    ) -> Option<(NestedSet<Artifact>, NestedSet<Artifact>)> {
        None
    }
}

/// A configured aspect (an `AspectKey` with its `AspectValue`) as the execution phase
/// (`BuildDriverFunction`, `AspectCompletor`) and `BuildResultPrinter` read it.
///
/// PORT: `AspectKey` and `AspectValue` live in the analysis crates, above this one; they
/// implement this view.
pub trait ConfiguredAspectView: Send + Sync {
    /// `AspectKey.getLabel()`.
    fn get_label(&self) -> Label;
    /// `AspectKey.getAspectClass().getName()`.
    fn get_aspect_class_name(&self) -> String;
    /// `AspectKey.getBaseConfiguredTargetKey()`.
    fn get_base_configured_target_key(&self) -> ActionLookupKeyRef;
    /// The aspect's `OutputGroupInfo`, in insertion order
    /// (`TopLevelArtifactHelper.getAllArtifactsToBuild` over the `AspectValue`).
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>>;

    /// The `AspectKey` itself (the key of the `AspectCompletionKey`).
    fn get_aspect_key(&self) -> ActionLookupKeyRef {
        blaze_util::unported!("ConfiguredAspectView.get_aspect_key (AspectKey of the view)")
    }

    /// `AspectValue.getWritesOutputToMasterLog()` (`AspectCompleteEvent`).
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:105 AspectValue.getWritesOutputToMasterLog
    fn get_writes_output_to_master_log(&self) -> bool {
        blaze_util::unported!("AspectValue.getWritesOutputToMasterLog (view)")
    }

    /// `AspectValue.getProvider(ExtraActionArtifactsProvider.class)`, as for
    /// [`ConfiguredTargetView::get_extra_action_artifacts_provider`].
    fn get_extra_action_artifacts_provider(
        &self,
    ) -> Option<(NestedSet<Artifact>, NestedSet<Artifact>)> {
        None
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

    /// `env.getValue(topLevelAspectsKey)` as the views of
    /// `TopLevelAspectsValue.getTopLevelAspectsMap()`, in the map's order. `Ok(None)` when the
    /// value is missing.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:323 BuildDriverFunction.compute
    fn get_top_level_aspects(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Vec<Arc<dyn ConfiguredAspectView>>>, ComputeError> {
        blaze_util::unported!("AnalysisSeam.get_top_level_aspects", "{key}")
    }

    /// `env.getValue(aspectKey)` as an aspect view (`CompletionFunction
    /// .getValueAndArtifactsToBuild` with an `AspectCompletionKey`). `Ok(None)` when missing.
    // Java: com/google/devtools/build/lib/skyframe/CompletionFunction.java:364 CompletionFunction.getValueAndArtifactsToBuild
    fn get_configured_aspect(
        &self,
        key: &ActionLookupKeyRef,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn ConfiguredAspectView>>, ComputeError> {
        blaze_util::unported!("AnalysisSeam.get_configured_aspect", "{key}")
    }

    /// `extraActionFilterSupplier.get().isIncluded(ownerLabel)`: the
    /// `--experimental_extra_action_filter` of the current build applied to an extra action
    /// artifact's owner label (`true` for the default, empty filter).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:691 BuildDriverFunction.addArtifactsToBuilder
    fn is_extra_action_owner_included(&self, _owner_label: &str) -> bool {
        true
    }
}
