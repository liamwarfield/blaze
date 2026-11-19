//! Port of `buildtool/ExecutionProgressReceiver.java` and of `ExecutionTool`'s
//! `ExecutionProgressReceiverSetup` (the Skymeld subscriber that creates the receiver on the first
//! `SomeExecutionStartedEvent`).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use blaze_actions::ActionLookupData;
use blaze_events::{EventBus, Postable};
use blaze_sky_exec::events::{SomeExecutionStartedEvent, downcast_event};
use blaze_sky_exec::keys::ActionExecutionKey;
use blaze_skyframe::{ErrorInfo, EvaluationProgressReceiver, EvaluationState, GroupedDeps};
use blaze_skyframe::{Key, SkyKey, SkyValue};
use parking_lot::Mutex;

use crate::buildtool::buildevent::ExecutionProgressReceiverAvailableEvent;
use crate::runtime::ui_state_tracker::ExecutionProgress;

/// `NumberFormat.getIntegerInstance(Locale.ENGLISH)` with grouping: `12,345`.
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:58 ExecutionProgressReceiver.PROGRESS_MESSAGE_NUMBER_FORMATTER
fn format_grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Listener for executed actions and built artifacts. We use a listener so that we have an
/// accurate set of successfully run actions and built artifacts, even if the build is
/// interrupted.
///
/// PORT: the `TopLevelTargetBuiltEvent` / `AspectBuiltEvent` posts of `evaluated` are read from
/// the graph after the evaluation by the build command (blaze_runtime `commands/build.rs`,
/// `top_level_target_built` and `top_level_aspect_built`), which predates this receiver; they are
/// not posted here a second time. `changePruned`'s `ActionChangePrunedEvent` needs change pruning
/// of action nodes, which a command-per-process build never does. The inactivity monitor and
/// reporter (`createInactivityMonitor`, `createInactivityReporter`) belong to
/// `ActionExecutionInactivityWatchdog`, which is not ported.
// Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:54 ExecutionProgressReceiver
pub struct ExecutionProgressReceiver {
    enqueued_actions: Mutex<HashSet<ActionLookupData>>,
    completed_actions: Mutex<HashSet<ActionLookupData>>,
    /// Number of exclusive tests. To be accounted for in progress messages.
    exclusive_tests_count: i32,
}

impl ExecutionProgressReceiver {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:77 ExecutionProgressReceiver.<init>
    pub fn new(exclusive_tests_count: i32) -> ExecutionProgressReceiver {
        ExecutionProgressReceiver {
            enqueued_actions: Mutex::new(HashSet::new()),
            completed_actions: Mutex::new(HashSet::new()),
            exclusive_tests_count,
        }
    }

    /// This method adds the action lookup data to `completedActions`.
    ///
    /// We could do this only in the `evaluated` method too, but as it happens the action
    /// executor tells the reporter about the completed action before the node is inserted into
    /// the graph, so the reporter would find out about the completed action sooner than we could
    /// have updated `completedActions`, which would result in incorrect numbers on the progress
    /// messages. However we have to store completed actions in `evaluated` too, because that's
    /// the only place we get notified about completed cached actions.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:175 ExecutionProgressReceiver.actionCompleted
    pub fn action_completed(&self, action_lookup_data: &ActionLookupData) {
        self.enqueued_actions
            .lock()
            .insert(action_lookup_data.clone());
        self.completed_actions
            .lock()
            .insert(action_lookup_data.clone());
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:181 ExecutionProgressReceiver.getProgressString
    pub fn get_progress_string(&self) -> String {
        let completed = self.completed_actions.lock().len();
        let enqueued = self.enqueued_actions.lock().len();
        format!(
            "[{} / {}]",
            format_grouped(completed),
            format_grouped(self.exclusive_tests_count as usize + enqueued)
        )
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:237 ExecutionProgressReceiver.hasActionsInFlight
    pub fn has_actions_in_flight(&self) -> bool {
        let completed = self.completed_actions.lock().len();
        let enqueued = self.enqueued_actions.lock().len();
        completed < self.exclusive_tests_count as usize + enqueued
    }
}

impl EvaluationProgressReceiver for ExecutionProgressReceiver {
    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:83 ExecutionProgressReceiver.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        if sky_key.function_name() == ActionExecutionKey::FUNCTION
            && let Some(key) = sky_key.downcast::<ActionExecutionKey>()
        {
            // Remember all enqueued actions for the benefit of progress reporting.
            // We discover most actions early in the build, well before we start executing them.
            // Some of these will be cache hits and won't be executed, so we'll need to account
            // for them in the evaluated method too.
            self.enqueued_actions.lock().insert(key.0.clone());
        }
    }

    // Java: com/google/devtools/build/lib/buildtool/ExecutionProgressReceiver.java:95 ExecutionProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        _state: EvaluationState,
        _new_value: Option<&SkyValue>,
        _new_error: Option<&ErrorInfo>,
        _direct_deps: Option<&GroupedDeps>,
    ) {
        if sky_key.function_name() == ActionExecutionKey::FUNCTION
            && let Some(key) = sky_key.downcast::<ActionExecutionKey>()
        {
            // Remember all completed actions, even those in error, regardless of having been
            // cached or really executed.
            self.action_completed(&key.0);
        }
        // PORT: TARGET_COMPLETION, ASPECT_COMPLETION and BUILD_DRIVER: see the type's comment.
    }
}

