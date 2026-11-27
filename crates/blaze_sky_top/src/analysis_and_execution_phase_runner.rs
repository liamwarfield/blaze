//! Port of `buildtool/AnalysisAndExecutionPhaseRunner.java`: the analysis and the execution of a
//! build with merged analysis and execution (Skymeld) in one Skyframe evaluation.

use std::sync::Arc;

use blaze_build_config::build_options::BuildOptions;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_util::profiler::{ProfilePhase, Profiler};
use blaze_vfs::PathFragment;

use crate::build_tool::{BuildRequest, BuildToolError, ExecutionPhase};
use crate::build_view::{self, AnalysisResult, BuildViewError, BuildViewUpdateOptions};
use crate::skyframe_executor::SkyframeExecutor;
use crate::target_pattern_phase::TargetPatternPhaseValue;

/// What [`execute`] throws besides a `BuildToolError`: the `BuildFailedException` or
/// `TestExecException` of the execution phase (`SkyframeErrorProcessor.rethrow`).
#[derive(Debug, Clone)]
pub enum AnalysisAndExecutionError {
    BuildTool(BuildToolError),
    Execution(crate::skyframe_error_processor::RethrownException),
}

impl From<BuildToolError> for AnalysisAndExecutionError {
    fn from(e: BuildToolError) -> Self {
        AnalysisAndExecutionError::BuildTool(e)
    }
}

/// `AnalysisAndExecutionPhaseRunner.execute`: the analysis and execution of the top-level
/// targets, then the "Found N targets..." line and the `AbortedEvent`s of the skipped targets.
/// `None` when the request does not analyze.
///
/// PORT: `executionSetupCallback`, `buildConfigurationCreatedCallback` and
/// `buildDriverKeyTestContext` come from the command's execution phase
/// ([`ExecutionPhase::merged_analysis_and_execution_args`]); `env.throwPendingException()` is
/// `throw_pending_exception` (`AnalysisPostProcessor::throw_pending_exception`).
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:74 AnalysisAndExecutionPhaseRunner.execute
pub fn execute(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    relative_working_directory: &PathFragment,
    request: &BuildRequest,
    build_options: &Arc<BuildOptions>,
    loading_result: &TargetPatternPhaseValue,
    execution_phase: &dyn ExecutionPhase,
    throw_pending_exception: &dyn Fn() -> Result<(), BuildToolError>,
) -> Result<Option<AnalysisResult>, AnalysisAndExecutionError> {
    // Compute the heuristic instrumentation filter if needed.
    let patched_build_options =
        crate::analysis_phase_runner::compute_heuristic_instrumentation_filter(
            skyframe_executor,
            reporter,
            request,
            loading_result,
            build_options,
        )?;
    let build_options = patched_build_options.as_ref().unwrap_or(build_options);

    // Exit if there are any pending exceptions from modules.
    throw_pending_exception()?;

    let mut analysis_and_execution_result = None;
    if request.perform_analysis_phase {
        Profiler::instance().mark_phase(ProfilePhase::ANALYZE_AND_EXECUTE);

        let result = {
            let _c = Profiler::instance().profile_info("runAnalysisAndExecutionPhase");
            // TopLevelTargetAnalysisWatcher.createAndRegisterWithEventBus(...); closed when the
            // try-with-resources block ends.
            execution_phase.register_top_level_target_analysis_watcher(request);
            let result = run_analysis_and_execution_phase(
                skyframe_executor,
                reporter,
                relative_working_directory,
                request,
                loading_result,
                build_options,
                execution_phase,
            );
            execution_phase.close_top_level_target_analysis_watcher();
            result?
        };

        let build_result_listener = execution_phase.build_result_listener();
        if request.should_run_tests() {
            crate::analysis_phase_runner::report_targets_with_tests(
                reporter.as_ref(),
                build_result_listener.get_analyzed_targets().len(),
                build_result_listener.get_analyzed_tests().len(),
            );
        } else {
            crate::analysis_phase_runner::report_targets(
                reporter.as_ref(),
                build_result_listener.get_analyzed_targets().len(),
            );
        }

        // AnalysisPhaseRunner.postAbortedEventsForSkippedTargets(env, skippedTargets)
        execution_phase
            .post_aborted_events_for_skipped_targets(&build_result_listener.get_skipped_targets());
        analysis_and_execution_result = Some(result);
    } else {
        reporter.handle(&Event::progress("Loading complete."));
        // env.getReporter().post(new NoAnalyzeEvent()): posted by the caller through
        // `BuildEventHooks::no_analyze` (the event is blaze_bep's), right after this returns.
        // PORT: skipped, logger.atInfo() ("No analysis requested, so finished").
        if let Some(failure_detail) =
            build_view::create_analysis_failure_detail(loading_result, None)
        {
            return Err(BuildToolError::BuildFailed {
                message: failure_detail.message.clone(),
                failure_detail,
            }
            .into());
        }
    }

    Ok(analysis_and_execution_result)
}

/// Performs all phases of the build: Setup, Loading, Analysis & Execution.
// Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:173 AnalysisAndExecutionPhaseRunner.runAnalysisAndExecutionPhase
fn run_analysis_and_execution_phase(
    skyframe_executor: &SkyframeExecutor,
    reporter: &Arc<dyn ExtendedEventHandler>,
    relative_working_directory: &PathFragment,
    request: &BuildRequest,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
    execution_phase: &dyn ExecutionPhase,
) -> Result<AnalysisResult, AnalysisAndExecutionError> {
    reporter.handle(&Event::progress("Loading complete.  Analyzing..."));

    let explicit_target_patterns = crate::analysis_phase_runner::get_explicit_target_patterns(
        skyframe_executor,
        reporter,
        relative_working_directory,
        &request.targets,
        request.keep_going,
        request.loading_phase_threads,
    )?;

    // PORT: `request.getTopLevelArtifactContext()` reaches the conflict pruning of
    // `handleActionConflicts` through the executor (see `skyframe_build_view::configure_targets`).
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
    let execution_args = execution_phase.merged_analysis_and_execution_args(request);
    // TODO(b/199053098) TestFilteringCompleteEvent.
    build_view::update_with_execution(
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
            include_execution_phase: true,
        },
        &aspects_parameters,
        Some(&execution_args),
        reporter.clone(),
    )
    .map_err(|e| match e {
        BuildViewError::ViewCreationFailed {
            message,
            failure_detail,
        } => AnalysisAndExecutionError::BuildTool(BuildToolError::ViewCreationFailed {
            message,
            failure_detail,
        }),
        BuildViewError::InvalidConfiguration(e) => {
            AnalysisAndExecutionError::BuildTool(BuildToolError::InvalidConfiguration(e))
        }
        BuildViewError::Interrupted => {
            AnalysisAndExecutionError::BuildTool(BuildToolError::Interrupted)
        }
        BuildViewError::Execution(e) => AnalysisAndExecutionError::Execution(e),
        BuildViewError::BuildTool(e) => AnalysisAndExecutionError::BuildTool(*e),
    })
}
