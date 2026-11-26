//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java.
//!
//! PORT: Java's factory is a Mockito spy of a real `WorkerFactory` over an in-memory file
//! system; here it is a real `WorkerFactory` over a temporary directory (it creates the worker
//! base directory; no worker process is started: the workers are only borrowed and returned).
//! The tests share `WorkerMultiplexerManager`'s static instances, as Java's do; they are run one
//! at a time (Java's JUnit runner runs them sequentially).

mod worker_common;

use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_exec::worker::worker::Worker;
use blaze_exec::worker::worker_factory::WorkerFactory;
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_lifecycle_manager::WorkerLifecycleManager;
use blaze_exec::worker::worker_pool::{WorkerPool, WorkerPoolConfig, WorkerPoolImpl};
use blaze_exec::worker::worker_process_metrics::WorkerProcessMetrics;
use blaze_exec::worker::worker_process_status::Status;
use blaze_optdefs::misc::worker_options::{MultiResourceEntry, WorkerOptions};
use blaze_vfs::{FileSystemExt, FileSystemRef};
use indexmap::IndexSet;
use parking_lot::Mutex;
use worker_common::*;

const DUMMY_MNEMONIC: &str = "dummy";
const PROCESS_ID_1: i64 = 1;
const PROCESS_ID_2: i64 = 2;
const PROCESS_ID_3: i64 = 3;
const PROCESS_ID_4: i64 = 4;
const PROCESS_ID_5: i64 = 5;

/// Java's runner runs a class's tests one at a time.
static SEQUENTIAL: Mutex<()> = Mutex::new(());

/// `new Reporter(new EventBus())`: drops every event.
struct NullReporter;

impl EventHandler for NullReporter {
    fn handle(&self, _event: &Event) {}
}

struct Fixture {
    _tmp: tempfile::TempDir,
    fs: FileSystemRef,
    factory: Arc<WorkerFactory>,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:58 WorkerLifecycleManagerTest.setUp
fn set_up() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs = fs();
    let factory = Arc::new(WorkerFactory::new(
        fs.get_path(
            tmp.path()
                .join("outputbase/bazel-workers")
                .to_str()
                .unwrap(),
        ),
        Arc::new(worker_options()),
    ));
    Fixture {
        _tmp: tmp,
        fs,
        factory,
    }
}

fn entry_list(entries: &[(&str, i32)]) -> Vec<MultiResourceEntry> {
    entries
        .iter()
        .map(|(k, v)| MultiResourceEntry {
            key: Some(k.to_string()),
            value: Some(*v),
        })
        .collect()
}

fn pool(f: &Fixture, singleplex: &[(&str, i32)], multiplex: &[(&str, i32)]) -> Arc<WorkerPoolImpl> {
    Arc::new(WorkerPoolImpl::new(
        f.factory.clone(),
        &WorkerPoolConfig::new(entry_list(singleplex), entry_list(multiplex)),
    ))
}

fn key(f: &Fixture, mnemonic: &str, multiplex: bool, sandboxed: bool) -> Arc<WorkerKey> {
    Arc::new(create_worker_key_mnemonic(
        mnemonic, &f.fs, multiplex, sandboxed,
    ))
}

fn options(total_worker_memory_limit_mb: i32, shrink: bool, verbose: bool) -> WorkerOptions {
    let mut options = worker_options();
    options.total_worker_memory_limit_mb = total_worker_memory_limit_mb;
    options.shrink_worker_pool = shrink;
    options.worker_verbose = verbose;
    options
}

fn manager(pool: &Arc<WorkerPoolImpl>, options: &WorkerOptions) -> WorkerLifecycleManager {
    WorkerLifecycleManager::new(
        pool.clone() as Arc<dyn WorkerPool>,
        options,
        Some(Arc::new(NullReporter)),
    )
}

