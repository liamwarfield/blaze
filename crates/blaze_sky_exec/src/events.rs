//! Events the execution phase posts on the event bus.
//!
//! PORT: Java events hold the `Action` itself; posted values must own their data here, so each
//! event keeps the action as an `ActionRef` (shared) plus the fields consumers read.

// Events and small classes of the execution cluster with nothing to port at the pin:
// PORT: skipped, no class of Bazel 9.3.2 posts or subscribes to it: InjectedActionExecutionEvent.InjectedActionExecutionEvent InjectedActionExecutionEvent.getAction
// PORT: skipped, posted with no subscriber in Bazel: ChangedFilesMessage.ChangedFilesMessage ChangedFilesMessage.create BaselineClDiffEvent.BaselineClDiffEvent BaselineClDiffEvent.getBaselineClDiff WorkspaceInfoFromDiff.getEvaluatingVersion WorkspaceInfoFromDiff.getSnapshot
// PORT: skipped, posted with no subscriber in Bazel (Google-internal consumers): SourceDirectoryEvent.SourceDirectoryEvent SourceDirectoryEvent.create ActionInputCollectedEvent.ActionInputCollectedEvent ActionInputCollectedEvent.create
// PORT: skipped, its only subscriber, ExecutionGraphModule, stops with unported! (--experimental_enable_execution_graph_log): SharedActionEvent.SharedActionEvent SharedActionEvent.getExecuted SharedActionEvent.getTransformed
// PORT: skipped, no user at the pin: SingleSourcePackageRoots.SingleSourcePackageRoots SingleSourcePackageRoots.getPackageRootsMap SingleSourcePackageRoots.getPackageRootLookup
// PORT: skipped, key interning: InternerWithPresenceCheck.intern InternerWithPresenceCheck.getCanonical
// PORT: skipped, no PRECOMPUTED function is registered: an unset precomputed value fails in the evaluator, as Java's compute throws IllegalStateException (blaze_skyframe precomputed): PrecomputedFunction.compute
// PORT: skipped, private constructors of static classes: ActionUtils.ActionUtils ActionInputMapHelper.ActionInputMapHelper PathMapperConstants.PathMapperConstants

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact};
use blaze_cmdline::Label;
use blaze_events::Postable;
use blaze_exec::actions::file_artifact_value::FileArtifactValue;
use blaze_exec::actions::{ActionExecutionException, ActionResult};
use blaze_vfs::{Path, PathFragment};

use crate::analysis_seam::ConfiguredTargetView;

/// This event is fired during the build, when an action is started.
// Java: com/google/devtools/build/lib/actions/ActionStartedEvent.java:24 ActionStartedEvent
// Java: com/google/devtools/build/lib/actions/ActionStartedEvent.java:39 ActionStartedEvent.getAction
// Java: com/google/devtools/build/lib/actions/ActionStartedEvent.java:43 ActionStartedEvent.getNanoTimeStart
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct ActionStartedEvent {
    pub action: ActionRef,
    pub nano_time_start: Instant,
}

/// An event that is fired after an action completes (either successfully or not).
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:25 ActionCompletionEvent
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:44 ActionCompletionEvent.getAction
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:53 ActionCompletionEvent.getRelativeActionStartTimeNanos
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:57 ActionCompletionEvent.getFinishTimeNanos
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:61 ActionCompletionEvent.getActionLookupData
// (the getters are the public fields)
#[derive(Clone)]
pub struct ActionCompletionEvent {
    pub related_time_start: Instant,
    pub finish_time: Instant,
    pub action: ActionRef,
    pub action_lookup_data: ActionLookupData,
}

/// An event that is fired after an action successfully executes, with its `ActionResult`.
// Java: com/google/devtools/build/lib/actions/ActionResultReceivedEvent.java:22 ActionResultReceivedEvent
// Java: com/google/devtools/build/lib/actions/ActionResultReceivedEvent.java:33 ActionResultReceivedEvent.getAction
// Java: com/google/devtools/build/lib/actions/ActionResultReceivedEvent.java:38 ActionResultReceivedEvent.getActionResult
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct ActionResultReceivedEvent {
    pub action: ActionRef,
    pub action_result: ActionResult,
}

/// `ActionExecutedEvent.ErrorTiming`.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:56 ActionExecutedEvent.ErrorTiming
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ErrorTiming {
    NO_ERROR,
    BEFORE_EXECUTION,
    AFTER_EXECUTION,
}

