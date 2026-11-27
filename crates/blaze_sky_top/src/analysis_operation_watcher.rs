//! Port of `analysis/AnalysisOperationWatcher.java`.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_events::{Event, EventHandler, ExtendedEventHandler, Postable};
use blaze_sky_exec::events::{TopLevelEntityAnalysisConcludedEvent, downcast_event};
use blaze_sky_exec::keys::BuildDriverKey;
use parking_lot::Mutex;

/// A callback to be called when all the expected keys have been analyzed.
// Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:100 AnalysisOperationWatcher.AnalysisOperationWatcherFinisher
pub type AnalysisOperationWatcherFinisher = Box<dyn FnOnce() + Send>;

/// A callback to signal that the delayed execution tasks can now go ahead.
// Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:106 AnalysisOperationWatcher.ExecutionGoAheadCallback
pub type ExecutionGoAheadCallback = Box<dyn Fn() + Send + Sync>;

/// A watcher for analysis-related work that sends out a signal when all such work in the build
/// is done. There's one instance of this class per build.
///
/// PORT: Java registers the watcher on the command's event bus. The events of the
/// `BuildDriverKey`s reach the bus through the evaluation's event handler, so the watcher is that
/// handler here: it passes every event on to the command's handler and then handles the
/// `TopLevelEntityAnalysisConcludedEvent`s, as a subscriber of the bus would after the event was
/// dispatched (the finisher's own posts follow the event). `close()` (unregistering) is the
/// end of the evaluation that holds the handler.
// Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:26 AnalysisOperationWatcher
// PORT: skipped, unregistering is dropping the evaluation's handler (see above): AnalysisOperationWatcher.close
pub struct AnalysisOperationWatcher {
    delegate: Arc<dyn ExtendedEventHandler>,
    thread_safe_expected_keys: Mutex<HashSet<BuildDriverKey>>,
    finisher: Mutex<Option<AnalysisOperationWatcherFinisher>>,
    /// When there's not more than this amount of top level target/aspect left to analyze, we
    /// can start with execution.
    lower_threshold_to_signal_for_execution: f32,
    signalled_execution_go_ahead: Mutex<bool>,
    execution_go_ahead_callback: ExecutionGoAheadCallback,
}

