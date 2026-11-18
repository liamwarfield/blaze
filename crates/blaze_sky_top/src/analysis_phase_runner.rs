//! Port of `buildtool/AnalysisPhaseRunner.java` (analysis only: target pattern results to
//! configured targets).

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_protos::failure_details::FailureDetail;

use crate::build_tool::{BuildRequest, BuildToolError};
use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::SkyframeExecutor;
use crate::target_pattern_phase::TargetPatternPhaseValue;

/// Performs target pattern eval, configuration creation, loading and analysis.
// Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:97 AnalysisPhaseRunner.execute
pub fn execute(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    request: &BuildRequest,
    target_pattern_phase_value: &TargetPatternPhaseValue,
    build_options: &Arc<BuildOptions>,
) -> Result<Option<AnalysisResult>, BuildToolError> {
    // Compute the heuristic instrumentation filter if needed.
    if request.needs_instrumentation_filter() {
        blaze_util::unported!("InstrumentationFilterSupport.computeInstrumentationFilter");
    }

    // PORT: env.throwPendingException() is checked by the command dispatcher.

    let mut analysis_result = None;
    if request.perform_analysis_phase {
        let result = run_analysis_phase(
            skyframe_executor,
            reporter,
            request,
            target_pattern_phase_value,
            build_options,
        )?;

        // PORT: BlazeModule.afterAnalysis is not dispatched (no module hooks yet).

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

        // PORT: postAbortedEventsForSkippedTargets posts BEP events (event bus).
        analysis_result = Some(result);
    } else {
        reporter.handle(&Event::progress("Loading complete."));
        // PORT: NoAnalyzeEvent is not posted (event bus).
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
    request: &BuildRequest,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
) -> Result<AnalysisResult, BuildToolError> {
    reporter.handle(&Event::progress("Loading complete.  Analyzing..."));

    // PORT: getExplicitTargetPatterns only feeds the incompatible-target checks
    // (skipIncompatibleExplicitTargets / reportIncompatibleTargets), which are not ported.

    let analysis_result = build_view::update(
        skyframe_executor,
        loading_result,
        target_options,
        &BuildViewUpdateOptions {
            keep_going: request.keep_going,
            check_for_action_conflicts: request.check_for_action_conflicts,
            aspects: request.aspects.clone(),
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
    // are not posted (event bus).
    Ok(analysis_result)
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