/// This event is fired during the build, when an action is executed. It contains information
/// about the action: the Action itself, and the output file names its stdout and stderr are
/// recorded in.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:66 ActionExecutedEvent
#[derive(Clone)]
pub struct ActionExecutedEvent {
    pub action_id: PathFragment,
    pub action: ActionRef,
    pub exception: Option<ActionExecutionException>,
    pub primary_output_path: Path,
    pub output_artifact: Artifact,
    pub stdout: Option<Path>,
    pub stderr: Option<Path>,
    pub error_timing: ErrorTiming,
    /// `getPrimaryOutputMetadata()`: set exactly when there is no exception.
    pub primary_output_metadata: Option<FileArtifactValue>,
    /// The first start time of the action's spawns (`null` without a timed spawn).
    pub start_time: Option<std::time::SystemTime>,
    /// The last end time of the action's spawns (start time plus wall time).
    pub end_time: Option<std::time::SystemTime>,
}

/// This event is fired after a top-level target is completed (all its requested outputs built).
// Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:83 TargetCompleteEvent
#[derive(Clone, Debug)]
pub struct TargetCompleteEvent {
    pub label: Label,
    pub configuration_checksum: Option<String>,
    /// Root causes of a failure (labels of failed actions' owners), empty on success.
    pub root_causes: Vec<Label>,
    /// The requested output groups and their artifacts, in order.
    pub output_groups: Vec<(String, Vec<Artifact>)>,
    pub is_test: bool,
    pub failed: bool,
    /// `artifactsToBuild.getAllArtifactsByOutputGroup()` (succeeded) or the
    /// `SuccessfulArtifactFilter`-filtered groups (failed), in order.
    pub artifacts_by_output_group: Vec<(String, ArtifactsInOutputGroup)>,
    /// The `CompletionContext`'s important input map: metadata of the built important artifacts.
    pub important_input_map: Vec<(Artifact, FileArtifactValue)>,
    /// Important tree artifacts with their metadata and their children's.
    pub important_tree_artifacts: Vec<(
        Artifact,
        FileArtifactValue,
        Vec<(Artifact, FileArtifactValue)>,
    )>,
    /// `getRootCauses()` with their failure details (failed targets).
    pub root_cause_details: Vec<CompletionRootCause>,
}

/// This event is fired as soon as a top-level aspect is either built or fails.
// Java: com/google/devtools/build/lib/analysis/AspectCompleteEvent.java:44 AspectCompleteEvent
#[derive(Clone, Debug)]
pub struct AspectCompleteEvent {
    /// `getAspectKey()`.
    pub aspect_key: ActionLookupKeyRef,
    /// `getLabel()` (`aspectKey.getLabel()`).
    pub label: Label,
    /// `getAspectName()` (`aspectKey.getAspectDescriptor().getAspectClass().getName()`).
    pub aspect_name: String,
    /// Labels of the root causes of a failure, empty on success.
    pub root_causes: Vec<Label>,
    /// `getRootCauses()` with their failure details (failed aspects).
    pub root_cause_details: Vec<CompletionRootCause>,
    /// The outputs (`artifactOutputGroups`): `artifactsToBuild.getAllArtifactsByOutputGroup()`
    /// (succeeded) or the `SuccessfulArtifactFilter`-filtered groups (failed), in order.
    pub artifacts_by_output_group: Vec<(String, ArtifactsInOutputGroup)>,
    /// The `CompletionContext`'s important input map: metadata of the built important artifacts.
    pub important_input_map: Vec<(Artifact, FileArtifactValue)>,
    /// Important tree artifacts with their metadata and their children's.
    pub important_tree_artifacts: Vec<(
        Artifact,
        FileArtifactValue,
        Vec<(Artifact, FileArtifactValue)>,
    )>,
    /// `printToMasterLog` (`AspectValue.getWritesOutputToMasterLog()`).
    pub print_to_master_log: bool,
}

impl AspectCompleteEvent {
    /// Determines whether the target has failed or succeeded.
    // Java: com/google/devtools/build/lib/analysis/AspectCompleteEvent.java:100 AspectCompleteEvent.failed
    pub fn failed(&self) -> bool {
        !self.root_cause_details.is_empty()
    }
}

/// `TargetCompleteEvent.ArtifactsInOutputGroup`: an output group's artifacts.
// Java: com/google/devtools/build/lib/analysis/TopLevelArtifactHelper.java:85 TopLevelArtifactHelper.ArtifactsInOutputGroup
#[derive(Clone, Debug)]
pub struct ArtifactsInOutputGroup {
    pub artifacts: blaze_collect::NestedSet<Artifact>,
    pub important: bool,
    pub incomplete: bool,
}

/// A root cause of a failed top-level target (Java `Cause`: `ActionFailed` or `LabelCause`).
// Java: com/google/devtools/build/lib/causes/Cause.java:26 Cause
#[derive(Clone, Debug, PartialEq)]
pub enum CompletionRootCause {
    ActionFailed {
        exec_path: blaze_vfs::PathFragment,
        label: Option<Label>,
        configuration_checksum: Option<String>,
        failure_detail: blaze_protos::failure_details::FailureDetail,
    },
    Label {
        label: Label,
        failure_detail: blaze_protos::failure_details::FailureDetail,
    },
}

