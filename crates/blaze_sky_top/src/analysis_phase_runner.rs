//! Port of `buildtool/AnalysisPhaseRunner.java` (analysis only: target pattern results to
//! configured targets).

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_protos::failure_details::FailureDetail;
use blaze_sky_analysis::top_level_status_events::{
    TopLevelTargetAnalyzedEvent, TopLevelTargetSkippedEvent,
};
use blaze_sky_exec::analysis_seam::ConfiguredTargetView;
use blaze_sky_exec::events::TestAnalyzedEvent;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::build_tool::{BuildRequest, BuildToolError};
use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::execution_seam::SkyframeConfiguredTargetView;
use crate::skyframe_executor::{SkyframeExecutor, get_configuration_from_graph};
use crate::target_pattern_phase::TargetPatternPhaseValue;

/// Performs target pattern eval, configuration creation, loading and analysis.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:97 AnalysisPhaseRunner.execute
pub fn execute(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    relative_working_directory: &PathFragment,
    request: &BuildRequest,
    target_pattern_phase_value: &TargetPatternPhaseValue,
    build_options: &Arc<BuildOptions>,
) -> Result<Option<AnalysisResult>, BuildToolError> {
    // Compute the heuristic instrumentation filter if needed.
    if request.needs_instrumentation_filter() {
        blaze_util::unported!("InstrumentationFilterSupport.computeInstrumentationFilter");
    }

    // PORT: env.throwPendingException() (an exit requested by a module while the command runs)
    // is checked by the command dispatcher.

    let mut analysis_result = None;
    if request.perform_analysis_phase {
        let result = run_analysis_phase(
            skyframe_executor,
            reporter,
            relative_working_directory,
            request,
            target_pattern_phase_value,
            build_options,
        )?;

        // PORT: BlazeModule.afterAnalysis has no ported implementer (the modules that override
        // it are Google-internal or the remote module).

        // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:156 AnalysisPhaseRunner.execute
        // PORT: with merged analysis and execution, AnalysisAndExecutionPhaseRunner.execute
        // reports the targets after the execution phase returned (it does not when the
        // execution phase throws): `report_analyzed_targets`, called by BuildTool.
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            report_analyzed_targets(reporter.as_ref(), request, &result);
        }

        // PORT: postAbortedEventsForSkippedTargets (an AbortedEvent per skipped target for the
        // build event stream) is the command's, from `AnalysisResult::get_targets_to_skip()`:
        // blaze_runtime's execution phase fails loudly for a build event stream with skipped
        // targets.
        analysis_result = Some(result);
    } else {
        reporter.handle(&Event::progress("Loading complete."));
        // PORT: NoAnalyzeEvent (the build event of --noanalyze) is the command's (blaze_bep
        // bep_module.rs).
        let failure_detail: Option<FailureDetail> =
            build_view::create_analysis_failure_detail(target_pattern_phase_value, None);
        if let Some(failure_detail) = failure_detail {
            return Err(BuildToolError::BuildFailed {
                message: failure_detail.message.clone(),
                failure_detail,
            });
        }
    }

    Ok(analysis_result)
}

