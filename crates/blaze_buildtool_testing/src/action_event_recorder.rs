//! Port of `ActionEventRecorder` (`src/test/java/com/google/devtools/build/lib/testutil/`): records
//! the action events of a build (a subscriber of the command's event bus) and checks their counts.

use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_events::{EventBus, Postable};
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionExecutedEvent, ActionResultReceivedEvent, ActionStartedEvent,
    downcast_event,
};
use blaze_sky_exec::rewinding::action_rewound_event::{
    ActionRewoundEvent, PostableActionRewindingStats,
};
use blaze_sky_exec::skyframe_action_executor::CachedActionEvent;
use parking_lot::Mutex;

/// The recorded events: the description (`progressMessageOrPrettyPrint`) of each event's
/// action, and the counts of each `PostableActionRewindingStats`.
///
/// PORT: Java keeps the event objects. Here an event can hold, through its action or spawn
/// results, a path to the command's Skyframe evaluator, and a recorder outlives its commands:
/// keeping the objects would keep the evaluator alive after the command (the server's kept
/// graph must be its evaluator's only holder). What the assertions read is taken when the event
/// is posted.
#[derive(Default)]
struct Recorded {
    action_started_events: Vec<String>,
    action_completion_events: Vec<String>,
    action_executed_events: Vec<String>,
    action_result_received_events: Vec<String>,
    cached_action_events: Vec<String>,
    action_rewound_events: Vec<String>,
    /// `(lostInputsCount, lostOutputsCount)`.
    action_rewinding_stats_posts: Vec<(usize, usize)>,
}

/// A subscriber for the action events of a build.
// Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:38 ActionEventRecorder
#[derive(Default)]
pub struct ActionEventRecorder {
    recorded: Mutex<Recorded>,
    action_rewound_event_subscriber: Mutex<Option<Arc<dyn Fn(&ActionRewoundEvent) + Send + Sync>>>,
}

impl EventBus for ActionEventRecorder {
    // Java: the @Subscribe methods (actionStarted, actionCompleted, actionExecuted,
    // actionResultReceived, cachedAction, actionRewound, actionRewindingStats).
    fn post(&self, event: Arc<dyn Postable>) {
        let mut recorded = self.recorded.lock();
        if let Some(e) = downcast_event::<ActionStartedEvent>(&event) {
            recorded
                .action_started_events
                .push(progress_message_or_pretty_print(&*e.action));
        } else if let Some(e) = downcast_event::<ActionCompletionEvent>(&event) {
            recorded
                .action_completion_events
                .push(progress_message_or_pretty_print(&*e.action));
        } else if let Some(e) = downcast_event::<ActionExecutedEvent>(&event) {
            recorded
                .action_executed_events
                .push(progress_message_or_pretty_print(&*e.action));
        } else if let Some(e) = downcast_event::<ActionResultReceivedEvent>(&event) {
            recorded
                .action_result_received_events
                .push(progress_message_or_pretty_print(&*e.action));
        } else if let Some(e) = downcast_event::<CachedActionEvent>(&event) {
            recorded
                .cached_action_events
                .push(progress_message_or_pretty_print(&*e.action));
        } else if let Some(rewound) = downcast_event::<ActionRewoundEvent>(&event) {
            let subscriber = self.action_rewound_event_subscriber.lock().clone();
            recorded
                .action_rewound_events
                .push(progress_message_or_pretty_print(
                    &*rewound.failed_rewound_action,
                ));
            drop(recorded);
            if let Some(subscriber) = subscriber {
                subscriber(rewound);
            }
        } else if let Some(stats) = downcast_event::<PostableActionRewindingStats>(&event) {
            recorded
                .action_rewinding_stats_posts
                .push((stats.lost_inputs_count, stats.lost_outputs_count));
        }
    }
}

/// `progressMessageOrPrettyPrint(action)`.
// Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:301 ActionEventRecorder.progressMessageOrPrettyPrint
pub fn progress_message_or_pretty_print(action: &dyn Action) -> String {
    action
        .get_progress_message(None)
        .unwrap_or_else(|| action.pretty_print())
}

