//! Port of the analysis-only part of `skyframe/SkyframeBuildView.java` (`configureTargets`) and
//! `skyframe/SkyframeAnalysisResult.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_actions::actions::action_lookup::ArtifactOwner;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_build_config::platform::PlatformValueKey;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::target::Target;
use blaze_protos::build_event_stream::build_metrics::BuildGraphMetrics;
use blaze_sky_analysis::platform::platform_function::PlatformFunction;
use blaze_sky_analysis::producers::errors::configuration_id_of_key;
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_skyframe::{EvaluationResult, Evaluator, Interrupted, Key, SkyKey, WalkableGraph};
use indexmap::{IndexMap, IndexSet};

use crate::action_conflict_exception::{ActionConflictException, ActionConflicts};
use crate::analysis_failure_event::{AnalysisFailureEvent, AnalysisRootCause};
use crate::artifact_conflict_finder;
use crate::build_view::AnalysisPhaseCompleteStats;
use crate::skyframe_error_processor::{self, ViewCreationFailedException};

use crate::skyframe_executor::{SkyframeExecutor, TargetAndConfiguration};

/// Encapsulates the raw analysis result of top level targets and aspects coming from Skyframe.
// Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:30 SkyframeAnalysisResult
pub struct SkyframeAnalysisResult {
    has_loading_error: bool,
    has_analysis_error: bool,
    has_action_conflicts: bool,
    configured_targets: Vec<ConfiguredTarget>,
    /// See [`SkyframeAnalysisResult::get_configured_targets_with_action_conflicts`].
    configured_targets_with_action_conflicts: Vec<ConfiguredTarget>,
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