/// `BlazeClock.instance().now()` once for the class.
const DEFAULT_INSTANT_MILLIS: i64 = 0;

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:555 WorkerLifecycleManagerTest.createWorkerMetric
fn create_worker_metric(
    worker: &Arc<dyn Worker>,
    process_id: i64,
    memory_in_kb: i32,
) -> Arc<WorkerProcessMetrics> {
    // We need to override the processId.
    let key = worker.get_worker_key();
    let wm = WorkerProcessMetrics::new(
        vec![worker.get_worker_id()],
        process_id,
        worker.get_status().clone(),
        key.get_mnemonic(),
        key.is_multiplex(),
        key.is_sandboxed(),
        key.hash_code(),
    );
    wm.add_collected_metrics(memory_in_kb, DEFAULT_INSTANT_MILLIS);
    Arc::new(wm)
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:571 WorkerLifecycleManagerTest.createMultiplexWorkerMetric
fn create_multiplex_worker_metric(
    workers: &[&Arc<dyn Worker>],
    process_id: i64,
    memory_in_kb: i32,
) -> Arc<WorkerProcessMetrics> {
    let key = workers[0].get_worker_key();
    let wpm = WorkerProcessMetrics::new(
        workers.iter().map(|w| w.get_worker_id()).collect(),
        process_id,
        workers[0].get_status().clone(),
        key.get_mnemonic(),
        key.is_multiplex(),
        key.is_sandboxed(),
        key.hash_code(),
    );
    wpm.add_collected_metrics(memory_in_kb, DEFAULT_INSTANT_MILLIS);
    Arc::new(wpm)
}

fn borrow(pool: &WorkerPoolImpl, key: &Arc<WorkerKey>) -> Arc<dyn Worker> {
    pool.borrow_worker(key).ok().expect("borrowWorker")
}

fn ids(workers: &[&Arc<dyn Worker>]) -> IndexSet<i32> {
    workers.iter().map(|w| w.get_worker_id()).collect()
}

fn status(w: &Arc<dyn Worker>) -> Status {
    w.get_status().get()
}

/// The three `doNothing` tests: one returned worker with 1000 KB, the given limit and metrics.
fn do_nothing(limit_mb: i32, with_metrics: bool) {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 1)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w1 = borrow(&worker_pool, &key);
    worker_pool.return_worker(&key, w1.clone());
    let worker_metrics = if with_metrics {
        vec![create_worker_metric(&w1, PROCESS_ID_1, 1000)]
    } else {
        vec![]
    };
    let manager = manager(&worker_pool, &options(limit_mb, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 1);
    assert_eq!(worker_pool.get_num_active(&key), 0);

    manager.evict_workers(&worker_metrics);

    assert_eq!(worker_pool.get_idle_workers().len(), 1);
    assert_eq!(worker_pool.get_num_active(&key), 0);
    // It should still have a valid status since it was not killed / marked to be killed.
    assert!(w1.get_status().is_valid());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:63 WorkerLifecycleManagerTest.testEvictWorkers_doNothing_lowMemoryUsage
#[test]
fn test_evict_workers_do_nothing_low_memory_usage() {
    do_nothing(1000 * 100, true);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:89 WorkerLifecycleManagerTest.testEvictWorkers_doNothing_zeroThreshold
#[test]
fn test_evict_workers_do_nothing_zero_threshold() {
    do_nothing(0, true);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:115 WorkerLifecycleManagerTest.testEvictWorkers_doNothing_emptyMetrics
#[test]
fn test_evict_workers_do_nothing_empty_metrics() {
    do_nothing(1, false);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:140 WorkerLifecycleManagerTest.testGetEvictionCandidates_selectOnlyWorker
#[test]
fn test_get_eviction_candidates_select_only_worker() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 1)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w1 = borrow(&worker_pool, &key);
    worker_pool.return_worker(&key, w1.clone());
    let worker_metrics = vec![create_worker_metric(&w1, PROCESS_ID_1, 2000)];
    let manager = manager(&worker_pool, &options(1, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 1);
    assert_eq!(worker_pool.get_num_active(&key), 0);
    assert!(w1.get_status().is_valid());

    manager.evict_workers(&worker_metrics);

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 0);
    // Directly killed since it is already returned.
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:165 WorkerLifecycleManagerTest.testGetEvictionCandidates_evictLargestWorkers
#[test]
fn test_get_eviction_candidates_evict_largest_workers() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 3)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w1 = borrow(&worker_pool, &key);
    let w2 = borrow(&worker_pool, &key);
    let w3 = borrow(&worker_pool, &key);
    worker_pool.return_worker(&key, w1.clone());
    worker_pool.return_worker(&key, w2.clone());
    worker_pool.return_worker(&key, w3.clone());
    let worker_metrics = vec![
        create_worker_metric(&w1, PROCESS_ID_1, 2000),
        create_worker_metric(&w2, PROCESS_ID_2, 1000),
        create_worker_metric(&w3, PROCESS_ID_3, 4000),
    ];
    let manager = manager(&worker_pool, &options(2, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 3);
    assert_eq!(worker_pool.get_num_active(&key), 0);
    assert!(w1.get_status().is_valid());
    assert!(w2.get_status().is_valid());
    assert!(w3.get_status().is_valid());

    manager.evict_workers(&worker_metrics);

    assert_eq!(worker_pool.get_idle_workers().len(), 1);
    assert_eq!(worker_pool.get_num_active(&key), 0);
    // Only w1 and w3 should have been killed.
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
    assert!(w2.get_status().is_valid());
    assert_eq!(status(&w3), Status::KilledDueToMemoryPressure);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:211 WorkerLifecycleManagerTest.testGetEvictionCandidates_numberOfWorkersIsMoreThanDefaultNumTests
#[test]
fn test_get_eviction_candidates_number_of_workers_is_more_than_default_num_tests() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 4)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w: Vec<Arc<dyn Worker>> = (0..4).map(|_| borrow(&worker_pool, &key)).collect();
    for wi in &w {
        worker_pool.return_worker(&key, wi.clone());
    }
    let worker_metrics = vec![
        create_worker_metric(&w[0], PROCESS_ID_1, 2000),
        create_worker_metric(&w[1], PROCESS_ID_2, 2000),
        create_worker_metric(&w[2], PROCESS_ID_3, 4000),
        create_worker_metric(&w[3], PROCESS_ID_4, 4000),
    ];
    let manager = manager(&worker_pool, &options(1, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 4);
    assert_eq!(worker_pool.get_num_active(&key), 0);

    manager.evict_workers(&worker_metrics);

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 0);
    for wi in &w {
        assert_eq!(status(wi), Status::KilledDueToMemoryPressure);
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:253 WorkerLifecycleManagerTest.testGetEvictionCandidates_evictWorkerWithSameMenmonicButDifferentKeys
#[test]
fn test_get_eviction_candidates_evict_worker_with_same_menmonic_but_different_keys() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 3)], &[]);
    let key1 = key(&f, DUMMY_MNEMONIC, false, false);
    let key2 = key(&f, DUMMY_MNEMONIC, false, true);
    let w1 = borrow(&worker_pool, &key1);
    let w2 = borrow(&worker_pool, &key2);
    let w3 = borrow(&worker_pool, &key2);
    worker_pool.return_worker(&key1, w1.clone());
    worker_pool.return_worker(&key2, w2.clone());
    worker_pool.return_worker(&key2, w3.clone());
    let worker_metrics = vec![
        create_worker_metric(&w1, PROCESS_ID_1, 3000),
        create_worker_metric(&w2, PROCESS_ID_2, 3000),
        create_worker_metric(&w3, PROCESS_ID_3, 1000),
    ];
    let manager = manager(&worker_pool, &options(2, false, true));

    assert_eq!(worker_pool.get_idle_workers(), ids(&[&w1, &w2, &w3]));
    assert!(w1.get_status().is_valid());
    assert!(w2.get_status().is_valid());
    assert!(w3.get_status().is_valid());

    manager.evict_workers(&worker_metrics);

    // Only w3 shouldn't be killed.
    assert_eq!(worker_pool.get_idle_workers(), ids(&[&w3]));
    assert_eq!(worker_pool.get_num_active(&key1), 0);
    assert_eq!(worker_pool.get_num_active(&key2), 0);
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
    assert_eq!(status(&w2), Status::KilledDueToMemoryPressure);
    assert!(w3.get_status().is_valid());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:295 WorkerLifecycleManagerTest.testGetEvictionCandidates_evictOnlyIdleWorkers
#[test]
fn test_get_eviction_candidates_evict_only_idle_workers() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 3)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w1 = borrow(&worker_pool, &key);
    let w2 = borrow(&worker_pool, &key);
    let w3 = borrow(&worker_pool, &key);
    worker_pool.return_worker(&key, w1.clone());
    worker_pool.return_worker(&key, w2.clone());
    let worker_metrics = vec![
        create_worker_metric(&w1, PROCESS_ID_1, 2000),
        create_worker_metric(&w2, PROCESS_ID_2, 1000),
        create_worker_metric(&w3, PROCESS_ID_3, 4000),
    ];
    let manager = manager(&worker_pool, &options(2, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 2);
    assert_eq!(worker_pool.get_num_active(&key), 1);

    manager.evict_workers(&worker_metrics);

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 1);
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
    assert_eq!(status(&w2), Status::KilledDueToMemoryPressure);
    // w3 is not killed because we're not shrinking the worker pool.
    assert!(w3.get_status().is_valid());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:335 WorkerLifecycleManagerTest.testGetEvictionCandidates_evictDifferentWorkerKeys
