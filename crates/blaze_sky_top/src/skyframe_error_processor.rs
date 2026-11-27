//! Port of `skyframe/SkyframeErrorProcessor.java` (`processAnalysisErrors` and the helpers it
//! reaches) and of `analysis/ViewCreationFailedException.java`.
//!
//! PORT: only the analysis half (`includeExecutionPhase == false`) is ported. Execution errors
//! (`processExecutionErrors`, `processResult`, `rethrow`) belong to the execution phase. The
//! Skymeld keys (`BuildDriverKey`, the completion keys) and `TopLevelConflictException` do not
//! exist in Blaze's analysis graph and reach `unported!` if they ever appear.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::{
    ASPECT, AspectKey, TOP_LEVEL_ASPECTS, TopLevelAspectsKey,
};
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_bzlmod_model::ExternalDepsException;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_exec::actions::action_execution::ActionExecutionException;
use blaze_packages::exceptions::{
    DetailedExitCode as PackagesDetailedExitCode, NoSuchPackageException, NoSuchTargetException,
    NoSuchThingException,
};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_analysis::aspect_function::AspectFunctionException;
use blaze_sky_analysis::load_aspects_function::TopLevelAspectsDetailsBuildFailedException;
use blaze_sky_analysis::producers::errors::{
    AspectCreationException, Cause, ConfiguredTargetFunctionException,
    ConfiguredValueCreationException, configuration_id_of_key,
};
use blaze_sky_analysis::toplevel_starlark_aspect_function::TopLevelStarlarkAspectFunctionException;
use blaze_sky_loading::bzl_load::BzlLoadFailedException;
use blaze_sky_loading::transitive_target::{TRANSITIVE_TARGET, TransitiveTargetKey};
use blaze_skyframe::{CycleInfo, ErrorInfo, EvaluationResult, SkyError, SkyKey};
use blaze_util::check_state;
use indexmap::IndexSet;

use crate::action_conflict_exception::{ActionConflictException, ActionConflicts};
use crate::analysis_failure_event::{AnalysisFailureEvent, AnalysisRootCause, LoadingFailureEvent};
use crate::cycles_reporter::CyclesReporter;

/// An exception indicating that there was a problem during the view construction (loading and
/// analysis phases) for one or more targets, that the configured target graph could not be
/// successfully constructed, and that a build cannot be started.
// Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:26 ViewCreationFailedException
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ViewCreationFailedException {
    message: String,
    failure_detail: FailureDetail,
}

impl ViewCreationFailedException {
    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:30 ViewCreationFailedException.<init>
    pub fn new(message: impl Into<String>, failure_detail: FailureDetail) -> Self {
        ViewCreationFailedException {
            message: message.into(),
            failure_detail,
        }
    }

    /// `ViewCreationFailedException(String message, FailureDetail failureDetail, Throwable
    /// cause)`; `cause_message` is `cause.getMessage()`.
    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:35 ViewCreationFailedException.<init>
    pub fn with_cause(
        message: impl Into<String>,
        failure_detail: FailureDetail,
        cause_message: Option<&str>,
    ) -> Self {
        ViewCreationFailedException {
            message: Self::combine_messages(message.into(), cause_message),
            failure_detail,
        }
    }

