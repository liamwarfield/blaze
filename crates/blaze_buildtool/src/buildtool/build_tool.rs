//! The execution half of `BuildTool.buildTargetsWithMergedAnalysisExecution` (Skymeld): what
//! runs once the target patterns are resolved and the top-level `BuildDriverKey`s exist.
//!
//! Seam for the analysis side: it builds the `Evaluator` with its own SkyFunctions plus
//! `blaze_sky_exec::register_execution_functions(builder, prepared.executor, seam, ..)`, creates
//! one `BuildDriverKey` per top-level target (`TestType` from `TestTypeResolver`), and calls
//! [`execute_top_level_targets`]; then prints with the summary helpers in
//! `execution_tool`; `build_result_printer::report_targets(_with_tests)` prints the
//! `Found N targets...` line.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_events::ExtendedEventHandler;
use blaze_sky_exec::keys::BuildDriverKey;
use blaze_skyframe::{Evaluator, Interrupted};

use crate::buildtool::execution_tool::{
    ExecutionEventCollector, ExecutionPhaseResult, ExecutionToolConfig, build_complete_event,
    evaluate_build_driver_keys, report_build_summary_stats,
};

/// Evaluates the top-level targets (analysis and execution together) plus the workspace status
/// artifacts.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:595 BuildTool.buildTargetsWithMergedAnalysisExecution
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:640 SkyframeBuildView.analyzeAndExecuteTargets
pub fn execute_top_level_targets(
    evaluator: &Evaluator,
    build_driver_keys: &[BuildDriverKey],
    workspace_status_artifacts: &[Artifact],
    config: &ExecutionToolConfig,
    reporter: Arc<dyn ExtendedEventHandler>,
) -> Result<ExecutionPhaseResult, Interrupted> {
    evaluate_build_driver_keys(
        evaluator,
        build_driver_keys,
        workspace_status_artifacts,
        config,
        reporter,
    )
}

/// The `BuildCompleteEvent` subscribers' lines: elapsed time, process counts, and
/// `Build completed successfully, N total actions`.
pub fn report_build_complete(
    reporter: &dyn ExtendedEventHandler,
    collector: &ExecutionEventCollector,
    result: &ExecutionPhaseResult,
    elapsed_seconds: f64,
    stop_time_millis: i64,
    failed_tests: i32,
) {
    report_build_summary_stats(reporter, collector, elapsed_seconds, stop_time_millis);
    reporter.handle(&build_complete_event(
        !result.evaluation.has_error(),
        failed_tests,
        collector,
    ));
}
