//! The `@Subscribe` methods of `UiEventHandler` for the events of a build: Guava's `EventBus`
//! calls them with the event objects; here the command registers this subscriber on its event
//! bus, and it unpacks each event for the state tracker ([`UiStateTracker`]) and the handler
//! (`blaze_events::ui::UiEventHandler`).
//!
//! PORT: events the execution and loading sides do not post yet, so the subscriber has nothing
//! to unpack for them: `LoadingPhaseStartedEvent`, `LoadingPhaseCompleteEvent`,
//! `ConfigurationPhaseStartedEvent`, `AnalysisPhaseCompleteEvent` (blaze_sky_top prints the
//! "Analyzed ..." line itself), `SomeExecutionStartedEvent`,
//! `ExecutionProgressReceiverAvailableEvent` (needs the evaluator's progress receiver),
//! `ScanningActionEvent`, `StoppedScanningActionEvent`, `CachingActionEvent`,
//! `SchedulingActionEvent`, `RunningActionEvent`, `ActionProgressEvent`,
//! `ActionScanningCompletedEvent`, `TestFilteringCompleteEvent`, `FetchProgress`,
//! `ActionUploadStartedEvent`/`ActionUploadFinishedEvent`, `AnnounceBuildEventTransportsEvent`,
//! `BuildEventTransportClosedEvent`, `CrashEvent`. The tracker's methods for them are ported.

use std::sync::Arc;

use blaze_events::ui::UiEventHandler;
use blaze_events::{EventBus, Postable, nano_time_of};
use blaze_protos::blaze::BlazeTestStatus;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionStartedEvent, TestAnalyzedEvent, downcast_event,
};
use parking_lot::Mutex;

use crate::runtime::test_summary::TestSummary;
use crate::runtime::ui_state_tracker::UiStateTracker;

/// `TEST_STATUS_TO_IGNORE_FOR_NEW_INFORMATION`.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:108 UiEventHandler.TEST_STATUS_TO_IGNORE_FOR_NEW_INFORMATION
const TEST_STATUS_TO_IGNORE_FOR_NEW_INFORMATION: [BlazeTestStatus; 4] = [
    BlazeTestStatus::Passed,
    BlazeTestStatus::FailedToBuild,
    BlazeTestStatus::BlazeHaltedBeforeTesting,
    BlazeTestStatus::NoStatus,
];

/// Return true, if the test summary provides information that is both worth being shown in the
/// scroll-back buffer and new with respect to the alreay shown failure messages.
// Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:928 UiEventHandler.testSummaryProvidesNewInformation
pub fn test_summary_provides_new_information(summary: &TestSummary) -> bool {
    if TEST_STATUS_TO_IGNORE_FOR_NEW_INFORMATION.contains(&summary.get_status()) {
        return false;
    }
    if summary.get_status() == BlazeTestStatus::Failed {
        return summary.get_failed_logs().len() != 1;
    }
    true
}

/// The UI's subscriber on the command's event bus.
pub struct UiEventSubscriber {
    handler: Arc<UiEventHandler>,
    tracker: Arc<UiStateTracker>,
    /// Events posted while the `BuildCompleteEvent` is dispatched (see
    /// [`UiEventSubscriber::begin_build_complete`]).
    deferred: Mutex<Option<Vec<Arc<dyn Postable>>>>,
}

impl UiEventSubscriber {
    pub fn new(handler: Arc<UiEventHandler>, tracker: Arc<UiStateTracker>) -> UiEventSubscriber {
        UiEventSubscriber {
            handler,
            tracker,
            deferred: Mutex::new(None),
        }
    }

    pub fn get_state_tracker(&self) -> &Arc<UiStateTracker> {
        &self.tracker
    }

    /// The `BuildCompleteEvent` is being dispatched: Guava's `EventBus` queues the events its
    /// subscribers post (`AggregatingTestListener.buildCompleteEvent` posts a `TestSummary` for
    /// each test that was not run) and delivers them after every subscriber of the
    /// `BuildCompleteEvent` ran, so the UI's `buildComplete` does not see them. Events posted
    /// from here to [`UiEventSubscriber::build_complete`] reach this subscriber after it.
    ///
    /// PORT: Blaze calls the `BuildCompleteEvent` subscribers in turn; this restores the order
    /// the UI observes.
    pub fn begin_build_complete(&self) {
        *self.deferred.lock() = Some(Vec::new());
    }

    /// `@Subscribe buildComplete(BuildCompleteEvent)`, then the events deferred since
    /// [`UiEventSubscriber::begin_build_complete`].
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:714 UiEventHandler.buildComplete
    pub fn build_complete(&self, success: bool) {
        self.handler.build_complete(success);
        let deferred = self.deferred.lock().take().unwrap_or_default();
        for event in deferred {
            self.dispatch(&event);
        }
    }

    fn dispatch(&self, event: &Arc<dyn Postable>) {
        if let Some(e) = downcast_event::<ActionStartedEvent>(event) {
            // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:819 UiEventHandler.actionStarted
            self.tracker
                .action_started(&e.action, nano_time_of(e.nano_time_start));
            self.handler.refresh_progress();
        } else if let Some(e) = downcast_event::<ActionCompletionEvent>(event) {
            // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:868 UiEventHandler.actionCompletion
            self.tracker
                .action_completion(&e.action, &e.action_lookup_data);
            self.handler.refresh_soon();
        } else if let Some(e) = downcast_event::<TestAnalyzedEvent>(event) {
            // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:918 UiEventHandler.singleTestAnalyzed
            let label = e.configured_target.get_label();
            self.tracker.single_test_analyzed(Some(&label));
            self.handler.refresh_soon();
        } else if let Some(summary) = downcast_event::<TestSummary>(event) {
            // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:938 UiEventHandler.testSummary
            self.tracker
                .test_summary(&summary.get_label(), summary.get_status());
            if test_summary_provides_new_information(summary) {
                let failed_logs: Vec<String> = summary
                    .get_failed_logs()
                    .iter()
                    .map(|p| p.get_path_string().to_string())
                    .collect();
                self.handler.test_summary_new_information(
                    summary.get_status().as_str_name(),
                    summary.get_status() == BlazeTestStatus::Flaky,
                    &summary.get_label().to_string(),
                    &failed_logs,
                );
            } else {
                self.handler.refresh_progress();
            }
        }
    }
}

impl EventBus for UiEventSubscriber {
    fn post(&self, event: Arc<dyn Postable>) {
        {
            let mut deferred = self.deferred.lock();
            if let Some(deferred) = deferred.as_mut() {
                deferred.push(event);
                return;
            }
        }
        self.dispatch(&event);
    }
}
