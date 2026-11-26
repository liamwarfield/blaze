//! Tests of the progress receivers (`progress.rs`). Bazel has no JUnit class for them (they are
//! exercised through `ParallelEvaluatorTest`, `MemoizingEvaluatorTest` and `EagerInvalidatorTest`,
//! ported with the evaluator in phase 2); these check the Java methods as written.

use super::tracking_progress_receiver::TrackingProgressReceiver;
use super::*;
use crate::progress::{
    DirtyAndInflightTrackingProgressReceiver, InflightOnlyTrackingProgressReceiver,
    InflightTrackingProgressReceiver,
};

fn key(name: &'static str) -> SkyKey {
    k(name)
}

fn set(keys: &[&'static str]) -> HashSet<SkyKey> {
    keys.iter().map(|n| key(n)).collect()
}

#[test]
fn evaluation_state_get_and_flags() {
    let v: SkyValue = Arc::new(1u32);
    assert_eq!(
        EvaluationState::get(Some(&v), true),
        EvaluationState::SuccessVersionChanged
    );
    assert_eq!(
        EvaluationState::get(Some(&v), false),
        EvaluationState::SuccessVersionUnchanged
    );
    assert_eq!(
        EvaluationState::get(None, true),
        EvaluationState::FailVersionChanged
    );
    assert_eq!(
        EvaluationState::get(None, false),
        EvaluationState::FailVersionUnchanged
    );
    assert!(EvaluationState::SuccessVersionChanged.succeeded());
    assert!(EvaluationState::SuccessVersionChanged.version_changed());
    assert!(EvaluationState::SuccessVersionUnchanged.succeeded());
    assert!(!EvaluationState::SuccessVersionUnchanged.version_changed());
    assert!(!EvaluationState::FailVersionChanged.succeeded());
    assert!(EvaluationState::FailVersionChanged.version_changed());
    assert!(!EvaluationState::FailVersionUnchanged.succeeded());
    // Java: FAIL_VERSION_UNCHANGED(false, true).
    assert!(EvaluationState::FailVersionUnchanged.version_changed());
}

#[test]
fn null_receiver_does_nothing() {
    let r = crate::progress::NULL;
    r.dirtied(&key("a"), DirtyType::Dirty);
    r.deleted(&key("a"));
    r.enqueueing(&key("a"));
    r.state_starting(&key("a"), NodeState::Compute);
    r.state_ending(&key("a"), NodeState::Compute);
    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionChanged,
        None,
        None,
        None,
    );
    r.change_pruned(&key("a"));
}

#[test]
fn dirty_and_inflight_tracks_dirty_until_enqueued() {
    let inner = Arc::new(TrackingProgressReceiver::new(false));
    let r = DirtyAndInflightTrackingProgressReceiver::new(inner.clone());
    r.dirtied(&key("a"), DirtyType::Dirty);
    r.dirtied(&key("b"), DirtyType::Change);
    r.dirtied(&key("c"), DirtyType::Dirty);
    assert_eq!(r.get_unenqueued_dirty_keys(), set(&["a", "b", "c"]));
    assert_eq!(*inner.dirty.lock(), set(&["a", "b", "c"]));

    r.enqueueing(&key("a"));
    assert_eq!(r.get_unenqueued_dirty_keys(), set(&["b", "c"]));
    assert!(r.is_inflight(&key("a")));
    assert_eq!(*inner.enqueued.lock(), set(&["a"]));

    // After an error the key is in flight but the external receiver is not told.
    r.enqueue_after_error(&key("b"));
    assert!(r.is_inflight(&key("b")));
    assert_eq!(r.get_unenqueued_dirty_keys(), set(&["c"]));
    assert_eq!(*inner.enqueued.lock(), set(&["a"]));

    // A second enqueueing of a key in flight is not forwarded.
    inner.enqueued.lock().clear();
    r.enqueueing(&key("b"));
    assert!(inner.enqueued.lock().is_empty());

    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionChanged,
        None,
        None,
        None,
    );
    assert!(!r.is_inflight(&key("a")));
    assert_eq!(*inner.evaluated.lock(), set(&["a"]));

    r.deleted(&key("c"));
    assert!(r.get_unenqueued_dirty_keys().is_empty());
    assert_eq!(*inner.deleted.lock(), set(&["c"]));

    assert_eq!(r.get_and_clear_inflight_keys(), set(&["b"]));
    assert!(r.get_and_clear_inflight_keys().is_empty());
    assert!(r.get_and_clear_unsuccessfully_rewound_keys().is_empty());
}