impl TargetCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:329 TargetCompleteEvent.failed
    pub fn failed(&self) -> bool {
        self.failed
    }
}

/// Posted when a top-level target or aspect finished analysis (Skymeld), with the packages it
/// transitively loaded, so that the execroot symlinks to their roots can be planted.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:68 TopLevelStatusEvents.TopLevelTargetReadyForSymlinkPlanting
// (`create` is the struct literal)
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:75 TopLevelStatusEvents.TopLevelTargetReadyForSymlinkPlanting.create
#[derive(Clone, Debug)]
pub struct TopLevelTargetReadyForSymlinkPlanting {
    pub transitive_packages_for_symlink_planting: blaze_collect::NestedSet<
        blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
    >,
}

/// Posted when a top-level target finished analysis and starts execution (Skymeld).
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:80 TopLevelStatusEvents.TopLevelTargetPendingExecutionEvent
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:134 TopLevelStatusEvents.TopLevelTargetPendingExecutionEvent.create
#[derive(Clone, Debug)]
pub struct TopLevelTargetPendingExecutionEvent {
    pub label: Label,
    pub is_test: bool,
}

/// Posted when a top-level test target finished analysis (Skymeld), before its tests are
/// requested; `AggregatingTestListener.populateTest` creates the target's aggregator from it.
///
/// PORT: Java holds the `BuildConfigurationValue`; the test summary classes read its
/// `checksum()` and `getMnemonic()` only (`ConfigurationView`), and this crate sits below the
/// configuration crate, so the event carries those two values.
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:185 TopLevelStatusEvents.TestAnalyzedEvent
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:195 TopLevelStatusEvents.TestAnalyzedEvent.create
#[derive(Clone, Debug)]
pub struct TestAnalyzedEvent {
    pub configured_target: Arc<dyn ConfiguredTargetView>,
    /// `buildConfigurationValue().checksum()`.
    pub configuration_checksum: String,
    /// `buildConfigurationValue().getMnemonic()`.
    pub configuration_mnemonic: String,
    pub is_skipped: bool,
}

/// Posted when a top-level target was built (Skymeld).
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:120 TopLevelStatusEvents.TopLevelTargetBuiltEvent
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:174 TopLevelStatusEvents.TopLevelTargetBuiltEvent.create
#[derive(Clone, Debug)]
pub struct TopLevelTargetBuiltEvent {
    pub label: Label,
}

/// Notifies that an in-flight action is running input discovery.
// Java: com/google/devtools/build/lib/actions/ScanningActionEvent.java:24 ScanningActionEvent
// Java: com/google/devtools/build/lib/actions/ScanningActionEvent.java:34 ScanningActionEvent.getActionMetadata
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct ScanningActionEvent {
    /// `getActionMetadata()`.
    pub action: ActionRef,
}

/// Notifies that an in-flight action stopped running input discovery.
// Java: com/google/devtools/build/lib/actions/StoppedScanningActionEvent.java:20 StoppedScanningActionEvent
// Java: com/google/devtools/build/lib/actions/StoppedScanningActionEvent.java:27 StoppedScanningActionEvent.getAction
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct StoppedScanningActionEvent {
    /// `getAction()`.
    pub action: ActionRef,
}

/// Notifies that an in-flight action is checking the cache.
// Java: com/google/devtools/build/lib/actions/CachingActionEvent.java:27 CachingActionEvent
// Java: com/google/devtools/build/lib/actions/CachingActionEvent.java:34 CachingActionEvent.create
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct CachingActionEvent {
    pub action: ActionRef,
    pub strategy: String,
}

/// Notifies that an in-flight action is being scheduled (waiting for resources).
// Java: com/google/devtools/build/lib/actions/SchedulingActionEvent.java:28 SchedulingActionEvent
// Java: com/google/devtools/build/lib/actions/SchedulingActionEvent.java:40 SchedulingActionEvent.getActionMetadata
// Java: com/google/devtools/build/lib/actions/SchedulingActionEvent.java:45 SchedulingActionEvent.getStrategy
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct SchedulingActionEvent {
    /// `getActionMetadata()`.
    pub action: ActionRef,
    /// `getStrategy()`.
    pub strategy: String,
}

/// Notifies that an in-flight action is running.
// Java: com/google/devtools/build/lib/actions/RunningActionEvent.java:29 RunningActionEvent
// Java: com/google/devtools/build/lib/actions/RunningActionEvent.java:41 RunningActionEvent.getActionMetadata
// Java: com/google/devtools/build/lib/actions/RunningActionEvent.java:46 RunningActionEvent.getStrategy
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct RunningActionEvent {
    /// `getActionMetadata()`.
    pub action: ActionRef,
    /// `getStrategy()`.
    pub strategy: String,
}