impl AnalysisOperationWatcher {
    /// Creates an AnalysisOperationWatcher that watches the events posted to `event_bus`.
    // Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:52 AnalysisOperationWatcher.createAndRegisterWithEventBus
    // Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:38 AnalysisOperationWatcher.<init>
    pub fn create_and_register_with_event_bus(
        thread_safe_expected_keys: HashSet<BuildDriverKey>,
        event_bus: Arc<dyn ExtendedEventHandler>,
        lower_threshold_to_signal_for_execution: f32,
        finisher: AnalysisOperationWatcherFinisher,
        execution_go_ahead_callback: ExecutionGoAheadCallback,
    ) -> Arc<AnalysisOperationWatcher> {
        Arc::new(AnalysisOperationWatcher {
            delegate: event_bus,
            thread_safe_expected_keys: Mutex::new(thread_safe_expected_keys),
            finisher: Mutex::new(Some(finisher)),
            lower_threshold_to_signal_for_execution,
            signalled_execution_go_ahead: Mutex::new(false),
            execution_go_ahead_callback,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisOperationWatcher.java:70 AnalysisOperationWatcher.handleTopLevelEntityAnalysisConcluded
    fn handle_top_level_entity_analysis_concluded(&self, e: &TopLevelEntityAnalysisConcludedEvent) {
        let now_empty = {
            let mut expected_keys = self.thread_safe_expected_keys.lock();
            if expected_keys.is_empty() {
                return;
            }
            expected_keys.remove(&e.analyzed_top_level_key);

            let mut signalled = self.signalled_execution_go_ahead.lock();
            if !*signalled
                && expected_keys.len() as f32 <= self.lower_threshold_to_signal_for_execution
            {
                *signalled = true;
                (self.execution_go_ahead_callback)();
            }
            expected_keys.is_empty()
        };

        if now_empty && let Some(finisher) = self.finisher.lock().take() {
            finisher();
        }
    }
}

impl EventHandler for AnalysisOperationWatcher {
    fn handle(&self, event: &Event) {
        self.delegate.handle(event);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for AnalysisOperationWatcher {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.delegate.post(obj.clone());
        if let Some(e) = downcast_event::<TopLevelEntityAnalysisConcludedEvent>(&obj) {
            self.handle_top_level_entity_analysis_concluded(e);
        }
    }
}

#[cfg(test)]
mod tests {
    // PORT: AnalysisOperationWatcher has no JUnit test at the pin; these check the port's
    // event handling.
    use std::sync::atomic::{AtomicUsize, Ordering};

    use blaze_actions::ActionLookupKeyRef;
    use blaze_analysis::configured_target_key::ConfiguredTargetKey;
    use blaze_cmdline::Label;
    use blaze_events::collectors::StoredEventHandler;

    use super::*;

    fn key(label: &str) -> BuildDriverKey {
        BuildDriverKey::of_configured_target(
            ActionLookupKeyRef::new(
                ConfiguredTargetKey::builder()
                    .set_label(Label::parse_canonical_unchecked(label))
                    .build(),
            ),
            vec![],
            false,
            false,
            false,
            true,
        )
    }

    fn concluded(key: &BuildDriverKey) -> Arc<dyn Postable> {
        Arc::new(TopLevelEntityAnalysisConcludedEvent {
            analyzed_top_level_key: key.clone(),
            succeeded: true,
        })
    }

    #[test]
    fn finisher_runs_once_when_the_last_key_concludes() {
        let finished = Arc::new(AtomicUsize::new(0));
        let go_ahead = Arc::new(AtomicUsize::new(0));
        let (a, b) = (key("//p:a"), key("//p:b"));
        let delegate = Arc::new(StoredEventHandler::new());
        let watcher = {
            let finished = finished.clone();
            let go_ahead = go_ahead.clone();
            AnalysisOperationWatcher::create_and_register_with_event_bus(
                [a.clone(), b.clone()].into_iter().collect(),
                delegate.clone(),
                /* lower_threshold_to_signal_for_execution= */ 0.0,
                Box::new(move || {
                    finished.fetch_add(1, Ordering::SeqCst);
                }),
                Box::new(move || {
                    go_ahead.fetch_add(1, Ordering::SeqCst);
                }),
            )
        };
        watcher.post(concluded(&a));
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        assert_eq!(go_ahead.load(Ordering::SeqCst), 0);
        // A key concluding twice (a restart) changes nothing.
        watcher.post(concluded(&a));
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        watcher.post(concluded(&b));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(go_ahead.load(Ordering::SeqCst), 1);
        watcher.post(concluded(&b));
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        // Every event reaches the bus.
        assert_eq!(delegate.get_posts().len(), 4);
    }

    #[test]
    fn go_ahead_at_the_threshold() {
        let go_ahead = Arc::new(AtomicUsize::new(0));
        let keys = [key("//p:a"), key("//p:b"), key("//p:c")];
        let watcher = {
            let go_ahead = go_ahead.clone();
            AnalysisOperationWatcher::create_and_register_with_event_bus(
                keys.iter().cloned().collect(),
                Arc::new(StoredEventHandler::new()),
                // 3 keys at 100%.
                3.0,
                Box::new(|| {}),
                Box::new(move || {
                    go_ahead.fetch_add(1, Ordering::SeqCst);
                }),
            )
        };
        watcher.post(concluded(&keys[0]));
        assert_eq!(go_ahead.load(Ordering::SeqCst), 1);
        watcher.post(concluded(&keys[1]));
        assert_eq!(go_ahead.load(Ordering::SeqCst), 1);
    }
}