#[test]
fn dirty_and_inflight_injected_clears_both() {
    let r = DirtyAndInflightTrackingProgressReceiver::new(Arc::new(TrackingProgressReceiver::new(
        false,
    )));
    r.dirtied(&key("a"), DirtyType::Change);
    r.injected(&key("a"));
    assert!(r.get_unenqueued_dirty_keys().is_empty());
    r.enqueueing(&key("b"));
    r.injected(&key("b"));
    assert!(!r.is_inflight(&key("b")));
    r.enqueueing(&key("c"));
    r.remove_from_inflight(&key("c"));
    assert!(r.get_and_clear_inflight_keys().is_empty());
}

#[test]
fn dirty_and_inflight_failed_evaluation_leaves_dirty_set() {
    let r = DirtyAndInflightTrackingProgressReceiver::new(Arc::new(TrackingProgressReceiver::new(
        false,
    )));
    r.dirtied(&key("a"), DirtyType::Dirty);
    r.evaluated(
        &key("a"),
        EvaluationState::FailVersionChanged,
        None,
        None,
        None,
    );
    assert!(r.get_unenqueued_dirty_keys().is_empty());
}

/// `DirtyAndInflightTrackingProgressReceiver.addToDirtySet` / `removeFromDirtySet` /
/// `getAndClearUnsuccessfullyRewoundKeys`: a rewound key is not an unenqueued dirty key; it stays
/// "unsuccessfully rewound" until it is evaluated successfully, including when enqueued and when
/// evaluated to an error.
#[test]
fn dirty_and_inflight_tracks_unsuccessfully_rewound_keys() {
    let r = DirtyAndInflightTrackingProgressReceiver::new(Arc::new(TrackingProgressReceiver::new(
        false,
    )));
    for k in ["a", "b", "c", "d"] {
        r.dirtied(&key(k), DirtyType::Rewind);
    }
    assert!(r.get_unenqueued_dirty_keys().is_empty());
    // a: rewound and evaluated successfully.
    r.enqueueing(&key("a"));
    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionChanged,
        None,
        None,
        None,
    );
    // b: enqueued but not evaluated.
    r.enqueueing(&key("b"));
    // c: evaluated to an error.
    r.enqueueing(&key("c"));
    r.evaluated(
        &key("c"),
        EvaluationState::FailVersionChanged,
        None,
        None,
        None,
    );
    // d: not yet enqueued.
    assert_eq!(
        r.get_and_clear_unsuccessfully_rewound_keys(),
        set(&["b", "c", "d"])
    );
    assert!(r.get_and_clear_unsuccessfully_rewound_keys().is_empty());
}

#[test]
fn inflight_only_tracks_inflight() {
    let inner = Arc::new(TrackingProgressReceiver::new(false));
    let r = InflightOnlyTrackingProgressReceiver::new(inner.clone());
    r.dirtied(&key("a"), DirtyType::Dirty);
    assert_eq!(*inner.dirty.lock(), set(&["a"]));
    r.enqueueing(&key("a"));
    r.enqueueing(&key("a"));
    assert_eq!(*inner.enqueued.lock(), set(&["a"]));
    r.enqueue_after_error(&key("b"));
    assert_eq!(*inner.enqueued.lock(), set(&["a"]));
    assert!(r.is_inflight(&key("b")));
    r.injected(&key("b"));
    assert!(!r.is_inflight(&key("b")));
    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionUnchanged,
        None,
        None,
        None,
    );
    assert!(r.get_and_clear_inflight_keys().is_empty());
    r.deleted(&key("a"));
    assert_eq!(*inner.deleted.lock(), set(&["a"]));
}

