//! Port of the analysis-only part of `skyframe/SkyframeBuildView.java` (`configureTargets`) and
//! `skyframe/SkyframeAnalysisResult.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_actions::actions::action_lookup::ArtifactOwner;
use blaze_analysis::aspect_key_creator::{
    ASPECT, AspectKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey, TopLevelAspectsValue,
};
use blaze_analysis::aspect_value::AspectValue;
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
use blaze_sky_exec::keys::BuildDriverKey;
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_skyframe::{
    EvaluationResult, Evaluator, Interrupted, Key, SkyKey, SkyValue, WalkableGraph,
};
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
    /// `aspects`: the top-level aspects.
    aspects: IndexMap<AspectKey, Arc<AspectValue>>,
    /// See [`SkyframeAnalysisResult::get_configured_targets_with_action_conflicts`].
    configured_targets_with_action_conflicts: Vec<ConfiguredTarget>,
    walkable_graph: Arc<dyn WalkableGraph>,
    targets_with_configuration: Vec<TargetAndConfiguration>,
    /// The `AnalysisGraphStatsEvent` posted by the conflict check (`None` when not checked).
    build_graph_metrics: Option<BuildGraphMetrics>,
    /// `SkyframeAnalysisAndExecutionResult.getRepresentativeExecutionExitCode()`: the most important
    /// execution exit code of a `--keep_going` build with merged analysis and execution.
    execution_detailed_exit_code: Option<blaze_util::DetailedExitCode>,
    /// The counters of the `AnalysisPhaseCompleteEvent` that
    /// `analysisFinishedCallback` posted (merged analysis and execution; `None` when the
    /// analysis of the top-level keys did not conclude).
    analysis_phase_complete_stats: Option<AnalysisPhaseCompleteStats>,
}

