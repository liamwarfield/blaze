//! Port of `EagerInvalidatorTest`, run as Java's two suites: `DeletingInvalidatorTest` (tests
//! named `<name>_deleting`) and `DirtyingInvalidatorTest` (`<name>_dirtying`).
//!
//! PORT: Java evaluates with a `ParallelEvaluator` made on the test's graph; here an evaluator
//! (whose graph the invalidators are run on) evaluates. Invalidation is not interruptible here
//! (`invalidation.rs`), so the interrupt tests are ignored stubs.

use super::invalidation_progress_receiver::{
    InvalidationProgressReceiver, InvalidationProgressReceiverAdapter, InvalidationState,
};
use super::*;
use crate::invalidation::{DeletingInvalidationState, EagerInvalidator, InvalidationType};
use crate::node_entry::NodeEntry;
use crate::progress::DirtyAndInflightTrackingProgressReceiver;

/// The suite: Java's `DeletingInvalidatorTest` or `DirtyingInvalidatorTest`.
#[derive(Clone, Copy, PartialEq)]
enum Suite {
    Deleting,
    Dirtying,
}

/// Java: the test's `state` field (`newInvalidationState()`).
enum State {
    Deleting(DeletingInvalidationState),
    Dirtying(crate::invalidation::InvalidationState),
}

/// Java: `EagerInvalidatorTest`'s fields: the graph (here the evaluator's), the tester and the
/// invalidation state.
struct Fixture {
    suite: Suite,
    tester: Arc<Tester>,
    ev: Evaluator,
    state: Mutex<State>,
}

