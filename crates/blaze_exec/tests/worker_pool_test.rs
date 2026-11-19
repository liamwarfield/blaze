//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerPoolTest.java.

mod worker_common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use blaze_exec::actions::execution_requirements::WorkerProtocolFormat;
use blaze_exec::worker::singleplex_worker::SingleplexWorker;
use blaze_exec::worker::worker::Worker;
use blaze_exec::worker::worker_factory::WorkerFactoryApi;
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_pool::{WorkerPool, WorkerPoolConfig, WorkerPoolImpl};
use blaze_exec::worker::worker_process_status::Status;
use blaze_optdefs::misc::worker_options::MultiResourceEntry;
use blaze_vfs::{FileSystemExt, FileSystemRef};
use indexmap::IndexSet;
use parking_lot::Mutex;
use worker_common::*;

/// The Mockito spy of `WorkerFactory` in the Java test: `create` makes a `TestWorker` with the
/// next id, `validateWorker` is the status's validity, `destroyWorker` destroys; calls are counted.
struct FactoryMock {
    fs: FileSystemRef,
    worker_ids: AtomicI32,
    creates: Mutex<HashMap<String, i32>>,
    destroyed: Mutex<Vec<(String, i32)>>,
}

impl FactoryMock {
    fn create_count(&self, key: &WorkerKey) -> i32 {
        self.creates.lock().get(&id_of(key)).copied().unwrap_or(0)
    }
}

impl WorkerFactoryApi for FactoryMock {
    fn create(&self, key: &Arc<WorkerKey>) -> std::io::Result<Arc<dyn Worker>> {
        *self.creates.lock().entry(id_of(key)).or_default() += 1;
        Ok(Arc::new(SingleplexWorker::new(
            key.clone(),
            self.worker_ids.fetch_add(1, Ordering::SeqCst),
            self.fs.get_path("/workDir"),
            self.fs.get_path("/logDir"),
            Arc::new(worker_options()),
        )))
    }

    fn destroy_worker(&self, key: &WorkerKey, worker: &Arc<dyn Worker>) {
        self.destroyed
            .lock()
            .push((key.to_string(), worker.get_worker_id()));
        worker.destroy();
    }

    fn validate_worker(&self, _key: &WorkerKey, worker: &Arc<dyn Worker>) -> bool {
        worker.get_status().is_valid()
    }
}

struct Fixture {
    fs: FileSystemRef,
    factory_mock: Arc<FactoryMock>,
    worker_pool: Arc<WorkerPoolImpl>,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerPoolTest.java:63 WorkerPoolTest.setUp
fn set_up() -> Fixture {
    let fs = fs();
    let factory_mock = Arc::new(FactoryMock {
        fs: fs.clone(),
        worker_ids: AtomicI32::new(1),
        creates: Mutex::new(HashMap::new()),
        destroyed: Mutex::new(Vec::new()),
    });
    let entry = || {
        vec![MultiResourceEntry {
            key: Some("mnem".to_string()),
            value: Some(2),
        }]
    };
    let worker_pool = Arc::new(WorkerPoolImpl::new(
        factory_mock.clone(),
        &WorkerPoolConfig::new(entry(), entry()),
    ));
    Fixture {
        fs,
        factory_mock,
        worker_pool,
    }
}

fn key(fx: &Fixture, mnemonic: &str, multiplex: bool) -> Arc<WorkerKey> {
    Arc::new(create_worker_key_mnemonic(
        mnemonic, &fx.fs, multiplex, false,
    ))
}

fn ids(set: &IndexSet<i32>) -> Vec<i32> {
    let mut v: Vec<i32> = set.iter().copied().collect();
    v.sort();
    v
}

fn borrow(fx: &Fixture, key: &Arc<WorkerKey>) -> Arc<dyn Worker> {
    match fx.worker_pool.borrow_worker(key) {
        Ok(w) => w,
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn test_borrow_creates_when_needed() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);
    assert_eq!(worker1.get_worker_id(), 1);
    assert_eq!(worker2.get_worker_id(), 2);
    assert_eq!(fx.factory_mock.create_count(&worker_key), 2);
}

#[test]
fn test_borrow_reuses_when_possible() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    fx.worker_pool.return_worker(&worker_key, worker1.clone());
    let worker2 = borrow(&fx, &worker_key);
    assert!(Arc::ptr_eq(&worker1, &worker2));
    assert_eq!(fx.factory_mock.create_count(&worker_key), 1);
}