#[test]
fn compound_receiver_forwards_to_each_in_order() {
    struct Log(Arc<Mutex<Vec<String>>>, &'static str);
    impl EvaluationProgressReceiver for Log {
        fn dirtied(&self, k: &SkyKey, t: DirtyType) {
            self.0
                .lock()
                .push(format!("{} dirtied {k:?} {t:?}", self.1));
        }
        fn deleted(&self, k: &SkyKey) {
            self.0.lock().push(format!("{} deleted {k:?}", self.1));
        }
        fn enqueueing(&self, k: &SkyKey) {
            self.0.lock().push(format!("{} enqueueing {k:?}", self.1));
        }
        fn state_starting(&self, k: &SkyKey, s: NodeState) {
            self.0
                .lock()
                .push(format!("{} starting {k:?} {s:?}", self.1));
        }
        fn state_ending(&self, k: &SkyKey, s: NodeState) {
            self.0.lock().push(format!("{} ending {k:?} {s:?}", self.1));
        }
        fn evaluated(
            &self,
            k: &SkyKey,
            s: EvaluationState,
            _: Option<&SkyValue>,
            _: Option<&ErrorInfo>,
            _: Option<&GroupedDeps>,
        ) {
            self.0
                .lock()
                .push(format!("{} evaluated {k:?} {s:?}", self.1));
        }
        fn change_pruned(&self, k: &SkyKey) {
            self.0.lock().push(format!("{} pruned {k:?}", self.1));
        }
    }
    let log = Arc::new(Mutex::new(vec![]));
    let r = CompoundEvaluationProgressReceiver::new(vec![
        Arc::new(Log(log.clone(), "1")),
        Arc::new(Log(log.clone(), "2")),
    ]);
    r.dirtied(&key("a"), DirtyType::Change);
    r.deleted(&key("a"));
    r.enqueueing(&key("a"));
    r.state_starting(&key("a"), NodeState::CheckDirty);
    r.state_ending(&key("a"), NodeState::CheckDirty);
    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionChanged,
        None,
        None,
        None,
    );
    r.change_pruned(&key("a"));
    assert_eq!(
        *log.lock(),
        vec![
            "1 dirtied a CHANGE",
            "2 dirtied a CHANGE",
            "1 deleted a",
            "2 deleted a",
            "1 enqueueing a",
            "2 enqueueing a",
            "1 starting a CheckDirty",
            "2 starting a CheckDirty",
            "1 ending a CheckDirty",
            "2 ending a CheckDirty",
            "1 evaluated a SuccessVersionChanged",
            "2 evaluated a SuccessVersionChanged",
            "1 pruned a",
            "2 pruned a",
        ]
    );
}

#[test]
fn tracking_receiver_check_evaluation_results() {
    let r = TrackingProgressReceiver::new(true);
    r.dirtied(&key("a"), DirtyType::Dirty);
    r.deleted(&key("b"));
    r.evaluated(
        &key("a"),
        EvaluationState::SuccessVersionUnchanged,
        None,
        None,
        None,
    );
    r.evaluated(
        &key("b"),
        EvaluationState::SuccessVersionChanged,
        None,
        None,
        None,
    );
    assert!(r.dirty.lock().is_empty());
    assert!(r.deleted.lock().is_empty());
    assert_eq!(*r.evaluated.lock(), set(&["a", "b"]));
    let ran = Arc::new(AtomicBool::new(false));
    let ran2 = ran.clone();
    r.set_next_invalidation_callback(move || ran2.store(true, Ordering::SeqCst));
    r.deleted(&key("c"));
    assert!(ran.load(Ordering::SeqCst));
    r.clear();
    assert!(r.deleted.lock().is_empty() && r.evaluated.lock().is_empty());
}

#[test]
fn key_hermeticity_is_declared_per_key_type() {
    assert_eq!(
        ERROR_TRANSIENCE_KEY.hermeticity(),
        FunctionHermeticity::Nonhermetic
    );
    assert_eq!(key("a").hermeticity(), FunctionHermeticity::Hermetic);
    assert_eq!(
        SkyKey::new(Num(1)).hermeticity(),
        FunctionHermeticity::Hermetic
    );
}