/// Performs the initial phases 0-2 of the build: Setup, Loading and Analysis.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:371 AnalysisPhaseRunner.runAnalysisPhase
fn run_analysis_phase(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    relative_working_directory: &PathFragment,
    request: &BuildRequest,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
) -> Result<AnalysisResult, BuildToolError> {
    reporter.handle(&Event::progress("Loading complete.  Analyzing..."));

    let explicit_target_patterns = get_explicit_target_patterns(
        skyframe_executor,
        reporter,
        relative_working_directory,
        &request.targets,
        request.keep_going,
        request.loading_phase_threads,
    )?;

    // PORT: `request.getTopLevelArtifactContext()` reaches SkyframeBuildView.configureTargets
    // (`topLevelArtifactContextForConflictPruning`) through the executor, see
    // `skyframe_build_view::configure_targets`.
    skyframe_executor.set_top_level_artifact_context_for_conflict_pruning(
        request.get_top_level_artifact_context_output_groups(),
    );

    let aspects_parameters =
        request
            .get_aspects_parameters()
            .map_err(|e| BuildToolError::ViewCreationFailed {
                message: e.get_message().to_owned(),
                failure_detail: e.get_failure_detail().clone(),
            })?;
    let analysis_result = build_view::update_with_aspects_parameters(
        skyframe_executor,
        loading_result,
        target_options,
        &BuildViewUpdateOptions {
            keep_going: request.keep_going,
            check_for_action_conflicts: request.check_for_action_conflicts,
            aspects: request.aspects.clone(),
            explicit_target_patterns,
            skip_incompatible_explicit_targets: request.skip_incompatible_explicit_targets,
            report_incompatible_targets: request.report_incompatible_targets,
            // PORT: Java's AnalysisPhaseRunner always passes false: a merged (Skymeld) request
            // goes through AnalysisAndExecutionPhaseRunner, which passes true. Blaze has the one
            // runner.
            include_execution_phase: request.with_merged_analysis_and_execution_source_of_truth(),
        },
        &aspects_parameters,
        reporter.clone(),
    )
    .map_err(|e| match e {
        BuildViewError::ViewCreationFailed {
            message,
            failure_detail,
        } => BuildToolError::ViewCreationFailed {
            message,
            failure_detail,
        },
        BuildViewError::InvalidConfiguration(e) => BuildToolError::InvalidConfiguration(e),
        BuildViewError::Interrupted => BuildToolError::Interrupted,
    })?;

    // TODO(bazel-team): Merge these into one event.
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:429 AnalysisPhaseRunner.runAnalysisPhase
    // PORT: with merged analysis and execution Java posts it from
    // SkyframeBuildView.analysisFinishedCallback (SkyframeBuildView.java:957), once the analysis
    // of every top-level target concluded; Blaze's merged build has concluded its analysis here.
    // Java posts it before the errors of a keep_going analysis are reported, Blaze after
    // (open: the "errors encountered while analyzing" warnings precede the summary line). The
    // event's counters for the build metrics are the command's
    // (`AnalysisResult::get_analysis_phase_complete_stats`).
    reporter.post(Arc::new(
        blaze_buildtool::buildtool::buildevent::AnalysisPhaseCompleteEvent::new(
            analysis_result.get_targets_to_build().len(),
        ),
    ));
    // PORT: Java's AnalysisPhaseRunner serves only requests without merged analysis and
    // execution; for a merged request BuildDriverFunction posts the top-level status events,
    // which `build_view::update` (analyzed, skipped) and the command (skipped tests) do here,
    // and no TestFilteringCompleteEvent is posted.
    if !request.with_merged_analysis_and_execution_source_of_truth() {
        // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:454 AnalysisPhaseRunner.runAnalysisPhase
        // PORT: the event carries the labels of the test targets (see the type);
        // AggregatingTestListener.populateTests does what populateTest does with the
        // TestAnalyzedEvents posted below.
        reporter.post(Arc::new(
            blaze_buildtool::buildtool::buildevent::TestFilteringCompleteEvent::new(
                analysis_result.get_targets_to_test().map(|targets| {
                    targets
                        .iter()
                        .map(|target| Some(target.get_label()))
                        .collect()
                }),
            ),
        ));
        post_top_level_status_events(skyframe_executor, reporter.as_ref(), &analysis_result);
    }

    Ok(analysis_result)
}

/// An event that marks the successful analysis of an aspect.
///
/// PORT: Java's class is in `TopLevelStatusEvents` (blaze_sky_analysis
/// `top_level_status_events`, not in this crate); defined here until it moves there.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:210 TopLevelStatusEvents.AspectAnalyzedEvent
#[derive(Clone, Debug)]
pub struct AspectAnalyzedEvent {
    /// `aspectKey()`.
    pub aspect_key: AspectKey,
    /// `configuredAspect()`.
    pub configured_aspect: Arc<AspectValue>,
}

impl AspectAnalyzedEvent {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:215 TopLevelStatusEvents.AspectAnalyzedEvent.create
    pub fn create(aspect_key: AspectKey, configured_aspect: Arc<AspectValue>) -> Self {
        AspectAnalyzedEvent {
            aspect_key,
            configured_aspect,
        }
    }
}

impl blaze_events::Postable for AspectAnalyzedEvent {}

