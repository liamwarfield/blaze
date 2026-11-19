//! Ports of the analysis-only path of `buildtool/BuildTool.java` (`processRequest`,
//! `buildTargets`, `buildTargetsWithoutMergedAnalysisExecution`, `evaluateTargetPatterns`) and
//! the parts of `buildtool/BuildRequest.java` it reads.
//!
//! PORT: `buildTargetsWithoutMergedAnalysisExecution` is the one path (Skymeld's
//! `buildTargetsWithMergedAnalysisExecution` is not ported); the execution phase is the
//! command's (`ExecutionPhase`). A build whose targets have a PROJECT.scl and remote analysis
//! caching fail loudly (`project.rs`). `stopRequest` (the build-completion lines) belongs to the
//! command.

use std::sync::Arc;

use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::TargetParsingException;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::command::analysis_options::AnalysisOptions;
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::loading_options::LoadingOptions;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::remote_analysis_caching_options::RemoteAnalysisCachingOptions;
use blaze_options::{Options, OptionsParser};
use blaze_protos::failure_details::{
    self, FailureDetail, Interrupted, failure_detail::Category, interrupted,
};
use blaze_query_engine::DetailedExitCode;
use blaze_vfs::PathFragment;

use crate::analysis_phase_runner;
use crate::build_view::AnalysisResult;
use crate::skyframe_executor::{SkyframeExecutor, TargetPatternPhaseDetailedError};

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
    /// `getViewOptions().skipIncompatibleExplicitTargets`.
    pub skip_incompatible_explicit_targets: bool,
    /// `getOptions(RemoteAnalysisCachingOptions.class)` (`None`: the command has no such
    /// options).
    remote_analysis_caching_options: Option<RemoteAnalysisCachingOptions>,
    /// `getBuildOptions()`.
    build_request_options: BuildRequestOptions,
    /// `getExecutionOptions().localTestJobs` (`None`: the command has no `ExecutionOptions`).
    local_test_jobs: Option<i32>,
    run_tests: bool,
    needs_instrumentation_filter: bool,
    /// `CommandEnvironment.withMergedAnalysisAndExecutionSourceOfTruth()` for this request.
    merged_analysis_and_execution: bool,
    /// The `targetsForProjectResolution` argument of `BuildTool.processRequest` (cquery sets it
    /// to the first pattern of a `somepath()` / `allpaths()` query; `None` otherwise).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:262 BuildTool.buildTargets
    pub targets_for_project_resolution: Option<Vec<String>>,
}

/// `ExtendedEventHandler.NOOP` as an owned handler (blaze_events' `NullEventHandler` has only a
/// static instance).
// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:25 ExtendedEventHandler.NOOP
struct NoopHandler;