#[test]
fn test_borrow_non_specified_key() {
    let fx = set_up();
    let worker_key1 = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key1);
    let worker1a = borrow(&fx, &worker_key1);
    assert_eq!(worker1.get_worker_id(), 1);
    assert_eq!(worker1a.get_worker_id(), 2);
    let worker_key2 = key(&fx, "other", false);
    let worker2 = borrow(&fx, &worker_key2);
    assert_eq!(worker2.get_worker_id(), 3);
    assert_eq!(fx.factory_mock.create_count(&worker_key1), 2);
    assert_eq!(fx.factory_mock.create_count(&worker_key2), 1);
}

#[test]
fn test_borrow_pooled_by_key() {
    let fx = set_up();
    let worker_key1 = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key1);
    let worker1a = borrow(&fx, &worker_key1);
    assert_eq!(worker1.get_worker_id(), 1);
    assert_eq!(worker1a.get_worker_id(), 2);
    let worker_key2 = Arc::new(create_worker_key_full(
        WorkerProtocolFormat::Proto,
        &fx.fs,
        "mnem",
        false,
        false,
        false,
        &["arg1"],
    ));
    let worker2 = borrow(&fx, &worker_key2);
    assert_eq!(worker2.get_worker_id(), 3);
    assert_eq!(fx.factory_mock.create_count(&worker_key1), 2);
    assert_eq!(fx.factory_mock.create_count(&worker_key2), 1);
}

#[test]
fn test_borrow_separate_multiplex_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    assert_eq!(worker1.get_worker_id(), 1);
    fx.worker_pool.return_worker(&worker_key, worker1);

    let multiplex_key = key(&fx, "mnem", true);
    let multiplex_worker1 = borrow(&fx, &multiplex_key);
    let multiplex_worker2 = borrow(&fx, &multiplex_key);
    let worker1a = borrow(&fx, &worker_key);

    assert_eq!(multiplex_worker1.get_worker_id(), 2);
    assert_eq!(multiplex_worker2.get_worker_id(), 3);
    assert_eq!(worker1a.get_worker_id(), 1);

    assert_eq!(fx.factory_mock.create_count(&worker_key), 1);
    assert_eq!(fx.factory_mock.create_count(&multiplex_key), 2);
}

#[test]
fn test_borrow_doomed_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);

    worker1
        .get_status()
        .maybe_update_status(Status::PendingKillDueToMemoryPressure);

    assert!(!worker1.get_status().is_killed());
    assert!(!worker2.get_status().is_killed());

    fx.worker_pool.return_worker(&worker_key, worker1.clone());

    assert!(worker1.get_status().is_killed());
    assert!(!worker2.get_status().is_killed());
}

/// `TestThread.joinAndAssertState(ms)` failing with "is still alive": the thread is still
/// blocked after `ms`.
fn still_blocked_after(handle: &std::thread::JoinHandle<()>, ms: u64) -> bool {
    std::thread::sleep(Duration::from_millis(ms));
    !handle.is_finished()
}

#[test]
fn test_borrow_blocks_when_unavailable() {
    let fx = Arc::new(set_up());
    let worker_key = key(&fx, "mnem", false);
    let _unused1 = borrow(&fx, &worker_key);
    let _unused2 = borrow(&fx, &worker_key);
    let fx2 = fx.clone();
    let key2 = worker_key.clone();
    let blocked_borrow_thread = std::thread::spawn(move || {
        let _unused = borrow(&fx2, &key2);
    });

    assert!(still_blocked_after(&blocked_borrow_thread, 1000));
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 2);
    // The blocked thread is left blocked, as in Java.
    drop(blocked_borrow_thread);
}

#[test]
fn test_borrow_blocked_thread_gets_returned_worker() {
    let fx = Arc::new(set_up());
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let _unused2 = borrow(&fx, &worker_key);
    let fx2 = fx.clone();
    let key2 = worker_key.clone();
    let w1 = worker1.clone();
    let blocked_borrow_thread = std::thread::spawn(move || {
        // This blocks until worker1 returns its object.
        let worker = borrow(&fx2, &key2);
        assert!(Arc::ptr_eq(&worker, &w1));
    });

    // We want to 3rd borrow to be blocked for some time.
    std::thread::sleep(Duration::from_millis(500));
    fx.worker_pool
        .return_worker(&worker1.get_worker_key().clone(), worker1);

    blocked_borrow_thread.join().unwrap();
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 2);
}

#[test]
fn test_borrow_blocked_thread_creates_worker_when_invalidated() {
    let fx = Arc::new(set_up());
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let _unused2 = borrow(&fx, &worker_key);
    let fx2 = fx.clone();
    let key2 = worker_key.clone();
    let blocked_borrow_thread = std::thread::spawn(move || {
        let worker = borrow(&fx2, &key2);
        // Create a new worker instead.
        assert_eq!(worker.get_worker_id(), 3);
    });

    // We want to 3rd borrow to be blocked for some time.
    std::thread::sleep(Duration::from_millis(500));
    fx.worker_pool.invalidate_worker(&worker1);

    blocked_borrow_thread.join().unwrap();
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 2);
}