impl ExecutionProgress for ExecutionProgressReceiver {
    fn get_progress_string(&self) -> String {
        ExecutionProgressReceiver::get_progress_string(self)
    }

    fn has_actions_in_flight(&self) -> bool {
        ExecutionProgressReceiver::has_actions_in_flight(self)
    }

    fn action_completed(&self, action_lookup_data: &ActionLookupData) {
        ExecutionProgressReceiver::action_completed(self, action_lookup_data)
    }
}

/// A listener that prepares the ExecutionProgressReceiver upon receiving the first
/// SomeExecutionStartedEvent. Only activated once a build.
///
/// PORT: `set_execution_progress_receiver` stands for
/// `skyframeExecutor.setExecutionProgressReceiver(executionProgressReceiver)` (the executor
/// sits above this crate). `setActionExecutionProgressReportingObjects` (the action executor's
/// progress supplier and completion receiver, the status reporter) and the inactivity watchdog
/// are not ported; the action completions reach the receiver through the UI's
/// `actionCompletion` and through `evaluated`. Blaze has no `executionUnstartedTimer`. Blaze's
/// build always drives execution with `BuildDriverKey`s, so the setup also serves a build
/// without merged analysis and execution, where Java's `SkyframeBuilder.buildArtifacts` posts
/// the receiver with the count of exclusive tests.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1030 ExecutionTool.ExecutionProgressReceiverSetup
pub struct ExecutionProgressReceiverSetup {
    event_bus: Arc<dyn EventBus>,
    set_execution_progress_receiver: Box<dyn Fn(Arc<ExecutionProgressReceiver>) + Send + Sync>,
    progress_receiver_started: AtomicBool,
}

impl ExecutionProgressReceiverSetup {
    pub fn new(
        event_bus: Arc<dyn EventBus>,
        set_execution_progress_receiver: Box<dyn Fn(Arc<ExecutionProgressReceiver>) + Send + Sync>,
    ) -> ExecutionProgressReceiverSetup {
        ExecutionProgressReceiverSetup {
            event_bus,
            set_execution_progress_receiver,
            progress_receiver_started: AtomicBool::new(false),
        }
    }

    /// `@Subscribe setupExecutionProgressReceiver(SomeExecutionStartedEvent)`.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1051 ExecutionTool.ExecutionProgressReceiverSetup.setupExecutionProgressReceiver
    pub fn setup_execution_progress_receiver(&self) {
        if self
            .progress_receiver_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            // TODO(leba): count test actions
            let execution_progress_receiver = Arc::new(ExecutionProgressReceiver::new(
                /* exclusive_tests_count= */ 0,
            ));
            self.event_bus
                .post(Arc::new(ExecutionProgressReceiverAvailableEvent::new(
                    execution_progress_receiver.clone(),
                )));
            (self.set_execution_progress_receiver)(execution_progress_receiver);
        }
    }
}

impl EventBus for ExecutionProgressReceiverSetup {
    fn post(&self, event: Arc<dyn Postable>) {
        if downcast_event::<SomeExecutionStartedEvent>(&event).is_some() {
            self.setup_execution_progress_receiver();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_numbers_are_grouped_as_locale_english() {
        assert_eq!(format_grouped(0), "0");
        assert_eq!(format_grouped(999), "999");
        assert_eq!(format_grouped(1000), "1,000");
        assert_eq!(format_grouped(1234567), "1,234,567");
        let receiver = ExecutionProgressReceiver::new(0);
        assert_eq!(receiver.get_progress_string(), "[0 / 0]");
        assert!(!receiver.has_actions_in_flight());
    }
}