/// Notifies that an in-flight action has made progress.
// Java: com/google/devtools/build/lib/actions/ActionProgressEvent.java:29 ActionProgressEvent
// Java: com/google/devtools/build/lib/actions/ActionProgressEvent.java:38 ActionProgressEvent.create
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct ActionProgressEvent {
    pub action: ActionRef,
    pub progress_id: String,
    pub progress: String,
    pub finished: bool,
}

/// Event for an action that has finished input discovery (the action of a shared action that
/// discovers inputs).
// Java: com/google/devtools/build/lib/actions/ActionScanningCompletedEvent.java:23 ActionScanningCompletedEvent
// Java: com/google/devtools/build/lib/actions/ActionScanningCompletedEvent.java:33 ActionScanningCompletedEvent.getAction
// Java: com/google/devtools/build/lib/actions/ActionScanningCompletedEvent.java:37 ActionScanningCompletedEvent.getActionLookupData
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct ActionScanningCompletedEvent {
    pub action: ActionRef,
    pub action_lookup_data: ActionLookupData,
}

/// Event for the metrics of input discovery of an action.
// Java: com/google/devtools/build/lib/actions/DiscoveredInputsEvent.java:20 DiscoveredInputsEvent
// Java: com/google/devtools/build/lib/actions/DiscoveredInputsEvent.java:31 DiscoveredInputsEvent.getMetrics
// Java: com/google/devtools/build/lib/actions/DiscoveredInputsEvent.java:35 DiscoveredInputsEvent.getAction
// Java: com/google/devtools/build/lib/actions/DiscoveredInputsEvent.java:39 DiscoveredInputsEvent.getStartTimeNanos
// (the getters are the public fields; `create` is the struct literal)
#[derive(Clone)]
pub struct DiscoveredInputsEvent {
    /// `getMetrics()`.
    pub metrics: blaze_exec::actions::spawn_metrics::SpawnMetrics,
    /// `getAction()`.
    pub action: ActionRef,
    /// `getStartTimeNanos()`.
    pub start_time_nanos: Instant,
}

/// Signals that the first target finished analysis and execution is about to start (or, not
/// counted in execution time, that the workspace status actions of a Skymeld build may run).
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:151 TopLevelStatusEvents.SomeExecutionStartedEvent
#[derive(Clone, Copy)]
pub struct SomeExecutionStartedEvent {
    pub counted_in_execution_time: bool,
}

impl SomeExecutionStartedEvent {
    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:154 TopLevelStatusEvents.SomeExecutionStartedEvent.create
    pub fn create() -> SomeExecutionStartedEvent {
        SomeExecutionStartedEvent {
            counted_in_execution_time: true,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:158 TopLevelStatusEvents.SomeExecutionStartedEvent.notCountedInExecutionTime
    pub fn not_counted_in_execution_time() -> SomeExecutionStartedEvent {
        SomeExecutionStartedEvent {
            counted_in_execution_time: false,
        }
    }
}

macro_rules! postable {
    ($($t:ty => $name:literal),* $(,)?) => {
        $(
            impl fmt::Debug for $t {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str($name)
                }
            }
            impl Postable for $t {}
        )*
    };
}

postable! {
    ActionStartedEvent => "ActionStartedEvent",
    ActionCompletionEvent => "ActionCompletionEvent",
    ActionResultReceivedEvent => "ActionResultReceivedEvent",
    ActionExecutedEvent => "ActionExecutedEvent",
    ScanningActionEvent => "ScanningActionEvent",
    StoppedScanningActionEvent => "StoppedScanningActionEvent",
    CachingActionEvent => "CachingActionEvent",
    SchedulingActionEvent => "SchedulingActionEvent",
    RunningActionEvent => "RunningActionEvent",
    ActionProgressEvent => "ActionProgressEvent",
    ActionScanningCompletedEvent => "ActionScanningCompletedEvent",
    DiscoveredInputsEvent => "DiscoveredInputsEvent",
    SomeExecutionStartedEvent => "SomeExecutionStartedEvent",
}

impl Postable for TargetCompleteEvent {}
impl Postable for AspectCompleteEvent {}
impl Postable for TopLevelTargetPendingExecutionEvent {}
impl Postable for TopLevelTargetBuiltEvent {}
impl Postable for TopLevelTargetReadyForSymlinkPlanting {}
impl Postable for TestAnalyzedEvent {}

/// Downcasts a posted event (`Arc<dyn Postable>`) to `T`.
pub fn downcast_event<T: 'static>(event: &Arc<dyn Postable>) -> Option<&T> {
    let any: &dyn std::any::Any = &**event;
    any.downcast_ref::<T>()
}
