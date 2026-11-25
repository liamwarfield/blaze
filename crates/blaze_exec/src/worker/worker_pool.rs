//! Ports of `lib/worker/WorkerPool.java`, `WorkerPoolConfig.java` and `WorkerPoolImpl.java`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::misc::worker_options::MultiResourceEntry;
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Condvar, Mutex};

use super::worker::Worker;
use super::worker_factory::WorkerFactoryApi;
use super::worker_key::WorkerKey;
use super::worker_process_status::Status;

/// What `borrowWorker` throws: `IOException` or `InterruptedException`.
#[derive(Debug)]
pub enum BorrowError {
    Io(std::io::Error),
    Interrupted,
}

/// A worker pool that spawns multiple workers and delegates work to them. Allows separate
/// configuration for singleplex and multiplex workers. While the configuration is per mnemonic,
/// the actual pools need to be per WorkerKey, as different WorkerKeys may imply different process
/// startup options.
// Java: com/google/devtools/build/lib/worker/WorkerPool.java:32 WorkerPool
pub trait WorkerPool: Send + Sync {
    /// Returns the current maximum number of workers allowed in the pool.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:39 WorkerPool.getMaxTotalPerKey
    fn get_max_total_per_key(&self, key: &Arc<WorkerKey>) -> i32;

    /// Returns the number of active workers.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:46 WorkerPool.getNumActive
    fn get_num_active(&self, key: &Arc<WorkerKey>) -> i32;

    /// Returns whether there is quota available to create or use an existing worker.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:57 WorkerPool.hasAvailableQuota
    fn has_available_quota(&self, key: &Arc<WorkerKey>) -> bool;

    /// Evicts specified workers from the pool, destroying them. Returns the worker ids that were
    /// successfully evicted.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:67 WorkerPool.evictWorkers
    fn evict_workers(&self, worker_ids_to_evict: &IndexSet<i32>) -> IndexSet<i32>;

    /// Returns the idle workers in the pool (note that these workers can still become active).
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:71 WorkerPool.getIdleWorkers
    fn get_idle_workers(&self) -> IndexSet<i32>;

    /// Borrows a persistent worker from the pool, creating if necessary and blocking if
    /// unavailable.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:78 WorkerPool.borrowWorker
    fn borrow_worker(&self, key: &Arc<WorkerKey>) -> Result<Arc<dyn Worker>, BorrowError>;

    /// Returns an active worker back to the pool.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:86 WorkerPool.returnWorker
    fn return_worker(&self, key: &Arc<WorkerKey>, worker: Arc<dyn Worker>);

    /// Invalidates the worker, thus destroying it.
    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:93 WorkerPool.invalidateWorker
    fn invalidate_worker(&self, worker: &Arc<dyn Worker>);

    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:95 WorkerPool.reset
    fn reset(&self);

    // Java: com/google/devtools/build/lib/worker/WorkerPool.java:97 WorkerPool.close
    fn close(&self);
}

/// Describes the configuration of worker pool, e.g. number of maximal instances and priority of
/// the workers.
// Java: com/google/devtools/build/lib/worker/WorkerPoolConfig.java:24 WorkerPoolConfig
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerPoolConfig {
    worker_max_instances: Vec<MultiResourceEntry>,
    worker_max_multiplex_instances: Vec<MultiResourceEntry>,
}

