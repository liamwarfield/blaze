//! Port of `GraphTest`, run on `InMemoryGraphImpl` (Java's `InMemoryGraphTest`) and on the
//! edgeless graph (Java's `InMemoryGraphTest.EdgelessInMemoryGraphTest`, tests named
//! `<name>_edgeless`).

use super::*;
use crate::in_memory_graph::{InMemoryGraphImpl, Node};
use crate::node_entry::{DependencyState, DirtyType, NodeEntry};
use crate::value_with_metadata::ValueWithMetadata;
use crate::version::{IntVersion, Version};

/// What `InMemoryGraphTest` and `EdgelessInMemoryGraphTest` override.
#[derive(Clone, Copy)]
struct GraphKind {
    keep_edges: bool,
}

const INCREMENTAL: GraphKind = GraphKind { keep_edges: true };
const EDGELESS: GraphKind = GraphKind { keep_edges: false };

impl GraphKind {
    // Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:28 InMemoryGraphTest.getStartingVersion
    fn get_starting_version(self) -> Version {
        if self.keep_edges {
            IntVersion::of(0).into()
        } else {
            Version::constant()
        }
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:33 InMemoryGraphTest.getNextVersion
    fn get_next_version(self, v: Version) -> Version {
        match v {
            Version::Int(v) if self.keep_edges => v.next().into(),
            _ => panic!("UnsupportedOperationException"),
        }
    }
    // Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:40 InMemoryGraphTest.makeGraph
    fn make_graph(self) -> Arc<InMemoryGraphImpl> {
        Arc::new(InMemoryGraphImpl::new(self.keep_edges))
    }
    fn should_test_incrementality(self) -> bool {
        self.keep_edges
    }
}

fn key(name: &'static str) -> SkyKey {
    k(name)
}

fn string_value(v: &str) -> ValueWithMetadata {
    ValueWithMetadata::Bare(Arc::new(v.to_string()))
}

fn value_of(node: &Node) -> String {
    node.lock()
        .entry
        .get_value()
        .expect("value")
        .downcast_ref::<String>()
        .expect("string")
        .clone()
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:530 GraphTest.startEvaluation
fn start_evaluation(node: &Node) -> DependencyState {
    node.lock().entry.add_reverse_dep_and_check_if_done(None)
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:119 GraphTest.createIfAbsentBatch
fn create_if_absent_batch_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let cat = key("cat");
    let dog = key("dog");
    let batch = graph.create_if_absent_batch([&cat, &dog]);
    let cat_node = graph.get_if_present(&cat).expect("cat");
    let dog_node = graph.get_if_present(&dog).expect("dog");
    assert!(Arc::ptr_eq(&batch[0], &cat_node));
    assert!(Arc::ptr_eq(&batch[1], &dog_node));
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:134 GraphTest.createIfAbsentBatch_interveningCallToRemove
fn create_if_absent_batch_intervening_call_to_remove_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let key = key("key");
    let batch = graph.create_if_absent_batch([&key]);
    graph.remove(&key);
    // PORT: the batch holds the node itself, so it stays available.
    assert_eq!(batch.len(), 1);
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:142 GraphTest.getBatchAndGetBatchMapConsistency
fn get_batch_and_get_batch_map_consistency_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let cat = key("cat");
    let dog = key("dog");
    let keys = [cat.clone(), dog.clone()];
    graph.create_if_absent_batch(&keys);
    // PORT: the in-memory graph's getBatch is getIfPresent per key.
    let cat_entry = graph.get_if_present(&cat).expect("cat");
    let dog_entry = graph.get_if_present(&dog).expect("dog");
    let batch_map = graph.get_batch_map(&keys);
    assert_eq!(batch_map.len(), 2);
    assert!(Arc::ptr_eq(&batch_map[0].1, &cat_entry));
    assert!(Arc::ptr_eq(&batch_map[1].1, &dog_entry));
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:161 GraphTest.createIfAbsentBatchConcurrentWithGet
fn create_if_absent_batch_concurrent_with_get_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let num_iters = 50;
    let key = key("key");
    for _ in 0..num_iters {
        let (g, k2) = (graph.clone(), key.clone());
        let t = std::thread::spawn(move || {
            g.get_if_present(&k2);
        });
        let batch = graph.create_if_absent_batch([&key]);
        assert_eq!(batch.len(), 1);
        graph.remove(&key);
        t.join().unwrap();
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:183 GraphTest.testCreateIfAbsentBatchWithConcurrentGet
fn test_create_if_absent_batch_with_concurrent_get_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let key = key("foo");
    let num_threads = 50;
    let start_threads = Latch::new(1);
    let mut threads = vec![];
    for _ in 0..num_threads {
        let (g, k2, start) = (graph.clone(), key.clone(), start_threads.clone());
        threads.push(std::thread::spawn(move || {
            start.wait("threads not started");
            g.create_if_absent_batch([&k2]);
        }));
        let (g, k2, start) = (graph.clone(), key.clone(), start_threads.clone());
        threads.push(std::thread::spawn(move || {
            start.wait("threads not started");
            g.get_if_present(&k2);
        }));
    }
    start_threads.count_down();
    for t in threads {
        t.join().unwrap();
    }
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:225 GraphTest.testAddRemoveRdeps
// Tests adding and removing Rdeps of a NodeEntry while a node transitions from not done to done.
fn test_add_remove_rdeps_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let starting_version = kind.get_starting_version();
    let key = key("foo");
    let entry = graph.create_if_absent_batch([&key]).pop().unwrap();
    // These numbers are arbitrary.
    let num_threads = 50;
    let num_keys = num_threads;
    // One chunk will be used to add and remove rdeps before setting the node value. The second
    // chunk of work will have the node value set and the last chunk will be to add and remove
    // rdeps after the value has been set.
    let chunk_size = 40;
    let num_iterations = chunk_size * 2;
    let wait_for_start = Latch::new(1);
    let wait_for_added_rdep = Latch::new(num_threads);
    let wait_for_set_value = Latch::new(1);
    // Add single rdep before transition to done.
    assert_eq!(
        entry
            .lock()
            .entry
            .add_reverse_dep_and_check_if_done(Some(k("rdep"))),
        DependencyState::NeedsScheduling
    );
    let rdep_keys: Vec<SkyKey> = (0..num_keys).map(|i| k(leak(format!("rdep{i}")))).collect();
    graph.create_if_absent_batch(&rdep_keys);
    let mut threads = vec![];
    for rdep in rdep_keys {
        let (entry, start, added, set_value) = (
            entry.clone(),
            wait_for_start.clone(),
            wait_for_added_rdep.clone(),
            wait_for_set_value.clone(),
        );
        let incremental = kind.should_test_incrementality();
        threads.push(std::thread::spawn(move || {
            start.wait("start");
            assert_ne!(
                entry
                    .lock()
                    .entry
                    .add_reverse_dep_and_check_if_done(Some(rdep.clone())),
                DependencyState::Done
            );
            added.count_down();
            set_value.wait("set value");
            for _ in chunk_size..=num_iterations {
                let mut state = entry.lock();
                if incremental {
                    state.entry.remove_reverse_dep(rdep.clone());
                }
                state
                    .entry
                    .add_reverse_dep_and_check_if_done(Some(rdep.clone()));
                if incremental {
                    state.entry.get_reverse_deps_for_done_entry();
                }
            }
        }));
    }
    wait_for_start.count_down();
    wait_for_added_rdep.wait("added rdeps");
    {
        let mut state = entry.lock();
        state.entry.mark_rebuilding();
        state
            .entry
            .set_value(string_value("foo1"), starting_version, None);
    }
    wait_for_set_value.count_down();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(value_of(&graph.get_if_present(&key).unwrap()), "foo1");

    if !kind.should_test_incrementality() {
        return;
    }

    assert_eq!(
        graph
            .get_if_present(&key)
            .unwrap()
            .lock()
            .entry
            .get_reverse_deps_for_done_entry()
            .len(),
        num_keys + 1
    );

    let next_version = kind.get_next_version(starting_version);
    let same_entry = graph.get_if_present(&key).expect("present");
    {
        let mut state = same_entry.lock();
        // Mark the node as dirty again and check that the reverse deps have been preserved.
        state.entry.mark_dirty(DirtyType::Change);
    }
    start_evaluation(&same_entry);
    {
        let mut state = same_entry.lock();
        state.entry.mark_rebuilding();
        state
            .entry
            .set_value(string_value("foo2"), next_version, None);
    }
    assert_eq!(value_of(&graph.get_if_present(&key).unwrap()), "foo2");
    assert_eq!(
        graph
            .get_if_present(&key)
            .unwrap()
            .lock()
            .entry
            .get_reverse_deps_for_done_entry()
            .len(),
        num_keys + 1
    );
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:310 GraphTest.testAddingInflightNodes
// Tests adding inflight nodes with a given key while an existing node with the same key
// undergoes a transition from not done to done.
fn test_adding_inflight_nodes_impl(kind: GraphKind) {
    let graph = kind.make_graph();
    let starting_version = kind.get_starting_version();
    let num_threads = 50;
    let num_keys: usize = 500;
    let keys: Arc<Vec<SkyKey>> =
        Arc::new((0..num_keys).map(|i| k(leak(format!("foo{i}")))).collect());
    // Add each pair of keys 10 times.
    let node_created = Arc::new(Mutex::new(HashSet::new()));
    let values_set = Arc::new(Mutex::new(HashSet::new()));
    let tasks: Arc<Vec<(usize, usize)>> = Arc::new(
        (0..10)
            .flat_map(|_| (0..num_keys).flat_map(move |j| (j + 1..num_keys).map(move |k| (j, k))))
            .collect(),
    );
    let next = Arc::new(AtomicUsize::new(0));
    let mut threads = vec![];
    for _ in 0..num_threads {
        let (graph, keys, node_created, values_set, tasks, next) = (
            graph.clone(),
            keys.clone(),
            node_created.clone(),
            values_set.clone(),
            tasks.clone(),
            next.clone(),
        );
        threads.push(std::thread::spawn(move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(key_num1, key_num2)) = tasks.get(i) else {
                    break;
                };
                let pair = [keys[key_num1].clone(), keys[key_num2].clone()];
                for key in &pair {
                    if graph.get_if_present(key).is_none() {
                        node_created.lock().insert(key.clone());
                    }
                }
                let entries = graph.create_if_absent_batch(&pair);
                for (key_num, entry) in [key_num1, key_num2].into_iter().zip(&entries) {
                    let key = &keys[key_num];
                    // `entry.addReverseDepAndCheckIfDone(null)` should return NEEDS_SCHEDULING at
                    // most once.
                    let mut state = entry.lock();
                    if state.entry.add_reverse_dep_and_check_if_done(None)
                        == DependencyState::NeedsScheduling
                    {
                        state.entry.mark_rebuilding();
                        assert!(values_set.lock().insert(key.clone()));
                        // Set to done.
                        state.entry.set_value(
                            string_value(&format!("bar{key_num}")),
                            starting_version,
                            None,
                        );
                        assert!(state.entry.is_done());
                    }
                }
                // This shouldn't cause any problems from the other threads.
                graph.create_if_absent_batch(&pair);
            }
        }));
    }
    for t in threads {
        t.join().unwrap();
    }
    // Check that all the values are as expected.
    for (i, key) in keys.iter().enumerate() {
        assert!(node_created.lock().contains(key));
        assert!(values_set.lock().contains(key));
        let node = graph.get_if_present(key).unwrap();
        assert_eq!(value_of(&node), format!("bar{i}"));
        assert_eq!(node.lock().entry.get_version(), starting_version);
    }
}

/// Java: `GraphTest.BatchMethod` (`getBatch`, `getBatchMap`): both are lookups of present nodes
/// in the in-memory graph.
#[derive(Clone, Copy)]
enum BatchMethod {
    GetBatch,
    GetBatchMap,
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:390 GraphTest.testDoneToDirty
/// Initially calling `NodeEntry.setValue` and then making sure concurrent calls to
/// `QueryableGraph.get` and `QueryableGraph.getBatchMap` do not interfere with the node.
fn test_done_to_dirty_impl(kind: GraphKind, batch_method: BatchMethod) {
    assert!(kind.should_test_incrementality());
    let graph = kind.make_graph();
    let starting_version = kind.get_starting_version();
    let next_version = kind.get_next_version(starting_version);
    let num_keys = 1000;
    let num_batch_requests = 100;
    // Create a bunch of done nodes.
    let keys: Vec<SkyKey> = (0..num_keys).map(|i| k(leak(format!("foo{i}")))).collect();
    for entry in graph.create_if_absent_batch(&keys) {
        start_evaluation(&entry);
        let mut state = entry.lock();
        state.entry.mark_rebuilding();
        state
            .entry
            .set_value(string_value("bar"), starting_version, None);
    }
    assert!(graph.get_if_present(&keys[0]).is_some());

    // Only start all the threads once the batch requests are ready.
    let make_batch = Latch::new(num_batch_requests);
    // Do at least 5 single requests and batch requests before transitioning node.
    let get_batch = Latch::new(5);
    let get = Latch::new(5);
    let dep = key("dep");
    let mut threads = vec![];
    for (key_num, key) in keys.iter().cloned().enumerate() {
        // Transition the nodes from done to dirty and then back to done.
        let (graph1, key1, make1, get_batch1, get1, dep) = (
            graph.clone(),
            key.clone(),
            make_batch.clone(),
            get_batch.clone(),
            get.clone(),
            dep.clone(),
        );
        threads.push(std::thread::spawn(move || {
            make1.wait("make batch");
            get_batch1.wait("get batch");
            get1.wait("get");
            let entry = graph1.get_if_present(&key1).unwrap();
            let mut state = entry.lock();
            state.entry.mark_dirty(DirtyType::Change);
            // Make some changes, like adding a dep and rdep.
            state
                .entry
                .add_reverse_dep_and_check_if_done(Some(k("rdep")));
            state.entry.mark_rebuilding();
            state.entry.add_singleton_temporary_direct_dep(dep.clone());
            state.entry.signal_dep(next_version, Some(&dep));
            state
                .entry
                .set_value(string_value(&format!("bar{key_num}")), next_version, None);
        }));
        // Start a bunch of get() calls while the node transitions from dirty to done and back.
        let (graph2, make2, get2) = (graph.clone(), make_batch.clone(), get.clone());
        threads.push(std::thread::spawn(move || {
            make2.wait("make batch");
            let entry = graph2.get_if_present(&key).expect("present");
            // Requests for the value are made at the same time that the version increments from
            // the base. Check that there is no problem in requesting the version and that the
            // number is sane.
            let v = entry.lock().entry.get_version();
            assert!(v == starting_version || v == next_version, "{v:?}");
            get2.count_down();
        }));
    }
    // Start a bunch of getBatch() calls while the node transitions from dirty to done and back.
    // PORT: Java's seeded `Random` sample of the keys is replaced by a fixed pattern.
    for i in 0..num_batch_requests {
        let batch: Vec<SkyKey> = keys
            .iter()
            .enumerate()
            .filter(|(j, _)| (j * 7 + i * 13) % 2 == 0)
            .map(|(_, k)| k.clone())
            .collect();
        make_batch.count_down();
        let (graph3, make3, get_batch3) = (graph.clone(), make_batch.clone(), get_batch.clone());
        threads.push(std::thread::spawn(move || {
            make3.wait("make batch");
            let result: Vec<Arc<Node>> = match batch_method {
                BatchMethod::GetBatch => batch
                    .iter()
                    .map(|k| graph3.get_if_present(k).expect("present"))
                    .collect(),
                BatchMethod::GetBatchMap => graph3
                    .get_batch_map(&batch)
                    .into_iter()
                    .map(|(_, n)| n)
                    .collect(),
            };
            get_batch3.count_down();
            assert_eq!(result.len(), batch.len());
            for node in result {
                let v = node.lock().entry.get_version();
                assert!(v == starting_version || v == next_version, "{v:?}");
            }
        }));
    }
    for t in threads {
        t.join().unwrap();
    }
    for (i, key) in keys.iter().enumerate() {
        let node = graph.get_if_present(key).unwrap();
        assert_eq!(value_of(&node), format!("bar{i}"));
        let mut state = node.lock();
        assert_eq!(state.entry.get_version(), next_version);
        assert_eq!(
            state.entry.get_reverse_deps_for_done_entry(),
            vec![k("rdep")]
        );
        assert_eq!(state.entry.get_direct_deps(), vec![dep.clone()]);
    }
}

macro_rules! graph_tests {
    ($($name:ident, $edgeless:ident => $body:expr;)*) => {
        $(
            #[test]
            fn $name() {
                ($body)(INCREMENTAL)
            }
            #[test]
            fn $edgeless() {
                ($body)(EDGELESS)
            }
        )*
    };
}

graph_tests! {
    create_if_absent_batch, create_if_absent_batch_edgeless => create_if_absent_batch_impl;
    create_if_absent_batch_intervening_call_to_remove,
        create_if_absent_batch_intervening_call_to_remove_edgeless =>
        create_if_absent_batch_intervening_call_to_remove_impl;
    get_batch_and_get_batch_map_consistency, get_batch_and_get_batch_map_consistency_edgeless =>
        get_batch_and_get_batch_map_consistency_impl;
    create_if_absent_batch_concurrent_with_get,
        create_if_absent_batch_concurrent_with_get_edgeless =>
        create_if_absent_batch_concurrent_with_get_impl;
    test_create_if_absent_batch_with_concurrent_get,
        test_create_if_absent_batch_with_concurrent_get_edgeless =>
        test_create_if_absent_batch_with_concurrent_get_impl;
    test_add_remove_rdeps, test_add_remove_rdeps_edgeless => test_add_remove_rdeps_impl;
    test_adding_inflight_nodes, test_adding_inflight_nodes_edgeless =>
        test_adding_inflight_nodes_impl;
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:390 GraphTest.testDoneToDirty
#[test]
fn test_done_to_dirty_get_batch() {
    test_done_to_dirty_impl(INCREMENTAL, BatchMethod::GetBatch)
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:390 GraphTest.testDoneToDirty
#[test]
fn test_done_to_dirty_get_batch_map() {
    test_done_to_dirty_impl(INCREMENTAL, BatchMethod::GetBatchMap)
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:390 GraphTest.testDoneToDirty
#[test]
#[ignore = "Java skips it on the edgeless graph: assumeTrue(shouldTestIncrementality())"]
fn test_done_to_dirty_get_batch_edgeless() {
    test_done_to_dirty_impl(EDGELESS, BatchMethod::GetBatch)
}

// Java: src/test/java/com/google/devtools/build/skyframe/GraphTest.java:390 GraphTest.testDoneToDirty
#[test]
#[ignore = "Java skips it on the edgeless graph: assumeTrue(shouldTestIncrementality())"]
fn test_done_to_dirty_get_batch_map_edgeless() {
    test_done_to_dirty_impl(EDGELESS, BatchMethod::GetBatchMap)
}