impl ActionEventRecorder {
    pub fn new() -> Arc<ActionEventRecorder> {
        Arc::new(ActionEventRecorder::default())
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:57 ActionEventRecorder.setActionRewoundEventSubscriber
    pub fn set_action_rewound_event_subscriber(
        &self,
        subscriber: impl Fn(&ActionRewoundEvent) + Send + Sync + 'static,
    ) {
        *self.action_rewound_event_subscriber.lock() = Some(Arc::new(subscriber));
    }

    /// The descriptions (`progressMessageOrPrettyPrint`) of the actions of the recorded
    /// `ActionStartedEvent`s, in order.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:61 ActionEventRecorder.getActionStartedEvents
    pub fn get_action_started_events(&self) -> Vec<String> {
        self.recorded.lock().action_started_events.clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:65 ActionEventRecorder.getActionCompletionEvents
    pub fn get_action_completion_events(&self) -> Vec<String> {
        self.recorded.lock().action_completion_events.clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:69 ActionEventRecorder.getActionExecutedEvents
    pub fn get_action_executed_events(&self) -> Vec<String> {
        self.recorded.lock().action_executed_events.clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:73 ActionEventRecorder.getActionResultReceivedEvents
    pub fn get_action_result_received_events(&self) -> Vec<String> {
        self.recorded.lock().action_result_received_events.clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:77 ActionEventRecorder.getActionRewoundEvents
    pub fn get_action_rewound_events(&self) -> Vec<String> {
        self.recorded.lock().action_rewound_events.clone()
    }

    /// The `lostInputsCount()` and `lostOutputsCount()` of each `PostableActionRewindingStats`.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:81 ActionEventRecorder.getActionRewindingStatsPosts
    pub fn get_action_rewinding_stats_posts(&self) -> Vec<(usize, usize)> {
        self.recorded.lock().action_rewinding_stats_posts.clone()
    }

    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:135 ActionEventRecorder.clear
    pub fn clear(&self) {
        *self.recorded.lock() = Recorded::default();
    }

    /// Checks how many of each type of event was emitted during a successful build.
    ///
    /// `run_once`: actions which ran and are not rewound; `completed_rewound`: actions which ran
    /// and then are rewound by a later failed action; `failed_rewound`: actions which fail because
    /// of lost inputs and which rewind themselves and the actions that generate those lost inputs.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:153 ActionEventRecorder.assertEvents
    pub fn assert_events(
        &self,
        run_once: &[&str],
        completed_rewound: &[&str],
        failed_rewound: &[&str],
        action_rewinding_post_lost_input_counts: &[usize],
    ) {
        self.assert_events_with_result_received(
            run_once,
            completed_rewound,
            failed_rewound,
            /* expect_result_received_for_failed_rewound= */ true,
            action_rewinding_post_lost_input_counts,
        );
    }

    /// [`ActionEventRecorder::assert_events`]; `expect_result_received_for_failed_rewound` should
    /// be true iff the failed rewound actions ever successfully complete (no
    /// `ActionResultReceivedEvent` is emitted for a failed action).
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:175 ActionEventRecorder.assertEvents
    pub fn assert_events_with_result_received(
        &self,
        run_once: &[&str],
        completed_rewound: &[&str],
        failed_rewound: &[&str],
        expect_result_received_for_failed_rewound: bool,
        action_rewinding_post_lost_input_counts: &[usize],
    ) {
        let asserter = EventCountAsserter {
            run_once,
            completed_rewound,
            failed_rewound,
        };
        let recorded = self.recorded.lock();
        asserter.assert_event_counts(
            &(recorded.action_started_events),
            "actionStartedEvents",
            1,
            1,
            2,
        );
        asserter.assert_event_counts(
            &(recorded.action_completion_events),
            "actionCompletionEvents",
            1,
            1,
            1,
        );
        asserter.assert_event_counts(
            &(recorded.action_executed_events),
            "actionExecutedEvents",
            1,
            1,
            1,
        );
        asserter.assert_event_counts(
            &(recorded.action_result_received_events),
            "actionResultReceivedEvents",
            1,
            1,
            if expect_result_received_for_failed_rewound {
                1
            } else {
                0
            },
        );
        asserter.assert_event_counts(
            &(recorded.action_rewound_events),
            "actionRewoundEvents",
            0,
            0,
            1,
        );
        drop(recorded);
        self.assert_total_lost_input_counts_from_stats(action_rewinding_post_lost_input_counts);
        assert!(
            self.recorded.lock().cached_action_events.is_empty(),
            "cachedActionEvents is not empty"
        );
    }

    /// Asserts that the total lost input counts from the posted `PostableActionRewindingStats`
    /// match `total_lost_input_counts`.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:224 ActionEventRecorder.assertTotalLostInputCountsFromStats
    pub fn assert_total_lost_input_counts_from_stats(&self, total_lost_input_counts: &[usize]) {
        let posts = self.get_action_rewinding_stats_posts();
        assert_eq!(posts.len(), total_lost_input_counts.len(), "{posts:?}");
        for (post, expected) in posts.iter().zip(total_lost_input_counts) {
            assert_eq!(post.0, *expected, "lostInputsCount of {posts:?}");
        }
    }

    /// Asserts that the total lost output counts from the posted `PostableActionRewindingStats`
    /// match `total_lost_output_counts`.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:251 ActionEventRecorder.assertTotalLostOutputCountsFromStats
    pub fn assert_total_lost_output_counts_from_stats(&self, total_lost_output_counts: &[usize]) {
        let posts = self.get_action_rewinding_stats_posts();
        assert_eq!(posts.len(), total_lost_output_counts.len(), "{posts:?}");
        for (post, expected) in posts.iter().zip(total_lost_output_counts) {
            assert_eq!(post.1, *expected, "lostOutputsCount of {posts:?}");
        }
    }
}

// Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:260 ActionEventRecorder.EventCountAsserter
struct EventCountAsserter<'a> {
    run_once: &'a [&'a str],
    completed_rewound: &'a [&'a str],
    failed_rewound: &'a [&'a str],
}

impl EventCountAsserter<'_> {
    // Java: src/test/java/com/google/devtools/build/lib/testutil/ActionEventRecorder.java:274 ActionEventRecorder.EventCountAsserter.assertEventCounts
    fn assert_event_counts(
        &self,
        event_descriptions: &[String],
        events_name: &str,
        expected_run_once_event_count: usize,
        expected_completed_rewound_event_count: usize,
        expected_failed_rewound_event_count: usize,
    ) {
        let count = |action: &str| event_descriptions.iter().filter(|d| *d == action).count();
        for run_once_action in self.run_once {
            assert_eq!(
                count(run_once_action),
                expected_run_once_event_count,
                "Run-once action \"{run_once_action}\" in {events_name}: {event_descriptions:?}"
            );
        }
        for rewound_action in self.completed_rewound {
            assert_eq!(
                count(rewound_action),
                expected_completed_rewound_event_count,
                "Completed rewound action \"{rewound_action}\" in {events_name}: \
                 {event_descriptions:?}"
            );
        }
        for failed_rewound_action in self.failed_rewound {
            assert_eq!(
                count(failed_rewound_action),
                expected_failed_rewound_event_count,
                "Failed rewound action \"{failed_rewound_action}\" in {events_name}: \
                 {event_descriptions:?}"
            );
        }
    }
}