impl WorkerPoolConfig {
    // Java: com/google/devtools/build/lib/worker/WorkerPoolConfig.java:28 WorkerPoolConfig.<init>
    pub fn new(
        worker_max_instances: Vec<MultiResourceEntry>,
        worker_max_multiplex_instances: Vec<MultiResourceEntry>,
    ) -> WorkerPoolConfig {
        WorkerPoolConfig {
            worker_max_instances,
            worker_max_multiplex_instances,
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolConfig.java:35 WorkerPoolConfig.getWorkerMaxInstances
    pub fn get_worker_max_instances(&self) -> &[MultiResourceEntry] {
        &self.worker_max_instances
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolConfig.java:39 WorkerPoolConfig.getWorkerMaxMultiplexInstances
    pub fn get_worker_max_multiplex_instances(&self) -> &[MultiResourceEntry] {
        &self.worker_max_multiplex_instances
    }
}

/// Unless otherwise specified, the max number of workers per WorkerKey.
// Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:53 WorkerPoolImpl.DEFAULT_MAX_SINGLEPLEX_WORKERS
const DEFAULT_MAX_SINGLEPLEX_WORKERS: i32 = 4;

/// Unless otherwise specified, the max number of multiplex workers per WorkerKey.
// Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:56 WorkerPoolImpl.DEFAULT_MAX_MULTIPLEX_WORKERS
const DEFAULT_MAX_MULTIPLEX_WORKERS: i32 = 8;

/// Implementation of the WorkerPool.
///
/// This implementation flattens this to have a single `WorkerKeyPool` for each worker key (we
/// don't need the indirection in referencing both mnemonic and worker key since the mnemonic is
/// part of the key). Additionally, it bakes in pool shrinking logic so that we can handle
/// concurrent calls.
// Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:48 WorkerPoolImpl
pub struct WorkerPoolImpl {
    factory: Arc<dyn WorkerFactoryApi>,
    singleplex_max_instances: IndexMap<String, i32>,
    multiplex_max_instances: IndexMap<String, i32>,
    /// PORT: java-hash-order. Java's `ConcurrentHashMap<WorkerKey, ...>` order reaches only
    /// `close()` (the order workers are destroyed in, and the verbose "Destroying ..." messages)
    /// and the eviction sets; the pools are kept in creation order.
    pools: Mutex<IndexMap<WorkerKey, Arc<WorkerKeyPool>>>,
}

impl WorkerPoolImpl {
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:66 WorkerPoolImpl.<init>
    pub fn new(factory: Arc<dyn WorkerFactoryApi>, config: &WorkerPoolConfig) -> WorkerPoolImpl {
        WorkerPoolImpl {
            factory,
            singleplex_max_instances: WorkerPoolImpl::get_max_instances(
                config.get_worker_max_instances(),
                DEFAULT_MAX_SINGLEPLEX_WORKERS,
            ),
            multiplex_max_instances: WorkerPoolImpl::get_max_instances(
                config.get_worker_max_multiplex_instances(),
                DEFAULT_MAX_MULTIPLEX_WORKERS,
            ),
            pools: Mutex::new(IndexMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:74 WorkerPoolImpl.getMaxInstances
    fn get_max_instances(
        max_instances: &[MultiResourceEntry],
        default_max_workers: i32,
    ) -> IndexMap<String, i32> {
        let mut new_config_builder: IndexMap<String, i32> = IndexMap::new();
        for entry in max_instances {
            if let Some(value) = entry.value {
                // LinkedHashMap.put: first position, last value. A null key is kept as Java's
                // null key, which no mnemonic equals.
                if let Some(key) = &entry.key {
                    new_config_builder.insert(key.clone(), value);
                }
            } else if let Some(key) = &entry.key {
                new_config_builder.insert(key.clone(), default_max_workers);
            }
        }
        new_config_builder
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:157 WorkerPoolImpl.getPool
    fn get_pool(&self, key: &Arc<WorkerKey>) -> Arc<WorkerKeyPool> {
        let mut pools = self.pools.lock();
        if let Some(p) = pools.get(&**key) {
            return p.clone();
        }
        let pool = Arc::new(self.create_pool(key));
        pools.insert((**key).clone(), pool.clone());
        pool
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:161 WorkerPoolImpl.createPool
    fn create_pool(&self, key: &Arc<WorkerKey>) -> WorkerKeyPool {
        if key.is_multiplex() {
            return WorkerKeyPool::new(
                key.clone(),
                WorkerPoolImpl::get_max_worker_instances(
                    &self.multiplex_max_instances,
                    key.get_mnemonic(),
                    DEFAULT_MAX_MULTIPLEX_WORKERS,
                ),
                self.factory.clone(),
            );
        }
        WorkerKeyPool::new(
            key.clone(),
            WorkerPoolImpl::get_max_worker_instances(
                &self.singleplex_max_instances,
                key.get_mnemonic(),
                DEFAULT_MAX_SINGLEPLEX_WORKERS,
            ),
            self.factory.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:174 WorkerPoolImpl.getMaxWorkerInstances
    fn get_max_worker_instances(
        max_instances: &IndexMap<String, i32>,
        mnemonic: &str,
        default_max_instances: i32,
    ) -> i32 {
        if let Some(v) = max_instances.get(mnemonic) {
            return *v;
        }
        // Empty-string contains the user-specified worker maximum instances.
        max_instances
            .get("")
            .copied()
            .unwrap_or(default_max_instances)
    }

    fn all_pools(&self) -> Vec<Arc<WorkerKeyPool>> {
        self.pools.lock().values().cloned().collect()
    }
}

impl WorkerPool for WorkerPoolImpl {
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:88 WorkerPoolImpl.getMaxTotalPerKey
    fn get_max_total_per_key(&self, key: &Arc<WorkerKey>) -> i32 {
        self.get_pool(key).get_effective_max()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:93 WorkerPoolImpl.getNumActive
    fn get_num_active(&self, key: &Arc<WorkerKey>) -> i32 {
        self.get_pool(key).get_num_active()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:98 WorkerPoolImpl.hasAvailableQuota
    fn has_available_quota(&self, key: &Arc<WorkerKey>) -> bool {
        self.get_pool(key).has_available_quota()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:103 WorkerPoolImpl.evictWorkers
    fn evict_workers(&self, worker_ids_to_evict: &IndexSet<i32>) -> IndexSet<i32> {
        // TODO: Without having the Worker objects themselves, we can't directly pass the worker
        // to the pool to be evicted.
        let mut evicted_worker_ids: HashSet<i32> = HashSet::new();
        for p in self.all_pools() {
            evicted_worker_ids.extend(p.evict_workers(worker_ids_to_evict));
        }
        worker_ids_to_evict
            .iter()
            .filter(|id| evicted_worker_ids.contains(id))
            .copied()
            .collect()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:115 WorkerPoolImpl.getIdleWorkers
    fn get_idle_workers(&self) -> IndexSet<i32> {
        let mut result = IndexSet::new();
        for p in self.all_pools() {
            result.extend(p.get_idle_workers());
        }
        result
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:122 WorkerPoolImpl.borrowWorker
    fn borrow_worker(&self, key: &Arc<WorkerKey>) -> Result<Arc<dyn Worker>, BorrowError> {
        self.get_pool(key).borrow_worker(key)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:127 WorkerPoolImpl.returnWorker
    fn return_worker(&self, key: &Arc<WorkerKey>, obj: Arc<dyn Worker>) {
        self.get_pool(key).return_worker(key, obj);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:132 WorkerPoolImpl.invalidateWorker
    fn invalidate_worker(&self, worker: &Arc<dyn Worker>) {
        self.get_pool(worker.get_worker_key())
            .invalidate_worker(worker, worker.get_status().is_pending_eviction());
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:137 WorkerPoolImpl.reset
    fn reset(&self) {
        for pool in self.all_pools() {
            pool.reset();
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:144 WorkerPoolImpl.close
    fn close(&self) {
        for pool in self.all_pools() {
            pool.close();
        }
    }
}

/// Used to pass workers from threads that are returning the worker to the pool, bypassing the
/// queue.
// Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:410 WorkerPoolImpl.PendingWorkerRequest
struct PendingWorkerRequest {
    /// `(signalled, worker)`.
    state: Mutex<(bool, Option<Arc<dyn Worker>>)>,
    cv: Condvar,
}

impl PendingWorkerRequest {
    fn new() -> PendingWorkerRequest {
        PendingWorkerRequest {
            state: Mutex::new((false, None)),
            cv: Condvar::new(),
        }
    }

    /// Returns a worker instance that has been freed, or `None` if the worker needs to be
    /// created.
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:416 WorkerPoolImpl.PendingWorkerRequest.await
    fn await_worker(&self) -> Result<Option<Arc<dyn Worker>>, BorrowError> {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(BorrowError::Interrupted);
        }
        let mut st = self.state.lock();
        loop {
            if st.0 {
                return Ok(st.1.take());
            }
            // PORT: the interrupt flag is polled every 10ms while waiting.
            self.cv.wait_for(&mut st, Duration::from_millis(10));
            if !st.0 && blaze_shell::thread_interrupt::interrupted() {
                return Err(BorrowError::Interrupted);
            }
        }
    }

    /// Signals to the thread #await(ing) to proceed. When calling this, the
    /// `WorkerKeyPool.acquired` quota associated to this worker should not be released because
    /// that allows for race conditions with other threads attempting to borrow from the pool.
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:427 WorkerPoolImpl.PendingWorkerRequest.signal
    fn signal(&self, worker: Option<Arc<dyn Worker>>) {
        let mut st = self.state.lock();
        st.1 = worker;
        st.0 = true;
        self.cv.notify_all();
    }
}

/// The fields `WorkerKeyPool` guards with its monitor.
struct KeyPoolState {
    /// The number of workers in use.
    acquired: i32,
    /// The number by which the overall quota is shrunk by.
    shrunk: i32,
    idle_workers: VecDeque<Arc<dyn Worker>>,
    /// The waiting queue is meant to provide fairness in borrowing from the pool (first come
    /// first serve), any freeing up of quota (either through returning or invalidating a worker)
    /// will service requests this queue first.
    waiting_queue: VecDeque<Arc<PendingWorkerRequest>>,
    /// Workers by id (Java's `HashSet<Worker>` with identity equality).
    active_set: HashMap<i32, Arc<dyn Worker>>,
}

/// Actual pool implementation that handles the borrowing, returning and invalidation of workers
/// of a single worker key.
// Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:209 WorkerPoolImpl.WorkerKeyPool
struct WorkerKeyPool {
    /// Read only by Java's logging, which is skipped.
    #[allow(dead_code)]
    key: Arc<WorkerKey>,
    max: i32,
    factory: Arc<dyn WorkerFactoryApi>,
    state: Mutex<KeyPoolState>,
}

impl WorkerKeyPool {
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:238 WorkerPoolImpl.WorkerKeyPool.<init>
    fn new(key: Arc<WorkerKey>, max: i32, factory: Arc<dyn WorkerFactoryApi>) -> WorkerKeyPool {
        WorkerKeyPool {
            key,
            max,
            factory,
            state: Mutex::new(KeyPoolState {
                acquired: 0,
                shrunk: 0,
                idle_workers: VecDeque::new(),
                waiting_queue: VecDeque::new(),
                active_set: HashMap::new(),
            }),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:243 WorkerPoolImpl.WorkerKeyPool.evictWorkers
    fn evict_workers(&self, worker_ids_to_evict: &IndexSet<i32>) -> HashSet<i32> {
        let mut st = self.state.lock();
        let mut evicted_worker_ids = HashSet::new();
        let idle: Vec<Arc<dyn Worker>> = st.idle_workers.iter().cloned().collect();
        for worker in idle {
            if worker_ids_to_evict.contains(&worker.get_worker_id()) {
                evicted_worker_ids.insert(worker.get_worker_id());
                worker
                    .get_status()
                    .maybe_update_status(Status::PendingKillDueToMemoryPressure);
                self.invalidate_worker_locked(
                    &mut st, &worker, /* should_shrink_pool= */ true,
                );
                st.idle_workers
                    .retain(|w| w.get_worker_id() != worker.get_worker_id());
                // Evicted %s worker (id %d, key hash %d). (logged)
            }
            // TODO(b/323880131): Move postponing of invalidation from {@code
            // WorkerLifecycleManager} here, since all we need to do is to update the statuses. We
            // keep it like this for now to preserve the existing behavior.
        }
        evicted_worker_ids
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:265 WorkerPoolImpl.WorkerKeyPool.getNumActive
    fn get_num_active(&self) -> i32 {
        self.state.lock().acquired
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:269 WorkerPoolImpl.WorkerKeyPool.getEffectiveMax
    fn get_effective_max(&self) -> i32 {
        self.max - self.state.lock().shrunk
    }

    fn effective_max_locked(&self, st: &KeyPoolState) -> i32 {
        self.max - st.shrunk
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:273 WorkerPoolImpl.WorkerKeyPool.hasAvailableQuota
    fn has_available_quota(&self) -> bool {
        let st = self.state.lock();
        self.has_available_quota_locked(&st)
    }

    fn has_available_quota_locked(&self, st: &KeyPoolState) -> bool {
        self.effective_max_locked(st) - st.acquired > 0
    }

    /// Callers should atomically check to confirm that workers are available before calling
    /// this method or risk being blocked waiting for a worker to be available.
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:279 WorkerPoolImpl.WorkerKeyPool.borrowWorker
    fn borrow_worker(&self, key: &Arc<WorkerKey>) -> Result<Arc<dyn Worker>, BorrowError> {
        let mut worker: Option<Arc<dyn Worker>> = None;
        let mut pending_req: Option<Arc<PendingWorkerRequest>> = None;
        // We don't want to hold the lock on the pool while creating or waiting for a worker or
        // quota to be available.
        {
            let mut st = self.state.lock();
            while let Some(candidate) = st.idle_workers.back().cloned() {
                // LIFO: It's better to re-use a worker as often as possible and keep it hot, in
                // order to profit from JIT optimizations as much as possible.
                // We need to validate with the passed in `key` rather than
                // `worker.getWorkerKey()` because the former can contain a different combined
                // files hash if the files changed.
                if self.factory.validate_worker(key, &candidate) {
                    st.acquired += 1;
                    st.idle_workers
                        .retain(|w| w.get_worker_id() != candidate.get_worker_id());
                    worker = Some(candidate);
                    break;
                }
                self.invalidate_worker_locked(
                    &mut st, &candidate, /* should_shrink_pool= */ false,
                );
            }

            if worker.is_none() {
                // If we were unable to get an idle worker, then either create or wait for one.
                if self.has_available_quota_locked(&st) {
                    // No idle workers, but we have space to create another.
                    st.acquired += 1;
                } else {
                    let req = Arc::new(PendingWorkerRequest::new());
                    st.waiting_queue.push_back(req.clone());
                    pending_req = Some(req);
                }
            }
        }

        if let Some(pending_req) = pending_req {
            // Wait until the resources are available. We cannot do this while synchronized
            // because that would deadlock by blocking other threads from returning and thus
            // freeing up quota for this to proceed.
            worker = pending_req.await_worker()?;
        }

        let worker = match worker {
            Some(w) => w,
            None => self.factory.create(key).map_err(BorrowError::Io)?,
        };

        self.state
            .lock()
            .active_set
            .insert(worker.get_worker_id(), worker.clone());
        Ok(worker)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:330 WorkerPoolImpl.WorkerKeyPool.returnWorker
    fn return_worker(&self, key: &Arc<WorkerKey>, worker: Arc<dyn Worker>) {
        let mut st = self.state.lock();
        if !self.factory.validate_worker(key, &worker) {
            self.invalidate_worker_locked(&mut st, &worker, true);
            return;
        }

        if st.active_set.remove(&worker.get_worker_id()).is_none() {
            panic!(
                "IllegalStateException: Worker {} (id {}) is not in the active set",
                worker.get_worker_key().get_mnemonic(),
                worker.get_worker_id()
            );
        }

        let pending_req = st.waiting_queue.pop_front();
        if let Some(pending_req) = pending_req {
            // Pass the worker directly to the waiting thread.
            pending_req.signal(Some(worker));
        } else {
            st.acquired -= 1;
            st.idle_workers.push_back(worker);
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:355 WorkerPoolImpl.WorkerKeyPool.invalidateWorker
    fn invalidate_worker(&self, worker: &Arc<dyn Worker>, should_shrink_pool: bool) {
        let mut st = self.state.lock();
        self.invalidate_worker_locked(&mut st, worker, should_shrink_pool);
    }

    fn invalidate_worker_locked(
        &self,
        st: &mut KeyPoolState,
        worker: &Arc<dyn Worker>,
        should_shrink_pool: bool,
    ) {
        self.factory.destroy_worker(worker.get_worker_key(), worker);

        let id = worker.get_worker_id();
        if st.idle_workers.iter().any(|w| w.get_worker_id() == id) {
            st.idle_workers.retain(|w| w.get_worker_id() != id);
            return;
        }

        // If it isn't idle, then we're destroying an active worker.
        if st.active_set.remove(&id).is_none() {
            panic!(
                "IllegalStateException: Worker {} (id {}) is not in the active set",
                worker.get_worker_key().get_mnemonic(),
                id
            );
        }

        // We don't want to shrink the pool to 0.
        if should_shrink_pool && self.effective_max_locked(st) > 1 {
            // When shrinking, there is no effective change in the availability, so there is no
            // need to signal a waiting thread to proceed.
            st.acquired -= 1;
            st.shrunk += 1;
            return;
        }

        let pending_req = st.waiting_queue.pop_front();
        match pending_req {
            // Since there is no pending request, we free up this quota.
            None => st.acquired -= 1,
            // Since there is a pending request, hold onto this quota (and do not decrement
            // acquired) so that other threads aren't able to borrow before this pending request
            // (thus creating a race condition).
            Some(pending_req) => pending_req.signal(None),
        }
    }

    /// It is not important that we synchronize here, the `WorkerLifecycleManager` takes the idle
    /// workers and figures out (non-atomically with respect to this instance) which workers to
    /// evict.
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:398 WorkerPoolImpl.WorkerKeyPool.getIdleWorkers
    fn get_idle_workers(&self) -> Vec<i32> {
        self.state
            .lock()
            .idle_workers
            .iter()
            .map(|w| w.get_worker_id())
            .collect()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:402 WorkerPoolImpl.WorkerKeyPool.reset
    fn reset(&self) {
        self.state.lock().shrunk = 0;
        // PORT: skipped, logging ("clearing shrunk values for %s (key hash %d) worker pool").
    }

    /// Destroys all workers created in this pool.
    // Java: com/google/devtools/build/lib/worker/WorkerPoolImpl.java:410 WorkerPoolImpl.WorkerKeyPool.close
    fn close(&self) {
        let st = self.state.lock();
        for worker in &st.idle_workers {
            self.factory.destroy_worker(worker.get_worker_key(), worker);
        }
        for worker in st.active_set.values() {
            // Interrupting and shutting down active worker %s (id %d) due to pool shutdown
            // (logged).
            self.factory.destroy_worker(worker.get_worker_key(), worker);
        }
    }
}