#[test]
fn test_get_eviction_candidates_evict_different_worker_keys() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 2), ("smart", 2)], &[]);
    let key1 = key(&f, DUMMY_MNEMONIC, false, false);
    let key2 = key(&f, "smart", false, false);
    let w1 = borrow(&worker_pool, &key1);
    let w2 = borrow(&worker_pool, &key1);
    let w3 = borrow(&worker_pool, &key2);
    let w4 = borrow(&worker_pool, &key2);
    worker_pool.return_worker(&key1, w1.clone());
    worker_pool.return_worker(&key1, w2.clone());
    worker_pool.return_worker(&key2, w3.clone());
    worker_pool.return_worker(&key2, w4.clone());
    let worker_metrics = vec![
        create_worker_metric(&w1, PROCESS_ID_1, 1000),
        create_worker_metric(&w2, PROCESS_ID_2, 4000),
        create_worker_metric(&w3, PROCESS_ID_3, 3000),
        create_worker_metric(&w4, PROCESS_ID_4, 1000),
    ];
    let manager = manager(&worker_pool, &options(2, false, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 4);
    assert_eq!(worker_pool.get_num_active(&key1), 0);
    assert_eq!(worker_pool.get_num_active(&key2), 0);

    manager.evict_workers(&worker_metrics);

    // Only w1 and w4 should be alive.
    assert_eq!(worker_pool.get_idle_workers(), ids(&[&w1, &w4]));
    assert_eq!(worker_pool.get_num_active(&key1), 0);
    assert_eq!(worker_pool.get_num_active(&key2), 0);
    assert_eq!(
        borrow(&worker_pool, &key1).get_worker_id(),
        w1.get_worker_id()
    );
    assert_eq!(
        borrow(&worker_pool, &key2).get_worker_id(),
        w4.get_worker_id()
    );
    assert!(w1.get_status().is_valid());
    assert_eq!(status(&w2), Status::KilledDueToMemoryPressure);
    assert_eq!(status(&w3), Status::KilledDueToMemoryPressure);
    assert!(w4.get_status().is_valid());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:386 WorkerLifecycleManagerTest.testGetEvictionCandidates_testDoomedWorkers
#[test]
fn test_get_eviction_candidates_test_doomed_workers() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 2)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w1 = borrow(&worker_pool, &key);
    let w2 = borrow(&worker_pool, &key);
    let worker_metrics = vec![
        create_worker_metric(&w1, PROCESS_ID_1, 2000),
        create_worker_metric(&w2, PROCESS_ID_2, 2000),
    ];
    let manager = manager(&worker_pool, &options(1, true, false));

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 2);

    manager.evict_workers(&worker_metrics);

    assert_eq!(status(&w1), Status::PendingKillDueToMemoryPressure);
    assert_eq!(status(&w2), Status::PendingKillDueToMemoryPressure);

    // Return only one worker.
    worker_pool.return_worker(&key, w1.clone());
    // w1 gets destroyed when it is returned, so there are 0 idle workers.
    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 1);
    // Since w1 is already returned, it is killed on return.
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
    // Since w2 is still active, it is marked to be killed which will happen when it is returned.
    assert_eq!(status(&w2), Status::PendingKillDueToMemoryPressure);

    // Return the remaining worker.
    worker_pool.return_worker(&key, w2.clone());
    assert_eq!(status(&w2), Status::KilledDueToMemoryPressure);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:430 WorkerLifecycleManagerTest.testGetEvictionCandidates_testDoomedAndIdleWorkers