impl blaze_events::EventHandler for NoopHandler {
    fn handle(&self, _event: &Event) {}

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for NoopHandler {
    fn post(&self, _obj: Arc<dyn blaze_events::Postable>) {}
}

/// `AspectCollection.VALIDATION_ASPECT_NAME`.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:153 AspectCollection.VALIDATION_ASPECT_NAME
const VALIDATION_ASPECT_NAME: &str = "ValidateTarget";

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
            aspects: Self::get_aspects(&build_options),
            check_for_action_conflicts,
            report_incompatible_targets,
            // Java: getViewOptions() is getOptions(AnalysisOptions.class).
            skip_incompatible_explicit_targets: options
                .get_options::<AnalysisOptions>()
                .unwrap_or_else(Options::<AnalysisOptions>::get_defaults)
                .skip_incompatible_explicit_targets,
            remote_analysis_caching_options: options.get_options::<RemoteAnalysisCachingOptions>(),
            build_request_options: build_options,
            local_test_jobs: options
                .get_options::<ExecutionOptions>()
                .map(|o| o.local_test_jobs),
            run_tests: false,
            needs_instrumentation_filter: false,
            merged_analysis_and_execution: false,
            targets_for_project_resolution: None,
        }
    }

    /// The aspects of the request: `--aspects`, and the validation aspect when
    /// `--experimental_use_validation_aspect` asks for it.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:420 BuildRequest.getAspects
    fn get_aspects(build_options: &BuildRequestOptions) -> Vec<String> {
        let aspects = &build_options.aspects;
        let mut result: Vec<String> = aspects.clone();
        if !aspects.iter().any(|a| a == VALIDATION_ASPECT_NAME)
            && Self::use_validation_aspect(build_options)
        {
            result.push(VALIDATION_ASPECT_NAME.to_owned());
        }
        result
    }

    /// Whether `ValidateTarget` is in use.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:447 BuildRequest.useValidationAspect
    fn use_validation_aspect(build_options: &BuildRequestOptions) -> bool {
        // validationMode() == OutputGroupInfo.ValidationMode.ASPECT
        // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:452 BuildRequest.validationMode
        build_options.run_validation_actions && build_options.use_validation_aspect
    }

    /// Validates the options for this BuildRequest.
    ///
    /// Issues warnings for the use of deprecated options, and warnings or errors for any option
    /// settings that conflict.
    ///
    /// PORT: a command without `ExecutionOptions` (Java reads them unconditionally; every command
    /// that builds a request has them) has no warnings.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:393 BuildRequest.validateOptions
    pub fn validate_options(&self) -> Vec<String> {
        let mut warnings: Vec<String> = Vec::new();

        if let Some(local_test_jobs) = self.local_test_jobs {
            let jobs = self.build_request_options.jobs;
            if local_test_jobs > jobs {
                warnings.push(format!(
                    "High value for --local_test_jobs: {}. This exceeds the value for --jobs: \
                     {}. Only up to {} local tests will run concurrently.",
                    local_test_jobs, jobs, jobs
                ));
            }
        }

        warnings
    }

    /// The `--aspects_parameters` of the request as a map (a parameter given twice is an error).
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:430 BuildRequest.getAspectsParameters
    pub fn get_aspects_parameters(
        &self,
    ) -> Result<
        indexmap::IndexMap<String, String>,
        crate::skyframe_error_processor::ViewCreationFailedException,
    > {
        let aspects_parameters_list = &self.build_request_options.aspects_parameters;
        // aspectsParametersList.stream().collect(toImmutableMap(Map.Entry::getKey,
        // Map.Entry::getValue)): IllegalArgumentException on a duplicate key.
        let mut result: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        for (key, value) in aspects_parameters_list {
            if result.contains_key(key) {
                let error_message = "Error in top-level aspects parameters";
                // The cause is Guava's IllegalArgumentException from ImmutableMap's
                // builder ("Multiple entries with same key: k=v1 and k=v2"); the three-argument
                // constructor appends its message.
                return Err(crate::skyframe_error_processor::ViewCreationFailedException::with_cause(
                    error_message,
                    blaze_protos::failure_details::FailureDetail {
                        message: error_message.to_owned(),
                        category: Some(blaze_protos::failure_details::failure_detail::Category::Analysis(
                            blaze_protos::failure_details::Analysis {
                                code: blaze_protos::failure_details::analysis::Code::AspectCreationFailed
                                    as i32,
                            },
                        )),
                    },
                    Some(&format!(
                        "Multiple entries with same key: {}={} and {}={}",
                        key, result[key], key, value
                    )),
                ));
            }
            result.insert(key.clone(), value.clone());
        }
        Ok(result)
    }

    /// `getTopLevelArtifactContext().outputGroups()`: the requested output groups.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:410 BuildRequest.getTopLevelArtifactContext
    pub fn get_top_level_artifact_context_output_groups(&self) -> Vec<String> {
        crate::execution_seam::top_level_output_groups(
            &self.build_request_options,
            self.should_run_tests(),
        )
    }

    /// `getOptions(RemoteAnalysisCachingOptions.class)`.
    pub fn get_remote_analysis_caching_options(&self) -> Option<&RemoteAnalysisCachingOptions> {
        self.remote_analysis_caching_options.as_ref()
    }

    /// `getBuildOptions()`.
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:330 BuildRequest.getBuildOptions
    pub fn get_build_options(&self) -> &BuildRequestOptions {
        &self.build_request_options
    }

    /// Records `CommandEnvironment.setMergedAnalysisAndExecution(value)` for this request
    /// (`SkymeldModule.determineIfMergingAnalysisExecution`, decided by the command).
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:580 CommandEnvironment.setMergedAnalysisAndExecution
    pub fn set_merged_analysis_and_execution(&mut self, value: bool) {
        self.merged_analysis_and_execution = value;
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:576 CommandEnvironment.withMergedAnalysisAndExecutionSourceOfTruth
    pub fn with_merged_analysis_and_execution_source_of_truth(&self) -> bool {
        self.merged_analysis_and_execution
    }

    /// `BuildRequest.Builder.setRunTests(true)` (the `test` command).
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:190 BuildRequest.Builder.setRunTests
    pub fn set_run_tests(&mut self, run_tests: bool) {
        self.run_tests = run_tests;
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
    /// `TargetParsingException`, with its `getDetailedExitCode()`.
    #[error("{}", .exception.get_message())]
    TargetParsing {
        exception: TargetParsingException,
        detailed_exit_code: DetailedExitCode,
    },
    /// `LoadingFailedException`.
    #[error("{0}")]
    LoadingFailed(crate::project::LoadingFailedException),
    /// `RepositoryMappingResolutionException` (its message and `getDetailedExitCode()`).
    #[error("{message}")]
    RepositoryMappingResolution {
        message: String,
        detailed_exit_code: DetailedExitCode,
    },
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

/// The execution phase of a build (Java `ExecutionTool.executeBuild` and the Skymeld
/// execution of `BuildTool.buildTargetsWithMergedAnalysisExecution`), implemented by the
/// command, which holds the execution options and the event subscribers.
pub trait ExecutionPhase {
    fn execute_build(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        loading_result: &crate::target_pattern_phase::TargetPatternPhaseValue,
        analysis_result: &AnalysisResult,
    ) -> Result<(), BuildToolError>;
}

/// Validates the targets of a build request before building begins (`run`: exactly one
/// executable target).
// Java: com/google/devtools/build/lib/buildtool/TargetValidator.java:26 TargetValidator
pub trait TargetValidator {
    /// Hook for subclasses to validate a build request before building begins. Implementors
    /// should print warnings for invalid targets iff keepGoing.
    // Java: com/google/devtools/build/lib/buildtool/TargetValidator.java:36 TargetValidator.validateTargets
    fn validate_targets(
        &self,
        targets: &[blaze_packages::target::Target],
        keep_going: bool,
    ) -> Result<(), crate::project::LoadingFailedException>;
}

/// The build events `BuildTool` and the functions it runs post on the command's event bus, at
/// their points of the build.
///
/// PORT: the events are built by the command's build event stream support (blaze_runtime
/// commands/bep.rs), which reads them from these values; this crate sits below it.
pub trait BuildEventHooks {
    /// At the start of the `try` block of `BuildTool.buildTargets`: `BuildStartingEvent`.
    ///
    /// PORT: `build`, `test` and `run` post it themselves before calling `process_request`
    /// (blaze_runtime commands/build.rs, where OutputFilteringModule's subscriber runs too); the
    /// commands whose `BuildTool` is constructed in another crate (`cquery`, `aquery`) get it
    /// here, where Java posts it.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:288 BuildTool.buildTargets
    fn build_starting(&self, _request: &BuildRequest) {}

    /// After the target patterns were evaluated: `TargetParsingCompleteEvent`
    /// (`TargetPatternPhaseFunction`) and `ExecRootEvent` (`BuildTool.buildTargets`).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:311 BuildTool.buildTargets
    fn target_patterns_evaluated(
        &self,
        _request: &BuildRequest,
        _skyframe_executor: &SkyframeExecutor,
        _target_pattern_phase_value: &crate::target_pattern_phase::TargetPatternPhaseValue,
    ) {
    }

    /// After the project file was evaluated: the release of the tentative canonical command
    /// line (`ReleaseReplaceableBuildEvent`).
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:343 BuildTool.buildTargets
    fn project_evaluated(&self) {}

    /// The finally block of `BuildTool.buildTargets` when there was no catastrophe:
    /// `env.ensureBuildInfoPosted()`. The workspace status actions will not run with certain
    /// flags, or if an error occurs early in the build; build info is posted on every build.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:450 BuildTool.buildTargets
    fn ensure_build_info_posted(&self) {}

    /// A request that analyzes but does not execute: `NoExecutionEvent`.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:560 BuildTool.buildTargetsWithoutMergedAnalysisExecution
    fn no_execution(&self) {}

    /// After a successful analysis: the analysis metrics (`AnalysisPhaseRunner`) and a
    /// `TargetConfiguredEvent` per top-level target.
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:431 AnalysisPhaseRunner.execute
    fn analysis_complete(
        &self,
        _request: &BuildRequest,
        _skyframe_executor: &SkyframeExecutor,
        _target_pattern_phase_value: &crate::target_pattern_phase::TargetPatternPhaseValue,
        _analysis_result: &AnalysisResult,
    ) {
    }
}

/// Provides the bulk of the implementation of the 'blaze build' command.
// Java: com/google/devtools/build/lib/buildtool/BuildTool.java:206 BuildTool
pub struct BuildTool<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    reporter: Arc<dyn ExtendedEventHandler>,
    relative_working_directory: PathFragment,
    analysis_post_processor: &'a dyn AnalysisPostProcessor,
    execution_phase: Option<&'a dyn ExecutionPhase>,
    /// The `validator` argument of `processRequest` (`null` but for `run`).
    validator: Option<&'a dyn TargetValidator>,
    /// The build event stream's posting points (`None` without a build event stream).
    build_event_hooks: Option<&'a dyn BuildEventHooks>,
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
            execution_phase: None,
            validator: None,
            build_event_hooks: None,
        }
    }

    /// The build event stream's posting points.
    pub fn with_build_event_hooks(mut self, hooks: &'a dyn BuildEventHooks) -> Self {
        self.build_event_hooks = Some(hooks);
        self
    }

    /// The target validator `processRequest` passes to `evaluateTargetPatterns` (`run`).
    pub fn with_target_validator(mut self, validator: &'a dyn TargetValidator) -> Self {
        self.validator = Some(validator);
        self
    }

    /// A build tool whose requests may execute (`build`, `test`).
    pub fn with_execution_phase(mut self, execution_phase: &'a dyn ExecutionPhase) -> Self {
        self.execution_phase = Some(execution_phase);
        self
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
                // Java: env.getEventBus().post(new BuildInterruptedEvent()).
                // PORT: not posted here: the event's class lives in blaze_bep, which this crate
                // does not depend on. The command has to post it when it gets this exit code
                // back (the test summary and the build event stream subscribe to it).
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
            // catch (TargetParsingException | LoadingFailedException e)
            Err(BuildToolError::TargetParsing {
                exception,
                detailed_exit_code,
            }) => {
                self.report_exception_error(exception.get_message());
                detailed_exit_code
            }
            Err(BuildToolError::LoadingFailed(e)) => {
                self.report_exception_error(&e.message);
                DetailedExitCode::of_exit_code(e.exit_code, e.failure_detail)
            }
            // catch (RepositoryMappingResolutionException e)
            Err(BuildToolError::RepositoryMappingResolution {
                message,
                detailed_exit_code,
            }) => {
                self.report_exception_error(&message);
                detailed_exit_code
            }
            Err(BuildToolError::ViewCreationFailed {
                message,
                failure_detail,
            }) => {
                self.report_exception_error(&message);
                DetailedExitCode::of_code(blaze_util::ExitCode::PARSING_FAILURE, failure_detail)
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
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1211 BuildTool.validateOptions
        for issue in request.validate_options() {
            self.reporter.handle(&Event::warn(issue));
        }
        // try { ... } finally { if (!catastrophe) { ... env.ensureBuildInfoPosted(); } }
        // PORT: a catastrophe (Error | RuntimeException) is a panic that unwinds past this
        // point, so the finally block runs on every return.
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:436 BuildTool.buildTargets
        if let Some(hooks) = self.build_event_hooks {
            hooks.build_starting(request);
        }
        let result = self.build_targets_try(request);
        if let Some(hooks) = self.build_event_hooks {
            hooks.ensure_build_info_posted();
        }
        result
    }

    /// The `try` block of `BuildTool.buildTargets`.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:286 BuildTool.buildTargets
    fn build_targets_try(&self, request: &BuildRequest) -> Result<(), BuildToolError> {
        // buildOptions = runtime.createBuildOptions(request): the executor was created with
        // the request's build options as its baseline (see skyframe_executor.rs).
        let build_options = self
            .skyframe_executor
            .get_baseline_configuration()
            .expect("IllegalStateException: the executor has no build options");

        // PORT: the progress receivers are process-wide (one command per process); Java resets
        // them when the build's loading starts (SkyframeExecutor / SkyframeBuildView).
        // PORT: the package progress is not reset here: Java resets it in
        // SkyframeExecutor.preparePackageLoading, before the build settings of the command line
        // are loaded (BlazeOptionHandler.parseStarlarkOptions), and counts those packages too
        // (blaze_sky_loading::package_progress_receiver, module comment).
        // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1742 SkyframeExecutor.preparePackageLoading
        blaze_sky_analysis::analysis_progress_receiver::ANALYSIS_PROGRESS.reset();
        // PORT: BuildStartingEvent and ExecRootEvent are posted by the command, which builds
        // the build event stream (blaze_runtime commands/bep.rs); initializeOutputFilter belongs
        // to the UI.
        let target_pattern_phase_value = self.evaluate_target_patterns(request)?;
        if let Some(hooks) = self.build_event_hooks {
            hooks.target_patterns_evaluated(
                request,
                self.skyframe_executor,
                &target_pattern_phase_value,
            );
        }

        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:315 BuildTool.buildTargets
        let target_patterns_for_project_resolution = match &request.targets_for_project_resolution {
            None => None,
            // Don't report this because it'll throw off our tracking of the complete target set.
            Some(targets) => Some(
                self.skyframe_executor
                    .load_target_patterns_with_filters_detailed(
                        Arc::new(NoopHandler),
                        targets,
                        &self.relative_working_directory,
                        &request.loading_options,
                        request.loading_phase_threads,
                        request.keep_going,
                        request.should_run_tests(),
                    )
                    .map_err(|e| match e {
                        TargetPatternPhaseDetailedError::TargetParsing {
                            exception,
                            detailed_exit_code,
                        } => BuildToolError::TargetParsing {
                            exception,
                            detailed_exit_code,
                        },
                        TargetPatternPhaseDetailedError::Interrupted => BuildToolError::Interrupted,
                    })?,
            ),
        };
        let target_patterns_for_project_resolution = target_patterns_for_project_resolution
            .as_ref()
            .unwrap_or(&target_pattern_phase_value);
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:343 BuildTool.buildTargets
        crate::project::evaluate_project_file(
            request.get_remote_analysis_caching_options(),
            request.get_build_options(),
            &build_options,
            target_patterns_for_project_resolution.get_non_expanded_labels(),
            self.skyframe_executor,
            self.reporter.clone(),
        )
        .map_err(BuildToolError::LoadingFailed)?;
        if let Some(hooks) = self.build_event_hooks {
            hooks.project_evaluated();
        }

        // buildTargetsWithoutMergedAnalysisExecution
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:496 BuildTool.buildTargetsWithoutMergedAnalysisExecution
        let analysis_result = analysis_phase_runner::execute(
            self.skyframe_executor,
            &self.reporter,
            &self.relative_working_directory,
            request,
            &target_pattern_phase_value,
            &build_options,
        )?;
        if request.perform_analysis_phase {
            let analysis_result =
                analysis_result.expect("analysis result of a request that analyzes");
            if let Some(hooks) = self.build_event_hooks {
                hooks.analysis_complete(
                    request,
                    self.skyframe_executor,
                    &target_pattern_phase_value,
                    &analysis_result,
                );
            }
            self.analysis_post_processor.process(
                request,
                self.skyframe_executor,
                &analysis_result,
            )?;

            if request.perform_execution_phase {
                match self.execution_phase {
                    Some(execution_phase) => execution_phase.execute_build(
                        request,
                        self.skyframe_executor,
                        &target_pattern_phase_value,
                        &analysis_result,
                    )?,
                    None => blaze_util::unported!("ExecutionTool.executeBuild"),
                }
                // AnalysisAndExecutionPhaseRunner.execute reports the targets once the merged
                // phase returned (analysis_phase_runner::execute did not).
                // Java: com/google/devtools/build/lib/buildtool/AnalysisAndExecutionPhaseRunner.java:135 AnalysisAndExecutionPhaseRunner.execute
                if request.with_merged_analysis_and_execution_source_of_truth() {
                    analysis_phase_runner::report_analyzed_targets(
                        self.reporter.as_ref(),
                        request,
                        &analysis_result,
                    );
                }
            } else if let Some(hooks) = self.build_event_hooks {
                // env.getReporter().post(new NoExecutionEvent())
                // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:560 BuildTool.buildTargetsWithoutMergedAnalysisExecution
                hooks.no_execution();
            }
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
        let result = self
            .skyframe_executor
            .load_target_patterns_with_filters_detailed(
                self.reporter.clone(),
                &request.targets,
                &self.relative_working_directory,
                &request.loading_options,
                request.loading_phase_threads,
                request.keep_going,
                request.should_run_tests(),
            )
            .map_err(|e| match e {
                TargetPatternPhaseDetailedError::TargetParsing {
                    exception,
                    detailed_exit_code,
                } => BuildToolError::TargetParsing {
                    exception,
                    detailed_exit_code,
                },
                TargetPatternPhaseDetailedError::Interrupted => BuildToolError::Interrupted,
            })?;
        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:488 BuildTool.evaluateTargetPatterns
        if let Some(validator) = self.validator {
            let target_labels = result
                .get_targets(
                    self.reporter.clone(),
                    self.skyframe_executor.get_package_manager(),
                )
                .map_err(|_| BuildToolError::Interrupted)?;
            validator
                .validate_targets(&target_labels, request.keep_going)
                .map_err(BuildToolError::LoadingFailed)?;
        }
        Ok(result)
    }
}

/// `DetailedExitCode.of(failureDetail)` for the categories the analysis phase produces: the
/// numeric exit code from the code's `(metadata) = { exit_code: N }` in failure_details.proto.
pub fn exit_code_of(failure_detail: &FailureDetail) -> i32 {
    match &failure_detail.category {
        // The generated FailureDetails metadata (failure_details.proto `exit_code`).
        Some(Category::Analysis(_)) => {
            blaze_util::DetailedExitCode::get_numeric_exit_code(failure_detail)
        }
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
    // An InvalidConfigurationException that wraps a ToolchainException keeps its Toolchain code
    // (SkyframeExecutor.getConfiguration: `new InvalidConfigurationException(
    // detailedException.getDetailedExitCode(), e)`).
    // A wrapped NoSuchThingException keeps its PackageLoading code
    // (SkyframeExecutor.createBuildConfigurationKey: `new InvalidConfigurationException(
    // noSuchThingException.getDetailedExitCode(), e)`; analysis review R2-9).
    if let Some(code) = e.package_loading_code {
        return DetailedExitCode::of(FailureDetail {
            message: e.get_message().to_owned(),
            category: Some(Category::PackageLoading(failure_details::PackageLoading {
                code,
            })),
        });
    }
    if let Some(name) = &e.wrapped_failure_code {
        let (category, code) = match name.split_once('.') {
            Some((category, code)) => (category, Some(code)),
            None => (name.as_str(), None),
        };
        let util = blaze_util::DetailedExitCode::of_category_name(e.get_message(), category, code)
            .unwrap_or_else(|| panic!("IllegalArgumentException: no FailureDetail code {name}"));
        return DetailedExitCode::of(util.get_failure_detail().expect("a FailureDetail").clone());
    }
    if let Some(toolchain_code) = e.code.strip_prefix("Toolchain.") {
        let code = failure_details::toolchain::Code::from_str_name(toolchain_code)
            .unwrap_or_else(|| panic!("IllegalArgumentException: Toolchain.Code {toolchain_code}"));
        return DetailedExitCode::of(FailureDetail {
            message: e.get_message().to_owned(),
            category: Some(Category::Toolchain(failure_details::Toolchain {
                code: code as i32,
            })),
        });
    }
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

    // SkyframeExecutor.createBuildConfigurationKey wraps a NoSuchThingException with its own
    // detailed exit code; PACKAGE_MISSING (11) has exit_code 1 in failure_details.proto, where
    // PLATFORM_MAPPING_EVALUATION_FAILURE has 2 (analysis review R2-9).
    #[test]
    fn wrapped_no_such_thing_keeps_its_package_loading_code() {
        let e = InvalidConfigurationException::with_package_loading_code("no such package", 11);
        let d = invalid_configuration_detailed_exit_code(&e);
        assert_eq!(d.get_exit_code(), 1);
        assert!(matches!(
            d.get_failure_detail().and_then(|f| f.category.as_ref()),
            Some(Category::PackageLoading(p)) if p.code == 11
        ));
    }
}
