//! Ports of the analysis-only path of `buildtool/BuildTool.java` (`processRequest`,
//! `buildTargets`, `buildTargetsWithoutMergedAnalysisExecution`, `evaluateTargetPatterns`) and
//! the parts of `buildtool/BuildRequest.java` it reads.
//!
//! PORT: execution (`ExecutionTool`), Skymeld, PROJECT.scl evaluation and remote analysis caching
//! are not ported here; a request that needs the execution phase fails loudly. The UI's
//! `stopRequest` summary lines are not printed.

use std::sync::Arc;

use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::TargetParsingException;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_options::{Options, OptionsParser};
use blaze_protos::failure_details::{
    self, FailureDetail, Interrupted, failure_detail::Category, interrupted,
};
use blaze_query_engine::DetailedExitCode;
use blaze_vfs::PathFragment;

use crate::analysis_phase_runner;
use crate::build_view::AnalysisResult;
use crate::skyframe_executor::{SkyframeExecutor, TargetPatternPhaseError};

/// `ExitCode.PARSING_FAILURE`.
const PARSING_FAILURE: i32 = 1;
/// `Interrupted.Code.INTERRUPTED` exit code.
const INTERRUPTED_EXIT_CODE: i32 = 8;

/// A BuildRequest represents a single invocation of the build tool by a user (the parts the
/// analysis-only path reads).
// Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:57 BuildRequest
pub struct BuildRequest {
    /// `getCommandName()`.
    pub command_name: String,
    /// `getTargets()`.
    pub targets: Vec<String>,
    /// `getKeepGoing()`.
    pub keep_going: bool,
    /// `getLoadingPhaseThreadCount()`.
    pub loading_phase_threads: usize,
    /// `getLoadingOptions()`.
    pub loading_options: LoadingOptions,
    /// `getBuildOptions().performAnalysisPhase`.
    pub perform_analysis_phase: bool,
    /// `getBuildOptions().performExecutionPhase`.
    pub perform_execution_phase: bool,
    /// `getAspects()`.
    pub aspects: Vec<String>,
    /// `getCheckForActionConflicts()`.
    pub check_for_action_conflicts: bool,
    /// `reportIncompatibleTargets()`.
    pub report_incompatible_targets: bool,
    run_tests: bool,
    needs_instrumentation_filter: bool,
}

