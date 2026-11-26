//! Port of `actions/ActionExecutionStatusReporter.java`.
//!
//! PORT: Java keeps it in `lib/actions`; it subscribes to the execution events of
//! `blaze_sky_exec::events`, so it lives in this crate, above them.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use blaze_actions::actions::action::ActionRef;
use blaze_events::{Clock, Event, EventBus, EventHandler, JavaClock, Postable};
use blaze_sky_exec::events::{
    ActionStartedEvent, RunningActionEvent, ScanningActionEvent, SchedulingActionEvent,
    StoppedScanningActionEvent, downcast_event,
};
use parking_lot::Mutex;

// Maximum number of lines to output per each status category before truncation.
// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:51 ActionExecutionStatusReporter.MAX_LINES
const MAX_LINES: usize = 10;

// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:53 ActionExecutionStatusReporter.PREPARING_MESSAGE
const PREPARING_MESSAGE: &str = "Preparing";

/// Java's `ActionExecutionMetadata` keys use identity: the address of the shared action.
fn action_id(action: &ActionRef) -> usize {
    Arc::as_ptr(action) as *const () as usize
}

/// Implements "Still waiting..." message functionality, displaying current status for
/// "in-flight" actions. Used by the ParallelBuilder.
///
/// TODO(bazel-team): (2010) It would be nice if "duplicated" actions (e.g. test shards and
/// multiple test runs) were merged into the single line.
///
/// Its users: the action executor (`blaze_sky_exec::skyframe_action_executor::ActionStatusReporter`)
/// and `crate::buildtool::action_execution_inactivity_watchdog`.
// Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:49 ActionExecutionStatusReporter
pub struct ActionExecutionStatusReporter {
    event_handler: Arc<dyn EventHandler>,
    clock: Arc<dyn Clock>,
    /// The status of each action "in flight", i.e. whose ExecuteBuildAction.call() method is
    /// active. Used for implementing the "still waiting" message.
    action_status: Mutex<HashMap<usize, (ActionRef, String, i64)>>,
}