impl Fixture {
    fn new(suite: Suite) -> Self {
        let tester = new_tester();
        let ev = tester_evaluator(tester.clone());
        Fixture {
            suite,
            tester,
            ev,
            state: Mutex::new(match suite {
                // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:645 DeletingInvalidatorTest.newInvalidationState
                Suite::Deleting => State::Deleting(DeletingInvalidationState::new()),
                // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:787 DirtyingInvalidatorTest.newInvalidationState
                Suite::Dirtying => {
                    State::Dirtying(crate::invalidation::InvalidationState::dirtying())
                }
            }),
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:76 EagerInvalidatorTest.expectedState
    fn expected_state(&self) -> InvalidationState {
        match self.suite {
            Suite::Deleting => InvalidationState::Deleted,
            Suite::Dirtying => InvalidationState::Dirty,
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:89 EagerInvalidatorTest.gcExpected
    fn gc_expected(&self) -> bool {
        self.suite == Suite::Deleting
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:128 EagerInvalidatorTest.reverseDepsPresent
    fn reverse_deps_present(&self) -> bool {
        self.suite == Suite::Dirtying
    }

    /// `invalidate(graph, progressReceiver, keys)` of the suite.
    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:616 DeletingInvalidatorTest.invalidate
    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:763 DirtyingInvalidatorTest.invalidate
    fn invalidate(
        &self,
        progress_receiver: &Arc<DirtyAndInflightTrackingProgressReceiver>,
        keys: &[SkyKey],
    ) {
        let diff: Vec<SkyKey> = keys.to_vec();
        match &mut *self.state.lock() {
            State::Deleting(state) => {
                let wrapper =
                    DirtyAndInflightTrackingProgressReceiver::new(progress_receiver.clone());
                if let Some(mut v) = EagerInvalidator::create_deleting_visitor_if_needed(
                    &self.ev.graph,
                    diff,
                    &wrapper,
                    state,
                    true,
                ) {
                    v.run();
                }
            }
            State::Dirtying(state) => {
                if let Some(mut v) = EagerInvalidator::create_invalidating_visitor_if_needed(
                    &self.ev.graph,
                    diff,
                    progress_receiver,
                    state,
                ) {
                    v.run();
                }
            }
        }
    }

    fn state_is_empty(&self) -> bool {
        match &*self.state.lock() {
            State::Deleting(s) => s.is_empty(),
            State::Dirtying(s) => s.is_empty(),
        }
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:167 EagerInvalidatorTest.invalidateWithoutError
    fn invalidate_without_error(
        &self,
        progress_receiver: &Arc<DirtyAndInflightTrackingProgressReceiver>,
        keys: &[SkyKey],
    ) {
        self.invalidate(progress_receiver, keys);
        assert!(self.state_is_empty());
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:92 EagerInvalidatorTest.isInvalidated
    fn is_invalidated(&self, key: &SkyKey) -> bool {
        let entry = self.ev.graph.get_if_present(key);
        if self.gc_expected() {
            entry.is_none()
        } else {
            entry.is_none_or(|e| e.lock().entry.is_dirty())
        }
    }

    fn eval(&self, keep_going: bool, keys: &[SkyKey]) -> EvaluationResult {
        eval(&self.ev, keep_going, 4, keys)
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:180 EagerInvalidatorTest.assertValueValue
    fn assert_value_value(&self, name: &'static str, expected_value: &str) {
        let r = self.eval(false, &[k(name)]);
        assert_eq!(*r.get(&T(name)).unwrap(), expected_value);
    }

    fn reverse_deps_for_done_entry(&self, key: &SkyKey) -> HashSet<SkyKey> {
        self.ev
            .graph
            .get_if_present(key)
            .unwrap()
            .lock()
            .entry
            .get_reverse_deps_for_done_entry()
            .into_iter()
            .collect()
    }

    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:599 EagerInvalidatorTest.setupInvalidatableGraph
    fn setup_invalidatable_graph(&self) {
        let a_key = nk("a");
        self.tester.set_key(a_key.clone(), "a");
        self.tester.set("b", "b");
        self.tester
            .add_dependency_key(k("ab"), a_key.clone())
            .add_dependency("ab", "b");
        self.assert_value_value("ab", "ab");
        self.tester.set_key(a_key, "c");
    }
}

fn null_receiver() -> Arc<DirtyAndInflightTrackingProgressReceiver> {
    Arc::new(DirtyAndInflightTrackingProgressReceiver::new(Arc::new(
        crate::progress::NULL,
    )))
}

/// An `InvalidationProgressReceiver` that checks the state and records the keys.
struct Recording {
    expected: Option<InvalidationState>,
    invalidated: Arc<Mutex<HashSet<SkyKey>>>,
}

impl InvalidationProgressReceiver for Recording {
    fn invalidated(&self, sky_key: &SkyKey, state: InvalidationState) {
        if let Some(expected) = self.expected {
            blaze_util::check_state!(state == expected);
        }
        self.invalidated.lock().insert(sky_key.clone());
    }
}

fn recording_receiver(
    expected: Option<InvalidationState>,
) -> (
    Arc<DirtyAndInflightTrackingProgressReceiver>,
    Arc<Mutex<HashSet<SkyKey>>>,
) {
    let invalidated = Arc::new(Mutex::new(HashSet::new()));
    let receiver = Arc::new(DirtyAndInflightTrackingProgressReceiver::new(Arc::new(
        InvalidationProgressReceiverAdapter(Recording {
            expected,
            invalidated: invalidated.clone(),
        }),
    )));
    (receiver, invalidated)
}

fn set_of(keys: &[SkyKey]) -> HashSet<SkyKey> {
    keys.iter().cloned().collect()
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:192 EagerInvalidatorTest.receiverWorks
fn receiver_works(suite: Suite) {
    let f = Fixture::new(suite);
    let (receiver, invalidated) = recording_receiver(Some(f.expected_state()));
    let a_key = nk("a");
    let b_key = nk("b");
    f.tester.set_key(a_key.clone(), "a");
    f.tester.set_key(b_key.clone(), "b");
    f.tester
        .add_dependency_key(k("ab"), a_key.clone())
        .add_dependency_key(k("ab"), b_key.clone());
    f.assert_value_value("ab", "ab");

    f.tester.set_key(a_key.clone(), "c");
    f.invalidate_without_error(&receiver, std::slice::from_ref(&a_key));
    assert_eq!(*invalidated.lock(), set_of(&[a_key.clone(), k("ab")]));
    f.assert_value_value("ab", "cb");
    f.tester.set_key(b_key.clone(), "d");
    f.invalidate_without_error(&receiver, std::slice::from_ref(&b_key));
    assert_eq!(*invalidated.lock(), set_of(&[a_key, k("ab"), b_key]));
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:221 EagerInvalidatorTest.receiverIsNotifiedAboutNodesInError
fn receiver_is_notified_about_nodes_in_error(suite: Suite) {
    let f = Fixture::new(suite);
    let (receiver, invalidated) = recording_receiver(Some(f.expected_state()));
    // Given a graph consisting of two nodes, "a" and "ab" such that "ab" depends on "a",
    // And given "ab" is in error,
    let a_key = nk("a");
    f.tester.set_key(a_key.clone(), "a");
    f.tester
        .add_dependency_key(k("ab"), a_key.clone())
        .set_has_error("ab", true);
    f.eval(false, &[k("ab")]);

    // When "a" is invalidated,
    f.invalidate_without_error(&receiver, std::slice::from_ref(&a_key));

    // Then the invalidation receiver is notified of both "a" and "ab"'s invalidations.
    assert_eq!(*invalidated.lock(), set_of(&[a_key, k("ab")]));
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:252 EagerInvalidatorTest.invalidateValuesNotInGraph
fn invalidate_values_not_in_graph(suite: Suite) {
    let f = Fixture::new(suite);
    let (receiver, invalidated) = recording_receiver(Some(InvalidationState::Dirty));
    let a_key = nk("a");
    f.invalidate_without_error(&receiver, std::slice::from_ref(&a_key));
    assert!(invalidated.lock().is_empty());
    f.tester.set_key(a_key.clone(), "a");
    let r = f.eval(false, std::slice::from_ref(&a_key));
    assert_eq!(*r.get(&N("a")).unwrap(), "a");
    f.invalidate_without_error(&receiver, &[nk("b")]);
    assert!(invalidated.lock().is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:275 EagerInvalidatorTest.invalidatedValuesAreGCedAsExpected
fn invalidated_values_are_gced_as_expected(suite: Suite) {
    let f = Fixture::new(suite);
    let key = nk("a");
    f.tester.set_key(key.clone(), "heavy");
    let r = f.eval(false, std::slice::from_ref(&key));
    // Java: a WeakReference to the HeavyValue.
    let weak_ref = Arc::downgrade(r.get_dyn(&key).unwrap());
    drop(r);
    f.invalidate(&null_receiver(), std::slice::from_ref(&key));
    // Java: tester = null; heavyValue = null (the tester's constant is a copy here).
    if f.gc_expected() {
        assert!(weak_ref.upgrade().is_none());
    } else {
        assert!(weak_ref.upgrade().is_some());
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:299 EagerInvalidatorTest.reverseDepsConsistent
fn reverse_deps_consistent(suite: Suite) {
    let f = Fixture::new(suite);
    f.tester.set("a", "a").set("b", "b").set("c", "c");
    let ab_key = nk("ab");
    f.tester
        .add_dependency_key(ab_key.clone(), k("a"))
        .add_dependency_key(ab_key.clone(), k("b"));
    f.tester.add_dependency("bc", "b").add_dependency("bc", "c");
    f.tester
        .add_dependency_key(k("ab_c"), ab_key.clone())
        .add_dependency("ab_c", "c");
    f.eval(false, &[k("ab_c"), k("bc")]);

    assert_eq!(
        f.reverse_deps_for_done_entry(&k("a")),
        set_of(std::slice::from_ref(&ab_key))
    );
    assert_eq!(
        f.reverse_deps_for_done_entry(&k("b")),
        set_of(&[ab_key.clone(), k("bc")])
    );
    assert_eq!(
        f.reverse_deps_for_done_entry(&k("c")),
        set_of(&[k("ab_c"), k("bc")])
    );

    f.invalidate_without_error(&null_receiver(), std::slice::from_ref(&ab_key));
    f.eval(false, &[]);

    // The graph values should be gone.
    assert!(f.is_invalidated(&ab_key));
    assert!(f.is_invalidated(&k("abc")));

    // The reverse deps to ab and ab_c should have been removed if reverse deps are cleared.
    let mut reverse_deps = HashSet::new();
    if f.reverse_deps_present() {
        reverse_deps.insert(ab_key.clone());
    }
    assert_eq!(f.reverse_deps_for_done_entry(&k("a")), reverse_deps);
    reverse_deps.insert(k("bc"));
    assert_eq!(f.reverse_deps_for_done_entry(&k("b")), reverse_deps);
    reverse_deps.clear();
    if f.reverse_deps_present() {
        reverse_deps.insert(k("ab_c"));
    }
    reverse_deps.insert(k("bc"));
    assert_eq!(f.reverse_deps_for_done_entry(&k("c")), reverse_deps);
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:424 EagerInvalidatorTest.deepGraph
fn deep_graph(suite: Suite) {
    let f = Fixture::new(suite);
    let depth = 1 << 15;
    let leaf_key = nk(leak(depth.to_string()));
    for i in 0..depth {
        let dep = if i + 1 == depth {
            leaf_key.clone()
        } else {
            k(leak((i + 1).to_string()))
        };
        f.tester.add_dependency_key(k(leak(i.to_string())), dep);
    }
    f.tester.set_key(leaf_key.clone(), "leaf");
    // PORT: Java evaluates "0" at once. The blocking engine would hold a thread per
    // `max_inline_depth` levels of the chain (1024 nested threads), so the chain is built from
    // the bottom in steps of 16 levels; the graph (deps and reverse deps) is the same.
    for i in (0..depth).rev().step_by(16) {
        f.eval(false, &[k(leak(i.to_string()))]);
    }
    f.eval(false, &[k("0")]);
    f.invalidate_without_error(&null_receiver(), &[leaf_key]);
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:474 EagerInvalidatorTest.allNodesProcessed
fn all_nodes_processed(suite: Suite) {
    let f = Fixture::new(suite);
    // Java: InvalidatingNodeVisitor.DEFAULT_THREAD_COUNT (the number of processors).
    let default_thread_count = std::thread::available_parallelism().map_or(4, |n| n.get());
    let keys_to_delete: Vec<SkyKey> = (0..default_thread_count - 1)
        .map(|i| nk(leak(format!("key{i}"))))
        .collect();
    f.invalidate(&null_receiver(), &keys_to_delete);
    assert!(f.state_is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:486 EagerInvalidatorTest.deletingInsideForkJoinPoolWorks
// PORT: the invalidator runs on the calling thread; here the calling thread is another
// evaluator-free thread, as Java's is a fork-join worker.
fn deleting_inside_fork_join_pool_works(suite: Suite) {
    let f = Arc::new(Fixture::new(suite));
    let f2 = f.clone();
    std::thread::spawn(move || f2.invalidate(&null_receiver(), &[nk("a")]))
        .join()
        .unwrap();
}

macro_rules! suite_tests {
    ($($name:ident: $deleting:ident, $dirtying:ident;)*) => {
        $(
            #[test]
            fn $deleting() {
                $name(Suite::Deleting)
            }
            #[test]
            fn $dirtying() {
                $name(Suite::Dirtying)
            }
        )*
    };
}

suite_tests! {
    receiver_works: receiver_works_deleting, receiver_works_dirtying;
    receiver_is_notified_about_nodes_in_error:
        receiver_is_notified_about_nodes_in_error_deleting,
        receiver_is_notified_about_nodes_in_error_dirtying;
    invalidate_values_not_in_graph:
        invalidate_values_not_in_graph_deleting, invalidate_values_not_in_graph_dirtying;
    invalidated_values_are_gced_as_expected:
        invalidated_values_are_gced_as_expected_deleting,
        invalidated_values_are_gced_as_expected_dirtying;
    reverse_deps_consistent: reverse_deps_consistent_deleting, reverse_deps_consistent_dirtying;
    deep_graph: deep_graph_deleting, deep_graph_dirtying;
    all_nodes_processed: all_nodes_processed_deleting, all_nodes_processed_dirtying;
    deleting_inside_fork_join_pool_works:
        deleting_inside_fork_join_pool_works_deleting,
        deleting_inside_fork_join_pool_works_dirtying;
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:730 DeletingInvalidatorTest.dirtyTrackingProgressReceiverWorksWithDeletingInvalidator
#[test]
fn dirty_tracking_progress_receiver_works_with_deleting_invalidator() {
    let f = Fixture::new(Suite::Deleting);
    f.setup_invalidatable_graph();
    let receiver = null_receiver();

    // Dirty the node, and ensure that the tracker is aware of it:
    let diff = vec![nk("a")];
    let mut state1 = crate::invalidation::InvalidationState::dirtying();
    EagerInvalidator::create_invalidating_visitor_if_needed(
        &f.ev.graph,
        diff.clone(),
        &receiver,
        &mut state1,
    )
    .expect("NullPointerException")
    .run();
    assert_eq!(
        receiver.get_unenqueued_dirty_keys(),
        set_of(&[diff[0].clone(), k("ab")])
    );

    // Delete the node, and ensure that the tracker is no longer tracking it:
    let State::Deleting(state) = &mut *f.state.lock() else {
        unreachable!()
    };
    EagerInvalidator::create_deleting_visitor_if_needed(&f.ev.graph, diff, &receiver, state, true)
        .expect("NullPointerException")
        .run();
    assert!(receiver.get_unenqueued_dirty_keys().is_empty());
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:802 DirtyingInvalidatorTest.dirtyTrackingProgressReceiverWorksWithDirtyingInvalidator
#[test]
fn dirty_tracking_progress_receiver_works_with_dirtying_invalidator() {
    let f = Fixture::new(Suite::Dirtying);
    f.setup_invalidatable_graph();
    let receiver = null_receiver();

    // Dirty the node, and ensure that the tracker is aware of it:
    f.invalidate(&receiver, &[nk("a")]);
    assert_eq!(receiver.get_unenqueued_dirty_keys().len(), 2);
}

/// The pending visitations Java's `getValuesToInvalidate` builds have the suite's type.
#[test]
fn default_invalidation_types() {
    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:650 DeletingInvalidatorTest.defaultInvalidationType
    let mut deleting = DeletingInvalidationState::new();
    deleting.update([nk("a")]);
    assert_eq!(
        deleting.get_invalidations_for_testing(),
        HashSet::from([(nk("a"), InvalidationType::Deleted)])
    );
    // Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:792 DirtyingInvalidatorTest.defaultInvalidationType
    let mut dirtying = crate::invalidation::InvalidationState::dirtying();
    dirtying.update([nk("a")]);
    assert_eq!(
        dirtying.get_invalidations_for_testing(),
        HashSet::from([(nk("a"), InvalidationType::Changed)])
    );
}

macro_rules! ignored {
    ($($name:ident: $java:literal, $reason:literal;)*) => {
        $(
            #[doc = concat!("Java: ", $java)]
            #[test]
            #[ignore = $reason]
            fn $name() {
                unimplemented!(concat!("not ported: ", $java))
            }
        )*
    };
}

// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:349 EagerInvalidatorTest.interruptChild
// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:502 EagerInvalidatorTest.interruptRecoversNextTime
// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:538 EagerInvalidatorTest.interruptThreadInReceiver
// Java: src/test/java/com/google/devtools/build/skyframe/EagerInvalidatorTest.java:666 DeletingInvalidatorTest.interruptDuringRewind_parentAlreadySignaled
ignored! {
    interrupt_child_deleting: "EagerInvalidatorTest.interruptChild",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_child_dirtying: "EagerInvalidatorTest.interruptChild",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_recovers_next_time_deleting: "EagerInvalidatorTest.interruptRecoversNextTime",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_recovers_next_time_dirtying: "EagerInvalidatorTest.interruptRecoversNextTime",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_thread_in_receiver_deleting: "EagerInvalidatorTest.interruptThreadInReceiver",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_thread_in_receiver_dirtying: "EagerInvalidatorTest.interruptThreadInReceiver",
        "invalidation is not interruptible here (invalidation.rs)";
    interrupt_during_rewind_parent_already_signaled:
        "DeletingInvalidatorTest.interruptDuringRewind_parentAlreadySignaled",
        "SkyFunction.Reset and node rewinding are not ported";
}