impl BuildRequest {
    /// `BuildRequest.builder().setCommandName(...).setOptions(options).setTargets(targets)
    /// .setCheckforActionConflicts(...).setReportIncompatibleTargets(...).build()`.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:225 BuildRequest.<init>
    pub fn new(
        command_name: &str,
        options: &OptionsParser,
        targets: Vec<String>,
        check_for_action_conflicts: bool,
        report_incompatible_targets: bool,
    ) -> BuildRequest {
        let keep_going = options
            .get_options::<KeepGoingOption>()
            .expect("KeepGoingOption")
            .keep_going;
        let loading_phase_threads = options
            .get_options::<LoadingPhaseThreadsOption>()
            .expect("LoadingPhaseThreadsOption")
            .threads;
        let build_options: BuildRequestOptions = options
            .get_options::<BuildRequestOptions>()
            .unwrap_or_else(Options::<BuildRequestOptions>::get_defaults);
        BuildRequest {
            command_name: command_name.to_owned(),
            targets,
            keep_going,
            loading_phase_threads: loading_phase_threads.max(1) as usize,
            loading_options: options
                .get_options::<LoadingOptions>()
                .unwrap_or_else(Options::<LoadingOptions>::get_defaults),
            perform_analysis_phase: build_options.perform_analysis_phase,
            perform_execution_phase: build_options.perform_execution_phase,
            aspects: build_options.aspects.clone(),
            check_for_action_conflicts,
            report_incompatible_targets,
            run_tests: false,
            needs_instrumentation_filter: false,
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:310 BuildRequest.shouldRunTests
    pub fn should_run_tests(&self) -> bool {
        self.run_tests
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:381 BuildRequest.needsInstrumentationFilter
    pub fn needs_instrumentation_filter(&self) -> bool {
        self.needs_instrumentation_filter
    }
}

/// The exceptions `BuildTool.buildTargets` throws (each mapped to a `DetailedExitCode` by
/// `processRequest`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildToolError {
    /// `BuildFailedException`.
    #[error("{message}")]
    BuildFailed {
        message: String,
        failure_detail: FailureDetail,
    },
    /// `TargetParsingException`.
    #[error("{}", .0.get_message())]
    TargetParsing(TargetParsingException),
    /// `ViewCreationFailedException`.
    #[error("{message}")]
    ViewCreationFailed {
        message: String,
        failure_detail: FailureDetail,
    },
    /// `InvalidConfigurationException`.
    #[error("{}", .0.get_message())]
    InvalidConfiguration(InvalidConfigurationException),
    /// `ExitException` / `AbruptExitException` (already a detailed exit code).
    #[error("{}", .0.get_failure_detail().map(|f| f.message.as_str()).unwrap_or(""))]
    Exit(DetailedExitCode),
    /// `InterruptedException`.
    #[error("interrupted")]
    Interrupted,
}

/// Something that operates on the results of the analysis phase (Java
/// `BuildTool.AnalysisPostProcessor`): cquery and aquery.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:198 BuildTool.AnalysisPostProcessor
pub trait AnalysisPostProcessor {
    fn process(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError>;
}

/// Provides the bulk of the implementation of the 'blaze build' command.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:206 BuildTool
pub struct BuildTool<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    reporter: Arc<dyn ExtendedEventHandler>,
    relative_working_directory: PathFragment,
    analysis_post_processor: &'a dyn AnalysisPostProcessor,
}

impl<'a> BuildTool<'a> {
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:225 BuildTool.<init>
    pub fn new(
        skyframe_executor: &'a SkyframeExecutor,
        reporter: Arc<dyn ExtendedEventHandler>,
        relative_working_directory: PathFragment,
        post_processor: &'a dyn AnalysisPostProcessor,
    ) -> Self {
        BuildTool {
            skyframe_executor,
            reporter,
            relative_working_directory,
            analysis_post_processor: post_processor,
        }
    }

    /// The crux of the build system: performs loading and analysis (and runs the post
    /// processor) for the targets of the request. Returns the command's detailed exit code.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:892 BuildTool.processRequest
    pub fn process_request(&self, request: &BuildRequest) -> DetailedExitCode {
        match self.build_targets(request) {
            Ok(()) => DetailedExitCode::success(),
            Err(BuildToolError::BuildFailed {
                message,
                failure_detail,
            }) => {
                // The actual error has not already been reported by the Builder.
                // PORT: keep-going BuildFailedExceptions without a message are reported at the
                // error site; this mirrors !e.isErrorAlreadyShown().
                if !message.is_empty() {
                    self.reporter.handle(&Event::error(message));
                }
                DetailedExitCode::of_exit_code(exit_code_of(&failure_detail), failure_detail)
            }
            Err(BuildToolError::Interrupted) => {
                let message = "build interrupted";
                self.reporter.handle(&Event::error(message));
                // PORT: BuildInterruptedEvent is not posted (event bus).
                DetailedExitCode::of_exit_code(
                    INTERRUPTED_EXIT_CODE,
                    FailureDetail {
                        message: message.to_owned(),
                        category: Some(Category::Interrupted(Interrupted {
                            code: interrupted::Code::Interrupted as i32,
                        })),
                    },
                )
            }
            Err(BuildToolError::TargetParsing(e)) => {
                self.report_exception_error(e.get_message());
                DetailedExitCode::of_exit_code(
                    e.get_code().exit_code(),
                    FailureDetail {
                        message: e.get_message().to_owned(),
                        category: Some(Category::TargetPatterns(failure_details::TargetPatterns {
                            code: e.get_code() as i32,
                        })),
                    },
                )
            }
            Err(BuildToolError::ViewCreationFailed {
                message,
                failure_detail,
            }) => {
                self.report_exception_error(&message);
                DetailedExitCode::of_exit_code(PARSING_FAILURE, failure_detail)
            }
            Err(BuildToolError::Exit(detailed_exit_code)) => {
                if let Some(failure_detail) = detailed_exit_code.get_failure_detail() {
                    self.report_exception_error(&failure_detail.message);
                }
                detailed_exit_code
            }
            Err(BuildToolError::InvalidConfiguration(e)) => {
                self.report_exception_error(e.get_message());
                invalid_configuration_detailed_exit_code(&e)
            }
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:852 BuildTool.reportExceptionError
    fn report_exception_error(&self, message: &str) {
        self.reporter.handle(&Event::error(message));
    }

    /// The crux of the build system. Builds the targets specified in the request.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:257 BuildTool.buildTargets
    fn build_targets(&self, request: &BuildRequest) -> Result<(), BuildToolError> {
        // PORT: validateOptions only issues warnings for execution options.
        // buildOptions = runtime.createBuildOptions(request): the executor was created with
        // the request's build options as its baseline (see skyframe_executor.rs).
        let build_options = self
            .skyframe_executor
            .get_baseline_configuration()
            .cloned()
            .expect("IllegalStateException: the executor has no build options");

        // PORT: BuildStartingEvent is not posted (event bus); initializeOutputFilter belongs to
        // the UI.
        let target_pattern_phase_value = self.evaluate_target_patterns(request)?;
        // PORT: ExecRootEvent is not posted; PROJECT.scl evaluation (evaluateProjectFile) is not
        // ported (no project files in the corpus).

        // buildTargetsWithoutMergedAnalysisExecution
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:496 BuildTool.buildTargetsWithoutMergedAnalysisExecution
        let analysis_result = analysis_phase_runner::execute(
            self.skyframe_executor,
            &self.reporter,
            request,
            &target_pattern_phase_value,
            &build_options,
        )?;
        if request.perform_analysis_phase {
            let analysis_result =
                analysis_result.expect("analysis result of a request that analyzes");
            self.analysis_post_processor.process(
                request,
                self.skyframe_executor,
                &analysis_result,
            )?;

            if request.perform_execution_phase {
                blaze_util::unported!("ExecutionTool.executeBuild");
            }
            // PORT: NoExecutionEvent is not posted (event bus).
            if let Some(delayed_failure_detail) = analysis_result.get_failure_detail() {
                return Err(BuildToolError::BuildFailed {
                    message: delayed_failure_detail.message.clone(),
                    failure_detail: delayed_failure_detail.clone(),
                });
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:468 BuildTool.evaluateTargetPatterns
    fn evaluate_target_patterns(
        &self,
        request: &BuildRequest,
    ) -> Result<crate::target_pattern_phase::TargetPatternPhaseValue, BuildToolError> {
        self.skyframe_executor
            .load_target_patterns_with_filters(
                self.reporter.clone(),
                &request.targets,
                &self.relative_working_directory,
                &request.loading_options,
                request.loading_phase_threads,
                request.keep_going,
                request.should_run_tests(),
            )
            .map_err(|e| match e {
                TargetPatternPhaseError::TargetParsing(e) => BuildToolError::TargetParsing(e),
                TargetPatternPhaseError::Interrupted => BuildToolError::Interrupted,
            })
        // PORT: no TargetValidator is passed by cquery/aquery.
    }
}

/// `DetailedExitCode.of(failureDetail)` for the categories the analysis phase produces: the
/// numeric exit code from the code's `(metadata) = { exit_code: N }` in failure_details.proto.
pub fn exit_code_of(failure_detail: &FailureDetail) -> i32 {
    match &failure_detail.category {
        Some(Category::Analysis(a)) => match a.code() {
            // All Analysis codes the analysis phase reports have exit code 1.
            failure_details::analysis::Code::GenericLoadingPhaseFailure
            | failure_details::analysis::Code::NotAllTargetsAnalyzed
            | failure_details::analysis::Code::UnexpectedAnalysisException
            | failure_details::analysis::Code::ConfiguredValueCreationFailed => 1,
            other => blaze_util::unported!(
                "DetailedExitCode.getNumericExitCode",
                "exit code of Analysis.{:?}",
                other
            ),
        },
        Some(Category::TargetPatterns(t)) => {
            blaze_query_engine::detailed_exit_code::target_patterns_code_exit_code(t.code)
        }
        Some(Category::Interrupted(_)) => INTERRUPTED_EXIT_CODE,
        Some(Category::BuildConfiguration(b)) => build_configuration_exit_code(b.code),
        _ => DetailedExitCode::get_exit_code_for(failure_detail),
    }
}

/// `BuildConfiguration.Code` exit codes (failure_details.proto).
fn build_configuration_exit_code(code: i32) -> i32 {
    match code {
        // BUILD_CONFIGURATION_UNKNOWN
        0 => 37,
        // PLATFORM_MAPPINGS_FILE_IS_DIRECTORY, PLATFORM_MAPPINGS_FILE_NOT_FOUND,
        // TOP_LEVEL_CONFIGURATION_CREATION_FAILURE
        2..=4 => 1,
        _ => 2,
    }
}

/// `InvalidConfigurationException.getDetailedExitCode()`.
pub fn invalid_configuration_detailed_exit_code(
    e: &InvalidConfigurationException,
) -> DetailedExitCode {
    let code = failure_details::build_configuration::Code::from_str_name(e.code)
        .unwrap_or(failure_details::build_configuration::Code::BuildConfigurationUnknown);
    DetailedExitCode::of_exit_code(
        build_configuration_exit_code(code as i32),
        FailureDetail {
            message: e.get_message().to_owned(),
            category: Some(Category::BuildConfiguration(
                failure_details::BuildConfiguration { code: code as i32 },
            )),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_configuration_exit_codes_follow_the_proto_metadata() {
        assert_eq!(build_configuration_exit_code(0), 37);
        assert_eq!(build_configuration_exit_code(1), 2);
        assert_eq!(build_configuration_exit_code(3), 1);
        assert_eq!(build_configuration_exit_code(6), 2);
    }
}