impl ActionExecutionStatusReporter {
    /// `create(eventHandler)` / `create(eventHandler, clock)`: the caller registers the reporter
    /// on its event bus (Java's `create(eventHandler, eventBus)` does it).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:80 ActionExecutionStatusReporter.create
    pub fn create(
        event_handler: Arc<dyn EventHandler>,
        clock: Option<Arc<dyn Clock>>,
    ) -> Arc<ActionExecutionStatusReporter> {
        Arc::new(ActionExecutionStatusReporter {
            event_handler,
            clock: clock.unwrap_or_else(|| Arc::new(JavaClock)),
            action_status: Mutex::new(HashMap::new()),
        })
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:104 ActionExecutionStatusReporter.setStatus
    fn set_status(&self, action: &ActionRef, message: String) {
        self.action_status.lock().insert(
            action_id(action),
            (action.clone(), message, self.clock.nano_time()),
        );
    }

    /// Remove action from the list of active actions. Action must be present.
    ///
    /// PORT: `BugReport.sendNonFatalBugReport` of a missing action is not ported (it logs).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:109 ActionExecutionStatusReporter.remove
    pub fn remove(&self, action: &ActionRef) {
        self.action_status.lock().remove(&action_id(action));
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:151 ActionExecutionStatusReporter.getCount
    pub fn get_count(&self) -> usize {
        self.action_status.lock().len()
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:155 ActionExecutionStatusReporter.appendGroupStatus
    fn append_group_status(
        buffer: &mut String,
        status_map: &[(ActionRef, String, i64)],
        status: &str,
        current_time: i64,
    ) {
        let mut actions: Vec<(i64, &ActionRef)> = status_map
            .iter()
            .filter(|(_, s, _)| s == status)
            .map(|(action, _, time)| (*time, action))
            .collect();
        if actions.is_empty() {
            return;
        }
        actions.sort_by_key(|(time, _)| *time);

        buffer.push_str(&format!("\n      {status}:"));

        let truncate_list = actions.len() > MAX_LINES;
        let shown = if truncate_list {
            MAX_LINES - 1
        } else {
            actions.len()
        };
        for (time, action) in &actions[..shown] {
            let message = match action.get_progress_message(None) {
                Some(message) => message,
                // Actions will a null progress message should run so
                // fast we never see them here.  In any case...
                None => action.pretty_print(),
            };
            buffer.push_str("\n        ");
            buffer.push_str(&message);
            let run_time = (current_time - time) / 1000000000; // Convert to seconds.
            buffer.push_str(&format!(", {run_time} s"));
        }
        if truncate_list {
            buffer.push_str(&format!(
                "\n        ... {} more jobs",
                actions.len() - MAX_LINES + 1
            ));
        }
    }

    /// Get message showing currently executing actions.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:192 ActionExecutionStatusReporter.getExecutionStatusMessage
    fn get_execution_status_message(&self, status_map: &[(ActionRef, String, i64)]) -> String {
        let count = status_map.len();
        let mut s = if count != 1 {
            format!("Still waiting for {count} jobs to complete:")
        } else {
            String::from("Still waiting for 1 job to complete:")
        };

        let current_time = self.clock.nano_time();

        // A tree is just as fast as HashSet for small data sets.
        let statuses: BTreeSet<&str> = status_map.iter().map(|(_, s, _)| s.as_str()).collect();

        for status in statuses {
            Self::append_group_status(&mut s, status_map, status, current_time);
        }
        s
    }

    fn snapshot(&self) -> Vec<(ActionRef, String, i64)> {
        // Defensive copy to ensure thread safety.
        self.action_status.lock().values().cloned().collect()
    }

    /// Show currently executing actions.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:216 ActionExecutionStatusReporter.showCurrentlyExecutingActions
    pub fn show_currently_executing_actions(&self, progress_percentage_message: &str) {
        let status_map = self.snapshot();
        if !status_map.is_empty() {
            self.event_handler.handle(&Event::progress(format!(
                "{progress_percentage_message}{}",
                self.get_execution_status_message(&status_map)
            )));
        }
    }

    /// Warn about actions that are still being executed.
    /// Method is used to produce informative message when build is interrupted.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:229 ActionExecutionStatusReporter.warnAboutCurrentlyExecutingActions
    pub fn warn_about_currently_executing_actions(&self) {
        let mut status_map = self.snapshot();
        if status_map.is_empty() {
            // There are no tasks in the queue so there is nothing to report.
            self.event_handler.handle(&Event::warn(
                "There are no active jobs - stopping the build",
            ));
            return;
        }
        // Filter out actions that are not executed yet.
        status_map.retain(|(_, status, _)| status != PREPARING_MESSAGE);
        if !status_map.is_empty() {
            self.event_handler.handle(&Event::warn(format!(
                "{}\nBuild will be stopped after these tasks terminate",
                self.get_execution_status_message(&status_map)
            )));
        } else {
            // It is possible that one or more tasks in "Preparing" state just started being
            // executed. So warn user just in case.
            self.event_handler
                .handle(&Event::warn("Still waiting for unfinished jobs"));
        }
    }

    /// Returns the number of seconds to wait before reporting slow progress again.
    ///
    /// `user_specified_progress_interval`: value of the --progress_report_interval flag; 0 means
    /// use default 10, then 30, then 60 seconds wait times. `previous_wait_time`: previous value
    /// returned by this method.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:261 ActionExecutionStatusReporter.getWaitTime
    pub fn get_wait_time(user_specified_progress_interval: i32, previous_wait_time: i32) -> i32 {
        if user_specified_progress_interval > 0 {
            return user_specified_progress_interval;
        }

        // Increase waitTime to 10, then to 30 and then to 60 seconds to reduce
        // spamming during long wait periods.  If the user specified a
        // waitTime directly through progressReportInterval, then use
        // that value.
        if previous_wait_time == 0 {
            10
        } else if previous_wait_time == 10 {
            30
        } else {
            60
        }
    }
}

/// The `@Subscribe updateStatus` methods.
impl EventBus for ActionExecutionStatusReporter {
    fn post(&self, event: Arc<dyn Postable>) {
        if let Some(e) = downcast_event::<ActionStartedEvent>(&event) {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:119 ActionExecutionStatusReporter.updateStatus
            self.set_status(&e.action, PREPARING_MESSAGE.to_owned());
        } else if let Some(e) = downcast_event::<ScanningActionEvent>(&event) {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:126 ActionExecutionStatusReporter.updateStatus
            self.set_status(&e.action, "Scanning".to_owned());
        } else if let Some(e) = downcast_event::<SchedulingActionEvent>(&event) {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:133 ActionExecutionStatusReporter.updateStatus
            self.set_status(&e.action, "Scheduling".to_owned());
        } else if let Some(e) = downcast_event::<RunningActionEvent>(&event) {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:140 ActionExecutionStatusReporter.updateStatus
            self.set_status(&e.action, format!("Running ({})", e.strategy));
        } else if let Some(e) = downcast_event::<StoppedScanningActionEvent>(&event) {
            // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:147 ActionExecutionStatusReporter.updateStatus
            self.remove(&e.action);
        }
    }
}

/// The action runner's calls (`statusReporter.updateStatus(event)`, `statusReporter.remove(action)`).
impl blaze_sky_exec::skyframe_action_executor::ActionStatusReporter
    for ActionExecutionStatusReporter
{
    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:119 ActionExecutionStatusReporter.updateStatus
    fn update_status(&self, event: &ActionStartedEvent) {
        self.set_status(&event.action, PREPARING_MESSAGE.to_owned());
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutionStatusReporter.java:109 ActionExecutionStatusReporter.remove
    fn remove(&self, action: &ActionRef) {
        ActionExecutionStatusReporter::remove(self, action);
    }
}