/// Post the appropriate `TopLevelStatusEvents`.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:467 AnalysisPhaseRunner.postTopLevelStatusEvents
fn post_top_level_status_events(
    skyframe_executor: &SkyframeExecutor,
    reporter: &dyn ExtendedEventHandler,
    analysis_result: &AnalysisResult,
) {
    // Set membership of a configured target: one instance per lookup key in a graph.
    fn contains(targets: &[ConfiguredTarget], target: &ConfiguredTarget) -> bool {
        targets
            .iter()
            .any(|t| t.get_lookup_key() == target.get_lookup_key())
    }
    let graph = skyframe_executor.get_walkable_graph();
    for configured_target in analysis_result.get_targets_to_build() {
        reporter.post(Arc::new(TopLevelTargetAnalyzedEvent::create(
            configured_target.clone(),
        )));
        let skipped = contains(analysis_result.get_targets_to_skip(), configured_target);
        if skipped {
            reporter.post(Arc::new(TopLevelTargetSkippedEvent::create(
                configured_target.clone(),
            )));
        }

        if analysis_result
            .get_targets_to_test()
            .is_some_and(|targets_to_test| contains(targets_to_test, configured_target))
        {
            // configurationMap.get(configuredTarget.getConfigurationKey())
            let build_configuration_value = get_configuration_from_graph(
                &*graph,
                configured_target.get_configuration_key().as_ref(),
            )
            .expect("NullPointerException: buildConfigurationValue");
            let view: Arc<dyn ConfiguredTargetView> =
                Arc::new(SkyframeConfiguredTargetView::new(configured_target.clone()));
            reporter.post(Arc::new(TestAnalyzedEvent {
                configured_target: view,
                configuration_checksum: build_configuration_value.checksum().to_owned(),
                configuration_mnemonic: build_configuration_value.get_mnemonic().to_owned(),
                is_skipped: skipped,
            }));
        }
    }

    for (aspect_key, configured_aspect) in analysis_result.get_aspects_map() {
        reporter.post(Arc::new(AspectAnalyzedEvent::create(
            aspect_key.clone(),
            configured_aspect.clone(),
        )));
    }
}

/// Returns the set of labels of target patterns that represent single targets.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:541 AnalysisPhaseRunner.getExplicitTargetPatterns
fn get_explicit_target_patterns(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    relative_working_directory: &PathFragment,
    requested_target_patterns: &[String],
    keep_going: bool,
    loading_phase_threads: usize,
) -> Result<IndexSet<Label>, BuildToolError> {
    let main_repo_mapping = match skyframe_executor.get_main_repo_mapping(
        keep_going,
        loading_phase_threads,
        reporter.clone(),
    ) {
        Ok(mapping) => mapping,
        Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
            return Err(BuildToolError::Interrupted);
        }
        // BuildTool: `catch (RepositoryMappingResolutionException e) { detailedExitCode =
        // e.getDetailedExitCode(); reportExceptionError(e); }`
        Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
            return Err(BuildToolError::RepositoryMappingResolution {
                message: e.to_string(),
                detailed_exit_code: blaze_query_engine::DetailedExitCode::of(
                    crate::skyframe_error_processor::packages_failure_detail(&e.detailed_exit_code),
                ),
            });
        }
    };
    // PORT: the pattern loop is `execution_seam::get_explicit_target_patterns` (the same loop of
    // AnalysisAndExecutionPhaseRunner); a pattern that fails to parse a second time fails there.
    Ok(crate::execution_seam::get_explicit_target_patterns(
        relative_working_directory,
        &main_repo_mapping,
        requested_target_patterns,
    ))
}

// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:494 AnalysisPhaseRunner.reportTargetsWithTests
fn report_targets_with_tests(
    reporter: &dyn ExtendedEventHandler,
    targets_to_build: usize,
    test_count: usize,
) {
    let target_count = targets_to_build - test_count;
    if target_count == 0 {
        reporter.handle(&Event::info(format!(
            "Found {}{}",
            test_count,
            if test_count == 1 {
                " test target..."
            } else {
                " test targets..."
            }
        )));
    } else {
        reporter.handle(&Event::info(format!(
            "Found {}{}{}{}",
            target_count,
            if target_count == 1 {
                " target and "
            } else {
                " targets and "
            },
            test_count,
            if test_count == 1 {
                " test target..."
            } else {
                " test targets..."
            }
        )));
    }
}

// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:519 AnalysisPhaseRunner.reportTargets
fn report_targets(reporter: &dyn ExtendedEventHandler, target_count: usize) {
    reporter.handle(&Event::info(format!(
        "Found {}{}",
        target_count,
        if target_count == 1 {
            " target..."
        } else {
            " targets..."
        }
    )));
}

/// The `reportTargetsWithTests` / `reportTargets` of `AnalysisPhaseRunner.execute` (and, with
/// merged analysis and execution, of `AnalysisAndExecutionPhaseRunner.execute`, after the
/// execution phase returned).
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:156 AnalysisPhaseRunner.execute
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:135 AnalysisAndExecutionPhaseRunner.execute
pub fn report_analyzed_targets(
    reporter: &dyn ExtendedEventHandler,
    request: &BuildRequest,
    result: &AnalysisResult,
) {
    if request.should_run_tests() {
        report_targets_with_tests(
            reporter,
            result.get_targets_to_build().len(),
            result
                .get_targets_to_test()
                .expect("NullPointerException: targets to test")
                .len(),
        );
    } else {
        report_targets(reporter, result.get_targets_to_build().len());
    }
}
