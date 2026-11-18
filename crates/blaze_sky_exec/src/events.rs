//! Events the execution phase posts on the event bus.
//!
//! PORT: Java events hold the `Action` itself; posted values must own their data here, so each
//! event keeps the action as an `ActionRef` (shared) plus the fields consumers read.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use blaze_actions::actions::action::ActionRef;
use blaze_actions::{ActionLookupData, Artifact};
use blaze_cmdline::Label;
use blaze_events::Postable;
use blaze_exec::actions::{ActionExecutionException, ActionResult};
use blaze_vfs::{Path, PathFragment};

/// This event is fired during the build, when an action is started.
// Java: com/google/devtools/build/lib/actions/ActionStartedEvent.java:24 ActionStartedEvent
#[derive(Clone)]
pub struct ActionStartedEvent {
    pub action: ActionRef,
    pub nano_time_start: Instant,
}

/// An event that is fired after an action completes (either successfully or not).
// Java: com/google/devtools/build/lib/actions/ActionCompletionEvent.java:25 ActionCompletionEvent
#[derive(Clone)]
pub struct ActionCompletionEvent {
    pub related_time_start: Instant,
    pub finish_time: Instant,
    pub action: ActionRef,
    pub action_lookup_data: ActionLookupData,
}

/// An event that is fired after an action successfully executes, with its `ActionResult`.
// Java: com/google/devtools/build/lib/actions/ActionResultReceivedEvent.java:22 ActionResultReceivedEvent
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
}

impl TargetCompleteEvent {
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:329 TargetCompleteEvent.failed
    pub fn failed(&self) -> bool {
        self.failed
    }
}

/// Posted when a top-level target finished analysis and starts execution (Skymeld).
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:80 TopLevelStatusEvents.TopLevelTargetPendingExecutionEvent
#[derive(Clone, Debug)]
pub struct TopLevelTargetPendingExecutionEvent {
    pub label: Label,
    pub is_test: bool,
}

/// Posted when a top-level target was built (Skymeld).
// Java: com/google/devtools/build/lib/skyframe/TopLevelStatusEvents.java:120 TopLevelStatusEvents.TopLevelTargetBuiltEvent
#[derive(Clone, Debug)]
pub struct TopLevelTargetBuiltEvent {
    pub label: Label,
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
}

impl Postable for TargetCompleteEvent {}
impl Postable for TopLevelTargetPendingExecutionEvent {}
impl Postable for TopLevelTargetBuiltEvent {}

/// Downcasts a posted event (`Arc<dyn Postable>`) to `T`.
pub fn downcast_event<T: 'static>(event: &Arc<dyn Postable>) -> Option<&T> {
    let any: &dyn std::any::Any = &**event;
    any.downcast_ref::<T>()
}