    /// The top-level targets of a `--keep_going` build that were analyzed but are not among
    /// [`SkyframeAnalysisResult::get_configured_targets`] because an action in their transitive
    /// closure is in conflict, in top-level order.
    ///
    /// PORT: not in Java's class. With Skymeld these are the targets whose `BuildDriverFunction`
    /// announced them as analyzed (`TopLevelTargetAnalyzedEvent`: they count in
    /// `BuildResultListener.getAnalyzedTargets()`, "Found N targets...") and then threw a
    /// `TopLevelConflictException`. Blaze finds conflicts after the analysis of all top-level
    /// targets, so the result carries them to the code that stands in for the build drivers.
    // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:187 BuildDriverFunction.compute
    pub fn get_configured_targets_with_action_conflicts(&self) -> &[ConfiguredTarget] {
        &self.configured_targets_with_action_conflicts
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:83 SkyframeAnalysisResult.getWalkableGraph
    pub fn get_walkable_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.walkable_graph
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:95 SkyframeAnalysisResult.getTargetsWithConfiguration
    pub fn get_targets_with_configuration(&self) -> &[TargetAndConfiguration] {
        &self.targets_with_configuration
    }

    /// Returns an equivalent `SkyframeAnalysisResult`, except with errored targets removed from
    /// the configured target list.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:107 SkyframeAnalysisResult.withAdditionalErroredTargets
    pub fn with_additional_errored_targets(
        self,
        errored_targets: &[ConfiguredTarget],
    ) -> SkyframeAnalysisResult {
        // Sets.difference(configuredTargets, erroredTargets)
        let configured_targets = self
            .configured_targets
            .into_iter()
            .filter(|ct| {
                !errored_targets
                    .iter()
                    .any(|errored| same_configured_target(errored, ct))
            })
            .collect();
        SkyframeAnalysisResult {
            has_loading_error: self.has_loading_error,
            has_analysis_error: true,
            has_action_conflicts: self.has_action_conflicts,
            configured_targets,
            configured_targets_with_action_conflicts: self.configured_targets_with_action_conflicts,
            walkable_graph: self.walkable_graph,
            targets_with_configuration: self.targets_with_configuration,
            build_graph_metrics: self.build_graph_metrics,
        }
    }
}

/// `ConfiguredTarget.equals` as the sets of configured targets here need it: the same lookup
/// key (one configured target per key in a build).
fn same_configured_target(a: &ConfiguredTarget, b: &ConfiguredTarget) -> bool {
    a.get_lookup_key().0.key_equals(&*b.get_lookup_key().0)
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

impl From<ViewCreationFailedException> for ConfigureTargetsError {
    fn from(e: ViewCreationFailedException) -> Self {
        ConfigureTargetsError::ViewCreationFailed {
            message: e.get_message().to_owned(),
            failure_detail: e.get_failure_detail().clone(),
        }
    }
}

/// Analyzes the specified targets using Skyframe as the driving framework.
///
/// Returns the configured targets that should be built along with a WalkableGraph of the
/// analysis.
///
/// PORT: `topLevelArtifactContextForConflictPruning` is read from the executor
/// (`SkyframeExecutor::get_top_level_artifact_context_for_conflict_pruning`, set by
/// `AnalysisPhaseRunner` from the request) because `BuildView.update`'s options do not carry it
/// yet. `eventBus` is the event handler (`post`); `bugReporter` and `executors` have no port.
/// Top-level aspect keys are not ported (`BuildView.createTopLevelAspectKeys` fails loudly).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:400 SkyframeBuildView.configureTargets
pub fn configure_targets(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    label_to_target_map: &IndexMap<Label, Target>,
    ct_keys: &[ConfiguredTargetKey],
    keep_going: bool,
    check_for_action_conflicts: bool,
) -> Result<SkyframeAnalysisResult, ConfigureTargetsError> {
    // PORT: skipped, incremental: enableAnalysis(true/false) guards the reuse of analysis nodes
    // between builds.
    let result = skyframe_executor.configure_targets(
        event_handler.clone(),
        label_to_target_map,
        ct_keys,
        keep_going,
    )?;

    let mut cts = result.configured_targets;
    let evaluation_result: EvaluationResult = result.evaluation_result;

    let mut inter_target_conflicts = ActionConflicts::new();
    let mut build_graph_metrics = None;
    // PORT: `newKeys` (the top-level keys of this build) only feeds the incremental conditions of
    // shouldCheckForConflicts.
    if should_check_for_conflicts(
        check_for_action_conflicts,
        skyframe_executor.get_evaluator(),
    ) {
        // This operation is somewhat expensive, so we only do it if the graph might have changed
        // in some way -- either we analyzed a new target or we invalidated an old one or are
        // building targets together that haven't been built before.
        let analysis_traversal_result = collect_action_lookup_values_in_build(
            skyframe_executor.get_walkable_graph().as_ref(),
            ct_keys,
        );
        let action_key_context = skyframe_executor
            .get_action_key_context()
            .expect("IllegalStateException: the command does not configure targets");
        let conflicts_and_stats = artifact_conflict_finder::find_and_store_artifact_conflicts(
            &analysis_traversal_result.action_lookup_values,
            analysis_traversal_result.action_count,
            action_key_context,
        )?;
        let mut metrics = analysis_traversal_result.get_metrics();
        metrics.output_artifact_count = conflicts_and_stats.output_artifact_count();
        // PORT: Java posts `new AnalysisGraphStatsEvent(buildGraphMetrics)` on the event bus;
        // the metrics travel in the result to their one subscriber (the metrics collector).
        build_graph_metrics = Some(metrics);
        inter_target_conflicts = conflicts_and_stats.conflicts().clone();
    }

    // Intra-target conflict would mean an error in evaluationResult.
    if !evaluation_result.has_error() && inter_target_conflicts.is_empty() {
        return Ok(SkyframeAnalysisResult {
            has_loading_error: false,
            has_analysis_error: false,
            has_action_conflicts: false,
            configured_targets: cts,
            configured_targets_with_action_conflicts: Vec::new(),
            walkable_graph: skyframe_executor.get_walkable_graph(),
            targets_with_configuration: result.targets_with_configuration,
            build_graph_metrics,
        });
    }

    let error_processing_result = skyframe_error_processor::process_analysis_errors(
        &evaluation_result,
        skyframe_executor.get_cycles_reporter(),
        &event_handler,
        keep_going,
    )?;

    // ImmutableMap.builder().putAll(interTargetConflicts)
    //     .putAll(errorProcessingResult.actionConflicts()) // Intra-target conflicts.
    //     .buildOrThrow()
    let mut action_conflicts = ActionConflicts::new();
    action_conflicts.put_all(&inter_target_conflicts);
    for (action, exception) in error_processing_result.action_conflicts().entries() {
        if action_conflicts.contains_key(action) {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                action.pretty_print()
            );
        }
        action_conflicts.put(action, exception.clone());
    }
    let found_action_conflict_in_latest_check = !action_conflicts.is_empty();
    let mut cts_with_action_conflicts: Vec<ConfiguredTarget> = Vec::new();
    let mut no_keep_going_exception_due_to_conflict: Option<ViewCreationFailedException> = None;
    // Sometimes there are action conflicts, but the actions aren't actually required to run by
    // the build. In such cases, the conflict should still be reported to the user.
    // See OutputArtifactConflictTest#unusedActionsStillConflict.
    let mut reported_action_conflict_exceptions: HashSet<String> = HashSet::new();
    for (bad_action, ace) in action_conflicts.entries() {
        let failure_detail = ace.get_failure_detail();
        if reported_action_conflict_exceptions.insert(ace.get_message().to_owned()) {
            ace.report_to(event_handler.as_ref());
            if keep_going {
                event_handler.handle(&Event::warn(format!(
                    "errors encountered while analyzing target '{}': it will not be built",
                    // String concatenation of a possibly null label.
                    bad_action
                        .get_owner()
                        .get_label()
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "null".to_owned())
                )));
            }
        }
        if !keep_going {
            no_keep_going_exception_due_to_conflict = Some(ViewCreationFailedException::of_cause(
                failure_detail,
                ace.get_message(),
            ));
        }
    }