#[test]
fn test_get_eviction_candidates_test_doomed_and_idle_workers() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[(DUMMY_MNEMONIC, 5)], &[]);
    let key = key(&f, DUMMY_MNEMONIC, false, false);
    let w: Vec<Arc<dyn Worker>> = (0..5).map(|_| borrow(&worker_pool, &key)).collect();
    worker_pool.return_worker(&key, w[0].clone());
    worker_pool.return_worker(&key, w[1].clone());
    let worker_metrics = vec![
        create_worker_metric(&w[0], PROCESS_ID_1, 2000),
        create_worker_metric(&w[1], PROCESS_ID_2, 1000),
        create_worker_metric(&w[2], PROCESS_ID_3, 4000),
        create_worker_metric(&w[3], PROCESS_ID_4, 5000),
        create_worker_metric(&w[4], PROCESS_ID_5, 1000),
    ];
    let manager = manager(&worker_pool, &options(2, true, false));

    assert_eq!(worker_pool.get_idle_workers().len(), 2);
    assert_eq!(worker_pool.get_num_active(&key), 3);
    for wi in &w {
        assert!(wi.get_status().is_valid());
    }

    manager.evict_workers(&worker_metrics);

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 3);
    // w1 and w2 are killed immediately.
    assert_eq!(status(&w[0]), Status::KilledDueToMemoryPressure);
    assert_eq!(status(&w[1]), Status::KilledDueToMemoryPressure);
    // w3 and w4 are killed only when returned.
    assert_eq!(status(&w[2]), Status::PendingKillDueToMemoryPressure);
    assert_eq!(status(&w[3]), Status::PendingKillDueToMemoryPressure);
    assert!(w[4].get_status().is_valid());
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:481 WorkerLifecycleManagerTest.evictWorkers_testMultiplexWorkers
#[test]
fn evict_workers_test_multiplex_workers() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[], &[(DUMMY_MNEMONIC, 2)]);
    let key = key(&f, DUMMY_MNEMONIC, true, false);
    let w1 = borrow(&worker_pool, &key);
    let w2 = borrow(&worker_pool, &key);
    // Multiplex workers should share the same status instance.
    assert!(Arc::ptr_eq(w1.get_status(), w2.get_status()));
    worker_pool.return_worker(&key, w1.clone());
    worker_pool.return_worker(&key, w2.clone());
    let worker_metrics = vec![create_multiplex_worker_metric(
        &[&w1, &w2],
        PROCESS_ID_1,
        4000,
    )];
    let manager = manager(&worker_pool, &options(1, false, false));

    manager.evict_workers(&worker_metrics);

    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 0);
    // Since both w1 and w2 have been returned, it is killed.
    assert_eq!(status(&w1), Status::KilledDueToMemoryPressure);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerLifecycleManagerTest.java:513 WorkerLifecycleManagerTest.evictWorkers_doomMultiplexWorker
