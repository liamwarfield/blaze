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
use crate::runtime::ui_state_tracker::{
    AnalysisProgress, ExecutionProgress, PackageProgress, TransportId, UiStateTracker,
};
use blaze_actions::ActionLookupData;
use blaze_actions::actions::action::ActionRef;
use blaze_cmdline::{Label, RepositoryMapping};

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

    /// `@Subscribe loadingStarted(LoadingPhaseStartedEvent)`: the event's package progress
    /// receiver.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:666 UiEventHandler.loadingStarted
    pub fn loading_started(&self, package_progress_receiver: Option<Arc<dyn PackageProgress>>) {
        self.handler.maybe_add_date();
        self.tracker.loading_started(package_progress_receiver);
        // As a new phase started, inform immediately.
        self.handler.ignore_refresh_limit_once();
        self.handler.refresh_progress();
        self.handler.start_update_thread();
    }

    /// `@Subscribe configurationStarted(ConfigurationPhaseStartedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:676 UiEventHandler.configurationStarted
    pub fn configuration_started(
        &self,
        analysis_progress_receiver: Option<Arc<dyn AnalysisProgress>>,
    ) {
        self.handler.maybe_add_date();
        self.tracker
            .configuration_started(analysis_progress_receiver);
        // As a new phase started, inform immediately.
        self.handler.ignore_refresh_limit_once();
        self.handler.refresh_progress();
        self.handler.start_update_thread();
    }

    /// `@Subscribe loadingComplete(LoadingPhaseCompleteEvent)`: the event's labels and main
    /// repository mapping.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:686 UiEventHandler.loadingComplete
    pub fn loading_complete(
        &self,
        labels: &[Label],
        main_repository_mapping: Option<RepositoryMapping>,
    ) {
        self.tracker
            .loading_complete(labels, main_repository_mapping);
        self.handler.refresh_progress();
    }

    /// `@Subscribe executionPhaseStarted(SomeExecutionStartedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:698 UiEventHandler.executionPhaseStarted
    pub fn execution_phase_started(&self, counted_in_execution_time: bool) {
        if counted_in_execution_time {
            self.tracker.execution_phase_started();
            self.handler.refresh_progress();
        }
    }

    /// `@Subscribe progressReceiverAvailable(ExecutionProgressReceiverAvailableEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:706 UiEventHandler.progressReceiverAvailable
    pub fn progress_receiver_available(&self, receiver: Arc<dyn ExecutionProgress>) {
        self.tracker.progress_receiver_available(receiver);
        // As this is the first time we have a progress message, update immediately.
        self.handler.ignore_refresh_limit_once();
        self.handler.start_update_thread();
    }

    /// `@Subscribe downloadProgress(FetchProgress)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:807 UiEventHandler.downloadProgress
    pub fn download_progress(&self, resource_identifier: &str, progress: &str, finished: bool) {
        self.handler.maybe_add_date();
        self.tracker
            .download_progress(resource_identifier, progress, finished);
        if !finished {
            self.handler.refresh_progress();
        } else {
            self.handler.check_activities();
        }
    }

    /// `@Subscribe scanningAction(ScanningActionEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:826 UiEventHandler.scanningAction
    pub fn scanning_action(&self, action: &ActionRef) {
        self.tracker.scanning_action(action);
        self.handler.refresh_progress();
    }

    /// `@Subscribe stopScanningAction(StoppedScanningActionEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:833 UiEventHandler.stopScanningAction
    pub fn stop_scanning_action(&self, action: &ActionRef) {
        self.tracker.stop_scanning_action(action);
        self.handler.refresh_progress();
    }

    /// `@Subscribe checkingActionCache(CachingActionEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:840 UiEventHandler.checkingActionCache
    pub fn checking_action_cache(&self, action: &ActionRef, strategy: &str) {
        self.tracker.caching_action(action, strategy);
        self.handler.refresh_progress();
    }

    /// `@Subscribe schedulingAction(SchedulingActionEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:847 UiEventHandler.schedulingAction
    pub fn scheduling_action(&self, action: &ActionRef, strategy: &str) {
        self.tracker.scheduling_action(action, strategy);
        self.handler.refresh_progress();
    }

    /// `@Subscribe runningAction(RunningActionEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:854 UiEventHandler.runningAction
    pub fn running_action(&self, action: &ActionRef, strategy: &str) {
        self.tracker.running_action(action, strategy);
        self.handler.refresh_progress();
    }

    /// `@Subscribe actionProgress(ActionProgressEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:861 UiEventHandler.actionProgress
    pub fn action_progress(
        &self,
        action: &ActionRef,
        progress_id: &str,
        progress: &str,
        finished: bool,
    ) {
        self.tracker
            .action_progress(action, progress_id, progress, finished);
        self.handler.refresh_soon();
    }

    /// `@Subscribe actionCompletion(ActionScanningCompletedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:868 UiEventHandler.actionCompletion
    pub fn action_scanning_completed(
        &self,
        action: &ActionRef,
        action_lookup_data: &ActionLookupData,
    ) {
        self.tracker
            .action_scanning_completed(action, action_lookup_data);
        self.handler.refresh_soon();
    }

    /// `@Subscribe crash(CrashEvent)`: Java posts the tracker's `InflightActionInfo` on the
    /// event bus; here it is returned (the count and the most frequent mnemonics).
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:882 UiEventHandler.crash
    pub fn crash(&self) -> (i32, Vec<(String, i32)>) {
        self.tracker.log_and_get_inflight_actions()
    }

    /// `@Subscribe actionUploadStarted(ActionUploadStartedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:899 UiEventHandler.actionUploadStarted
    pub fn action_upload_started(&self) {
        self.tracker.action_upload_started();
        self.handler.refresh_soon();
    }

    /// `@Subscribe actionUploadFinished(ActionUploadFinishedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:906 UiEventHandler.actionUploadFinished
    pub fn action_upload_finished(&self) {
        self.tracker.action_upload_finished();
        self.handler.check_activities();
    }

    /// `@Subscribe testFilteringComplete(TestFilteringCompleteEvent)`: the labels of the
    /// event's test targets.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:912 UiEventHandler.testFilteringComplete
    pub fn test_filtering_complete(&self, test_targets: Option<&[Option<Label>]>) {
        self.tracker.test_filtering_complete(test_targets);
        self.handler.refresh_progress();
    }

    /// `@Subscribe buildEventTransportsAnnounced(AnnounceBuildEventTransportsEvent)`.
    ///
    /// PORT: `--experimental_ui_debug_all_events`'s "Transports announced:" line is the
    /// handler's debug output, written through its `handle`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:969 UiEventHandler.buildEventTransportsAnnounced
    pub fn build_event_transports_announced(
        &self,
        transports: &[TransportId],
        debug_all_events: bool,
    ) {
        self.tracker.build_event_transports_announced(transports);
        if debug_all_events {
            let mut message = String::from("Transports announced:");
            for transport in transports {
                message.push(' ');
                message.push_str(&transport.name);
            }
            blaze_events::EventHandler::handle(&*self.handler, &blaze_events::Event::info(message));
        }
    }

    /// `@Subscribe buildEventTransportClosed(BuildEventTransportClosedEvent)`.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:981 UiEventHandler.buildEventTransportClosed
    pub fn build_event_transport_closed(&self, transport: &TransportId, debug_all_events: bool) {
        self.tracker.build_event_transport_closed(transport);
        if debug_all_events {
            blaze_events::EventHandler::handle(
                &*self.handler,
                &blaze_events::Event::info(format!("Transport {} closed", transport.name)),
            );
        }

        self.handler.check_activities();
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