#[test]
fn test_borrow_blocked_thread_remains_blocked_when_invalidated_and_shrunk() {
    let fx = Arc::new(set_up());
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let _unused2 = borrow(&fx, &worker_key);
    let fx2 = fx.clone();
    let key2 = worker_key.clone();
    let blocked_borrow_thread = std::thread::spawn(move || {
        let _unused = borrow(&fx2, &key2);
    });

    // There's no need to wait here as it doesn't matter whether #invalidateObject gets called
    // before or after the 3rd #borrowObject, the pool would not have the quota and borrowing
    // will still get blocked.
    worker1
        .get_status()
        .maybe_update_status(Status::PendingKillDueToMemoryPressure);
    fx.worker_pool.invalidate_worker(&worker1);

    assert!(still_blocked_after(&blocked_borrow_thread, 1000));
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 1);
}

#[test]
fn test_evict_evicts_idle_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);
    fx.worker_pool.return_worker(&worker_key, worker1.clone());
    fx.worker_pool.return_worker(&worker_key, worker2.clone());
    let evicted = fx.worker_pool.evict_workers(
        &[worker1.get_worker_id(), worker2.get_worker_id()]
            .into_iter()
            .collect(),
    );
    assert_eq!(
        ids(&evicted),
        vec![worker1.get_worker_id(), worker2.get_worker_id()]
    );
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 0);
}

#[test]
fn test_evict_does_not_evict_active_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);
    fx.worker_pool.return_worker(&worker_key, worker1.clone());
    let evicted = fx.worker_pool.evict_workers(
        &[worker1.get_worker_id(), worker2.get_worker_id()]
            .into_iter()
            .collect(),
    );
    // Worker2 does not get evicted because it is still active.
    assert_eq!(ids(&evicted), vec![worker1.get_worker_id()]);
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 1);
}

#[test]
fn test_get_idle_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);

    assert!(fx.worker_pool.get_idle_workers().is_empty());
    fx.worker_pool.return_worker(&worker_key, worker1.clone());
    fx.worker_pool.return_worker(&worker_key, worker2.clone());

    assert_eq!(
        ids(&fx.worker_pool.get_idle_workers()),
        vec![worker1.get_worker_id(), worker2.get_worker_id()]
    );
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 0);
}

#[test]
fn test_shrinking_pool_does_not_shrink_below_one_worker() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 2);

    let worker1 = borrow(&fx, &worker_key);
    // Shrink the worker pool by 1.
    worker1
        .get_status()
        .maybe_update_status(Status::PendingKillDueToMemoryPressure);
    fx.worker_pool.return_worker(&worker_key, worker1);
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 1);

    let worker2 = borrow(&fx, &worker_key);
    // Attempt to shrink the pool again.
    worker2
        .get_status()
        .maybe_update_status(Status::PendingKillDueToMemoryPressure);
    fx.worker_pool.return_worker(&worker_key, worker2);
    // It should not be shrunk below 1.
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 1);
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 0);
}

#[test]
fn test_get_num_active() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 0);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 2);
    fx.worker_pool.return_worker(&worker_key, worker1);
    fx.worker_pool.return_worker(&worker_key, worker2);
    assert_eq!(fx.worker_pool.get_num_active(&worker_key), 0);
}

#[test]
fn test_reset_removes_previously_shrunk_values() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 2);

    let worker1 = borrow(&fx, &worker_key);
    // Shrink the worker pool by 1.
    worker1
        .get_status()
        .maybe_update_status(Status::PendingKillDueToMemoryPressure);
    fx.worker_pool.return_worker(&worker_key, worker1);
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 1);

    fx.worker_pool.reset();
    assert_eq!(fx.worker_pool.get_max_total_per_key(&worker_key), 2);
}

#[test]
fn test_close_destroys_workers() {
    let fx = set_up();
    let worker_key = key(&fx, "mnem", false);
    let worker1 = borrow(&fx, &worker_key);
    let worker2 = borrow(&fx, &worker_key);
    fx.worker_pool.return_worker(&worker_key, worker1.clone());
    fx.worker_pool.return_worker(&worker_key, worker2.clone());
    fx.worker_pool.close();
    let destroyed = fx.factory_mock.destroyed.lock().clone();
    assert!(destroyed.contains(&(worker_key.to_string(), worker1.get_worker_id())));
    assert!(destroyed.contains(&(worker_key.to_string(), worker2.get_worker_id())));
}

/// The identity of a key for Mockito's `verify(factoryMock).create(key)` (`equals`).
fn id_of(key: &WorkerKey) -> String {
    format!("{}#{}#{}", key.get_mnemonic(), key.hash_code(), key)
}