    if found_action_conflict_in_latest_check {
        // In order to determine the set of configured targets transitively error free from
        // action conflict issues, we run a post-processing update() that uses the bad action map.
        let top_level_artifact_context_for_conflict_pruning = skyframe_executor
            .get_top_level_artifact_context_for_conflict_pruning()
            .unwrap_or_else(|| {
                panic!("NullPointerException: topLevelArtifactContextForConflictPruning")
            });
        let top_level_action_conflict_report = skyframe_executor
            .filter_action_conflicts_for_configured_targets_and_aspects(
                event_handler.clone(),
                ct_keys,
                &action_conflicts,
                &top_level_artifact_context_for_conflict_pruning,
            )?;
        // Report an AnalysisFailureEvent to BEP for the top-level targets with discoverable
        // action conflicts, then finally throw if evaluation is --nokeep_going.
        for action_lookup_key in ct_keys {
            if !top_level_action_conflict_report.is_error_free(action_lookup_key) {
                let Some(conflict_exception) =
                    top_level_action_conflict_report.get_conflict_exception(action_lookup_key)
                else {
                    continue;
                };
                let failed_cause =
                    make_artifact_conflict_analysis_failed_cause(&conflict_exception);
                let failed_cause_failure_detail = conflict_exception.get_failure_detail();
                let mut target_configured = true;
                let mut action_lookup_key = action_lookup_key.clone();
                // Attempt to promote any ConfiguredTargetKey to the one embedded in the
                // ConfiguredTarget to reflect any transitions or trimming.
                match evaluation_result.get(&action_lookup_key) {
                    None => target_configured = false,
                    Some(value) => {
                        // PORT: skipped, incremental: "It's possible that the ConfiguredTarget
                        // has been cleared" (analysis objects are not discarded here).
                        action_lookup_key = value
                            .get_configured_target()
                            .get_lookup_key()
                            .as_any()
                            .downcast_ref::<ConfiguredTargetKey>()
                            .unwrap_or_else(|| {
                                panic!("ClassCastException: lookup key of a configured target")
                            })
                            .clone();
                    }
                }
                if !target_configured {
                    event_handler.post(Arc::new(AnalysisFailureEvent::while_analyzing_target(
                        action_lookup_key,
                        vec![failed_cause],
                    )));
                } else {
                    event_handler.post(Arc::new(AnalysisFailureEvent::action_conflict(
                        action_lookup_key,
                        vec![failed_cause],
                    )));
                }

                if !keep_going {
                    no_keep_going_exception_due_to_conflict =
                        Some(ViewCreationFailedException::of_cause(
                            failed_cause_failure_detail,
                            conflict_exception.get_message(),
                        ));
                }
            }
        }

        // If we're here and we're --nokeep_going, then there was a conflict due to actions not
        // discoverable by TopLevelActionLookupConflictFindingFunction. This includes extra
        // actions, coverage artifacts, and artifacts produced by aspects in output groups not
        // present in --output_groups. Throw the exception produced by the ArtifactConflictFinder
        // which cannot identify root-cause top-level keys but does catch all possible conflicts.
        if !keep_going {
            // PORT: skipped, incremental: resetActionConflictsStoredInSkyframe frees the
            // conflict-finding nodes.
            return Err(no_keep_going_exception_due_to_conflict
                .unwrap_or_else(|| panic!("NullPointerException"))
                .into());
        }

        // Filter cts and aspects to only error-free keys. Note that any analysis failure - not
        // just action conflicts - will be observed here and lead to a key's exclusion.
        // ImmutableSet: configured targets are deduplicated.
        cts = Vec::new();
        for k in ct_keys
            .iter()
            .filter(|k| top_level_action_conflict_report.is_error_free(k))
        {
            let configured_target = evaluation_result
                .get(k)
                .unwrap_or_else(|| panic!("NullPointerException: {:?}", k))
                .get_configured_target()
                .clone();
            if !cts
                .iter()
                .any(|ct| same_configured_target(ct, &configured_target))
            {
                cts.push(configured_target);
            }
        }

        // PORT: the top-level targets that were analyzed but are not error-free. With Skymeld,
        // Java's BuildDriverFunction announces such a target as analyzed and then throws a
        // TopLevelConflictException for it (`checkActionConflicts`), before it requests any
        // execution. Blaze's conflict check is this one, so the result carries the targets
        // (`SkyframeAnalysisResult::get_configured_targets_with_action_conflicts`).
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:198 BuildDriverFunction.compute
        for k in ct_keys
            .iter()
            .filter(|k| !top_level_action_conflict_report.is_error_free(k))
        {
            let Some(value) = evaluation_result.get(k) else {
                // The analysis of the target failed (also for a conflict inside the target).
                continue;
            };
            let configured_target = value.get_configured_target().clone();
            if !cts_with_action_conflicts
                .iter()
                .any(|ct| same_configured_target(ct, &configured_target))
            {
                cts_with_action_conflicts.push(configured_target);
            }
        }
    }