    /// `ViewCreationFailedException(FailureDetail failureDetail, Throwable cause)`: the message
    /// is the cause's.
    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:40 ViewCreationFailedException.<init>
    pub fn of_cause(failure_detail: FailureDetail, cause_message: impl Into<String>) -> Self {
        ViewCreationFailedException {
            message: cause_message.into(),
            failure_detail,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:45 ViewCreationFailedException.getFailureDetail
    pub fn get_failure_detail(&self) -> &FailureDetail {
        &self.failure_detail
    }

    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:49 ViewCreationFailedException.combineMessages
    fn combine_messages(message: String, cause_message: Option<&str>) -> String {
        match cause_message {
            // isNullOrEmpty(cause.getMessage())
            None | Some("") => message,
            Some(cause_message) => format!("{}: {}", message, cause_message),
        }
    }
}

/// Indicates if there are errors with the various phases, and an exception to be thrown to halt
/// the build, in case of --nokeep_going.
///
/// The various attributes will be used later on to construct the FailureDetail in
/// `BuildView.createAnalysisFailureDetail`.
///
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:116 SkyframeErrorProcessor.ErrorProcessingResult
// (`newBuilder()` is `Default`, and the aggregating builder is the value itself.)
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:126 SkyframeErrorProcessor.ErrorProcessingResult.newBuilder
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:160 SkyframeErrorProcessor.ErrorProcessingResult.AggregatingBuilder.build
// PORT: skipped, a private constructor of a static class: SkyframeErrorProcessor.<init>
// PORT: skipped, logging only (BugReport.logUnexpected and logger lines, not shown to the user): SkyframeErrorProcessor.assertValidAnalysisException SkyframeErrorProcessor.assertValidAnalysisOrExecutionException SkyframeErrorProcessor.logUnexpectedExceptionOrigin SkyframeErrorProcessor.logUnexpectedException SkyframeErrorProcessor.isExecutionCauseWorthLogging
// PORT: skipped, BuildViewTestCase only (inBuildViewTest: no event bus): SkyframeErrorProcessor.isValidErrorKeyType
#[derive(Debug, Default)]
pub struct ErrorProcessingResult {
    /// Whether there are loading errors.
    has_loading_error: bool,
    /// Whether there are analysis errors.
    has_analysis_error: bool,
    /// The action conflicts encountered during analysis.
    action_conflicts: ActionConflicts,
    /// The detailed exit code for execution errors: `None` if the result had no errors or the
    /// errors were all analysis errors.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:154 SkyframeErrorProcessor.ErrorProcessingResult.AggregatingBuilder.setExecutionDetailedExitCode
    execution_detailed_exit_code: Option<blaze_util::DetailedExitCode>,
    /// `aspectKeysForConflictReporting`: the aspect keys of the intra-aspect action conflicts.
    aspect_keys_for_conflict_reporting: Vec<blaze_actions::ActionLookupKeyRef>,
}

impl ErrorProcessingResult {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:122 SkyframeErrorProcessor.ErrorProcessingResult.aspectKeysForConflictReporting
    pub fn aspect_keys_for_conflict_reporting(&self) -> &[blaze_actions::ActionLookupKeyRef] {
        &self.aspect_keys_for_conflict_reporting
    }

    pub fn has_loading_error(&self) -> bool {
        self.has_loading_error
    }

    pub fn has_analysis_error(&self) -> bool {
        self.has_analysis_error
    }

    pub fn action_conflicts(&self) -> &ActionConflicts {
        &self.action_conflicts
    }

    pub fn execution_detailed_exit_code(&self) -> Option<&blaze_util::DetailedExitCode> {
        self.execution_detailed_exit_code.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:140 SkyframeErrorProcessor.ErrorProcessingResult.AggregatingBuilder.aggregateSingleResult
    fn aggregate_single_result(
        &mut self,
        individual_error_processing_result: IndividualErrorProcessingResult,
    ) {
        self.has_loading_error =
            self.has_loading_error || individual_error_processing_result.is_loading_error();
        self.has_analysis_error =
            self.has_analysis_error || individual_error_processing_result.is_analysis_error();
        self.action_conflicts
            .put_all(&individual_error_processing_result.action_conflicts);
        self.execution_detailed_exit_code =
            blaze_util::DetailedExitCode::choose_more_important_with_first_if_tie(
                self.execution_detailed_exit_code.take(),
                individual_error_processing_result.execution_detailed_exit_code,
            );
        if let Some(aspect_key) =
            individual_error_processing_result.aspect_key_for_conflict_reporting
            && !self
                .aspect_keys_for_conflict_reporting
                .contains(&aspect_key)
        {
            self.aspect_keys_for_conflict_reporting.push(aspect_key);
        }
    }
}

/// Represents the information around one single error in the build. These are the building
/// blocks for the final `ErrorProcessingResult`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:176 SkyframeErrorProcessor.IndividualErrorProcessingResult
// (`create(...)` is the struct literal.)
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:200 SkyframeErrorProcessor.IndividualErrorProcessingResult.create
struct IndividualErrorProcessingResult {
    action_conflicts: ActionConflicts,
    execution_detailed_exit_code: Option<blaze_util::DetailedExitCode>,
    /// `NestedSet<Cause>`, flattened.
    analysis_root_causes: Vec<AnalysisRootCause>,
    /// PORT: an `ImmutableSet` copy of a `HashSet<Label>` in Java, whose iteration order (the
    /// order of the `LoadingFailureEvent`s) is not reproduced; insertion order here.
    loading_root_causes: IndexSet<Label>,
    aspect_key_for_conflict_reporting: Option<blaze_actions::ActionLookupKeyRef>,
}

impl IndividualErrorProcessingResult {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:187 SkyframeErrorProcessor.IndividualErrorProcessingResult.isActionConflictError
    fn is_action_conflict_error(&self) -> bool {
        !self.action_conflicts.is_empty()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:191 SkyframeErrorProcessor.IndividualErrorProcessingResult.isLoadingError
    fn is_loading_error(&self) -> bool {
        !self.loading_root_causes.is_empty()
    }

    /// This is true for all non-execution errors: including loading & action conflict errors.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:196 SkyframeErrorProcessor.IndividualErrorProcessingResult.isAnalysisError
    fn is_analysis_error(&self) -> bool {
        self.execution_detailed_exit_code.is_none()
    }
}

/// The Java class of an exception at a top-level key, as far as `SkyframeErrorProcessor`
/// distinguishes classes.
enum AnalysisException<'a> {
    /// `ActionConflictException`.
    ActionConflict(&'a ActionConflictException),
    /// `ConfiguredValueCreationException`.
    ConfiguredValueCreation(&'a ConfiguredValueCreationException),
    /// `NoSuchThingException` (`NoSuchPackageException`, `NoSuchTargetException`).
    NoSuchThing(PackagesDetailedExitCode),
    /// `ExternalDepsException`.
    ExternalDeps(&'a ExternalDepsException),
    /// `BzlLoadFailedException` (a `SaneAnalysisException`).
    BzlLoadFailed(&'a BzlLoadFailedException),
    /// `AspectCreationException` (a `SaneAnalysisException`).
    AspectCreation(&'a AspectCreationException),
    /// `TopLevelAspectsDetailsBuildFailedException` (a `SaneAnalysisException`).
    TopLevelAspectsDetailsBuildFailed(&'a TopLevelAspectsDetailsBuildFailedException),
    /// `ExecGroupCollection.InvalidExecGroupException` (a `SaneAnalysisException`), carried by
    /// its message.
    InvalidExecGroup(&'a str),
    /// `TopLevelConflictException` (Skymeld's `BuildDriverFunction`).
    TopLevelConflict(&'a crate::top_level_conflict_exception::TopLevelConflictException),
    /// `TargetCompatibilityCheckException` (a `SaneAnalysisException`; Skymeld's
    /// `BuildDriverFunction`).
    TargetCompatibilityCheck(
        &'a blaze_analysis::constraints::top_level_constraint_semantics::TargetCompatibilityCheckException,
    ),
    /// Any other class.
    Other,
}

/// PORT: Java tests the exception's class (`instanceof`). `ConfiguredTargetFunction`'s wrapper
/// exceptions (`ReportedException`, `UnreportedException`, `DependencyException`) are one Rust
/// enum, whose variants stand for the wrapped classes; so are `AspectFunction`'s
/// (`AspectFunctionException`) and `ToplevelStarlarkAspectFunction`'s. Of Java's
/// `SaneAnalysisException`s, `ConfiguredValueCreationException`, `ActionConflictException`,
/// `BzlLoadFailedException`, `AspectCreationException`,
/// `TopLevelAspectsDetailsBuildFailedException` and `InvalidExecGroupException` are recognised;
/// `InvalidPlatformException`, `TargetCompatibilityCheckException` and
/// `AnalysisFailurePropagationException` are not thrown to a top-level key by the ported
/// functions (their causes are wrapped in a `ConfiguredValueCreationException`).
fn classify<'a>(exception: &'a (dyn std::error::Error + 'static)) -> AnalysisException<'a> {
    if let Some(e) = exception.downcast_ref::<AspectFunctionException>() {
        return match e {
            AspectFunctionException::NoSuchThing(n) => {
                AnalysisException::NoSuchThing(n.get_detailed_exit_code())
            }
            AspectFunctionException::AspectCreation(e) => AnalysisException::AspectCreation(e),
            AspectFunctionException::ConfiguredValueCreation(e) => {
                AnalysisException::ConfiguredValueCreation(e)
            }
            AspectFunctionException::InvalidExecGroup(message) => {
                AnalysisException::InvalidExecGroup(message)
            }
            AspectFunctionException::ActionConflict(e) => AnalysisException::ActionConflict(e),
            // RuleErrorException is not a SaneAnalysisException.
            AspectFunctionException::RuleError(_) => AnalysisException::Other,
        };
    }
    if let Some(e) = exception.downcast_ref::<TopLevelStarlarkAspectFunctionException>() {
        return match e {
            // ActionConflictException.withAspectKeyInfo(e, aspectKey): an ActionConflictException.
            TopLevelStarlarkAspectFunctionException::ActionConflictWithAspectKeyInfo {
                cause,
                ..
            } => AnalysisException::ActionConflict(cause),
            TopLevelStarlarkAspectFunctionException::TopLevelAspectsDetailsBuildFailed(e) => {
                AnalysisException::TopLevelAspectsDetailsBuildFailed(e)
            }
        };
    }
    if let Some(e) = exception.downcast_ref::<AspectCreationException>() {
        return AnalysisException::AspectCreation(e);
    }
    if let Some(e) = exception.downcast_ref::<TopLevelAspectsDetailsBuildFailedException>() {
        return AnalysisException::TopLevelAspectsDetailsBuildFailed(e);
    }
    if let Some(e) = exception.downcast_ref::<ConfiguredTargetFunctionException>() {
        return match e {
            ConfiguredTargetFunctionException::Reported(c)
            | ConfiguredTargetFunctionException::Unreported(c) => {
                AnalysisException::ConfiguredValueCreation(c)
            }
            // `UnreportedException(ActionConflictException)`: a conflict inside one target.
            ConfiguredTargetFunctionException::ActionConflict(e) => {
                AnalysisException::ActionConflict(e)
            }
            ConfiguredTargetFunctionException::NoSuchThing(n) => {
                AnalysisException::NoSuchThing(n.get_detailed_exit_code())
            }
            ConfiguredTargetFunctionException::InconsistentNullConfig(_) => {
                AnalysisException::Other
            }
        };
    }
    if let Some(e) = exception.downcast_ref::<ConfiguredValueCreationException>() {
        return AnalysisException::ConfiguredValueCreation(e);
    }
    if let Some(e) = exception.downcast_ref::<ActionConflictException>() {
        return AnalysisException::ActionConflict(e);
    }
    if let Some(e) = exception.downcast_ref::<NoSuchThingException>() {
        return AnalysisException::NoSuchThing(e.get_detailed_exit_code());
    }
    if let Some(e) = exception.downcast_ref::<NoSuchPackageException>() {
        return AnalysisException::NoSuchThing(e.get_detailed_exit_code());
    }
    if let Some(e) = exception.downcast_ref::<NoSuchTargetException>() {
        return AnalysisException::NoSuchThing(e.get_detailed_exit_code());
    }
    if let Some(e) = exception.downcast_ref::<ExternalDepsException>() {
        return AnalysisException::ExternalDeps(e);
    }
    if let Some(e) = exception.downcast_ref::<BzlLoadFailedException>() {
        return AnalysisException::BzlLoadFailed(e);
    }
    if let Some(e) =
        exception.downcast_ref::<crate::top_level_conflict_exception::TopLevelConflictException>()
    {
        return AnalysisException::TopLevelConflict(e);
    }
    if let Some(e) = exception.downcast_ref::<
        blaze_analysis::constraints::top_level_constraint_semantics::TargetCompatibilityCheckException,
    >() {
        return AnalysisException::TargetCompatibilityCheck(e);
    }
    AnalysisException::Other
}

/// `exception instanceof AbstractSaneAnalysisException` for the classes [`classify`] recognises
/// (`BuildDriverFunction`'s `env.getValueOrThrow(actionLookupKey,
/// AbstractSaneAnalysisException.class)`).
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:174 BuildDriverFunction.compute
pub(crate) fn is_sane_analysis_exception(exception: &(dyn std::error::Error + 'static)) -> bool {
    matches!(
        classify(exception),
        AnalysisException::ActionConflict(_)
            | AnalysisException::ConfiguredValueCreation(_)
            | AnalysisException::BzlLoadFailed(_)
            | AnalysisException::AspectCreation(_)
            | AnalysisException::TopLevelAspectsDetailsBuildFailed(_)
            | AnalysisException::InvalidExecGroup(_)
            | AnalysisException::TargetCompatibilityCheck(_)
    )
}

/// The aspect key of an `ActionConflictException` (`ace.getAspectKey()`): the key
/// `ActionConflictException.withAspectKeyInfo` attached.
fn aspect_key_of_action_conflict(
    exception: &(dyn std::error::Error + 'static),
) -> Option<blaze_actions::ActionLookupKeyRef> {
    match exception.downcast_ref::<TopLevelStarlarkAspectFunctionException>() {
        Some(TopLevelStarlarkAspectFunctionException::ActionConflictWithAspectKeyInfo {
            aspect_key,
            ..
        }) => Some(blaze_actions::ActionLookupKeyRef::new(aspect_key.clone())),
        _ => None,
    }
}

/// The `FailureDetail` of a blaze_packages `DetailedExitCode` (`getFailureDetail()`).
///
/// PORT: blaze_packages keeps a `DetailedExitCode` as a message and `<Category>.<CODE>`; the
/// categories the loading and analysis functions produce are rebuilt here.
pub fn packages_failure_detail(detailed_exit_code: &PackagesDetailedExitCode) -> FailureDetail {
    detailed_exit_code
        .to_util()
        .get_failure_detail()
        .expect("a DetailedExitCode of a FailureDetail")
        .clone()
}

/// `ExternalDepsException.getDetailedExitCode().getFailureDetail()`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:78 ExternalDepsException.getDetailedExitCode
fn external_deps_failure_detail(e: &ExternalDepsException) -> FailureDetail {
    FailureDetail {
        message: e.get_message().to_owned(),
        category: Some(Category::ExternalDeps(failure_details::ExternalDeps {
            code: e.get_code().number(),
        })),
    }
}

/// Process only loading/analysis errors. Returns an `ErrorProcessingResult`.
///
/// In case of --nokeep_going: immediately throw the exception.
///
/// PORT: `keepEdges` and `bugReporter` only feed `logUnexpectedExceptionOrigin` (bug-report
/// logging, skipped); `eventBus` is the event handler (`post`).
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:226 SkyframeErrorProcessor.processAnalysisErrors
pub fn process_analysis_errors(
    result: &EvaluationResult,
    cycles_reporter: &CyclesReporter,
    event_handler: &Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
) -> Result<ErrorProcessingResult, ViewCreationFailedException> {
    process_errors(
        result.error_map(),
        cycles_reporter,
        event_handler,
        keep_going,
    )
}

/// What `processErrors` throws: `ViewCreationFailedException` when the root cause is
/// analysis-related, `BuildFailedException` / `TestExecException` (the [`RethrownException`])
/// when it is execution-related.
#[derive(Clone, Debug)]
pub enum ProcessErrorsException {
    ViewCreationFailed(ViewCreationFailedException),
    Rethrown(RethrownException),
}

impl From<ViewCreationFailedException> for ProcessErrorsException {
    fn from(e: ViewCreationFailedException) -> Self {
        ProcessErrorsException::ViewCreationFailed(e)
    }
}

/// `processErrors(result, ..., includeExecutionPhase)` of an evaluation that may hold
/// execution-phase keys (`BuildDriverKey`s, the exclusive tests' `TestCompletionKey`s, the
/// coverage artifacts).
///
/// PORT: `includeExecutionPhase` only selects which logging-only validity check runs
/// (`assertValidAnalysisOrExecutionException`, skipped); the processing is the same.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:302 SkyframeErrorProcessor.processErrors
pub fn process_errors_with_execution(
    result: &EvaluationResult,
    cycles_reporter: &CyclesReporter,
    event_handler: &Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
) -> Result<ErrorProcessingResult, ProcessErrorsException> {
    process_errors_impl(
        result.error_map(),
        cycles_reporter,
        event_handler,
        keep_going,
    )
}

/// Process errors encountered during analysis/execution.
///
/// This method has different goals depending on --(no)keep_going:
///
/// - In case of --keep_going: post the necessary events, then construct an
///   `ErrorProcessingResult`.
/// - In case of --nokeep_going: post the necessary events, then throw an appropriate exception
///   ASAP, except when the error is caused by an action conflict: we need more downstream
///   information.
///
/// Throws `ViewCreationFailedException` when the root cause is analysis-related. Returns an
/// `ErrorProcessingResult` (only in --keep_going mode, or action conflict).
///
/// PORT: takes `result.errorMap()` (an `EvaluationResult` cannot be built outside
/// `blaze_skyframe`, and nothing else of the result is read: `getWalkableGraph()` only feeds the
/// skipped bug-report logging).
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:306 SkyframeErrorProcessor.processErrors
pub(crate) fn process_errors(
    error_map: &[(SkyKey, ErrorInfo)],
    cycles_reporter: &CyclesReporter,
    event_handler: &Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
) -> Result<ErrorProcessingResult, ViewCreationFailedException> {
    // processAnalysisErrors: `catch (BuildFailedException | TestExecException unexpected)`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:240 SkyframeErrorProcessor.processAnalysisErrors
    process_errors_impl(error_map, cycles_reporter, event_handler, keep_going).map_err(
        |e| match e {
            ProcessErrorsException::ViewCreationFailed(e) => e,
            ProcessErrorsException::Rethrown(unexpected) => {
                panic!(
                    "IllegalStateException: Unexpected execution phase exception: {unexpected:?}"
                )
            }
        },
    )
}

fn process_errors_impl(
    error_map: &[(SkyKey, ErrorInfo)],
    cycles_reporter: &CyclesReporter,
    event_handler: &Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
) -> Result<ErrorProcessingResult, ProcessErrorsException> {
    let mut aggregating_result_builder = ErrorProcessingResult::default();
    let mut no_keep_going_analysis_exception_aspect: Option<ViewCreationFailedException> = None;

    for (error_entry_key, error_info) in error_map {
        // The cycle reporter requires that the path to the cycle starts at the top level key
        // (requested via SkyframeExecutor), hence we need to provide the original top level key
        // here.
        cycles_reporter.report_cycles(
            error_info.get_cycle_info(),
            /* top_level_key= */ error_entry_key,
            event_handler,
        );

        let error_key = &get_effective_error_key(error_entry_key);
        // PORT: skipped, assertValidAnalysisException / assertValidAnalysisOrExecutionException:
        // they only log the origin of an unexpected exception class (BugReport.logUnexpected).
        let nullable_cause = error_info.get_exception();
        check_state!(
            nullable_cause.is_some() || !error_info.get_cycle_info().is_empty(),
            "{:?}",
            error_info
        );

        let label = get_label(error_key);
        let individual_error_processing_result = process_individual_error(error_key, error_info);

        // For action conflicts, more downstream operations are required to have all the
        // information. We intentionally don't send out any failure event, throw any exception
        // (even with --nokeep_going) or print a warning message at this point. These will be done
        // elsewhere at a later point.
        if individual_error_processing_result.is_action_conflict_error() {
            aggregating_result_builder.aggregate_single_result(individual_error_processing_result);
            continue;
        }

        maybe_post_failure_events_for_non_conflict_error(
            event_handler.as_ref(),
            error_key,
            label.as_ref(),
            &individual_error_processing_result,
        );

        let execution_exception = nullable_cause.and_then(ExecutionExceptionCause::of);
        let is_execution_exception = execution_exception.is_some();
        if keep_going {
            let cause_message = nullable_cause.map(|cause| cause.to_string());
            aggregating_result_builder.aggregate_single_result(individual_error_processing_result);
            log_or_print_warnings_keep_going(
                is_execution_exception,
                label.as_ref(),
                event_handler.as_ref(),
                cause_message.as_deref(),
            );
        } else {
            no_keep_going_analysis_exception_aspect =
                Some(throw_or_return_aspect_analysis_exception(
                    nullable_cause,
                    error_key,
                    execution_exception.as_ref(),
                    /* has_execution_cycle= */
                    individual_error_processing_result.execution_detailed_exit_code
                        == Some(cycle_code()),
                )?);
        }
    }

    if let Some(e) = no_keep_going_analysis_exception_aspect {
        return Err(e.into());
    }

    Ok(aggregating_result_builder)
}

/// Post the relevant failure events if we're not in test.
///
/// There is 1 exception: for aspects, the failures should already have been reported to the
/// event handler, so we do nothing here.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:411 SkyframeErrorProcessor.maybePostFailureEventsForNonConflictError
fn maybe_post_failure_events_for_non_conflict_error(
    event_bus: &dyn ExtendedEventHandler,
    error_key: &SkyKey,
    label: Option<&Label>,
    individual_error_processing_result: &IndividualErrorProcessingResult,
) {
    check_state!(!individual_error_processing_result.is_action_conflict_error());
    // PORT: skipped, inBuildViewTest (the BuildViewTestCase branch without an event bus).
    // Top-level aspect failures may be the only processed error in nokeep_going Skymeld.
    if let Some(top_level_aspects_key) = error_key.downcast::<TopLevelAspectsKey>() {
        if individual_error_processing_result.is_analysis_error() {
            event_bus.post(Arc::new(AnalysisFailureEvent::while_analyzing_target(
                top_level_aspects_key
                    .get_base_configured_target_key()
                    .clone(),
                individual_error_processing_result
                    .analysis_root_causes
                    .clone(),
            )));
        }
        return;
    }
    let Some(ct_key) = error_key.downcast::<ConfiguredTargetKey>() else {
        return;
    };

    // For loading errors, we expect both LoadingFailureEvent and AnalysisFailureEvent.
    if individual_error_processing_result.is_loading_error() {
        for loading_root_cause in &individual_error_processing_result.loading_root_causes {
            // This event is only for backwards compatibility with the old event protocol. Remove
            // once we've migrated to the build event protocol.
            event_bus.post(Arc::new(LoadingFailureEvent::new(
                label
                    .cloned()
                    .unwrap_or_else(|| panic!("NullPointerException")),
                loading_root_cause.clone(),
            )));
        }
    }

    if individual_error_processing_result.is_analysis_error() {
        event_bus.post(Arc::new(AnalysisFailureEvent::while_analyzing_target(
            ct_key.clone(),
            individual_error_processing_result
                .analysis_root_causes
                .clone(),
        )));
    }
}

/// Throw the necessary exceptions based on the error processing result.
///
/// This method should be called in --nokeep_going mode, unless the error is an action conflict.
///
/// Special case: if the analysis error belongs to a top-level Aspect, we don't throw the
/// ViewCreationFailedException immediately to make sure that a target analysis error is preferred
/// over an aspect one.
///
/// PORT: `Err` is Java's throw, `Ok` the exception returned for a top-level aspect.
/// `execution_exception` is the cause when `isExecutionException(cause)`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:473 SkyframeErrorProcessor.throwOrReturnAspectAnalysisException
fn throw_or_return_aspect_analysis_exception(
    cause: Option<&SkyError>,
    error_key: &SkyKey,
    execution_exception: Option<&ExecutionExceptionCause<'_>>,
    has_execution_cycle: bool,
) -> Result<ViewCreationFailedException, ProcessErrorsException> {
    // If the error is execution-related: straightaway rethrow. No further steps required.
    if let Some(execution_exception) = execution_exception {
        // cause is not null for execution exceptions.
        return Err(ProcessErrorsException::Rethrown(rethrow(
            execution_exception,
        )));
    }
    // If a --nokeep_going build found a cycle, that means there were no other errors thrown
    // during evaluation (otherwise, it wouldn't have bothered to find a cycle). So the best
    // we can do is throw a generic build failure exception, since we've already reported the
    // cycles above. Analysis cycles are handled below.
    if has_execution_cycle {
        // throw new BuildFailedException(null, CYCLE_CODE)
        return Err(ProcessErrorsException::Rethrown(
            RethrownException::build_failed(crate::build_tool::BuildToolError::BuildFailed {
                message: String::new(),
                failure_detail: cycle_code()
                    .get_failure_detail()
                    .cloned()
                    .expect("CYCLE_CODE has a failure detail"),
            }),
        ));
    }

    if let Some(aspect_key) = error_key.downcast::<TopLevelAspectsKey>() {
        let error_msg = format!(
            "Analysis of aspects '{}' failed; build aborted",
            aspect_key.get_description()
        );
        return Ok(create_view_creation_failed_exception(cause, error_msg));
    }

    let top_level_label = error_key
        .downcast::<ConfiguredTargetKey>()
        .unwrap_or_else(|| panic!("ClassCastException: {:?}", error_key))
        .get_label();
    Err(ProcessErrorsException::ViewCreationFailed(
        create_view_creation_failed_exception(
            cause,
            format!(
                "Analysis of target '{}' failed; build aborted",
                top_level_label
            ),
        ),
    ))
}

/// Processes one individual error from the result.
///
/// No exception should ever be thrown here: this is just to gather the relevant information
/// around 1 single error. `process_errors` will decide what to do with this information.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:514 SkyframeErrorProcessor.processIndividualError
fn process_individual_error(
    error_key: &SkyKey,
    error_info: &ErrorInfo,
) -> IndividualErrorProcessingResult {
    let exception = error_info.get_exception();
    let mut loading_root_causes: IndexSet<Label> = IndexSet::new();
    let mut action_conflicts = ActionConflicts::new();
    let mut execution_detailed_exit_code: Option<blaze_util::DetailedExitCode> = None;

    let function_name = error_key.function_name().0;
    // Legacy: analysis-related failure events for Aspects are sent somewhere else, so we don't
    // have to do any work related to constructing the analysis failure events here, only for the
    // other cases like action conflict or execution-related errors.
    // TODO(b/249690006): Can we simplify things by moving aspects events here?
    if function_name == ASPECT.0 || function_name == TOP_LEVEL_ASPECTS.0 {
        let mut aspect_key_for_conflict_reporting = None;
        let exception_class = match exception {
            Some(e) => classify(&**e),
            None => AnalysisException::Other,
        };
        if let AnalysisException::TopLevelConflict(tlce) = exception_class {
            action_conflicts = tlce.get_transitive_action_conflicts().clone();
        } else if let AnalysisException::ActionConflict(ace) = exception_class {
            action_conflicts.put(ace.get_attempted_action(), ace.clone());
            aspect_key_for_conflict_reporting =
                exception.and_then(|e| aspect_key_of_action_conflict(&**e));
        } else if let Some(execution_exception) = exception.and_then(ExecutionExceptionCause::of) {
            execution_detailed_exit_code =
                Some(execution_exception.get_execution_detailed_exit_code());
        } else if !error_info.get_cycle_info().is_empty()
            && is_execution_cycle(error_info.get_cycle_info())
        {
            execution_detailed_exit_code = Some(cycle_code());
        }
        return IndividualErrorProcessingResult {
            action_conflicts,
            execution_detailed_exit_code,
            analysis_root_causes: Vec::new(),
            loading_root_causes: IndexSet::new(),
            aspect_key_for_conflict_reporting,
        };
    }

    // Only possible with actions generating build-info.txt and build-changelist.txt.
    if function_name == blaze_actions::actions::action_lookup::ACTION_EXECUTION {
        let exception = exception
            .unwrap_or_else(|| panic!("NullPointerException: the exception of {:?}", error_key));
        return IndividualErrorProcessingResult {
            action_conflicts: ActionConflicts::new(),
            execution_detailed_exit_code: Some(get_execution_detailed_exit_code_from_cause(
                exception,
            )),
            analysis_root_causes: Vec::new(),
            loading_root_causes: IndexSet::new(),
            aspect_key_for_conflict_reporting: None,
        };
    }

    let ct_key = error_key
        .downcast::<ConfiguredTargetKey>()
        .unwrap_or_else(|| {
            panic!(
                "IllegalStateException: expected '{:?}' to be a ConfiguredTargetKey",
                error_key
            )
        });
    let top_level_label = ct_key.get_label();
    let analysis_root_causes: Vec<AnalysisRootCause>;

    let exception_class = match exception {
        Some(e) => classify(&**e),
        None => AnalysisException::Other,
    };
    let execution_exception = exception.and_then(ExecutionExceptionCause::of);
    match exception_class {
        AnalysisException::TopLevelConflict(tlce) => {
            action_conflicts = tlce.get_transitive_action_conflicts().clone();
            analysis_root_causes = Vec::new();
        }
        AnalysisException::ActionConflict(ace) => {
            action_conflicts.put(ace.get_attempted_action(), ace.clone());
            analysis_root_causes = Vec::new();
        }
        AnalysisException::ConfiguredValueCreation(ct_cause) => {
            // Previously, the nested set was de-duplicating loading root cause labels. Now that
            // we track Cause instances including a message, we get one event per label and
            // message. In order to keep backwards compatibility, we deduplicate root cause
            // labels here.
            // TODO(ulfjack): Remove this code once we've migrated to the BEP.
            let root_causes = ct_cause.get_root_causes().to_list();
            for root_cause in root_causes.iter() {
                if matches!(root_cause, Cause::LoadingFailed { .. }) {
                    loading_root_causes.insert(root_cause.get_label().clone());
                }
            }
            analysis_root_causes = root_causes
                .iter()
                .cloned()
                .map(AnalysisRootCause::Cause)
                .collect();
        }
        other if !error_info.get_cycle_info().is_empty() => {
            // PORT: Java reaches this branch for every exception class but the three above;
            // the classes below are tested after it.
            let _ = other;
            if is_execution_cycle(error_info.get_cycle_info()) {
                // If we have a cycle, cause would be null, so it's guaranteed that this
                // executionDetailedExitCode is final.
                execution_detailed_exit_code = Some(cycle_code());
                analysis_root_causes = Vec::new();
            } else {
                let analysis_root_cause = maybe_get_configured_target_cycle_culprit(
                    top_level_label,
                    error_info.get_cycle_info(),
                );
                analysis_root_causes = match analysis_root_cause {
                    Some(analysis_root_cause) => vec![AnalysisRootCause::Label {
                        label: analysis_root_cause,
                        failure_detail: create_failure_detail(
                            "Dependency cycle".to_owned(),
                            failure_details::analysis::Code::Cycle,
                        ),
                    }],
                    // TODO(ulfjack): We need to report the dependency cycle here. How?
                    None => Vec::new(),
                };
            }
        }
        AnalysisException::NoSuchThing(detailed_exit_code) => {
            // This branch is only taken in --nokeep_going builds. In a --keep_going build, the
            // AnalysisFailedCause is properly reported through the
            // ConfiguredValueCreationException.
            analysis_root_causes = vec![AnalysisRootCause::AnalysisFailed {
                label: top_level_label.clone(),
                configuration: configuration_id_of_key(ct_key.get_configuration_key())
                    .unwrap_or_else(|| "none".to_owned()),
                failure_detail: packages_failure_detail(&detailed_exit_code),
            }];
        }
        AnalysisException::ExternalDeps(external_deps_exception) => {
            analysis_root_causes = vec![AnalysisRootCause::AnalysisFailed {
                label: top_level_label.clone(),
                configuration: configuration_id_of_key(ct_key.get_configuration_key())
                    .unwrap_or_else(|| "none".to_owned()),
                failure_detail: external_deps_failure_detail(external_deps_exception),
            }];
        }
        AnalysisException::TargetCompatibilityCheck(_) => {
            analysis_root_causes = Vec::new();
        }
        _ if execution_exception.is_some() => {
            let execution_exception = execution_exception.expect("checked");
            execution_detailed_exit_code =
                Some(execution_exception.get_execution_detailed_exit_code());
            // actionExecutionException.getRootCauses()
            // PORT: the root causes of an execution error only feed the AnalysisFailureEvent of
            // an analysis error (`maybePostFailureEventsForNonConflictError`), which an error
            // with an execution exit code is not; they are not carried.
            analysis_root_causes = Vec::new();
        }
        // Unexpected causes.
        // PORT: skipped, BugReport.logUnexpected("Unexpected cause encountered while
        // evaluating") (bug reports).
        AnalysisException::BzlLoadFailed(_)
        | AnalysisException::AspectCreation(_)
        | AnalysisException::TopLevelAspectsDetailsBuildFailed(_)
        | AnalysisException::InvalidExecGroup(_)
        | AnalysisException::Other => {
            analysis_root_causes = Vec::new();
        }
    }

    IndividualErrorProcessingResult {
        action_conflicts,
        execution_detailed_exit_code,
        analysis_root_causes,
        loading_root_causes,
        aspect_key_for_conflict_reporting: None,
    }
}

/// `DetailedException.getDetailedExitCode(cause)`, or the "unknown" execution code.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:621 SkyframeErrorProcessor.getExecutionDetailedExitCodeFromCause
fn get_execution_detailed_exit_code_from_cause(cause: &SkyError) -> blaze_util::DetailedExitCode {
    if let Some(execution_exception) = ExecutionExceptionCause::of(cause) {
        return execution_exception.get_execution_detailed_exit_code();
    }
    if let Some(failure_detail) = detailed_exception_failure_detail(cause) {
        return blaze_util::DetailedExitCode::of(failure_detail);
    }
    create_unknown_execution_detailed_exit_code(&cause.to_string())
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:665 SkyframeErrorProcessor.logOrPrintWarningsKeepGoing
fn log_or_print_warnings_keep_going(
    is_execution_exception: bool,
    top_level_label: Option<&Label>,
    event_handler: &dyn ExtendedEventHandler,
    cause_message: Option<&str>,
) {
    // For execution exceptions, we don't print any extra warning.
    if is_execution_exception {
        // PORT: skipped, logging.
        return;
    }
    let mut message = format!(
        "errors encountered while analyzing target '{}', it will not be built.",
        // String.format("%s", null)
        top_level_label
            .map(|l| l.to_string())
            .unwrap_or_else(|| "null".to_owned())
    );
    if let Some(cause_message) = cause_message {
        message.push_str(&format!("\n{}", cause_message));
    }
    event_handler.handle(&Event::warn(message));
}

/// Peel away the wrapper layers to get to the ActionLookupKey of the top level target.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:698 SkyframeErrorProcessor.getEffectiveErrorKey
fn get_effective_error_key(error_entry_key: &SkyKey) -> SkyKey {
    use blaze_sky_exec::keys::{
        AspectCompletionKey, BuildDriverKey, TargetCompletionKey, TestCompletionKey,
    };
    if let Some(key) = error_entry_key.downcast::<BuildDriverKey>() {
        return crate::execution_seam::action_lookup_sky_key(&key.action_lookup_key);
    }
    // For exclusive tests.
    if let Some(key) = error_entry_key.downcast::<TestCompletionKey>() {
        return crate::execution_seam::action_lookup_sky_key(&key.configured_target_key);
    }
    // For non-skymeld action executions.
    if let Some(key) = error_entry_key.downcast::<TargetCompletionKey>() {
        return crate::execution_seam::action_lookup_sky_key(&key.action_lookup_key);
    }
    if let Some(key) = error_entry_key.downcast::<AspectCompletionKey>() {
        return crate::execution_seam::action_lookup_sky_key(&key.action_lookup_key);
    }
    error_entry_key.clone()
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:717 SkyframeErrorProcessor.getLabel
fn get_label(error_key: &SkyKey) -> Option<Label> {
    // errorKey instanceof ActionLookupKey ? ((ActionLookupKey) errorKey).getLabel() : null
    if let Some(k) = error_key.downcast::<ConfiguredTargetKey>() {
        return Some(k.get_label().clone());
    }
    if let Some(k) = error_key.downcast::<AspectKey>() {
        return Some(k.get_label().clone());
    }
    if let Some(k) = error_key.downcast::<TopLevelAspectsKey>() {
        return Some(k.get_label().clone());
    }
    None
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:721 SkyframeErrorProcessor.createViewCreationFailedException
fn create_view_creation_failed_exception(
    e: Option<&SkyError>,
    error_msg: String,
) -> ViewCreationFailedException {
    match e {
        None => ViewCreationFailedException::new(
            error_msg.clone(),
            create_failure_detail(
                format!("{} due to cycle", error_msg),
                failure_details::analysis::Code::Cycle,
            ),
        ),
        Some(e) => {
            let failure_detail = maybe_contextualize_failure_detail(e, &error_msg);
            ViewCreationFailedException::with_cause(error_msg, failure_detail, Some(&e.to_string()))
        }
    }
}

/// Returns a `FailureDetail` with message prefixed by `error_msg` derived from the failure
/// detail in `e` if it's a `DetailedException`, and otherwise returns one with `error_msg` and
/// `Code.UNEXPECTED_ANALYSIS_EXCEPTION`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:736 SkyframeErrorProcessor.maybeContextualizeFailureDetail
fn maybe_contextualize_failure_detail(e: &SkyError, error_msg: &str) -> FailureDetail {
    let Some(original_failure_detail) = convert_to_analysis_exception(e) else {
        return create_failure_detail(
            error_msg.to_owned(),
            failure_details::analysis::Code::UnexpectedAnalysisException,
        );
    };
    FailureDetail {
        message: format!("{}: {}", error_msg, original_failure_detail.message),
        ..original_failure_detail
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:749 SkyframeErrorProcessor.createFailureDetail
fn create_failure_detail(
    error_message: String,
    code: failure_details::analysis::Code,
) -> FailureDetail {
    FailureDetail {
        message: error_message,
        category: Some(Category::Analysis(failure_details::Analysis {
            code: code as i32,
        })),
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:756 SkyframeErrorProcessor.maybeGetConfiguredTargetCycleCulprit
fn maybe_get_configured_target_cycle_culprit(
    label_to_load: &Label,
    cycle_infos: &[CycleInfo],
) -> Option<Label> {
    for cycle_info in cycle_infos {
        let Some(culprit) = cycle_info.get_cycle().first() else {
            continue;
        };
        if culprit.function_name() == CONFIGURED_TARGET {
            return Some(
                culprit
                    .downcast::<ConfiguredTargetKey>()
                    .expect("CONFIGURED_TARGET key")
                    .get_label()
                    .clone(),
            );
        } else if culprit.function_name() == TRANSITIVE_TARGET {
            return Some(
                culprit
                    .downcast::<TransitiveTargetKey>()
                    .expect("TRANSITIVE_TARGET key")
                    .get_label()
                    .clone(),
            );
        } else {
            return Some(label_to_load.clone());
        }
    }
    None
}

/// `convertToAnalysisException(cause)`, reduced to what its callers read:
/// `getDetailedExitCode().getFailureDetail()` of the `DetailedException`, `None` where Java
/// returns null.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:855 SkyframeErrorProcessor.convertToAnalysisException
fn convert_to_analysis_exception(cause: &SkyError) -> Option<FailureDetail> {
    detailed_exception_failure_detail_of(&**cause)
}

/// `DetailedException.getDetailedExitCode(e).getFailureDetail()` for the exception classes this
/// file recognises (see `classify`), `None` for any other class.
pub(crate) fn detailed_exception_failure_detail(e: &SkyError) -> Option<FailureDetail> {
    detailed_exception_failure_detail_of(&**e)
}

/// [`detailed_exception_failure_detail`] for an exception of a cause chain.
pub(crate) fn detailed_exception_failure_detail_of(
    cause: &(dyn std::error::Error + 'static),
) -> Option<FailureDetail> {
    // The cause may be NoSuch{Target,Package}Exception if we run the reduced loading phase and
    // then analyze with --nokeep_going.
    match classify(cause) {
        AnalysisException::ActionConflict(e) => Some(e.get_failure_detail()),
        AnalysisException::ConfiguredValueCreation(e) => {
            Some(packages_failure_detail(e.get_detailed_exit_code()))
        }
        AnalysisException::NoSuchThing(detailed_exit_code) => {
            Some(packages_failure_detail(&detailed_exit_code))
        }
        AnalysisException::ExternalDeps(e) => Some(external_deps_failure_detail(e)),
        AnalysisException::BzlLoadFailed(e) => {
            Some(packages_failure_detail(e.get_detailed_exit_code()))
        }
        AnalysisException::AspectCreation(e) => {
            Some(packages_failure_detail(e.get_detailed_exit_code()))
        }
        AnalysisException::TopLevelAspectsDetailsBuildFailed(e) => {
            Some(packages_failure_detail(e.get_detailed_exit_code()))
        }
        AnalysisException::InvalidExecGroup(message) => blaze_util::unported!(
            "ExecGroupCollection.InvalidExecGroupException.getDetailedExitCode",
            "{message}"
        ),
        AnalysisException::TargetCompatibilityCheck(e) => Some(e.get_failure_detail().clone()),
        // TopLevelConflictException is not a DetailedException.
        AnalysisException::TopLevelConflict(_) | AnalysisException::Other => None,
    }
}

/// `CYCLE_CODE`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:1038 SkyframeErrorProcessor.CYCLE_CODE
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:1050 SkyframeErrorProcessor.createDetailedExecutionExitCode
pub fn cycle_code() -> blaze_util::DetailedExitCode {
    blaze_util::DetailedExitCode::of(FailureDetail {
        message: "cycle found during execution".to_owned(),
        category: Some(Category::Execution(failure_details::Execution {
            code: failure_details::execution::Code::Cycle as i32,
        })),
    })
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:1060 SkyframeErrorProcessor.isExecutionCycle
pub fn is_execution_cycle(cycle_info_collection: &[CycleInfo]) -> bool {
    for cycle_info in cycle_info_collection {
        if cycle_info
            .get_cycle()
            .iter()
            .all(crate::action_artifact_cycle_reporter::action_or_artifact_or_transitive_rdep)
        {
            // All these cycle info belong to the same top level key. If one of them is
            // execution-related, we consider the error to be execution-related.
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_events::StoredEventHandler;
    use blaze_skyframe::{SkyFunctionException, Transience};

    fn analysis_error_key() -> ConfiguredTargetKey {
        ConfiguredTargetKey::builder()
            .set_label(Label::parse_canonical_unchecked("//analysis_err"))
            .build()
    }

    fn analysis_error_info() -> ErrorInfo {
        let analysis_exception = ConfiguredValueCreationException::new(
            None,
            "analysis exception",
            &Label::parse_canonical_unchecked("//analysis_err"),
            None,
            None,
            None,
        );
        ErrorInfo::from_exception(
            &SkyFunctionException::new(
                ConfiguredTargetFunctionException::Reported(analysis_exception),
                Transience::Persistent,
            ),
            /* is_transitively_transient= */ false,
        )
    }

    /// PORT: the Java test is parameterized over `includeExecutionPhase`; only the analysis
    /// half (`false`) is ported. It asserts that the thrown exception's cause is the analysis
    /// exception; the port keeps the cause's message in the exception's message
    /// (`ViewCreationFailedException.combineMessages`).
    // Java: src/test/java/com/google/devtools/build/lib/skyframe/SkyframeErrorProcessorTest.java:46 SkyframeErrorProcessorTest.testProcessErrors_analysisErrorNoKeepGoing_throwsException
    #[test]
    fn test_process_errors_analysis_error_no_keep_going_throws_exception() {
        let event_handler: Arc<dyn ExtendedEventHandler> = Arc::new(StoredEventHandler::new());
        let thrown = process_errors(
            &[(SkyKey::new(analysis_error_key()), analysis_error_info())],
            &CyclesReporter::new(Vec::new()),
            &event_handler,
            /* keep_going= */ false,
        )
        .expect_err("ViewCreationFailedException");
        assert_eq!(
            thrown.get_message(),
            "Analysis of target '//analysis_err:analysis_err' failed; build aborted: analysis \
             exception"
        );
        // maybeContextualizeFailureDetail: the cause's failure detail, its message prefixed.
        assert_eq!(
            thrown.get_failure_detail(),
            &FailureDetail {
                message: "Analysis of target '//analysis_err:analysis_err' failed; build \
                          aborted: analysis exception"
                    .to_owned(),
                category: Some(Category::Analysis(failure_details::Analysis {
                    code: failure_details::analysis::Code::ConfiguredValueCreationFailed as i32,
                })),
            }
        );
    }

    /// The keep_going half of `processErrors` for the same error
    /// (SkyframeErrorProcessor.java:383-386, 665-683): no exception, an analysis error, and the
    /// warning `logOrPrintWarningsKeepGoing` prints.
    #[test]
    fn process_errors_analysis_error_keep_going_warns_and_aggregates() {
        let stored = Arc::new(StoredEventHandler::new());
        let event_handler: Arc<dyn ExtendedEventHandler> = stored.clone();
        let result = process_errors(
            &[(SkyKey::new(analysis_error_key()), analysis_error_info())],
            &CyclesReporter::new(Vec::new()),
            &event_handler,
            /* keep_going= */ true,
        )
        .expect("no exception with keep_going");
        assert!(result.has_analysis_error());
        assert!(!result.has_loading_error());
        assert!(result.action_conflicts().is_empty());
        let messages: Vec<String> = stored
            .get_events()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect();
        assert_eq!(
            messages,
            vec![
                "errors encountered while analyzing target '//analysis_err:analysis_err', it \
                 will not be built.\nanalysis exception"
                    .to_owned()
            ]
        );
        // AnalysisFailureEvent.whileAnalyzingTarget(ctKey, rootCauses)
        assert_eq!(stored.get_posts().len(), 1);
    }

    // Java: com/google/devtools/build/lib/analysis/ViewCreationFailedException.java:49 ViewCreationFailedException.combineMessages
    #[test]
    fn view_creation_failed_exception_combines_messages() {
        let detail = create_failure_detail(
            "m".to_owned(),
            failure_details::analysis::Code::UnexpectedAnalysisException,
        );
        assert_eq!(
            ViewCreationFailedException::with_cause("a", detail.clone(), Some("b")).get_message(),
            "a: b"
        );
        assert_eq!(
            ViewCreationFailedException::with_cause("a", detail.clone(), Some("")).get_message(),
            "a"
        );
        assert_eq!(
            ViewCreationFailedException::with_cause("a", detail, None).get_message(),
            "a"
        );
    }
}

/// `SkyframeErrorProcessor.isExecutionException(cause)`, with the cause's class.
///
/// PORT: `TestExecException` is not a class of its own in blaze_exec (the test strategy raises
/// it as a marked `UserExecException`); the `ActionExecutionException` it causes tells
/// (`is_caused_by_test_exec_exception`), which is the `cause.getCause() instanceof
/// TestExecException` of `rethrow`. A `TestExecException` is never the error of a node itself.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:873 SkyframeErrorProcessor.isExecutionException
pub enum ExecutionExceptionCause<'a> {
    ActionExecution(&'a ActionExecutionException),
    InputFileError(&'a blaze_sky_exec::completion_function::InputFileErrorException),
    ArtifactNestedSetEval(&'a blaze_sky_exec::artifact_function::ArtifactNestedSetEvalException),
    // For top-level outputs errors in CompletionFunction.
    TopLevelOutput(&'a blaze_sky_exec::completion_function::TopLevelOutputException),
}

impl<'a> ExecutionExceptionCause<'a> {
    pub fn of(cause: &'a blaze_skyframe::SkyError) -> Option<ExecutionExceptionCause<'a>> {
        if let Some(e) = cause.downcast_ref::<ActionExecutionException>() {
            return Some(ExecutionExceptionCause::ActionExecution(e));
        }
        if let Some(e) =
            cause.downcast_ref::<blaze_sky_exec::completion_function::InputFileErrorException>()
        {
            return Some(ExecutionExceptionCause::InputFileError(e));
        }
        if let Some(e) = cause
            .downcast_ref::<blaze_sky_exec::artifact_function::ArtifactNestedSetEvalException>()
        {
            return Some(ExecutionExceptionCause::ArtifactNestedSetEval(e));
        }
        if let Some(e) =
            cause.downcast_ref::<blaze_sky_exec::completion_function::TopLevelOutputException>()
        {
            return Some(ExecutionExceptionCause::TopLevelOutput(e));
        }
        None
    }

    /// `DetailedException.getDetailedExitCode(cause)`, or the "unknown" code for a cause that is
    /// not a `DetailedException`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:621 SkyframeErrorProcessor.getExecutionDetailedExitCodeFromCause
    pub fn get_execution_detailed_exit_code(&self) -> blaze_util::DetailedExitCode {
        match self {
            ExecutionExceptionCause::ActionExecution(e) => match e.get_failure_detail() {
                Some(failure_detail) => blaze_util::DetailedExitCode::of(failure_detail.clone()),
                None => panic!(
                    "NullPointerException: detailedExitCode of {}",
                    e.get_message()
                ),
            },
            ExecutionExceptionCause::InputFileError(e) => e.detailed_exit_code.clone(),
            ExecutionExceptionCause::TopLevelOutput(e) => e.detailed_exit_code.clone(),
            ExecutionExceptionCause::ArtifactNestedSetEval(e) => {
                create_unknown_execution_detailed_exit_code(&e.to_string())
            }
        }
    }
}

/// The exit code of an execution exception without a `DetailedExitCode`.
///
/// PORT: skipped, the non-fatal bug report Java sends first.
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:631 SkyframeErrorProcessor.sendBugReportAndCreateUnknownExecutionDetailedExitCode
pub fn create_unknown_execution_detailed_exit_code(
    cause_message: &str,
) -> blaze_util::DetailedExitCode {
    let message =
        format!("Unexpected exception, please file an issue with the Bazel team: {cause_message}");
    blaze_util::DetailedExitCode::of(FailureDetail {
        message,
        category: Some(Category::Execution(failure_details::Execution {
            code: failure_details::execution::Code::UnexpectedException as i32,
        })),
    })
}

/// What [`rethrow`] throws.
#[derive(Clone, Debug)]
pub enum RethrownException {
    /// A `BuildFailedException` (`crate::build_tool::BuildToolError::BuildFailed`), with its `isCatastrophic()`.
    BuildFailed {
        error: crate::build_tool::BuildToolError,
        catastrophic: bool,
    },
    /// The `TestExecException` (its message) that caused an `ActionExecutionException`: a test
    /// failed without `--test_keep_going`.
    TestExec(String),
}

impl RethrownException {
    pub fn build_failed(error: crate::build_tool::BuildToolError) -> RethrownException {
        RethrownException::BuildFailed {
            error,
            catastrophic: false,
        }
    }
}

/// Figure out why an action's analysis/execution failed and rethrow the right kind of exception.
///
/// PORT: returns the `BuildFailedException`; an empty message stands for
/// `isErrorAlreadyShown()` (`BuildTool.processRequest` reports the others).
// Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:974 SkyframeErrorProcessor.rethrow
pub fn rethrow(cause: &ExecutionExceptionCause<'_>) -> RethrownException {
    if let ExecutionExceptionCause::ActionExecution(e) = cause
        && e.is_caused_by_test_exec_exception()
    {
        // innerCause instanceof TestExecException: throw testExecException.
        // PORT: the ActionExecutionException's message is the TestExecException's
        // (`ActionExecutionException.fromExecException` without a subtask).
        return RethrownException::TestExec(e.get_message().to_owned());
    }
    match cause {
        ExecutionExceptionCause::ActionExecution(action_execution_cause) => {
            let mut message = action_execution_cause.get_message().to_owned();
            if let Some(action) = action_execution_cause.get_action() {
                message = format!("{} failed: {}", action.describe, message);
                // Sometimes ActionExecutionExceptions are caused by Actions with no owner.
                // PORT: the snapshot always has the owner's location.
                message = format!("{} {}", action.location, message);
            }
            let failure_detail = cause
                .get_execution_detailed_exit_code()
                .get_failure_detail()
                .cloned()
                .expect("NullPointerException: detailedExitCode");
            RethrownException::BuildFailed {
                error: crate::build_tool::BuildToolError::BuildFailed {
                    // errorAlreadyShown = !actionExecutionCause.showError()
                    message: if action_execution_cause.show_error() {
                        message
                    } else {
                        String::new()
                    },
                    failure_detail,
                },
                catastrophic: action_execution_cause.is_catastrophe(),
            }
        }
        // throw inputFileErrorException: a BuildFailedException whose error is already shown.
        ExecutionExceptionCause::InputFileError(input_file_error_exception) => {
            RethrownException::build_failed(crate::build_tool::BuildToolError::BuildFailed {
                message: String::new(),
                failure_detail: input_file_error_exception
                    .detailed_exit_code
                    .get_failure_detail()
                    .cloned()
                    .expect("NullPointerException: detailedExitCode"),
            })
        }
        // throw topLevelOutputException: a BuildFailedException whose error is already shown
        // (TopLevelOutputException.<init>: errorAlreadyShown = true, not catastrophic).
        // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:1000 SkyframeErrorProcessor.rethrow
        ExecutionExceptionCause::TopLevelOutput(top_level_output_exception) => {
            RethrownException::build_failed(crate::build_tool::BuildToolError::BuildFailed {
                message: String::new(),
                failure_detail: top_level_output_exception
                    .detailed_exit_code
                    .get_failure_detail()
                    .cloned()
                    .expect("NullPointerException: detailedExitCode"),
            })
        }
        // We encountered an exception we don't think we should have encountered. This can
        // indicate an exception-processing bug in our code, such as lower level exceptions not
        // being properly handled, or in our expectations in this method.
        ExecutionExceptionCause::ArtifactNestedSetEval(_) => {
            let failure_detail = cause
                .get_execution_detailed_exit_code()
                .get_failure_detail()
                .cloned()
                .expect("NullPointerException: unknownExitCode.getFailureDetail()");
            RethrownException::build_failed(crate::build_tool::BuildToolError::BuildFailed {
                message: failure_detail.message.clone(),
                failure_detail,
            })
        }
    }
}