#[test]
fn evict_workers_doom_multiplex_worker() {
    let _s = SEQUENTIAL.lock();
    let f = set_up();
    let worker_pool = pool(&f, &[], &[(DUMMY_MNEMONIC, 2)]);
    let key = key(&f, DUMMY_MNEMONIC, true, false);
    let w1 = borrow(&worker_pool, &key);
    let w2 = borrow(&worker_pool, &key);
    // Multiplex workers should share the same status instance.
    assert!(Arc::ptr_eq(w1.get_status(), w2.get_status()));
    worker_pool.return_worker(&key, w1.clone());
    let worker_metrics = vec![create_multiplex_worker_metric(
        &[&w1, &w2],
        PROCESS_ID_1,
        4000,
    )];
    let manager = manager(&worker_pool, &options(1, true, false));

    manager.evict_workers(&worker_metrics);

    // w1 should have been evicted already.
    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 1);
    // Not yet killed because w2 is still alive (and both share a WorkerProcessStatus).
    assert_eq!(status(&w1), Status::PendingKillDueToMemoryPressure);

    worker_pool.return_worker(&key, w2.clone());
    assert!(worker_pool.get_idle_workers().is_empty());
    assert_eq!(worker_pool.get_num_active(&key), 0);
    // Status is only set to killed after the last worker proxy is destroyed.
    assert_eq!(status(&w2), Status::KilledDueToMemoryPressure);
}