impl SkyframeAnalysisResult {
    /// The `BuildGraphMetrics` of the `AnalysisGraphStatsEvent` `configureTargets` posted, if it
    /// checked for action conflicts.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:454 SkyframeBuildView.configureTargets
    pub fn get_build_graph_metrics(&self) -> Option<&BuildGraphMetrics> {
        self.build_graph_metrics.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisAndExecutionResult.java:55 SkyframeAnalysisAndExecutionResult.getRepresentativeExecutionExitCode
    pub fn get_execution_detailed_exit_code(&self) -> Option<&blaze_util::DetailedExitCode> {
        self.execution_detailed_exit_code.as_ref()
    }

    /// See the field.
    pub fn get_analysis_phase_complete_stats(&self) -> Option<&AnalysisPhaseCompleteStats> {
        self.analysis_phase_complete_stats.as_ref()
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

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:91 SkyframeAnalysisResult.getAspects
    pub fn get_aspects(&self) -> &IndexMap<AspectKey, Arc<AspectValue>> {
        &self.aspects
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
            aspects: self.aspects,
            configured_targets_with_action_conflicts: self.configured_targets_with_action_conflicts,
            walkable_graph: self.walkable_graph,
            targets_with_configuration: self.targets_with_configuration,
            build_graph_metrics: self.build_graph_metrics,
            execution_detailed_exit_code: self.execution_detailed_exit_code,
            analysis_phase_complete_stats: self.analysis_phase_complete_stats,
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
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:400 SkyframeBuildView.configureTargets
pub fn configure_targets(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    label_to_target_map: &IndexMap<Label, Target>,
    ct_keys: &[ConfiguredTargetKey],
    top_level_aspects_keys: &[TopLevelAspectsKey],
    keep_going: bool,
    check_for_action_conflicts: bool,
) -> Result<SkyframeAnalysisResult, ConfigureTargetsError> {
    // PORT: skipped, incremental: enableAnalysis(true/false) guards the reuse of analysis nodes
    // between builds.
    let result = skyframe_executor.configure_targets(
        event_handler.clone(),
        label_to_target_map,
        ct_keys,
        top_level_aspects_keys,
        keep_going,
    )?;

    let mut cts = result.configured_targets;
    let mut aspects: IndexMap<AspectKey, Arc<AspectValue>> = result.aspects;
    let aspect_keys: Vec<AspectKey> = aspects.keys().cloned().collect();
    let evaluation_result: EvaluationResult = result.evaluation_result;

    // Iterables.concat(ctKeys, aspectKeys)
    let ct_and_aspect_keys: Vec<ActionLookupKeyRef> = ct_keys
        .iter()
        .map(|k| ActionLookupKeyRef::new(k.clone()))
        .chain(
            aspect_keys
                .iter()
                .map(|k| ActionLookupKeyRef::new(k.clone())),
        )
        .collect();

    let mut inter_target_conflicts = ActionConflicts::new();
    let mut build_graph_metrics = None;
    // newKeys = ImmutableSet.<ActionLookupKey>builder().addAll(ctKeys).addAll(aspectKeys)
    let new_keys_count = ct_and_aspect_keys.iter().collect::<IndexSet<_>>().len();
    if should_check_for_conflicts(
        check_for_action_conflicts,
        new_keys_count,
        skyframe_executor.get_evaluator(),
    ) {
        // This operation is somewhat expensive, so we only do it if the graph might have changed
        // in some way -- either we analyzed a new target or we invalidated an old one or are
        // building targets together that haven't been built before.
        let analysis_traversal_result = collect_action_lookup_values_in_build(
            skyframe_executor.get_walkable_graph().as_ref(),
            ct_keys,
            &aspect_keys,
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
            aspects,
            configured_targets_with_action_conflicts: Vec::new(),
            walkable_graph: skyframe_executor.get_walkable_graph(),
            targets_with_configuration: result.targets_with_configuration,
            build_graph_metrics,
            execution_detailed_exit_code: None,
            analysis_phase_complete_stats: None,
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
                &ct_and_aspect_keys,
                &action_conflicts,
                &top_level_artifact_context_for_conflict_pruning,
            )?;
        // Report an AnalysisFailureEvent to BEP for the top-level targets with discoverable
        // action conflicts, then finally throw if evaluation is --nokeep_going.
        for action_lookup_key in &ct_and_aspect_keys {
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
                if let Some(ct_key) = action_lookup_key
                    .as_any()
                    .downcast_ref::<ConfiguredTargetKey>()
                {
                    match evaluation_result.get(ct_key) {
                        None => target_configured = false,
                        Some(value) => {
                            // PORT: skipped, incremental: "It's possible that the
                            // ConfiguredTarget has been cleared" (analysis objects are not
                            // discarded here).
                            action_lookup_key =
                                value.get_configured_target().get_lookup_key().clone();
                        }
                    }
                }
                if !target_configured {
                    let ct_key = action_lookup_key
                        .as_any()
                        .downcast_ref::<ConfiguredTargetKey>()
                        .unwrap_or_else(|| {
                            panic!("ClassCastException: {action_lookup_key} is not a ConfiguredTargetKey")
                        })
                        .clone();
                    event_handler.post(Arc::new(AnalysisFailureEvent::while_analyzing_target(
                        ct_key,
                        vec![failed_cause],
                    )));
                } else {
                    event_handler.post(Arc::new(AnalysisFailureEvent::action_conflict(
                        &action_lookup_key,
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
            skyframe_executor.reset_action_conflicts_stored_in_skyframe();
            return Err(no_keep_going_exception_due_to_conflict
                .unwrap_or_else(|| panic!("NullPointerException"))
                .into());
        }

        // Filter cts and aspects to only error-free keys. Note that any analysis failure - not
        // just action conflicts - will be observed here and lead to a key's exclusion.
        // ImmutableSet: configured targets are deduplicated.
        cts = Vec::new();
        for k in ct_keys.iter().filter(|k| {
            top_level_action_conflict_report.is_error_free(&ActionLookupKeyRef::new((*k).clone()))
        }) {
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

        aspects.retain(|k, _| {
            top_level_action_conflict_report.is_error_free(&ActionLookupKeyRef::new(k.clone()))
        });

        // PORT: the top-level targets that were analyzed but are not error-free. With Skymeld,
        // Java's BuildDriverFunction announces such a target as analyzed and then throws a
        // TopLevelConflictException for it (`checkActionConflicts`), before it requests any
        // execution. Blaze's conflict check is this one, so the result carries the targets
        // (`SkyframeAnalysisResult::get_configured_targets_with_action_conflicts`).
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:198 BuildDriverFunction.compute
        for k in ct_keys.iter().filter(|k| {
            !top_level_action_conflict_report.is_error_free(&ActionLookupKeyRef::new((*k).clone()))
        }) {
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
        // PORT: Java resets the nodes in `filterActionConflictsForTopLevelArtifacts` (the
        // execution phase of a keep-going build), which is not ported; they are reset once this
        // post-processing is done.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2675 SkyframeExecutor.filterActionConflictsForTopLevelArtifacts
        skyframe_executor.reset_action_conflicts_stored_in_skyframe();
    }

    Ok(SkyframeAnalysisResult {
        has_loading_error: error_processing_result.has_loading_error(),
        has_analysis_error: evaluation_result.has_error() || found_action_conflict_in_latest_check,
        has_action_conflicts: found_action_conflict_in_latest_check,
        configured_targets: cts,
        aspects,
        configured_targets_with_action_conflicts: cts_with_action_conflicts,
        walkable_graph: skyframe_executor.get_walkable_graph(),
        targets_with_configuration: result.targets_with_configuration,
        build_graph_metrics,
        execution_detailed_exit_code: None,
        analysis_phase_complete_stats: None,
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

/// `shouldCheckForConflicts`: a build request checks unless it opted out (cquery).
///
/// PORT: Java's `someActionLookupValueEvaluated` is set by the progress receiver; the command's
/// evaluator is fresh, so it is true iff the graph holds a rule configured target.
/// `dirtiedActionLookupKeys` and `foundActionConflictInLatestCheck` are empty / false in a fresh
/// process, and `largestTopLevelKeySetCheckedForConflicts` is the empty set, so its
/// `containsAll(newKeys)` is false for any non-empty `newKeys` (analysis review R2-7: e.g.
/// `build //pkg:a.txt` of a source file still runs the traversal and reports graph metrics).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1254 SkyframeBuildView.shouldCheckForConflicts
fn should_check_for_conflicts(
    specified_value_in_request: bool,
    new_keys_count: usize,
    evaluator: &Evaluator,
) -> bool {
    if !specified_value_in_request {
        // A build request by default enables action conflict checking, except for some cases e.g.
        // cquery.
        return false;
    }

    // A top-level target was added and may introduce a conflict, or a top-level target was
    // recomputed and may introduce or resolve a conflict.
    let some_action_lookup_value_evaluated =
        evaluator.get_all_keys_for_testing().iter().any(|key| {
            if key.function_name() == ASPECT {
                // An AspectValue is an ActionLookupValue, except for an alias target's.
                return evaluator
                    .get_existing_value(key)
                    .and_then(|v| v.downcast::<AspectValue>().ok())
                    .is_some_and(|v| !AspectValue::is_for_alias_target(&v));
            }
            if key.function_name() == TOP_LEVEL_ASPECTS {
                return evaluator.get_existing_value(key).is_some();
            }
            key.downcast::<ConfiguredTargetKey>().is_some()
                && evaluator
                    .get_existing_value(key)
                    .and_then(|v| v.downcast::<ConfiguredTargetValue>().ok())
                    .is_some_and(|v| matches!(v.get_configured_target(), ConfiguredTarget::Rule(_)))
        });
    if some_action_lookup_value_evaluated {
        return true;
    }

    // Java: !largestTopLevelKeySetCheckedForConflicts.containsAll(newKeys), with the empty set
    // of a fresh process.
    if new_keys_count > 0 {
        return true;
    }

    false
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
    /// `accumulate` for a `ConfiguredTargetValue`: a rule's value is an `ActionLookupValue`
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

    /// `accumulate` for an `ActionLookupValue` that is not a `ConfiguredTargetValue` (an
    /// `AspectValue`, a `TopLevelAspectsValue`, the workspace status value).
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:48 ActionLookupValuesTraversal.accumulate
    fn accumulate_action_lookup_value(&mut self, actions: Vec<blaze_actions::ActionRef>) {
        self.configured_object_count += 1;
        self.action_count += actions.len() as i32;
        self.action_lookup_values.push(actions);
    }

    /// `accumulate(key, value)` for any `ActionLookupKey` of the graph.
    // Java: com/google/devtools/build/lib/skyframe/ActionLookupValuesTraversal.java:48 ActionLookupValuesTraversal.accumulate
    fn accumulate_key(&mut self, key: &SkyKey, value: SkyValue) {
        if let Some(ct_key) = key.downcast::<ConfiguredTargetKey>() {
            let value = value
                .downcast::<ConfiguredTargetValue>()
                .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key));
            self.accumulate(ct_key, value);
        } else if key.function_name() == ASPECT {
            let value = value
                .downcast::<AspectValue>()
                .unwrap_or_else(|_| panic!("ASPECT value of {:?}", key));
            self.accumulate_action_lookup_value(value.get_actions().to_vec());
        } else if key.function_name() == TOP_LEVEL_ASPECTS {
            let value = value
                .downcast::<TopLevelAspectsValue>()
                .unwrap_or_else(|_| panic!("TOP_LEVEL_ASPECTS value of {:?}", key));
            self.accumulate_action_lookup_value(value.get_actions().to_vec());
        } else if key.function_name()
            == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
        {
            let value = value
                .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
                .unwrap_or_else(|_| panic!("BUILD_INFO value"));
            self.accumulate_action_lookup_value(value.actions.clone());
        } else {
            panic!("IllegalStateException: not an ActionLookupKey: {:?}", key);
        }
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

/// Java `dep instanceof ActionLookupKey` for the keys of Blaze's analysis graph:
/// `ConfiguredTargetKey`, `AspectKey`, `TopLevelAspectsKey` and the workspace status key.
pub(crate) fn is_action_lookup_key(key: &SkyKey) -> bool {
    let name = key.function_name();
    name == <ConfiguredTargetKey as Key>::FUNCTION
        || name == ASPECT
        || name == TOP_LEVEL_ASPECTS
        || name == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
}

/// The actions of the value of an `ActionLookupKey` when the value is an `ActionLookupValue`
/// (`value instanceof ActionLookupValue`): a rule's configured target, an aspect, the top-level
/// aspects, the workspace status. `None` for a `NonRuleConfiguredTargetValue`.
pub(crate) fn action_lookup_value_actions(
    key: &SkyKey,
    value: &SkyValue,
) -> Option<Vec<blaze_actions::ActionRef>> {
    if key.downcast::<ConfiguredTargetKey>().is_some() {
        let value = value
            .clone()
            .downcast::<ConfiguredTargetValue>()
            .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key));
        return match value.get_configured_target() {
            ConfiguredTarget::Rule(_) => Some(value.get_actions().to_vec()),
            _ => None,
        };
    }
    if key.function_name() == ASPECT {
        let value = value
            .clone()
            .downcast::<AspectValue>()
            .unwrap_or_else(|_| panic!("ASPECT value of {:?}", key));
        return Some(value.get_actions().to_vec());
    }
    if key.function_name() == TOP_LEVEL_ASPECTS {
        let value = value
            .clone()
            .downcast::<TopLevelAspectsValue>()
            .unwrap_or_else(|_| panic!("TOP_LEVEL_ASPECTS value of {:?}", key));
        return Some(value.get_actions().to_vec());
    }
    if key.function_name()
        == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
    {
        let value = value
            .clone()
            .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
            .unwrap_or_else(|_| panic!("BUILD_INFO value"));
        return Some(value.actions.clone());
    }
    None
}

/// `SkyframeExecutor.collectActionLookupValuesInBuild` with incrementality tracked: the
/// top-down `TransitiveActionLookupKeysCollector` over the analysis graph, from the top-level
/// configured targets and then the top-level aspects.
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
    aspect_keys: &[AspectKey],
) -> ActionLookupValuesTraversal {
    let mut alv_traversal = ActionLookupValuesTraversal::default();
    let mut seen: IndexSet<SkyKey> = IndexSet::new();
    // The pending subtasks of each key being visited (`invokeAll(subtasks)`), innermost last.
    let mut stack: Vec<std::vec::IntoIter<SkyKey>> = Vec::new();
    let mut roots: Vec<SkyKey> = Vec::new();
    // Iterables.concat(topLevelCtKeys, aspectKeys)
    for key in top_level_ct_keys
        .iter()
        .map(|k| SkyKey::new(k.clone()))
        .chain(aspect_keys.iter().map(|k| SkyKey::new(k.clone())))
    {
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
        alv_traversal.accumulate_key(&key, value);

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
            if !is_action_lookup_key(&dep) {
                continue;
            }
            if seen.insert(dep.clone()) {
                subtasks.push(dep);
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
/// consumer, `MetricsCollector`, orders its own map).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1481 ActionLookupValueProgressReceiver.evaluated
pub fn get_evaluated_counts(evaluator: &Evaluator, time_in_ms: i64) -> AnalysisPhaseCompleteStats {
    let mut counts = EvaluatedCounts::default();
    for key in evaluator.get_all_keys_for_testing() {
        if let Some(value) = evaluator.get_existing_value(&key) {
            counts.accumulate(&key, &value);
        }
    }
    counts.to_stats(time_in_ms)
}

/// The counters of `ActionLookupValueProgressReceiver` and `PackageFunction`'s
/// `numPackagesSuccessfullyLoaded`, filled from evaluated nodes: by a sweep over a graph built
/// by one command ([`get_evaluated_counts`]) or, over a graph kept across commands, by the
/// executor's progress receiver as nodes are evaluated
/// (`crate::skyframe_progress_receiver::SkyframeProgressReceiver`).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1461 SkyframeBuildView.ActionLookupValueProgressReceiver
#[derive(Default, Clone)]
pub struct EvaluatedCounts {
    configured_object_count: i32,
    configured_target_count: i32,
    action_count: i32,
    configured_target_action_count: i32,
    action_count_by_mnemonic: IndexMap<String, i32>,
    packages_successfully_loaded: i32,
}

impl EvaluatedCounts {
    /// Counts one node evaluated with a new value.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1481 ActionLookupValueProgressReceiver.evaluated
    pub fn accumulate(&mut self, key: &SkyKey, value: &SkyValue) {
        let value = value.clone();
        let action_count_by_mnemonic = &mut self.action_count_by_mnemonic;
        if key.downcast::<PackageKey>().is_some() {
            // Java: com/google/devtools/build/lib/skyframe/PackageFunction.java:624 PackageFunction.compute
            if let Ok(value) = value.downcast::<PackageValue>()
                && !value.get_package().contains_errors()
            {
                self.packages_successfully_loaded += 1;
            }
            return;
        }
        if key.function_name()
            == <blaze_analysis::analysis_services::WorkspaceStatusKey as Key>::FUNCTION
        {
            // The workspace status value: an ActionLookupValue that is not a configured target.
            let value = value
                .downcast::<blaze_analysis::analysis_services::WorkspaceStatusValue>()
                .unwrap_or_else(|_| panic!("BUILD_INFO value"));
            self.configured_object_count += 1;
            for action in &value.actions {
                *action_count_by_mnemonic
                    .entry(action.get_mnemonic())
                    .or_insert(0) += 1;
            }
            self.action_count += value.actions.len() as i32;
            return;
        }
        if key.function_name() == ASPECT || key.function_name() == TOP_LEVEL_ASPECTS {
            self.configured_object_count += 1;
            let actions: Vec<blaze_actions::ActionRef> = if key.function_name() == ASPECT {
                let value = value
                    .downcast::<AspectValue>()
                    .unwrap_or_else(|_| panic!("ASPECT value of {:?}", key));
                if AspectValue::is_for_alias_target(&value) {
                    // Created actions will be counted from {@link AspectValue} on the original
                    // target.
                    return;
                }
                value.get_actions().to_vec()
            } else {
                value
                    .downcast::<TopLevelAspectsValue>()
                    .unwrap_or_else(|_| panic!("TOP_LEVEL_ASPECTS value of {:?}", key))
                    .get_actions()
                    .to_vec()
            };
            for action in &actions {
                *action_count_by_mnemonic
                    .entry(action.get_mnemonic())
                    .or_insert(0) += 1;
            }
            self.action_count += actions.len() as i32;
            return;
        }
        let Some(ct_key) = key.downcast::<ConfiguredTargetKey>() else {
            return;
        };
        let value = value
            .downcast::<ConfiguredTargetValue>()
            .unwrap_or_else(|_| panic!("CONFIGURED_TARGET value of {:?}", key));
        if ct_key.get_configuration_key().cloned()
            != value.get_configured_target().get_configuration_key()
        {
            // The node entry performs delegation and doesn't own the value. Skips it to avoid
            // overcounting.
            return;
        }
        self.configured_target_count += 1;
        self.configured_object_count += 1;
        if matches!(value.get_configured_target(), ConfiguredTarget::Rule(_)) {
            let actions = value.get_actions();
            for action in actions {
                *action_count_by_mnemonic
                    .entry(action.get_mnemonic())
                    .or_insert(0) += 1;
            }
            let num_actions = actions.len() as i32;
            self.action_count += num_actions;
            self.configured_target_action_count += num_actions;
        }
    }

    /// `ActionLookupValueProgressReceiver.reset`: the action lookup counts (the packages are
    /// `PackageFunction`'s counter, reset per command with the executor).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1540 ActionLookupValueProgressReceiver.reset
    pub fn reset(&mut self) {
        self.configured_object_count = 0;
        self.action_count = 0;
        self.configured_target_count = 0;
        self.configured_target_action_count = 0;
    }

    /// The counts as `AnalysisPhaseRunner` hands them to `AnalysisPhaseCompleteEvent`.
    pub fn to_stats(&self, time_in_ms: i64) -> AnalysisPhaseCompleteStats {
        AnalysisPhaseCompleteStats {
            actions_constructed_total: self.action_count,
            actions_constructed_configured_targets_only: self.configured_target_action_count,
            targets_configured_total: self.configured_object_count,
            targets_configured_configured_targets_only: self.configured_target_count,
            time_in_ms,
            packages_successfully_loaded: self.packages_successfully_loaded,
            actions_constructed_by_mnemonic: self
                .action_count_by_mnemonic
                .clone()
                .into_iter()
                .collect(),
        }
    }
}

// -------------------------------------------------------------------------------------------
// Merged analysis and execution (Skymeld)

/// What `SkyframeBuildView.analyzeAndExecuteTargets` (and `BuildView.update` before it) reach
/// through the command: Java's callbacks (`ExecutionSetup`, `BuildConfigurationsCreated`,
/// `CoverageReportActionsWrapperSupplier`) and the parts of `SkyframeExecutor` that belong to the
/// execution tool here (the watchdog, `isBuildingExclusiveArtifacts`, the order of the exclusive
/// tests).
pub trait SkymeldExecutionHooks {
    /// `executionSetupCallback.prepareForExecution()` (`ExecutionTool.prepareForExecution`).
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:391 BuildView.update
    fn prepare_for_execution(
        &self,
        skyframe_executor: &SkyframeExecutor,
    ) -> Result<(), crate::build_tool::BuildToolError>;

    /// `buildConfigurationCreatedCallback.run(topLevelConfig)` (`result::setBuildConfiguration`).
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:347 BuildView.update
    fn build_configuration_created(
        &self,
        configuration: &Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
    );

    /// `skyframeExecutor.getIsBuildingExclusiveArtifacts().set(value)`.
    fn set_is_building_exclusive_artifacts(&self, value: bool);

    /// `getExclusiveTests(mainEvaluationResult)`: the keys of the
    /// `ExclusiveTestBuildDriverValue`s of the result, in the order Java iterates the result's
    /// values. `root_keys` are the evaluated `BuildDriverKey`s.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1103 SkyframeBuildView.getExclusiveTests
    fn get_exclusive_tests(
        &self,
        main_evaluation_result: &EvaluationResult,
        root_keys: &[BuildDriverKey],
    ) -> Vec<BuildDriverKey>;

    /// `coverageReportActionsWrapperSupplier.getCoverageReportArtifacts(analyzedTargets,
    /// analyzedTests)`, then `eventBus.post(CoverageArtifactsKnownEvent.create(artifacts))`.
    fn get_coverage_report_artifacts(
        &self,
        skyframe_executor: &SkyframeExecutor,
        analyzed_targets: &[ConfiguredTarget],
        analyzed_tests: &[ConfiguredTarget],
    ) -> Vec<blaze_actions::Artifact>;

    /// `skyframeExecutor.clearExecutionStatesSkymeld(eventHandler)`: no more action execution
    /// beyond this point.
    fn clear_execution_states_skymeld(&self);
}

/// The arguments of `BuildView.update` that only a build with merged analysis and execution
/// reads.
pub struct MergedAnalysisAndExecutionArgs<'a> {
    pub hooks: &'a dyn SkymeldExecutionHooks,
    /// `env.getBuildResultListener()`.
    pub build_result_listener: Arc<crate::build_result_listener::BuildResultListener>,
    /// `topLevelOptions`: `request.getTopLevelArtifactContext()`'s output groups.
    pub top_level_artifact_context_output_groups: Vec<String>,
    /// `viewOptions.extraActionTopLevelOnly`.
    pub extra_action_top_level_only: bool,
    /// `viewOptions.extraActionFilter`.
    pub extra_action_filter: Option<blaze_optdefs::common::regex_filter::RegexFilter>,
    pub build_driver_key_test_context: crate::execution_seam::BuildDriverKeyTestContext,
    /// `request.getBuildOptions().skymeldAnalysisOverlapPercentage`.
    pub skymeld_analysis_overlap_percentage: i32,
    /// `skyframeExecutor.tracksStateForIncrementality()` (the command's
    /// `decideKeepIncrementalState`).
    pub tracks_state_for_incrementality: bool,
}

/// What `analyzeAndExecuteTargets` throws.
#[derive(Debug, Clone)]
pub enum AnalyzeAndExecuteError {
    Interrupted,
    ViewCreationFailed(ViewCreationFailedException),
    /// `BuildFailedException` / `TestExecException`.
    Execution(skyframe_error_processor::RethrownException),
}

impl From<Interrupted> for AnalyzeAndExecuteError {
    fn from(_: Interrupted) -> Self {
        AnalyzeAndExecuteError::Interrupted
    }
}

impl From<ViewCreationFailedException> for AnalyzeAndExecuteError {
    fn from(e: ViewCreationFailedException) -> Self {
        AnalyzeAndExecuteError::ViewCreationFailed(e)
    }
}

impl From<skyframe_error_processor::ProcessErrorsException> for AnalyzeAndExecuteError {
    fn from(e: skyframe_error_processor::ProcessErrorsException) -> Self {
        match e {
            skyframe_error_processor::ProcessErrorsException::ViewCreationFailed(e) => {
                AnalyzeAndExecuteError::ViewCreationFailed(e)
            }
            skyframe_error_processor::ProcessErrorsException::Rethrown(e) => {
                AnalyzeAndExecuteError::Execution(e)
            }
        }
    }
}

/// The live evaluator threads of the merged evaluation.
///
/// PORT: Blaze only. Java runs the analysis on a CPU-heavy pool and the execution on its own
/// pool of the same evaluation; Blaze's blocking engine has one pool, whose threads must have
/// the analysis' stack ([`blaze_starlark::eval::EVAL_STACK_SIZE`]: rule implementations and
/// `.bzl` files are evaluated on them). Spawns fork the server, and a fork fails with ENOMEM
/// once the writable private mappings of the server (each thread's reserved stack among them)
/// exceed what the kernel's heuristic overcommit allows (on this 30 GiB machine, between 120 and
/// 400 threads of 256 MiB): the number of threads is capped.
const MERGED_EVALUATION_MAX_THREADS: usize = 64;

/// Evaluates the given collections of CT/Aspect BuildDriverKeys.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2477 SkyframeExecutor.evaluateBuildDriverKeys
fn evaluate_build_driver_keys(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    build_driver_ct_keys: &[BuildDriverKey],
    build_driver_aspect_keys: &[BuildDriverKey],
    workspace_status_artifacts: &[blaze_actions::Artifact],
    keep_going: bool,
) -> Result<EvaluationResult, Interrupted> {
    let execution = skyframe_executor
        .get_execution()
        .expect("IllegalStateException: the executor was created without execution");
    // buildDriverFunction.setShouldCheckForConflictWithTraversal(...): the seam reads the mode
    // (execution_seam::conflict_checking_mode_in_this_build).
    if crate::execution_seam::conflict_checking_mode_in_this_build()
        != crate::execution_seam::ConflictCheckingMode::NONE
    {
        crate::execution_seam::initialize_skymeld_conflict_finding_states(
            skyframe_executor
                .get_action_key_context()
                .expect("IllegalStateException: the command does not configure targets")
                .clone(),
            skyframe_executor.get_walkable_graph(),
        );
    }
    event_handler.post(Arc::new(
        blaze_buildtool::buildtool::buildevent::ConfigurationPhaseStartedEvent::new(Arc::new(
            crate::skyframe_progress_receiver::AnalysisProgressReceiverView(
                &blaze_sky_analysis::analysis_progress_receiver::ANALYSIS_PROGRESS,
            ),
        )),
    ));
    // For the workspace status actions.
    event_handler.post(Arc::new(
        blaze_sky_exec::events::SomeExecutionStartedEvent::not_counted_in_execution_time(),
    ));
    // Iterables.concat(buildDriverCTKeys, buildDriverAspectKeys, Artifact.keys(workspaceStatusArtifacts))
    let mut roots: Vec<SkyKey> = build_driver_ct_keys
        .iter()
        .chain(build_driver_aspect_keys)
        .cloned()
        .map(SkyKey::new)
        .collect();
    roots.extend(
        workspace_status_artifacts
            .iter()
            .map(blaze_sky_exec::keys::artifact_key),
    );
    // PORT: setMergingSkyframeAnalysisExecutionPhases(true) selects Java's pools; see
    // MERGED_EVALUATION_MAX_THREADS.
    let ctx = blaze_skyframe::EvaluationContext {
        keep_going,
        parallelism: execution.config.jobs,
        event_handler,
        thread_stack_size: blaze_starlark::eval::EVAL_STACK_SIZE,
        max_threads: MERGED_EVALUATION_MAX_THREADS,
        ..blaze_skyframe::EvaluationContext::default()
    };
    skyframe_executor.get_evaluator().evaluate(&roots, &ctx)
}

/// `shouldCheckForConflicts(checkForActionConflicts, newKeys)` of `analyzeAndExecuteTargets`.
fn should_check_for_conflicts_skymeld(
    skyframe_executor: &SkyframeExecutor,
    check_for_action_conflicts: bool,
    new_keys_count: usize,
) -> bool {
    should_check_for_conflicts(
        check_for_action_conflicts,
        new_keys_count,
        skyframe_executor.get_evaluator(),
    )
}

/// Performs analysis & execution of the CTs and aspects with Skyframe.
///
/// In case of error: --nokeep_going will eventually throw a ViewCreationFailedException, whereas
/// --keep_going will return a SkyframeAnalysisAndExecutionResult which contains the failure
/// details.
///
/// PORT: `topLevelArtifactContext` is its output groups; `bugReporter`, `resourceManager` and
/// `executors` have no port (`resourceManager.resetResourceUsage()`: Blaze's resource manager
/// holds nothing between evaluations); `shouldClearSyscallCache` has no port (no syscall cache).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:618 SkyframeBuildView.analyzeAndExecuteTargets
#[allow(clippy::too_many_arguments)]
pub fn analyze_and_execute_targets(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    ct_keys: &[ConfiguredTargetKey],
    top_level_aspects_keys: &[TopLevelAspectsKey],
    tests_to_run: Option<&IndexSet<Label>>,
    label_target_map: &IndexMap<Label, Target>,
    explicit_target_patterns: &IndexSet<Label>,
    keep_going: bool,
    skip_incompatible_explicit_targets: bool,
    check_for_action_conflicts: bool,
    args: &MergedAnalysisAndExecutionArgs<'_>,
) -> Result<SkyframeAnalysisResult, AnalyzeAndExecuteError> {
    use crate::execution_seam::{self, ConflictCheckingMode};

    let analysis_work_timer = std::time::Instant::now();
    let hooks = args.hooks;
    let top_level_artifact_context = &args.top_level_artifact_context_output_groups;

    // ImmutableSet<ActionLookupKey> newKeys = ctKeys ∪ topLevelAspectsKeys
    let new_keys_count = ct_keys
        .iter()
        .map(|k| ActionLookupKeyRef::new(k.clone()))
        .chain(
            top_level_aspects_keys
                .iter()
                .map(|k| ActionLookupKeyRef::new(k.clone())),
        )
        .collect::<IndexSet<_>>()
        .len();

    let workspace_status_artifacts =
        skyframe_executor.get_workspace_status_artifacts(event_handler.clone())?;

    {
        let tests_to_run = tests_to_run.cloned();
        let label_target_map = label_target_map.clone();
        let build_driver_key_test_context = args.build_driver_key_test_context.clone();
        execution_seam::set_test_type_resolver(Some(Arc::new(
            move |label: &Label, event_handler: &dyn ExtendedEventHandler| {
                execution_seam::determine_test_type_impl(
                    tests_to_run.as_ref(),
                    &label_target_map,
                    label,
                    &build_driver_key_test_context,
                    event_handler,
                )
            },
        )));
    }

    let build_driver_ct_keys: Vec<BuildDriverKey> = ct_keys
        .iter()
        .map(|ct_key| {
            BuildDriverKey::of_configured_target(
                ActionLookupKeyRef::new(ct_key.clone()),
                top_level_artifact_context.clone(),
                /* explicitly_requested= */
                explicit_target_patterns.contains(ct_key.get_label()),
                skip_incompatible_explicit_targets,
                args.extra_action_top_level_only,
                keep_going,
            )
        })
        .collect::<IndexSet<_>>()
        .into_iter()
        .collect();

    let build_driver_aspect_keys: Vec<BuildDriverKey> = top_level_aspects_keys
        .iter()
        .map(|k| {
            BuildDriverKey::of_top_level_aspect(
                ActionLookupKeyRef::new(k.clone()),
                top_level_artifact_context.clone(),
                /* explicitly_requested= */ explicit_target_patterns.contains(k.get_label()),
                skip_incompatible_explicit_targets,
                args.extra_action_top_level_only,
                keep_going,
            )
        })
        .collect::<IndexSet<_>>()
        .into_iter()
        .collect();
    args.build_result_listener.set_top_level_keys(
        build_driver_ct_keys
            .iter()
            .chain(&build_driver_aspect_keys)
            .cloned()
            .collect(),
    );
    let mut detailed_exit_codes: Vec<Option<blaze_util::DetailedExitCode>> = Vec::new();
    // Sets.union(buildDriverCTKeys, buildDriverAspectKeys)
    let top_level_keys: std::collections::HashSet<BuildDriverKey> = build_driver_ct_keys
        .iter()
        .chain(&build_driver_aspect_keys)
        .cloned()
        .collect();

    let should_check_for_conflicts = should_check_for_conflicts_skymeld(
        skyframe_executor,
        check_for_action_conflicts,
        new_keys_count,
    );
    let conflict_checking_mode = if should_check_for_conflicts {
        if args.tracks_state_for_incrementality {
            ConflictCheckingMode::WITH_TRAVERSAL
        } else {
            ConflictCheckingMode::UPON_CONFIGURED_OBJECT_CREATION
        }
    } else {
        ConflictCheckingMode::NONE
    };
    if conflict_checking_mode == ConflictCheckingMode::UPON_CONFIGURED_OBJECT_CREATION {
        // The conflict check as each action is registered
        // (IncrementalArtifactConflictFinder.conflictCheckPerAction from the analysis).
        blaze_util::unported!(
            "SkyframeBuildView.analyzeAndExecuteTargets",
            "ConflictCheckingMode.UPON_CONFIGURED_OBJECT_CREATION (no incrementality tracking)"
        );
    }
    execution_seam::set_conflict_checking_mode_in_this_build(conflict_checking_mode);

    // The execution tasks wait for the go-ahead of the watcher (MultiThreadPoolsQuiescingExecutor
    // .launchQueuedUpExecutionPhaseTasks).
    // PORT: with 100 (the default) Java's gate opens when the first top-level key concludes its
    // analysis; the blocking engine runs execution work (the workspace status action, a root
    // of the evaluation) as soon as it is requested. A smaller overlap delays the execution
    // work of the analyzed keys, which the blocking engine cannot queue.
    if args.skymeld_analysis_overlap_percentage != 100 {
        blaze_util::unported!(
            "MultiExecutorQueueVisitor.execute",
            "--experimental_skymeld_analysis_overlap_percentage={} (execution tasks stalled until the go-ahead)",
            args.skymeld_analysis_overlap_percentage
        );
    }

    let analysis_phase_complete_stats: Arc<parking_lot::Mutex<Option<AnalysisPhaseCompleteStats>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let build_graph_metrics: Arc<parking_lot::Mutex<Option<BuildGraphMetrics>>> =
        Arc::new(parking_lot::Mutex::new(None));
    let finisher: crate::analysis_operation_watcher::AnalysisOperationWatcherFinisher = {
        let event_handler = event_handler.clone();
        let build_result_listener = args.build_result_listener.clone();
        let walkable_graph = skyframe_executor.get_walkable_graph();
        let ct_keys = ct_keys.to_vec();
        let evaluated_counts = skyframe_executor.evaluated_counts_supplier();
        let analysis_phase_complete_stats = analysis_phase_complete_stats.clone();
        let build_graph_metrics = build_graph_metrics.clone();
        Box::new(move || {
            analysis_finished_callback(
                event_handler.as_ref(),
                &build_result_listener,
                walkable_graph.as_ref(),
                &ct_keys,
                /* measured_analysis_time= */
                analysis_work_timer.elapsed().as_millis() as i64,
                conflict_checking_mode,
                evaluated_counts.as_ref(),
                &analysis_phase_complete_stats,
                &build_graph_metrics,
            )
        })
    };
    let lower_threshold_to_signal_for_execution = (top_level_keys.len() as f64
        * args.skymeld_analysis_overlap_percentage as f64
        / 100.0) as f32;
    let auto_closeable_watcher =
        crate::analysis_operation_watcher::AnalysisOperationWatcher::create_and_register_with_event_bus(
            top_level_keys,
            event_handler.clone(),
            lower_threshold_to_signal_for_execution,
            finisher,
            /* execution_go_ahead_callback= */ Box::new(|| {}),
        );
    let watched_event_handler: Arc<dyn ExtendedEventHandler> = auto_closeable_watcher;

    // try { ... } finally { clearExecutionStatesSkymeld }
    let executed = (|| -> Result<(EvaluationResult, bool), AnalyzeAndExecuteError> {
        hooks.set_is_building_exclusive_artifacts(false);
        let main_evaluation_result = {
            let _c = blaze_util::profiler::Profiler::instance()
                .profile_info("skyframeExecutor.evaluateBuildDriverKeys");
            // PORT: skipped, enableAnalysis(true): incremental bookkeeping of analysis nodes.
            let evaluated = evaluate_build_driver_keys(
                skyframe_executor,
                watched_event_handler.clone(),
                &build_driver_ct_keys,
                &build_driver_aspect_keys,
                &workspace_status_artifacts,
                keep_going,
            );
            // finally:
            // Required for incremental correctness.
            // We unconditionally reset the states here instead of in #analysisFinishedCallback
            // since in case of --nokeep_going & analysis error, the analysis phase is never
            // finished.
            execution_seam::clear_incremental_artifact_conflict_finding_states();
            blaze_sky_exec::build_driver_function::BuildDriverFunction::reset_states();
            execution_seam::set_test_type_resolver(None);
            // PORT: skipped, largestTopLevelKeySetCheckedForConflicts and
            // someActionLookupValueEvaluated: `should_check_for_conflicts` reads the graph.
            evaluated?
        };

        // The exclusive tests whose analysis succeeded i.e. those that can be run.
        let root_keys: Vec<BuildDriverKey> = build_driver_ct_keys
            .iter()
            .chain(&build_driver_aspect_keys)
            .cloned()
            .collect();
        let exclusive_tests_to_run = hooks.get_exclusive_tests(&main_evaluation_result, &root_keys);
        let continue_with_exclusive_tests = !main_evaluation_result.has_error() || keep_going;
        let mut has_exclusive_tests_error = false;

        if continue_with_exclusive_tests && !exclusive_tests_to_run.is_empty() {
            hooks.set_is_building_exclusive_artifacts(true);
            let execution = skyframe_executor
                .get_execution()
                .expect("IllegalStateException: the executor was created without execution");
            // Run exclusive tests sequentially.
            // TestCompletionValue.keys(exclusiveTestsToRun, topLevelArtifactContext,
            // /* exclusiveTesting= */ true)
            for key in &exclusive_tests_to_run {
                let test_completion_key = blaze_sky_exec::keys::TestCompletionKey {
                    configured_target_key: key.action_lookup_key.clone(),
                    output_groups: key.output_groups.clone(),
                    exclusive_testing: true,
                };
                let test_run_result =
                    blaze_buildtool::buildtool::execution_tool::run_exclusive_test_skymeld(
                        skyframe_executor.get_evaluator(),
                        &test_completion_key,
                        &execution.config,
                        event_handler.clone(),
                    )?;
                if test_run_result.has_error() {
                    has_exclusive_tests_error = true;
                    detailed_exit_codes.push(
                        skyframe_error_processor::process_errors_with_execution(
                            &test_run_result,
                            skyframe_executor.get_cycles_reporter(),
                            &event_handler,
                            keep_going,
                        )?
                        .execution_detailed_exit_code()
                        .cloned(),
                    );
                }
            }
        }

        // Coverage report generation should only be requested after all tests have executed.
        // When --nokeep_going and there's an earlier error, we should skip this and fail fast.
        if (!main_evaluation_result.has_error() && !has_exclusive_tests_error) || keep_going {
            let coverage_report_artifacts = hooks.get_coverage_report_artifacts(
                skyframe_executor,
                &args.build_result_listener.get_analyzed_targets(),
                &args.build_result_listener.get_analyzed_tests(),
            );
            let execution = skyframe_executor
                .get_execution()
                .expect("IllegalStateException: the executor was created without execution");
            let additional_artifacts_result =
                blaze_buildtool::buildtool::execution_tool::evaluate_artifacts(
                    skyframe_executor.get_evaluator(),
                    &coverage_report_artifacts,
                    &execution.config,
                    crate::skyframe_executor::DEFAULT_THREAD_COUNT,
                    event_handler.clone(),
                )?;
            if additional_artifacts_result.has_error() {
                detailed_exit_codes.push(
                    skyframe_error_processor::process_errors_with_execution(
                        &additional_artifacts_result,
                        skyframe_executor.get_cycles_reporter(),
                        &event_handler,
                        keep_going,
                    )?
                    .execution_detailed_exit_code()
                    .cloned(),
                );
            }
        }
        Ok((main_evaluation_result, has_exclusive_tests_error))
    })();
    // finally: No more action execution beyond this point.
    hooks.clear_execution_states_skymeld();
    let (main_evaluation_result, _) = executed?;

    let graph = skyframe_executor.get_walkable_graph();
    let analysis_phase_complete_stats = analysis_phase_complete_stats.lock().clone();
    let build_graph_metrics = build_graph_metrics.lock().clone();
    if !main_evaluation_result.has_error() && detailed_exit_codes.is_empty() {
        let successful_aspects = get_successful_aspect_map(
            graph.as_ref(),
            &main_evaluation_result,
            &build_driver_aspect_keys,
            /* top_level_action_conflict_report= */ None,
        );
        let mut targets_with_configuration: Vec<TargetAndConfiguration> = Vec::new();
        let successful_configured_targets = get_successful_configured_targets(
            graph.as_ref(),
            &main_evaluation_result,
            &build_driver_ct_keys,
            label_target_map,
            &mut targets_with_configuration,
            /* top_level_action_conflict_report= */ None,
        );

        // SkyframeAnalysisAndExecutionResult.success
        return Ok(SkyframeAnalysisResult {
            has_loading_error: false,
            has_analysis_error: false,
            has_action_conflicts: false,
            configured_targets: successful_configured_targets,
            aspects: successful_aspects,
            configured_targets_with_action_conflicts: Vec::new(),
            walkable_graph: graph,
            targets_with_configuration,
            build_graph_metrics,
            execution_detailed_exit_code: None,
            analysis_phase_complete_stats,
        });
    }

    let error_processing_result = skyframe_error_processor::process_errors_with_execution(
        &main_evaluation_result,
        skyframe_executor.get_cycles_reporter(),
        &event_handler,
        keep_going,
    )?;
    detailed_exit_codes.push(
        error_processing_result
            .execution_detailed_exit_code()
            .cloned(),
    );

    let found_action_conflict_in_latest_check =
        !error_processing_result.action_conflicts().is_empty();
    let top_level_action_conflict_report = if found_action_conflict_in_latest_check {
        Some(handle_action_conflicts(
            skyframe_executor,
            event_handler.clone(),
            graph.as_ref(),
            ct_keys,
            top_level_aspects_keys,
            top_level_artifact_context,
            keep_going,
            &error_processing_result,
        )?)
    } else {
        None
    };
    let successful_aspects = get_successful_aspect_map(
        graph.as_ref(),
        &main_evaluation_result,
        &build_driver_aspect_keys,
        top_level_action_conflict_report.as_ref(),
    );
    let mut targets_with_configuration: Vec<TargetAndConfiguration> = Vec::new();
    let successful_configured_targets = get_successful_configured_targets(
        graph.as_ref(),
        &main_evaluation_result,
        &build_driver_ct_keys,
        label_target_map,
        &mut targets_with_configuration,
        top_level_action_conflict_report.as_ref(),
    );

    // Collections.max(detailedExitCodes, DetailedExitCodeComparator.INSTANCE): the first of the
    // most important.
    let execution_detailed_exit_code = detailed_exit_codes.into_iter().fold(None, |max, code| {
        blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(max, code)
    });
    // SkyframeAnalysisAndExecutionResult.withErrors
    Ok(SkyframeAnalysisResult {
        has_loading_error: error_processing_result.has_loading_error(),
        // legacy behavior: action conflicts are considered analysis errors.
        has_analysis_error: error_processing_result.has_analysis_error()
            || found_action_conflict_in_latest_check,
        has_action_conflicts: found_action_conflict_in_latest_check,
        configured_targets: successful_configured_targets,
        aspects: successful_aspects,
        configured_targets_with_action_conflicts: Vec::new(),
        walkable_graph: graph,
        targets_with_configuration,
        build_graph_metrics,
        execution_detailed_exit_code,
        analysis_phase_complete_stats,
    })
}

/// The counters of the `AnalysisPhaseCompleteEvent` and the metrics of the
/// `AnalysisGraphStatsEvent` that `analysisFinishedCallback` posts, for their one subscriber
/// (the metrics collector).
///
/// PORT: Blaze's `AnalysisPhaseCompleteEvent` (blaze_buildtool) carries only the number of
/// analyzed targets (the UI's); the two Java events' other contents travel in this one.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:957 SkyframeBuildView.analysisFinishedCallback
#[derive(Clone, Debug)]
pub struct AnalysisPhaseCompleteStatsEvent {
    pub stats: AnalysisPhaseCompleteStats,
    pub build_graph_metrics: Option<BuildGraphMetrics>,
}

impl blaze_events::Postable for AnalysisPhaseCompleteStatsEvent {}

/// Handles the required steps after all analysis work in this build is done.
///
/// PORT: `AnalysisGraphStatsEvent` and the counters of `AnalysisPhaseCompleteEvent` reach their
/// one subscriber (the metrics collector) through the analysis result
/// (`build_graph_metrics`, `analysis_phase_complete_stats`); the `AnalysisPhaseCompleteEvent`
/// itself (the UI's) is posted here. `shouldDiscardAnalysisCache` (`clearAnalysisCache`: a heap
/// optimization) is the incremental module's, at the end of the command
/// (crate::incremental); `clearSyscallCache` and `enableAnalysis(false)` have no port.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:910 SkyframeBuildView.analysisFinishedCallback
#[allow(clippy::too_many_arguments)]
fn analysis_finished_callback(
    event_bus: &dyn ExtendedEventHandler,
    build_result_listener: &crate::build_result_listener::BuildResultListener,
    walkable_graph: &dyn WalkableGraph,
    configured_target_keys: &[ConfiguredTargetKey],
    measured_analysis_time: i64,
    conflict_checking_mode: crate::execution_seam::ConflictCheckingMode,
    evaluated_counts: &(dyn Fn(i64) -> AnalysisPhaseCompleteStats + Send + Sync),
    analysis_phase_complete_stats: &parking_lot::Mutex<Option<AnalysisPhaseCompleteStats>>,
    build_graph_metrics: &parking_lot::Mutex<Option<BuildGraphMetrics>>,
) {
    if conflict_checking_mode != crate::execution_seam::ConflictCheckingMode::NONE {
        // Now that we have the full picture, it's time to collect the metrics of the whole
        // graph.
        let aspect_keys: Vec<AspectKey> = build_result_listener
            .get_analyzed_aspects()
            .keys()
            .cloned()
            .collect();
        let mut build_graph_metrics_builder = collect_action_lookup_values_in_build(
            walkable_graph,
            configured_target_keys,
            &aspect_keys,
        )
        .get_metrics();
        if let Some(incremental_artifact_conflict_finder) =
            crate::execution_seam::get_checker_for_conflict_checking_mode(conflict_checking_mode)
        {
            build_graph_metrics_builder.output_artifact_count =
                incremental_artifact_conflict_finder.get_output_artifact_count();
        }
        // eventBus.post(new AnalysisGraphStatsEvent(buildGraphMetricsBuilder.build()))
        *build_graph_metrics.lock() = Some(build_graph_metrics_builder);
    }

    // At this point, it's safe to clear objects related to action conflict checking.
    // Clearing the states here is a performance optimization (reduce peak heap size) and isn't
    // required for correctness.
    crate::execution_seam::clear_incremental_artifact_conflict_finding_states();

    // eventBus.post(new AnalysisPhaseCompleteEvent(buildResultListener.getAnalyzedTargets(),
    // getEvaluatedCounts(), ..., measuredAnalysisTime, ...))
    let stats = evaluated_counts(measured_analysis_time);
    *analysis_phase_complete_stats.lock() = Some(stats.clone());
    event_bus.post(Arc::new(AnalysisPhaseCompleteStatsEvent {
        stats,
        build_graph_metrics: build_graph_metrics.lock().clone(),
    }));
    event_bus.post(Arc::new(
        blaze_buildtool::buildtool::buildevent::AnalysisPhaseCompleteEvent::new(
            build_result_listener.get_analyzed_targets().len(),
        ),
    ));
}

/// Report the appropriate conflicts and return a TopLevelActionConflictReport.
///
/// The TopLevelActionConflictReport is used to determine the set of top level targets that
/// depend on conflicted actions.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:970 SkyframeBuildView.handleActionConflicts
#[allow(clippy::too_many_arguments)]
fn handle_action_conflicts(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    graph: &dyn WalkableGraph,
    ct_keys: &[ConfiguredTargetKey],
    top_level_aspects_keys: &[TopLevelAspectsKey],
    top_level_artifact_context_for_conflict_pruning: &[String],
    keep_going: bool,
    error_processing_result: &skyframe_error_processor::ErrorProcessingResult,
) -> Result<crate::skyframe_executor::TopLevelActionConflictReport, AnalyzeAndExecuteError> {
    // TODO(b/332898055) Unify with the noskymeld code path.
    let handled = (|| -> Result<crate::skyframe_executor::TopLevelActionConflictReport, AnalyzeAndExecuteError> {
        // Here we already have the <TopLevelAspectKey, error> mapping, but what we need to fit
        // into the existing AnalysisFailureEvent is <AspectKey, error>. An extra Skyframe
        // evaluation is required.
        // If the conflict is intra-Aspect, the TopLevelAspectValue would be null and the
        // AspectKey isn't retrievable. It must be supplied via the ErrorProcessingResult.
        let mut effective_top_level_keys_for_conflict_reporting: IndexSet<ActionLookupKeyRef> =
            IndexSet::new();
        effective_top_level_keys_for_conflict_reporting
            .extend(ct_keys.iter().map(|k| ActionLookupKeyRef::new(k.clone())));
        effective_top_level_keys_for_conflict_reporting.extend(
            get_derived_aspect_keys_for_conflict_reporting(graph, top_level_aspects_keys)
                .into_iter()
                .map(ActionLookupKeyRef::new),
        );
        effective_top_level_keys_for_conflict_reporting.extend(
            error_processing_result
                .aspect_keys_for_conflict_reporting()
                .iter()
                .cloned(),
        );
        let effective_top_level_keys_for_conflict_reporting: Vec<ActionLookupKeyRef> =
            effective_top_level_keys_for_conflict_reporting
                .into_iter()
                .collect();
        // In order to determine the set of configured targets transitively error free from
        // action conflict issues, we run a post-processing update() that uses the bad action
        // map.
        // PORT: skipped, enableAnalysis(true/false) (incremental bookkeeping).
        let top_level_action_conflict_report = skyframe_executor
            .filter_action_conflicts_for_configured_targets_and_aspects(
                event_handler.clone(),
                &effective_top_level_keys_for_conflict_reporting,
                error_processing_result.action_conflicts(),
                top_level_artifact_context_for_conflict_pruning,
            )?;
        report_action_conflict_errors(
            &top_level_action_conflict_report,
            graph,
            &effective_top_level_keys_for_conflict_reporting,
            error_processing_result.action_conflicts(),
            event_handler.as_ref(),
            keep_going,
        )?;
        Ok(top_level_action_conflict_report)
    })();
    // finally
    skyframe_executor.reset_action_conflicts_stored_in_skyframe();
    handled
}

/// From the `top_level_action_conflict_report`, report the action conflict errors.
///
/// Throw a ViewCreationFailedException in case of --nokeep_going.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1022 SkyframeBuildView.reportActionConflictErrors
fn report_action_conflict_errors(
    top_level_action_conflict_report: &crate::skyframe_executor::TopLevelActionConflictReport,
    graph: &dyn WalkableGraph,
    effective_top_level_keys_for_conflict_reporting: &[ActionLookupKeyRef],
    action_conflicts: &ActionConflicts,
    event_handler: &dyn ExtendedEventHandler,
    keep_going: bool,
) -> Result<(), AnalyzeAndExecuteError> {
    // ArtifactPrefixConflictExceptions come in pairs, and only one should be reported.
    let mut reported_action_conflict_exceptions: HashSet<String> = HashSet::new();

    // Sometimes a conflicting action can't be traced to a top level target via
    // TopLevelActionConflictReport. We therefore need to print the errors from the conflicts
    // themselves. See SkyframeIntegrationTest#topLevelAspectsAndExtraActionsWithConflict.
    for (_, e) in action_conflicts.entries() {
        if reported_action_conflict_exceptions.insert(e.get_message().to_owned()) {
            e.report_to(event_handler);
            if keep_going {
                event_handler.handle(&Event::warn(format!(
                    "errors encountered while analyzing target '{}': it will not be built",
                    e.get_artifact()
                        .get_owner_label()
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| "null".to_owned())
                )));
            }
        }
    }
    // Report an AnalysisFailureEvent to BEP for the top-level targets with discoverable action
    // conflicts, then finally throw.
    for action_lookup_key in effective_top_level_keys_for_conflict_reporting {
        if top_level_action_conflict_report.is_error_free(action_lookup_key) {
            continue;
        }
        let Some(conflict_exception) =
            top_level_action_conflict_report.get_conflict_exception(action_lookup_key)
        else {
            continue;
        };

        let failed_cause = make_artifact_conflict_analysis_failed_cause(&conflict_exception);
        let failed_cause_failure_detail = conflict_exception.get_failure_detail();
        let mut target_configured = true;
        let mut action_lookup_key = action_lookup_key.clone();
        // Attempt to promote any ConfiguredTargetKey to the one embedded in the ConfiguredTarget
        // to reflect any transitions or trimming.
        if let Some(ct_key) = action_lookup_key
            .as_any()
            .downcast_ref::<ConfiguredTargetKey>()
        {
            // This is a graph lookup instead of an EvaluationResult lookup because Skymeld's
            // EvaluationResult does not contain ConfiguredTargetKey.
            match graph
                .get_value(&SkyKey::new(ct_key.clone()))
                .and_then(|v| v.downcast::<ConfiguredTargetValue>().ok())
            {
                None => target_configured = false,
                // It's possible that the ConfiguredTarget has been cleared.
                Some(value) => {
                    action_lookup_key = value.get_configured_target().get_lookup_key().clone();
                }
            }
        }
        if !target_configured {
            let ct_key = action_lookup_key
                .as_any()
                .downcast_ref::<ConfiguredTargetKey>()
                .unwrap_or_else(|| {
                    panic!("ClassCastException: {action_lookup_key} is not a ConfiguredTargetKey")
                })
                .clone();
            event_handler.post(Arc::new(AnalysisFailureEvent::while_analyzing_target(
                ct_key,
                vec![failed_cause],
            )));
        } else {
            event_handler.post(Arc::new(AnalysisFailureEvent::action_conflict(
                &action_lookup_key,
                vec![failed_cause],
            )));
        }

        if !keep_going {
            return Err(AnalyzeAndExecuteError::ViewCreationFailed(
                ViewCreationFailedException::of_cause(
                    failed_cause_failure_detail,
                    conflict_exception.get_message(),
                ),
            ));
        }
    }
    Ok(())
}

/// The derived aspect keys of the top-level aspects keys whose analysis completed: the keys of
/// each `TopLevelAspectsValue`'s map.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1162 SkyframeBuildView.getDerivedAspectKeysForConflictReporting
fn get_derived_aspect_keys_for_conflict_reporting(
    graph: &dyn WalkableGraph,
    top_level_aspects_keys: &[TopLevelAspectsKey],
) -> Vec<AspectKey> {
    let mut aspect_keys_builder: IndexSet<AspectKey> = IndexSet::new();
    for top_level_aspects_key in top_level_aspects_keys {
        // skyframeExecutor.getDoneSkyValueForIntrospection(topLevelAspectsKey)
        match graph
            .get_value(&SkyKey::new(top_level_aspects_key.clone()))
            .and_then(|v| v.downcast::<TopLevelAspectsValue>().ok())
        {
            Some(top_level_aspects_value) => aspect_keys_builder.extend(
                top_level_aspects_value
                    .get_top_level_aspects_map()
                    .keys()
                    .cloned(),
            ),
            // It could happen that the analysis of TopLevelAspectKey wasn't complete: either its
            // own analysis failed, or another error was raise in --nokeep_going mode. In that
            // case, it couldn't be involved in the conflict exception anyway, and we just move
            // on.
            None => {}
        }
    }
    aspect_keys_builder.into_iter().collect()
}

/// The configured targets of the `BuildDriverKey`s with a value (also the skipped ones), and
/// their targets with configuration.
///
/// PORT: the configured target is read from the graph at the key's `ActionLookupKey` (Java reads
/// it from the `BuildDriverValue`, which wraps that value).
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1183 SkyframeBuildView.getSuccessfulConfiguredTargets
fn get_successful_configured_targets(
    graph: &dyn WalkableGraph,
    evaluation_result: &EvaluationResult,
    build_driver_ct_keys: &[BuildDriverKey],
    label_to_target_map: &IndexMap<Label, Target>,
    targets_with_configuration: &mut Vec<TargetAndConfiguration>,
    top_level_action_conflict_report: Option<
        &crate::skyframe_executor::TopLevelActionConflictReport,
    >,
) -> Vec<ConfiguredTarget> {
    let mut cts: Vec<ConfiguredTarget> = Vec::new();
    for bd_ct_key in build_driver_ct_keys {
        if let Some(report) = top_level_action_conflict_report
            && !report.is_error_free(&bd_ct_key.action_lookup_key)
        {
            continue;
        }
        if evaluation_result.get(bd_ct_key).is_none() {
            continue;
        }
        let ct_value = graph
            .get_value(&crate::execution_seam::action_lookup_sky_key(
                &bd_ct_key.action_lookup_key,
            ))
            .and_then(|v| v.downcast::<ConfiguredTargetValue>().ok())
            .unwrap_or_else(|| {
                panic!(
                    "ClassCastException: the value of {} is not a ConfiguredTargetValue",
                    bd_ct_key.action_lookup_key
                )
            });
        let configured_target = ct_value.get_configured_target().clone();
        // ImmutableSet.Builder: configured targets are deduplicated.
        if !cts
            .iter()
            .any(|ct| same_configured_target(ct, &configured_target))
        {
            cts.push(configured_target.clone());
        }

        let configuration = crate::skyframe_executor::get_configuration_from_graph(
            graph,
            configured_target.get_configuration_key().as_ref(),
        );
        let label = bd_ct_key
            .action_lookup_key
            .get_label()
            .expect("the label of a top-level ConfiguredTargetKey");
        targets_with_configuration.push(TargetAndConfiguration::new(
            label_to_target_map
                .get(&label)
                .unwrap_or_else(|| panic!("NullPointerException: target of {label}"))
                .clone(),
            configuration,
        ));
    }
    cts
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:1222 SkyframeBuildView.getSuccessfulAspectMap
fn get_successful_aspect_map(
    graph: &dyn WalkableGraph,
    evaluation_result: &EvaluationResult,
    build_driver_aspect_keys: &[BuildDriverKey],
    top_level_action_conflict_report: Option<
        &crate::skyframe_executor::TopLevelActionConflictReport,
    >,
) -> IndexMap<AspectKey, Arc<AspectValue>> {
    // There can't be duplicate Aspects after resolving --aspects, so this is safe.
    let mut aspects: IndexMap<AspectKey, Arc<AspectValue>> = IndexMap::new();
    for bd_aspect_key in build_driver_aspect_keys {
        if let Some(report) = top_level_action_conflict_report
            && !report.is_error_free(&bd_aspect_key.action_lookup_key)
        {
            continue;
        }
        if evaluation_result.get(bd_aspect_key).is_none() {
            // Skip aspects that couldn't be applied to targets.
            continue;
        }
        let top_level_aspects_value = graph
            .get_value(&crate::execution_seam::action_lookup_sky_key(
                &bd_aspect_key.action_lookup_key,
            ))
            .and_then(|v| v.downcast::<TopLevelAspectsValue>().ok())
            .unwrap_or_else(|| {
                panic!(
                    "ClassCastException: the value of {} is not a TopLevelAspectsValue",
                    bd_aspect_key.action_lookup_key
                )
            });
        for (k, v) in top_level_aspects_value.get_top_level_aspects_map() {
            // ImmutableMap.Builder.buildOrThrow
            if aspects.insert(k.clone(), v.clone()).is_some() {
                panic!("IllegalArgumentException: Multiple entries with same key: {k:?}");
            }
        }
    }
    aspects
}

#[cfg(test)]
mod should_check_for_conflicts_tests {
    use super::should_check_for_conflicts;

    // Java SkyframeBuildView.shouldCheckForConflicts on a fresh SkyframeBuildView (empty
    // largestTopLevelKeySetCheckedForConflicts, nothing dirtied, no earlier conflict) and a graph
    // without rule configured targets (e.g. `build //pkg:a.txt` of a source file): true for any
    // non-empty key set, false for none or when the request opts out (analysis review R2-7).
    #[test]
    fn fresh_view_checks_any_non_empty_key_set() {
        let evaluator = blaze_skyframe::EvaluatorBuilder::new().build();
        assert!(should_check_for_conflicts(true, 1, &evaluator));
        assert!(!should_check_for_conflicts(true, 0, &evaluator));
        assert!(!should_check_for_conflicts(false, 1, &evaluator));
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