    Ok(SkyframeAnalysisResult {
        has_loading_error: error_processing_result.has_loading_error(),
        has_analysis_error: evaluation_result.has_error() || found_action_conflict_in_latest_check,
        has_action_conflicts: found_action_conflict_in_latest_check,
        configured_targets: cts,
        configured_targets_with_action_conflicts: cts_with_action_conflicts,
        walkable_graph: skyframe_executor.get_walkable_graph(),
        targets_with_configuration: result.targets_with_configuration,
        build_graph_metrics,
    })
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1241 SkyframeBuildView.makeArtifactConflictAnalysisFailedCause
fn make_artifact_conflict_analysis_failed_cause(
    ace: &ActionConflictException,
) -> AnalysisRootCause {
    let failure_detail = ace.get_failure_detail();
    let artifact_owner = ace.get_artifact().get_artifact_owner();
    let cause_label = artifact_owner.get_label().unwrap_or_else(|| {
        panic!(
            "NullPointerException: owner of {} has no label",
            ace.get_artifact()
        )
    });
    let mut cause_config_key = None;
    if let ArtifactOwner::ActionLookupKey(key) = &artifact_owner
        && let Some(ct_key) = key.as_any().downcast_ref::<ConfiguredTargetKey>()
    {
        cause_config_key = ct_key.get_configuration_key().cloned();
    }
    AnalysisRootCause::AnalysisFailed {
        label: cause_label,
        configuration: configuration_id_of_key(cause_config_key.as_ref())
            .unwrap_or_else(|| "none".to_owned()),
        failure_detail,
    }
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
/// PORT: Java forks one task per key on a pool, so the order in which values are accumulated
/// (and with it which of two conflicting actions `ArtifactConflictFinder` registers first)
/// depends on thread timing. The port visits the keys one after the other: the roots in their
/// order, each key before its dependencies, the dependencies of a key in their order (a key is
/// marked seen when its parent finds it, as in Java). The counters do not depend on the order.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:3683 SkyframeExecutor.collectActionLookupValuesInBuild
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4274 TransitiveActionLookupKeysCollector
fn collect_action_lookup_values_in_build(
    walkable_graph: &dyn WalkableGraph,
    top_level_ct_keys: &[ConfiguredTargetKey],
) -> ActionLookupValuesTraversal {
    let mut alv_traversal = ActionLookupValuesTraversal::default();
    let mut seen: IndexSet<SkyKey> = IndexSet::new();
    // The pending subtasks of each key being visited (`invokeAll(subtasks)`), innermost last.
    let mut stack: Vec<std::vec::IntoIter<SkyKey>> = Vec::new();
    let mut roots: Vec<SkyKey> = Vec::new();
    for key in top_level_ct_keys {
        let key = SkyKey::new(key.clone());
        if seen.insert(key.clone()) {
            roots.push(key);
        }
    }
    stack.push(roots.into_iter());
    while let Some(frame) = stack.last_mut() {
        let Some(key) = frame.next() else {
            stack.pop();
            continue;
        };
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

        let mut subtasks: Vec<SkyKey> = Vec::new();
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
                    subtasks.push(dep);
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
        stack.push(subtasks.into_iter());
    }
    alv_traversal
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

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/buildtool/OutputArtifactConflictTest.java
    // PORT: these are BuildIntegrationTestCase tests (they build scratch workspaces with a real
    // BlazeRuntime); the harness is not ported. The cases are compared with the oracle end to end
    // by the probe scenario scripts/w4/scenarios/top-conflicts.

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (steps 14 to 20)"]
    fn test_artifact_prefix() {
        unimplemented!("not ported: OutputArtifactConflictTest.testArtifactPrefix")
    }

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (steps 3 and 29)"]
    fn test_conflict_error_and_analysis_error() {
        unimplemented!("not ported: OutputArtifactConflictTest.testConflictErrorAndAnalysisError")
    }

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (steps 1 to 7)"]
    fn unused_actions_still_conflict() {
        unimplemented!("not ported: OutputArtifactConflictTest.unusedActionsStillConflict")
    }

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (steps 1 to 7)"]
    fn test_multiple_conflict_errors() {
        unimplemented!("not ported: OutputArtifactConflictTest.testMultipleConflictErrors")
    }

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (step 16)"]
    fn test_two_conflicting_targets_keep_going_behavior_differences() {
        unimplemented!(
            "not ported: OutputArtifactConflictTest.testTwoConflictingTargets_keepGoing_behaviorDifferences"
        )
    }

    #[test]
    #[ignore = "needs BuildIntegrationTestCase; see scripts/w4/scenarios/top-conflicts (steps 23 to 26)"]
    fn dependency_has_conflict_keep_going_both_top_level_targets_fail() {
        unimplemented!(
            "not ported: OutputArtifactConflictTest.dependencyHasConflict_keepGoing_bothTopLevelTargetsFail"
        )
    }
}
