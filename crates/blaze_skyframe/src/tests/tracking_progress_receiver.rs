//! Port of the test utility `TrackingProgressReceiver`.

use crate::{
    DirtyType, ErrorInfo, EvaluationProgressReceiver, EvaluationState, GroupedDeps, SkyKey,
    SkyValue,
};
use blaze_util::check_state;
use parking_lot::Mutex;
use std::collections::HashSet;

type Callback = Box<dyn FnOnce() + Send>;

/// A testing utility to keep track of evaluation.
// Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:24 TrackingProgressReceiver
pub(crate) struct TrackingProgressReceiver {
    check_evaluation_results: bool,

    /// Callback to be executed on a next `dirtied` or `deleted` call. It will be run once and is
    /// expected to be run if set.
    next_invalidation_callback: Mutex<Option<Callback>>,

    pub(crate) dirty: Mutex<HashSet<SkyKey>>,
    pub(crate) deleted: Mutex<HashSet<SkyKey>>,
    pub(crate) enqueued: Mutex<HashSet<SkyKey>>,
    pub(crate) evaluated: Mutex<HashSet<SkyKey>>,
}

impl TrackingProgressReceiver {
    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:38 TrackingProgressReceiver.<init>
    pub(crate) fn new(check_evaluation_results: bool) -> Self {
        TrackingProgressReceiver {
            check_evaluation_results,
            next_invalidation_callback: Mutex::new(None),
            dirty: Mutex::new(HashSet::new()),
            deleted: Mutex::new(HashSet::new()),
            enqueued: Mutex::new(HashSet::new()),
            evaluated: Mutex::new(HashSet::new()),
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:56 TrackingProgressReceiver.runInvalidationCallbackIfPresent
    fn run_invalidation_callback_if_present(&self) {
        let callback = self.next_invalidation_callback.lock().take();
        if let Some(callback) = callback {
            callback();
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:84 TrackingProgressReceiver.clear
    pub(crate) fn clear(&self) {
        self.dirty.lock().clear();
        self.deleted.lock().clear();
        self.enqueued.lock().clear();
        self.evaluated.lock().clear();
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:91 TrackingProgressReceiver.setNextInvalidationCallback
    pub(crate) fn set_next_invalidation_callback(&self, runnable: impl FnOnce() + Send + 'static) {
        let old_callback = self
            .next_invalidation_callback
            .lock()
            .replace(Box::new(runnable));
        check_state!(
            old_callback.is_none(),
            "Overwriting a left-over callback: {}",
            "<callback>"
        );
    }
}

impl EvaluationProgressReceiver for TrackingProgressReceiver {
    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:43 TrackingProgressReceiver.dirtied
    fn dirtied(&self, sky_key: &SkyKey, _dirty_type: DirtyType) {
        self.run_invalidation_callback_if_present();
        self.dirty.lock().insert(sky_key.clone());
        check_state!(!self.deleted.lock().contains(sky_key), "{:?}", sky_key);
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:50 TrackingProgressReceiver.deleted
    fn deleted(&self, sky_key: &SkyKey) {
        self.run_invalidation_callback_if_present();
        self.dirty.lock().remove(sky_key);
        self.deleted.lock().insert(sky_key.clone());
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:64 TrackingProgressReceiver.enqueueing
    fn enqueueing(&self, sky_key: &SkyKey) {
        self.enqueued.lock().insert(sky_key.clone());
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/TrackingProgressReceiver.java:69 TrackingProgressReceiver.evaluated
    fn evaluated(
        &self,
        sky_key: &SkyKey,
        state: EvaluationState,
        _value: Option<&SkyValue>,
        _error: Option<&ErrorInfo>,
        _direct_deps: Option<&GroupedDeps>,
    ) {
        self.evaluated.lock().insert(sky_key.clone());
        if self.check_evaluation_results && state.succeeded() {
            self.deleted.lock().remove(sky_key);
            if !state.version_changed() {
                self.dirty.lock().remove(sky_key);
            }
        }
    }
}
