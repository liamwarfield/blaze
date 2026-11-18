//! Port of the analysis-only part of `skyframe/SkyframeBuildView.java` (`configureTargets`) and
//! `skyframe/SkyframeAnalysisResult.java`.

use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::platform::PlatformValueKey;
use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_packages::target::Target;
use blaze_protos::build_event_stream::build_metrics::BuildGraphMetrics;
use blaze_sky_analysis::platform::platform_function::PlatformFunction;
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_skyframe::{EvaluationResult, Evaluator, Interrupted, Key, SkyKey, WalkableGraph};
use indexmap::{IndexMap, IndexSet};

use crate::build_view::AnalysisPhaseCompleteStats;

use crate::skyframe_executor::{SkyframeExecutor, TargetAndConfiguration};

/// Encapsulates the raw analysis result of top level targets and aspects coming from Skyframe.
// Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:30 SkyframeAnalysisResult
pub struct SkyframeAnalysisResult {
    has_loading_error: bool,
    has_analysis_error: bool,
    has_action_conflicts: bool,
    configured_targets: Vec<ConfiguredTarget>,
    walkable_graph: Arc<dyn WalkableGraph>,
    targets_with_configuration: Vec<TargetAndConfiguration>,
    /// The `AnalysisGraphStatsEvent` posted by the conflict check (`None` when not checked).
    build_graph_metrics: Option<BuildGraphMetrics>,
}

