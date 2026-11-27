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

    /// `getTransitivePackages()` of the value: the packages transitively loaded for it, `None`
    /// when they are not tracked or were cleared.
    // Java: com/google/devtools/build/lib/skyframe/AbstractConfiguredTargetValue.java:49 AbstractConfiguredTargetValue.getTransitivePackages
    fn get_transitive_packages(
        &self,
    ) -> Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        None
    }

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

    /// The view as `Any`, for the analysis crates that created it to reach the `ConfiguredTarget`
    /// behind it (Java passes the `ConfiguredTarget` itself). Views that do not override it are
    /// reached by no one.
    fn as_any(&self) -> &dyn std::any::Any {
        &()
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

    /// `getTransitivePackages()` of the value: the packages transitively loaded for it, `None`
    /// when they are not tracked or were cleared.
    // Java: com/google/devtools/build/lib/analysis/AspectValue.java:124 AspectValue.getTransitivePackages
    fn get_transitive_packages(
        &self,
    ) -> Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        None
    }
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

    /// `env.getValueOrThrow(actionLookupKey, AbstractSaneAnalysisException.class)` of
    /// `BuildDriverFunction.compute`: the analyzed top-level value of a configured-target or a
    /// top-level-aspects key, `Ok(Ok(None))` when it is missing, `Ok(Err(e))` with the
    /// dependency's exception when it is an `AbstractSaneAnalysisException`.
    ///
    /// The default reads the value without catching ([`AnalysisSeam::get_configured_target`],
    /// [`AnalysisSeam::get_top_level_aspects`]).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:174 BuildDriverFunction.compute
    fn get_top_level_value_or_throw(
        &self,
        key: &crate::keys::BuildDriverKey,
        env: &mut Environment<'_>,
    ) -> Result<
        Result<Option<crate::build_driver_function::TopLevelSkyValue>, blaze_skyframe::SkyError>,
        ComputeError,
    > {
        use crate::build_driver_function::TopLevelSkyValue;
        Ok(Ok(if key.is_top_level_aspect_driver() {
            self.get_top_level_aspects(&key.action_lookup_key, env)?
                .map(TopLevelSkyValue::TopLevelAspects)
        } else {
            self.get_configured_target(&key.action_lookup_key, env)?
                .map(TopLevelSkyValue::ConfiguredTarget)
        }))
    }

    /// Whether `BuildDriverFunction` runs in a build with merged analysis and execution
    /// (Skymeld): the build has installed the suppliers Java's function reads (the
    /// `TestTypeResolver`, the conflict checking mode, the constraint semantics) and expects the
    /// function to announce the analysis of its key. `false` (the default) for the path without
    /// merged analysis and execution, whose analysis announced every top-level target before
    /// the keys are evaluated.
    fn is_merged_analysis_and_execution(&self) -> bool {
        false
    }

    /// `announceTopLevelConfiguredTargetAnalyzed`: posts the `TargetConfiguredEvent` of the
    /// target (when `post_target_configured`, the first time for the key) and returns the
    /// `TopLevelTargetAnalyzedEvent` to post. `Ok(None)` when a value is missing.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
    fn announce_top_level_configured_target_analyzed(
        &self,
        _configured_target: &Arc<dyn ConfiguredTargetView>,
        _post_target_configured: bool,
        _env: &mut Environment<'_>,
    ) -> Result<Option<Arc<dyn blaze_events::Postable>>, ComputeError> {
        blaze_util::unported!("AnalysisSeam.announce_top_level_configured_target_analyzed")
    }

    /// `announceTopLevelAspectAnalyzed` (once per key): the `ToplevelAspectsIdentifiedEvent`, then
    /// an `AspectConfiguredEvent` and an `AspectAnalyzedEvent` per aspect.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:449 BuildDriverFunction.announceTopLevelAspectAnalyzed
    fn announce_top_level_aspect_analyzed(
        &self,
        _key: &crate::keys::BuildDriverKey,
        _env: &mut Environment<'_>,
    ) -> Result<Option<()>, ComputeError> {
        blaze_util::unported!("AnalysisSeam.announce_top_level_aspect_analyzed")
    }

    /// `shouldCheckForConflictWithTraversal.get()`: the build's conflict checking mode is
    /// `WITH_TRAVERSAL`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2487 SkyframeExecutor.evaluateBuildDriverKeys
    fn should_check_for_conflict_with_traversal(&self) -> bool {
        false
    }

    /// `checkActionConflicts(actionLookupKey)`: `Ok(())` without conflicts (or when the checker
    /// is shut down), otherwise the `BuildDriverFunctionException(TopLevelConflictException(
    /// "Action conflict(s) detected while analyzing top-level target <label>", conflicts))` to
    /// throw.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:661 BuildDriverFunction.checkActionConflicts
    fn check_action_conflicts(
        &self,
        _key: &crate::keys::BuildDriverKey,
    ) -> Result<(), ComputeError> {
        Ok(())
    }

    /// `testTypeResolver.get().determineTestType(configuredTarget)`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:723 BuildDriverFunction.TestTypeResolver.determineTestType
    fn determine_test_type(
        &self,
        _configured_target: &Arc<dyn ConfiguredTargetView>,
        _env: &mut Environment<'_>,
    ) -> crate::keys::TestType {
        blaze_util::unported!("AnalysisSeam.determine_test_type")
    }

    /// `isConfiguredTargetCompatible`: `Ok(Some(compatible))`, `Ok(None)` when a value is missing,
    /// or the `BuildDriverFunctionException(TargetCompatibilityCheckException)` to throw.
    /// `checked_for_platform_compatibility` is the function's
    /// `state.checkedForPlatformCompatibility`.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:523 BuildDriverFunction.isConfiguredTargetCompatible
    fn is_configured_target_compatible(
        &self,
        _configured_target: &Arc<dyn ConfiguredTargetView>,
        _is_explicitly_requested: bool,
        _skip_incompatible_explicit_targets: bool,
        _checked_for_platform_compatibility: &mut bool,
        _env: &mut Environment<'_>,
    ) -> Result<Option<bool>, ComputeError> {
        blaze_util::unported!("AnalysisSeam.is_configured_target_compatible")
    }

    /// `TopLevelTargetSkippedEvent.create(configuredTarget)` (the event lives in the analysis
    /// crates).
    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:92 TopLevelStatusEvents.TopLevelTargetSkippedEvent.create
    fn top_level_target_skipped_event(
        &self,
        _configured_target: &Arc<dyn ConfiguredTargetView>,
    ) -> Arc<dyn blaze_events::Postable> {
        blaze_util::unported!("AnalysisSeam.top_level_target_skipped_event")
    }

    /// `extraActionFilterSupplier.get().isIncluded(ownerLabel)`: the
    /// `--experimental_extra_action_filter` of the current build applied to an extra action
    /// artifact's owner label (`true` for the default, empty filter).
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:691 BuildDriverFunction.addArtifactsToBuilder
    fn is_extra_action_owner_included(&self, _owner_label: &str) -> bool {
        true
    }
}
