//! Port of `buildtool/AnalysisPhaseRunner.java` (analysis only: target pattern results to
//! configured targets).

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_protos::failure_details::FailureDetail;
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

use crate::build_tool::{BuildRequest, BuildToolError};
use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::SkyframeExecutor;
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

        // AnalysisPhaseCompleteEvent → UiEventHandler.analysisComplete.
        report_analysis_complete(reporter.as_ref(), target_pattern_phase_value);

        // PORT: BlazeModule.afterAnalysis has no ported implementer (the modules that override
        // it are Google-internal or the remote module).

        if request.should_run_tests() {
            report_targets_with_tests(
                reporter.as_ref(),
                result.get_targets_to_build().len(),
                result
                    .get_targets_to_test()
                    .expect("NullPointerException: targets to test")
                    .len(),
            );
        } else {
            report_targets(reporter.as_ref(), result.get_targets_to_build().len());
        }

        // PORT: postAbortedEventsForSkippedTargets (an AbortedEvent per skipped target for the
        // build event stream) is the command's, from `AnalysisResult::get_targets_to_skip()`.
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

    let analysis_result = build_view::update(
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

    // PORT: AnalysisPhaseCompleteEvent, TestFilteringCompleteEvent and the TopLevelStatusEvents
    // are built by the command from the returned `AnalysisResult` (its counters are
    // `get_analysis_phase_complete_stats`).
    Ok(analysis_result)
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

/// The UI's summary of the analysis phase, `Analyzed N targets (P packages loaded, T targets
/// configured).` (Java: `UiEventHandler.analysisComplete` on `AnalysisPhaseCompleteEvent`, with
/// the labels of `LoadingPhaseCompleteEvent`).
// Java: com/google/devtools/build/lib/runtime/SkymeldUiStateTracker.java:199 SkymeldUiStateTracker.analysisComplete
fn report_analysis_complete(
    reporter: &dyn ExtendedEventHandler,
    loading_result: &TargetPatternPhaseValue,
) {
    // Java: com/google/devtools/build/lib/runtime/SkymeldUiStateTracker.java:178 SkymeldUiStateTracker.loadingComplete
    let labels = loading_result.get_target_labels();
    let additional_message = if labels.len() == 1 {
        format!("target {}", labels.iter().next().expect("one label"))
    } else {
        format!("{} targets", labels.len())
    };
    let work_done = format!(
        "Analyzed {} ({}, {}).",
        additional_message,
        blaze_sky_loading::package_progress_receiver::PACKAGE_PROGRESS.progress_state(),
        blaze_sky_analysis::analysis_progress_receiver::ANALYSIS_PROGRESS.get_progress_string()
    );
    reporter.handle(&Event::info(work_done));
}