impl SkyframeAnalysisResult {
    /// The `BuildGraphMetrics` of the `AnalysisGraphStatsEvent` `configureTargets` posted, if it
    /// checked for action conflicts.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:454 SkyframeBuildView.configureTargets
    pub fn get_build_graph_metrics(&self) -> Option<&BuildGraphMetrics> {
        self.build_graph_metrics.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:63 SkyframeAnalysisResult.hasLoadingError
    pub fn has_loading_error(&self) -> bool {
        self.has_loading_error
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:71 SkyframeAnalysisResult.hasAnalysisError
    pub fn has_analysis_error(&self) -> bool {
        self.has_analysis_error
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:75 SkyframeAnalysisResult.hasActionConflicts
    pub fn has_action_conflicts(&self) -> bool {
        self.has_action_conflicts
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:79 SkyframeAnalysisResult.getConfiguredTargets
    pub fn get_configured_targets(&self) -> &[ConfiguredTarget] {
        &self.configured_targets
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:83 SkyframeAnalysisResult.getWalkableGraph
    pub fn get_walkable_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.walkable_graph
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:95 SkyframeAnalysisResult.getTargetsWithConfiguration
    pub fn get_targets_with_configuration(&self) -> &[TargetAndConfiguration] {
        &self.targets_with_configuration
    }
}

/// What `configureTargets` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigureTargetsError {
    #[error("interrupted")]
    Interrupted,
    /// `ViewCreationFailedException` (message and failure detail).
    #[error("{message}")]
    ViewCreationFailed {
        message: String,
        failure_detail: blaze_protos::failure_details::FailureDetail,
    },
}

impl From<Interrupted> for ConfigureTargetsError {
    fn from(_: Interrupted) -> Self {
        ConfigureTargetsError::Interrupted
    }
}

/// Analyzes the specified targets using Skyframe as the driving framework.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:400 SkyframeBuildView.configureTargets
pub fn configure_targets(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    label_to_target_map: &IndexMap<Label, Target>,
    ct_keys: &[ConfiguredTargetKey],
    keep_going: bool,
    check_for_action_conflicts: bool,
) -> Result<SkyframeAnalysisResult, ConfigureTargetsError> {
    // PORT: enableAnalysis(true/false) guards incremental reuse; not needed for one evaluation.
    let result = skyframe_executor.configure_targets(
        event_handler,
        label_to_target_map,
        ct_keys,
        keep_going,
    )?;

    let cts = result.configured_targets;
    let evaluation_result: EvaluationResult = result.evaluation_result;

    // PORT: the conflicts ArtifactConflictFinder finds are not reported (no inter-target
    // conflict checking); its statistics (the AnalysisGraphStatsEvent) are computed.
    let mut build_graph_metrics = None;
    if should_check_for_conflicts(
        check_for_action_conflicts,
        skyframe_executor.get_evaluator(),
    ) {
        let analysis_traversal_result = collect_action_lookup_values_in_build(
            skyframe_executor.get_walkable_graph().as_ref(),
            ct_keys,
        );
        let output_artifact_count = find_artifact_conflicts_output_artifact_count(
            &analysis_traversal_result.action_lookup_values,
        );
        let mut metrics = analysis_traversal_result.get_metrics();
        metrics.output_artifact_count = output_artifact_count;
        build_graph_metrics = Some(metrics);
    }

    // Intra-target conflict would mean an error in evaluationResult.
    if !evaluation_result.has_error() {
        return Ok(SkyframeAnalysisResult {
            has_loading_error: false,
            has_analysis_error: false,
            has_action_conflicts: false,
            configured_targets: cts,
            walkable_graph: skyframe_executor.get_walkable_graph(),
            targets_with_configuration: result.targets_with_configuration,
            build_graph_metrics,
        });
    }

    blaze_util::unported!(
        "SkyframeErrorProcessor.processAnalysisErrors",
        "analysis errors in {:?}",
        evaluation_result
            .error_map()
            .iter()
            .map(|(k, _)| format!("{:?}", k))
            .collect::<Vec<_>>()
    )
}

/// `shouldCheckForConflicts`: a build request checks unless it opted out (cquery); in a
/// one-build process the graph changed iff some `ActionLookupValue` was evaluated.
///
/// PORT: Java's `someActionLookupValueEvaluated` is set by the progress receiver; the command's
/// evaluator is fresh, so it is true iff the graph holds a rule configured target. The
/// incremental conditions (dirtied keys, the previous check's conflicts, a larger key set) cannot
/// hold in a fresh graph.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1254 SkyframeBuildView.shouldCheckForConflicts
fn should_check_for_conflicts(specified_value_in_request: bool, evaluator: &Evaluator) -> bool {
    if !specified_value_in_request {
        // A build request by default enables action conflict checking, except for some cases e.g.
        // cquery.
        return false;
    }
    evaluator.get_all_keys_for_testing().iter().any(|key| {
        key.downcast::<ConfiguredTargetKey>().is_some()
            && evaluator
                .get_existing_value(key)
                .and_then(|v| v.downcast::<ConfiguredTargetValue>().ok())
                .is_some_and(|v| matches!(v.get_configured_target(), ConfiguredTarget::Rule(_)))
    })
}

/// Represents the traversal of the `ActionLookupValue`s in a build.
// Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:33 ActionLookupValuesTraversal
#[derive(Default)]
struct ActionLookupValuesTraversal {
    /// `actionLookupValueShards` (unsharded).
    action_lookup_values: Vec<Vec<blaze_actions::ActionRef>>,
    configured_object_count: i32,
    configured_target_count: i32,
    action_count: i32,
    action_count_not_including_aspects: i32,
    input_file_configured_target_count: i32,
    output_file_configured_target_count: i32,
    other_configured_target_count: i32,
}

impl ActionLookupValuesTraversal {
    /// PORT: the only `ActionLookupKey`s in Blaze's graph are `ConfiguredTargetKey`s (aspects
    /// and the workspace status are not graph nodes), so every value is a
    /// `ConfiguredTargetValue`; a rule's value is the `ActionLookupValue`
    /// (`RuleConfiguredTargetValue`), the others are `NonRuleConfiguredTargetValue`s.
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:48 ActionLookupValuesTraversal.accumulate
    fn accumulate(&mut self, key: &ConfiguredTargetKey, value: Arc<ConfiguredTargetValue>) {
        if key.get_configuration_key().cloned()
            != value.get_configured_target().get_configuration_key()
        {
            // The configuration of the key doesn't match the configuration of the value. This
            // means that the ConfiguredTargetValue is delegated from a different key. This
            // ConfiguredTargetValue will show up again under its own key. Avoids double counting
            // by skipping accumulation.
            return;
        }
        self.configured_object_count += 1;
        self.configured_target_count += 1;
        if matches!(value.get_configured_target(), ConfiguredTarget::Rule(_)) {
            let num_actions = value.get_actions().len() as i32;
            self.action_count += num_actions;
            self.action_count_not_including_aspects += num_actions;
            self.action_lookup_values.push(value.get_actions().to_vec());
            return;
        }
        match value.get_configured_target() {
            ConfiguredTarget::InputFile(_) => self.input_file_configured_target_count += 1,
            ConfiguredTarget::OutputFile(_) => self.output_file_configured_target_count += 1,
            _ => self.other_configured_target_count += 1,
        }
    }

    /// `accumulate` for an `ActionLookupValue` that is not a `ConfiguredTargetValue` (the
    /// workspace status value).
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:48 ActionLookupValuesTraversal.accumulate
    fn accumulate_action_lookup_value(&mut self, actions: Vec<blaze_actions::ActionRef>) {
        self.configured_object_count += 1;
        self.action_count += actions.len() as i32;
        self.action_lookup_values.push(actions);
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:111 ActionLookupValuesTraversal.getMetrics
    fn get_metrics(&self) -> BuildGraphMetrics {
        BuildGraphMetrics {
            action_lookup_value_count: self.configured_object_count,
            action_lookup_value_count_not_including_aspects: self.configured_target_count,
            action_count: self.action_count,
            action_count_not_including_aspects: self.action_count_not_including_aspects,
            input_file_configured_target_count: self.input_file_configured_target_count,
            output_file_configured_target_count: self.output_file_configured_target_count,
            other_configured_target_count: self.other_configured_target_count,
            ..Default::default()
        }
    }
}

/// `SkyframeExecutor.collectActionLookupValuesInBuild` with incrementality tracked: the
/// top-down `TransitiveActionLookupKeysCollector` over the analysis graph.
///
/// PORT: the traversal is sequential (Java forks one task per key); the counters do not depend
/// on the visiting order.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3683 SkyframeExecutor.collectActionLookupValuesInBuild
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4274 TransitiveActionLookupKeysCollector
fn collect_action_lookup_values_in_build(
    walkable_graph: &dyn WalkableGraph,
    top_level_ct_keys: &[ConfiguredTargetKey],
) -> ActionLookupValuesTraversal {
    let mut alv_traversal = ActionLookupValuesTraversal::default();
    let mut seen: IndexSet<SkyKey> = IndexSet::new();
    let mut stack: Vec<SkyKey> = Vec::new();
    for key in top_level_ct_keys {
        let key = SkyKey::new(key.clone());
        if seen.insert(key.clone()) {
            stack.push(key);
        }
    }
    while let Some(key) = stack.pop() {
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4327 VisitActionLookupKey.compute
        let Some(value) = walkable_graph.get_value(&key) else {
            // The value failed to evaluate.
            continue;
        };
        let ct_key = key
            .downcast::<ConfiguredTargetKey>()
            .unwrap_or_else(|| panic!("ActionLookupKey {:?} is not a ConfiguredTargetKey", key));
        let value = value
            .downcast::<ConfiguredTargetValue>()
            .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key));
        alv_traversal.accumulate(ct_key, value);

        for dep in walkable_graph.get_direct_deps(&key) {
            // Besides PlatformFunction, the subgraph of dependencies of ActionLookupKeys never has
            // a non-ActionLookupKey depending on an ActionLookupKey. So we can skip any other
            // non-ActionLookupKeys in the traversal as an optimization.
            let dep = match dep.downcast::<PlatformValueKey>() {
                Some(platform_key) => SkyKey::new(PlatformFunction::configured_target_dep(
                    &platform_key.platform_label,
                )),
                None => dep,
            };
            if dep.function_name() == <ConfiguredTargetKey as Key>::FUNCTION {
                if seen.insert(dep.clone()) {
                    stack.push(dep);
                }
            } else if dep.function_name()
                == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
            {
                if seen.insert(dep.clone()) {
                    // A leaf: the workspace status value has no ActionLookupKey dependencies.
                    if let Some(value) = walkable_graph.get_value(&dep) {
                        let value = value
                            .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
                            .unwrap_or_else(|_| panic!("BUILD_INFO value"));
                        alv_traversal.accumulate_action_lookup_value(value.actions.clone());
                    }
                }
            } else if matches!(dep.function_name().0, "ASPECT" | "TOP_LEVEL_ASPECTS") {
                blaze_util::unported!(
                    "ActionLookupValuesTraversal.accumulate",
                    "non-configured-target ActionLookupKey {:?}",
                    dep
                );
            }
        }
    }
    alv_traversal
}

/// `ArtifactConflictFinder.findAndStoreArtifactConflicts`'s statistics: the size of the action
/// graph (`MapBasedActionGraph.getSize()`, the number of distinct output artifacts) built from
/// the actions of the build's `ActionLookupValue`s.
///
/// `registerAction` stops registering an action's outputs at its first output another action
/// already generates when the two can be shared (`Actions.canBeShared`); otherwise the actions
/// conflict (`ActionConflictException`). `OwnerlessArtifactWrapper` compares artifacts without
/// their owner: by exec path (which includes the root).
///
/// PORT: reporting a conflict (`ActionConflictException` → the conflicting targets' analysis
/// errors) and the output-prefix conflict check (`findArtifactPrefixConflicts`) are not ported;
/// a conflict fails loudly.
// Java: com/google/devtools/build/lib/skyframe/ArtifactConflictFinder.java:61 ArtifactConflictFinder.findAndStoreArtifactConflicts
// Java: com/google/devtools/build/lib/actions/MapBasedActionGraph.java:46 MapBasedActionGraph.registerAction
fn find_artifact_conflicts_output_artifact_count(
    action_lookup_values: &[Vec<blaze_actions::ActionRef>],
) -> i32 {
    // Java: SkyframeExecutor.getActionKeyContext(); only a cache of key computations.
    let action_key_context = blaze_actions::ActionKeyContext::new();
    let mut generating_action_map: std::collections::HashMap<
        blaze_vfs::PathFragment,
        blaze_actions::actions::action::ActionRef,
    > = std::collections::HashMap::new();
    for value in action_lookup_values {
        'actions: for action in value {
            for artifact in action.get_outputs() {
                match generating_action_map.entry(artifact.get_exec_path().clone()) {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(action.clone());
                    }
                    std::collections::hash_map::Entry::Occupied(e) => {
                        let previous_action = e.get();
                        if !Arc::ptr_eq(previous_action, action) {
                            let shareable = blaze_analysis::rule::actions_util::can_be_shared(
                                &action_key_context,
                                action,
                                previous_action,
                            )
                            .unwrap_or_else(|_| panic!("InterruptedException"));
                            if shareable {
                                // All outputs can be shared. No need to register the remaining
                                // outputs.
                                continue 'actions;
                            }
                            // Java: com/google/devtools/build/lib/actions/ActionConflictException.java:48 ActionConflictException.create
                            blaze_util::unported!(
                                "ArtifactConflictFinder (ActionConflictException reporting)",
                                "file '{}' is generated by these conflicting actions: {} and {}",
                                artifact.get_exec_path_string(),
                                previous_action.pretty_print(),
                                action.pretty_print()
                            );
                        }
                    }
                }
            }
        }
    }
    generating_action_map.len() as i32
}

/// The counters of the analysis phase's `AnalysisPhaseCompleteEvent`:
/// `SkyframeBuildView.getEvaluatedCounts()`, `getEvaluatedActionCounts()`,
/// `getEvaluatedActionCountsByMnemonic()` (the `ActionLookupValueProgressReceiver`) and
/// `getAndClearPkgManagerStatistics()` (`PackageFunction`'s count of packages loaded without
/// errors).
///
/// PORT: Java counts with an `EvaluationProgressReceiver` as nodes are evaluated; Blaze's
/// evaluator has no progress receivers, and each command has its own fresh evaluator, so the
/// nodes evaluated in this command are the done nodes of its graph: a sweep over the graph
/// (Java's own non-incremental `collectActionLookupValuesInBuild` does the same). The
/// by-mnemonic map is a `ConcurrentHashMap` in Java; it is filled here in sweep order (its
/// consumer, `MetricsCollector`, orders its own map). Aspects and the workspace status are not
/// graph nodes in Blaze, so they are not counted.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1481 ActionLookupValueProgressReceiver.evaluated
pub fn get_evaluated_counts(evaluator: &Evaluator, time_in_ms: i64) -> AnalysisPhaseCompleteStats {
    let mut configured_object_count = 0;
    let mut configured_target_count = 0;
    let mut action_count = 0;
    let mut configured_target_action_count = 0;
    let mut action_count_by_mnemonic: IndexMap<String, i32> = IndexMap::new();
    let mut packages_successfully_loaded = 0;
    for key in evaluator.get_all_keys_for_testing() {
        if key.downcast::<PackageKey>().is_some() {
            // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:624 PackageFunction.compute
            if let Some(value) = evaluator.get_existing_value(&key)
                && let Ok(value) = value.downcast::<PackageValue>()
                && !value.get_package().contains_errors()
            {
                packages_successfully_loaded += 1;
            }
            continue;
        }
        if key.function_name()
            == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
        {
            // The workspace status value: an ActionLookupValue that is not a configured target.
            if let Some(value) = evaluator.get_existing_value(&key) {
                let value = value
                    .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
                    .unwrap_or_else(|_| panic!("BUILD_INFO value"));
                configured_object_count += 1;
                for action in &value.actions {
                    *action_count_by_mnemonic
                        .entry(action.get_mnemonic())
                        .or_insert(0) += 1;
                }
                action_count += value.actions.len() as i32;
            }
            continue;
        }
        let Some(ct_key) = key.downcast::<ConfiguredTargetKey>() else {
            continue;
        };
        let Some(value) = evaluator.get_existing_value(&key) else {
            continue;
        };
        let value = value
            .downcast::<ConfiguredTargetValue>()
            .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key));
        if ct_key.get_configuration_key().cloned()
            != value.get_configured_target().get_configuration_key()
        {
            // The node entry performs delegation and doesn't own the value. Skips it to avoid
            // overcounting.
            continue;
        }
        configured_target_count += 1;
        configured_object_count += 1;
        if matches!(value.get_configured_target(), ConfiguredTarget::Rule(_)) {
            let actions = value.get_actions();
            for action in actions {
                *action_count_by_mnemonic
                    .entry(action.get_mnemonic())
                    .or_insert(0) += 1;
            }
            let num_actions = actions.len() as i32;
            action_count += num_actions;
            configured_target_action_count += num_actions;
        }
    }
    AnalysisPhaseCompleteStats {
        actions_constructed_total: action_count,
        actions_constructed_configured_targets_only: configured_target_action_count,
        targets_configured_total: configured_object_count,
        targets_configured_configured_targets_only: configured_target_count,
        time_in_ms,
        packages_successfully_loaded,
        actions_constructed_by_mnemonic: action_count_by_mnemonic.into_iter().collect(),
    }
}
