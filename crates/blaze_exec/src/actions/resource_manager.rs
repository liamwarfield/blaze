//! Port of `lib/actions/ResourceManager.java` (+ `LocalHostCapacity.getLocalHostCapacity` as a
//! `ResourceSet`, and `ExecutionTool.configureResourceManager`).
//!
//! Used to keep track of resources consumed by the Blaze action execution threads and throttle
//! them when necessary.
//!
//! Threads which are known to consume a significant amount of resources should call
//! [`ResourceManager::acquire_resources`]. This method will check whether requested resources are
//! available and will either mark them as used and allow the thread to proceed or will block the
//! thread until requested resources will become available. When the thread completes its task, it
//! must release allocated resources by closing (dropping) the returned [`ResourceHandle`].
//!
//! The resource manager also allows a slight overallocation of the resources to account for the
//! fact that requested resources are usually estimated using a pessimistic approximation. It also
//! guarantees that at least one thread will always be able to acquire any amount of requested
//! resources (even if it is greater than amount of available resources). Therefore, assuming that
//! threads correctly release acquired resources, Blaze will never be fully blocked.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, local_execution};
use indexmap::IndexMap;
use parking_lot::{Condvar, Mutex};

use crate::actions::{ActionExecutionMetadata, ExecException, ResourceSet, UserExecException};
use crate::worker::worker::Worker;
use crate::worker::worker_pool::{BorrowError, WorkerPool};
use crate::worker::worker_process_status::Status as WorkerStatus;

/// What acquiring resources throws besides `ExecException`: the `IOException` of borrowing a
/// worker from the pool.
#[derive(Debug)]
pub enum AcquireResourcesError {
    Exec(ExecException),
    Io(std::io::Error),
}

impl From<ExecException> for AcquireResourcesError {
    fn from(e: ExecException) -> Self {
        AcquireResourcesError::Exec(e)
    }
}

impl From<BorrowError> for AcquireResourcesError {
    fn from(e: BorrowError) -> Self {
        match e {
            BorrowError::Io(e) => AcquireResourcesError::Io(e),
            BorrowError::Interrupted => AcquireResourcesError::Exec(ExecException::Interrupted),
        }
    }
}

impl AcquireResourcesError {
    /// For callers whose resource sets have no worker key (no `IOException` is possible): the
    /// `IOException` as `AbstractSpawnStrategy` reports it.
    fn into_exec(self) -> ExecException {
        match self {
            AcquireResourcesError::Exec(e) => e,
            AcquireResourcesError::Io(e) => {
                crate::exec::abstract_spawn_strategy::exec_io_exception(&e)
            }
        }
    }
}

/// The worker a [`ResourceLatch`] hands over (`ResourceLatch.worker`), shared between the waiting
/// thread and the thread that grants its request.
#[derive(Clone, Default)]
struct WorkerSlot(Arc<Mutex<Option<Arc<dyn Worker>>>>);

impl std::fmt::Debug for WorkerSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &*self.0.lock() {
            Some(w) => write!(f, "Some({w})"),
            None => f.write_str("None"),
        }
    }
}

/// Why a worker is invalidated (`ResourceHandle.invalidateAndClose(Exception e)`).
#[derive(Debug, Clone)]
pub enum InvalidateCause {
    /// `InterruptedException`.
    Interrupted,
    /// `IOException`.
    Io,
    /// `UserExecException`: its failure detail's worker code, if it has a worker detail.
    UserExec(Option<blaze_protos::failure_details::worker::Code>),
    /// Any other exception.
    Other,
}

/// Defines the possible priorities of resources. The earlier elements in this enum will get first
/// chance at grabbing resources.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:165 ResourceManager.ResourcePriority
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourcePriority {
    /// Local execution not under dynamic execution.
    Local,
    DynamicWorker,
    DynamicStandalone,
}

// Allocated resources are allowed to go "negative", but at least MIN_NECESSARY_RATIO portion of
// each resource should be available. Please note that this value is purely empirical - we assume
// that generally requested resources are somewhat pessimistic and thread would end up using less
// than requested amount.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:196 ResourceManager.DEFAULT_MIN_NECESSARY_RATIO
const DEFAULT_MIN_NECESSARY_RATIO: f64 = 1.0;
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:199 ResourceManager.MAX_ACTIONS_PER_CPU
const MAX_ACTIONS_PER_CPU: i32 = 3;

/// `MIN_NECESSARY_RATIO.getOrDefault(key, DEFAULT_MIN_NECESSARY_RATIO)`.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:197 ResourceManager.MIN_NECESSARY_RATIO
fn min_necessary_ratio(key: &str) -> f64 {
    if key == ResourceSet::CPU {
        0.6
    } else {
        DEFAULT_MIN_NECESSARY_RATIO
    }
}

/// Generates the ids for requests.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:352 ResourceManager.requestIdGenerator
static REQUEST_ID_GENERATOR: AtomicI32 = AtomicI32::new(0);

thread_local! {
    /// `ResourceManager.threadLocked`, one flag per manager (keyed by its address).
    static THREAD_LOCKED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// Request with the information of resource acquiring.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:355 ResourceManager.ResourceRequest
#[derive(Debug, Clone)]
pub struct ResourceRequest {
    /// `getOwner()`, as the text Java's `%s` would print in precondition messages.
    // PORT: Java keeps the owner itself; only its description is needed here.
    owner: String,
    resource_set: ResourceSet,
    priority: ResourcePriority,
    id: i32,
}

impl ResourceRequest {
    /// `getResourceSet()`.
    pub fn get_resource_set(&self) -> &ResourceSet {
        &self.resource_set
    }
    /// `getPriority()`.
    pub fn get_priority(&self) -> ResourcePriority {
        self.priority
    }
    /// `getId()`.
    pub fn get_id(&self) -> i32 {
        self.id
    }
}

/// `java.util.concurrent.CountDownLatch` with a count of 1, awaited interruptibly (the thread
/// interrupt of `blaze_shell::thread_interrupt`).
#[derive(Debug)]
struct CountDownLatch {
    count: Mutex<i32>,
    cv: Condvar,
}

impl CountDownLatch {
    fn new(count: i32) -> CountDownLatch {
        CountDownLatch {
            count: Mutex::new(count),
            cv: Condvar::new(),
        }
    }

    fn count_down(&self) {
        let mut c = self.count.lock();
        if *c > 0 {
            *c -= 1;
            if *c == 0 {
                self.cv.notify_all();
            }
        }
    }

    fn get_count(&self) -> i32 {
        *self.count.lock()
    }

    /// `await()`: `Err` when the thread is (or gets) interrupted; the flag is cleared.
    // PORT: the interrupt flag is polled every 10ms while waiting.
    fn await_interruptibly(&self) -> Result<(), ()> {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(());
        }
        let mut c = self.count.lock();
        loop {
            if *c == 0 {
                return Ok(());
            }
            self.cv.wait_for(&mut c, Duration::from_millis(10));
            if *c == 0 {
                return Ok(());
            }
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(());
            }
        }
    }
}

/// Latch which indicates the availability of resources.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:725 ResourceManager.ResourceLatch
#[derive(Debug, Clone)]
struct ResourceLatch {
    latch: Option<Arc<CountDownLatch>>,
    worker: WorkerSlot,
}

/// Pair of requested resources and latch represented it for waiting.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:202 ResourceManager.WaitingRequest
#[derive(Debug)]
struct WaitingRequest {
    resource_request: ResourceRequest,
    resource_latch: ResourceLatch,
}

/// The state Java guards with the manager's monitor.
// PORT: `window_size` is only read by the unported CPU-load window updater.
#[allow(dead_code)]
#[derive(Debug)]
struct State {
    allow_one_action_on_resource_unavailable: bool,
    // Lists of blocked threads. Associated CountDownLatch object will always be initialized to 1
    // during creation in the acquire() method.
    local_requests: VecDeque<WaitingRequest>,
    dynamic_worker_requests: VecDeque<WaitingRequest>,
    dynamic_standalone_requests: VecDeque<WaitingRequest>,
    // The total amount of available for Bazel resources on the local host. Must be set by an
    // explicit call to setAvailableResources(), often using
    // LocalHostCapacity.getLocalHostCapacity() as an argument.
    available_resources: Option<ResourceSet>,
    // Used amount of resources. Corresponds to the resource definition in the ResourceSet class.
    used_resources: HashMap<String, f64>,
    // Used local test count. Corresponds to the local test count definition in the ResourceSet
    // class.
    used_local_test_count: i32,
    // Enables experimental action scheduling using CPU load of a machine.
    cpu_load_scheduling: bool,
    // The size of window for running actions.
    window_size: Duration,
    // Estimation of CPU usage by actions started during the window.
    window_estimation_cpu: f64,
    // Set of request ids which resource acquiring started during the window.
    window_request_ids: HashSet<i32>,
    // Total number of actions running locally.
    running_actions: i32,
}

/// Schedules local execution against the available resources (`--local_resources`).
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:80 ResourceManager
pub struct ResourceManager {
    state: Mutex<State>,
    /// `workerPool`.
    worker_pool: parking_lot::RwLock<Option<Arc<dyn WorkerPool>>>,
}

impl std::fmt::Debug for ResourceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceManager")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// A handle returned by [`ResourceManager::acquire_resources`] that must be closed (or dropped)
/// in order to free the resources again.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:90 ResourceManager.ResourceHandle
pub struct ResourceHandle {
    manager: Option<Arc<ResourceManager>>,
    request: Option<ResourceRequest>,
    worker: Option<Arc<dyn Worker>>,
}

impl std::fmt::Debug for ResourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceHandle")
            .field("request", &self.request)
            .field("worker", &self.worker.as_ref().map(|w| w.to_string()))
            .finish()
    }
}

impl ResourceHandle {
    /// `getRequest()`.
    pub fn get_request(&self) -> Option<&ResourceRequest> {
        self.request.as_ref()
    }

    /// `getWorker()`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:104 ResourceManager.ResourceHandle.getWorker
    pub fn get_worker(&self) -> Option<&Arc<dyn Worker>> {
        self.worker.as_ref()
    }

    /// Closing the ResourceHandle releases the resources associated with it.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:115 ResourceManager.ResourceHandle.close
    pub fn close(mut self) -> Result<(), ExecException> {
        let worker = self.worker.take();
        match (self.manager.take(), self.request.take()) {
            (Some(manager), Some(request)) => {
                manager.release_resources_with_worker(&request, worker)
            }
            _ => Ok(()),
        }
        // PORT: skipped, profiling.
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:122 ResourceManager.ResourceHandle.invalidateAndClose
    pub fn invalidate_and_close(mut self, e: Option<InvalidateCause>) -> Result<(), ExecException> {
        let worker = self
            .worker
            .take()
            .expect("NullPointerException: invalidateAndClose without a worker");
        // If there is an exception, we need to set the kill cause before invalidating the object.
        // This ensures that the worker implementation updates their worker metrics accordingly
        // if/when it destroys itself.
        match e {
            Some(InvalidateCause::Interrupted) => {
                worker
                    .get_status()
                    .maybe_update_status(WorkerStatus::PendingKillDueToInterruptedException);
            }
            Some(InvalidateCause::Io) => {
                worker
                    .get_status()
                    .maybe_update_status(WorkerStatus::PendingKillDueToIoException);
            }
            Some(InvalidateCause::UserExec(code)) => {
                if let Some(code) = code {
                    worker.get_status().maybe_update_status_with_code(
                        WorkerStatus::PendingKillDueToUserExecException,
                        code,
                    );
                }
            }
            Some(InvalidateCause::Other) => {
                worker
                    .get_status()
                    .maybe_update_status(WorkerStatus::PendingKillDueToUserExecException);
            }
            None => {
                worker
                    .get_status()
                    .maybe_update_status(WorkerStatus::PendingKillDueToUnknown);
            }
        }

        let manager = self.manager.clone().expect("manager");
        let pool = manager
            .worker_pool
            .read()
            .clone()
            .expect("NullPointerException: workerPool");
        pool.invalidate_worker(&worker);
        self.close()
    }
}

impl Drop for ResourceHandle {
    fn drop(&mut self) {
        let worker = self.worker.take();
        if let (Some(manager), Some(request)) = (self.manager.take(), self.request.take()) {
            if std::thread::panicking() {
                let _ = manager.release_locked_and_unlock_thread(&request, worker);
            } else {
                let _ = manager.release_resources_with_worker(&request, worker);
            }
        }
    }
}

impl ResourceManager {
    /// `new ResourceManager()`, one per server (`CommandEnvironment.getLocalResourceManager`).
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:171 ResourceManager.<init>
    pub fn new() -> Arc<ResourceManager> {
        Arc::new(ResourceManager {
            state: Mutex::new(State {
                allow_one_action_on_resource_unavailable: false,
                local_requests: VecDeque::new(),
                dynamic_worker_requests: VecDeque::new(),
                dynamic_standalone_requests: VecDeque::new(),
                available_resources: None,
                used_resources: HashMap::new(),
                used_local_test_count: 0,
                cpu_load_scheduling: false,
                window_size: Duration::from_secs(5),
                window_estimation_cpu: 0.0,
                window_request_ids: HashSet::new(),
                running_actions: 0,
            }),
            worker_pool: parking_lot::RwLock::new(None),
        })
    }

    /// Sets worker pool for taking the workers. Must be called before requesting the workers.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:347 ResourceManager.setWorkerPool
    pub fn set_worker_pool(&self, worker_pool: Arc<dyn WorkerPool>) {
        *self.worker_pool.write() = Some(worker_pool);
    }

    fn worker_pool(&self) -> Arc<dyn WorkerPool> {
        self.worker_pool
            .read()
            .clone()
            .expect("NullPointerException: workerPool")
    }

    fn key(&self) -> usize {
        self as *const ResourceManager as usize
    }

    fn set_thread_locked(&self, locked: bool) {
        let key = self.key();
        THREAD_LOCKED.with(|t| {
            if locked {
                t.borrow_mut().insert(key);
            } else {
                t.borrow_mut().remove(&key);
            }
        });
    }

    /// `setAllowOneActionOnResourceUnavailable(...)`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:175 ResourceManager.setAllowOneActionOnResourceUnavailable
    pub fn set_allow_one_action_on_resource_unavailable(
        &self,
        allow_one_action_on_resource_unavailable: bool,
    ) {
        self.state.lock().allow_one_action_on_resource_unavailable =
            allow_one_action_on_resource_unavailable;
    }

    /// Returns prediction of RAM in Mb used by registered actions.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:181 ResourceManager.getUsedMemoryInMb
    pub fn get_used_memory_in_mb(&self) -> f64 {
        self.state
            .lock()
            .used_resources
            .get(ResourceSet::MEMORY)
            .copied()
            .unwrap_or(0.0)
    }

    /// Returns prediction of CPUs used by registered actions.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:187 ResourceManager.getUsedCPU
    pub fn get_used_cpu(&self) -> f64 {
        self.state
            .lock()
            .used_resources
            .get(ResourceSet::CPU)
            .copied()
            .unwrap_or(0.0)
    }

    /// `initializeCpuLoadFunctionality(machineLoadProvider, cpuLoadScheduling, windowSize)`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:267 ResourceManager.initializeCpuLoadFunctionality
    pub fn initialize_cpu_load_functionality(
        &self,
        cpu_load_scheduling: bool,
        window_size: Duration,
    ) {
        if cpu_load_scheduling {
            // PORT: MachineLoadProvider (the machine's CPU load) is not ported.
            blaze_util::unported!("MachineLoadProvider.instance");
        }
        let mut st = self.state.lock();
        st.cpu_load_scheduling = cpu_load_scheduling;
        st.window_size = window_size;
    }

    /// Resets resource manager state and releases all thread locks.
    ///
    /// Note - it does not reset available resources. Use separate call to
    /// `set_available_resources()`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:301 ResourceManager.resetResourceUsage
    pub fn reset_resource_usage(&self) {
        let mut st = self.state.lock();
        ResourceManager::reset_resource_usage_locked(&mut st);
    }

    fn reset_resource_usage_locked(st: &mut State) {
        st.used_resources = HashMap::new();
        st.used_local_test_count = 0;
        for request in st
            .local_requests
            .iter()
            .chain(st.dynamic_worker_requests.iter())
            .chain(st.dynamic_standalone_requests.iter())
        {
            if let Some(latch) = &request.resource_latch.latch {
                latch.count_down();
            }
        }
        st.local_requests.clear();
        st.dynamic_worker_requests.clear();
        st.dynamic_standalone_requests.clear();

        st.window_request_ids.clear();
        st.window_estimation_cpu = 0.0;
        st.running_actions = 0;
    }

    /// Sets available resources using given resource set.
    ///
    /// Must be called at least once before using resource manager.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:327 ResourceManager.setAvailableResources
    pub fn set_available_resources(&self, resources: ResourceSet) {
        let mut st = self.state.lock();
        ResourceManager::reset_resource_usage_locked(&mut st);
        st.available_resources = Some(resources);
        // PORT: skipped, tracing (Java log).
    }

    /// `availableResources` (`@VisibleForTesting public`).
    pub fn get_available_resources(&self) -> Option<ResourceSet> {
        self.state.lock().available_resources.clone()
    }

    /// `scheduleCpuLoadWindowUpdate()`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:334 ResourceManager.scheduleCpuLoadWindowUpdate
    pub fn schedule_cpu_load_window_update(&self) {
        // PORT: no window update is ever scheduled without CPU load scheduling, which needs the
        // unported MachineLoadProvider (initialize_cpu_load_functionality fails loudly).
        if self.state.lock().cpu_load_scheduling {
            blaze_util::unported!("ResourceManager.scheduleCpuLoadWindowUpdate");
        }
    }

    /// Acquires requested resource set. Will block if resource is not available. NB! This method
    /// must be thread-safe!
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:366 ResourceManager.acquireResources
    pub fn acquire_resources(
        self: &Arc<Self>,
        owner: &dyn ActionExecutionMetadata,
        resources: &ResourceSet,
        priority: ResourcePriority,
    ) -> Result<ResourceHandle, ExecException> {
        self.acquire_resources_with_worker(owner, resources, priority)
            .map_err(AcquireResourcesError::into_exec)
    }

    /// [`acquire_resources`](Self::acquire_resources) with Java's `IOException` (borrowing a
    /// worker for a resource set with a worker key) kept apart.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:366 ResourceManager.acquireResources
    pub fn acquire_resources_with_worker(
        self: &Arc<Self>,
        owner: &dyn ActionExecutionMetadata,
        resources: &ResourceSet,
        priority: ResourcePriority,
    ) -> Result<ResourceHandle, AcquireResourcesError> {
        let owner_description = owner.pretty_print();
        blaze_util::check_state!(
            !self.thread_has_resources(),
            "acquireResources with existing resource lock during {}",
            owner_description
        );

        // Validate requested resources exist before creating a request.
        self.assert_resources_tracked(resources)?;
        let request = ResourceRequest {
            owner: owner_description,
            resource_set: resources.clone(),
            priority,
            id: REQUEST_ID_GENERATOR.fetch_add(1, Ordering::SeqCst),
        };

        // PORT: skipped, profiling.
        let resource_latch = self.acquire(&request)?;
        if let Some(latch) = &resource_latch.latch
            && latch.await_interruptibly().is_err()
        {
            // Synchronize on this to avoid any racing with #processWaitingRequests
            let mut st = self.state.lock();
            if latch.get_count() == 0 {
                // Resources already acquired by other side. Release them.
                let worker = resource_latch.worker.0.lock().take();
                let _ = self.release_locked(&mut st, &request, worker);
            } else {
                // Inform other side that resources shouldn't be acquired.
                latch.count_down();
            }
            return Err(ExecException::Interrupted.into());
        }

        self.set_thread_locked(true);

        let worker = {
            let _st = self.state.lock();
            resource_latch.worker.0.lock().take()
        };

        Ok(ResourceHandle {
            manager: Some(self.clone()),
            request: Some(request),
            worker,
        })
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:423 ResourceManager.incrementResources
    fn increment_resources(
        &self,
        st: &mut State,
        request: &ResourceRequest,
    ) -> Result<Option<Arc<dyn Worker>>, AcquireResourcesError> {
        let resources = &request.resource_set;

        for (key, value) in resources.get_resources() {
            let mut value = *value;
            if let Some(used) = st.used_resources.get(key) {
                value += *used;
            }
            st.used_resources.insert(key.clone(), value);
        }

        st.window_request_ids.insert(request.id);
        st.window_estimation_cpu += resources
            .get_resources()
            .get(ResourceSet::CPU)
            .copied()
            .unwrap_or(0.0);
        st.used_local_test_count += resources.get_local_test_count();
        if let Some(worker_key) = resources.get_worker_key() {
            return Ok(Some(self.worker_pool().borrow_worker(worker_key)?));
        }

        st.running_actions += 1;
        Ok(None)
    }

    /// Return true if any resources have been claimed through this manager.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:449 ResourceManager.inUse
    pub fn in_use(&self) -> bool {
        let st = self.state.lock();
        !st.used_resources.is_empty()
            || st.used_local_test_count != 0
            || !st.local_requests.is_empty()
            || !st.dynamic_worker_requests.is_empty()
            || !st.dynamic_standalone_requests.is_empty()
    }

    /// Return true iff this thread has a lock on non-zero resources.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:458 ResourceManager.threadHasResources
    pub fn thread_has_resources(&self) -> bool {
        let key = self.key();
        THREAD_LOCKED.with(|t| t.borrow().contains(&key))
    }

    /// Releases previously requested resource.
    ///
    /// NB! This method must be thread-safe!
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:471 ResourceManager.releaseResources
    pub fn release_resources(&self, request: &ResourceRequest) -> Result<(), ExecException> {
        self.release_resources_with_worker(request, None)
    }

    /// `releaseResources(request, worker)`: also returns the worker to the pool.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:471 ResourceManager.releaseResources
    pub fn release_resources_with_worker(
        &self,
        request: &ResourceRequest,
        worker: Option<Arc<dyn Worker>>,
    ) -> Result<(), ExecException> {
        blaze_util::check_state!(
            self.thread_has_resources(),
            "releaseResources without resource lock during {}",
            request.owner
        );
        self.release_locked_and_unlock_thread(request, worker)
    }

    fn release_locked_and_unlock_thread(
        &self,
        request: &ResourceRequest,
        worker: Option<Arc<dyn Worker>>,
    ) -> Result<(), ExecException> {
        let result = {
            let mut st = self.state.lock();
            self.release_locked(&mut st, request, worker)
        };
        self.set_thread_locked(false);
        result
    }

    /// `releaseResourceOwnership()`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:490 ResourceManager.releaseResourceOwnership
    pub fn release_resource_ownership(&self) {
        self.set_thread_locked(false);
    }

    /// `acquireResourceOwnership()`.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:494 ResourceManager.acquireResourceOwnership
    pub fn acquire_resource_ownership(&self) {
        self.set_thread_locked(true);
    }

    /// Returns the latch, which isn't `None` if we could not acquire the resources right now and
    /// need to wait.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:503 ResourceManager.acquire
    fn acquire(&self, request: &ResourceRequest) -> Result<ResourceLatch, AcquireResourcesError> {
        let mut st = self.state.lock();
        if self.are_resources_available_locked(&st, &request.resource_set)? {
            let worker = self.increment_resources(&mut st, request)?;
            return Ok(ResourceLatch {
                latch: None,
                worker: WorkerSlot(Arc::new(Mutex::new(worker))),
            });
        }
        let waiting_request = WaitingRequest {
            resource_request: request.clone(),
            resource_latch: ResourceLatch {
                latch: Some(Arc::new(CountDownLatch::new(1))),
                worker: WorkerSlot::default(),
            },
        };
        let resource_latch = waiting_request.resource_latch.clone();
        match request.priority {
            ResourcePriority::Local => st.local_requests.push_back(waiting_request),
            // Dynamic requests should be LIFO, because we are more likely to win the race on
            // newer actions.
            ResourcePriority::DynamicWorker => {
                st.dynamic_worker_requests.push_front(waiting_request)
            }
            // Dynamic requests should be LIFO, because we are more likely to win the race on
            // newer actions.
            ResourcePriority::DynamicStandalone => {
                st.dynamic_standalone_requests.push_front(waiting_request)
            }
        }
        Ok(resource_latch)
    }

    /// Release resources and process the queues of waiting threads.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:526 ResourceManager.release
    fn release_locked(
        &self,
        st: &mut State,
        request: &ResourceRequest,
        worker: Option<Arc<dyn Worker>>,
    ) -> Result<(), ExecException> {
        if let Some(worker) = worker {
            self.worker_pool()
                .return_worker(&worker.get_worker_key().clone(), worker);
        }

        let resources = &request.resource_set;
        st.used_local_test_count -= resources.get_local_test_count();
        // TODO(bazel-team): (2010) rounding error can accumulate and value below can end up being
        // e.g. 1E-15. So if it is small enough, we set it to 0. But maybe there is a better
        // solution.
        let epsilon = 0.0001;
        let mut to_remove: HashSet<String> = HashSet::new();
        for (key, requested) in resources.get_resources() {
            let value = st.used_resources.get(key).copied().unwrap_or(0.0) - requested;
            st.used_resources.insert(key.clone(), value);
            if value < epsilon {
                to_remove.insert(key.clone());
            }
        }
        for key in &to_remove {
            st.used_resources.remove(key);
        }

        if st.window_request_ids.remove(&request.id) {
            st.window_estimation_cpu -= resources
                .get_resources()
                .get(ResourceSet::CPU)
                .copied()
                .unwrap_or(0.0);
        }
        st.running_actions -= 1;

        // PORT: an IOException of borrowing a worker for a waiting request leaves Java's close()
        // as an IOException; here it is the ExecException AbstractSpawnStrategy makes of one.
        self.process_all_waiting_requests(st)
            .map_err(AcquireResourcesError::into_exec)
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:559 ResourceManager.processAllWaitingRequests
    fn process_all_waiting_requests(&self, st: &mut State) -> Result<(), AcquireResourcesError> {
        let mut local = std::mem::take(&mut st.local_requests);
        let r = self.process_waiting_requests(st, &mut local);
        st.local_requests = local;
        r?;
        let mut worker = std::mem::take(&mut st.dynamic_worker_requests);
        let r = self.process_waiting_requests(st, &mut worker);
        st.dynamic_worker_requests = worker;
        r?;
        let mut standalone = std::mem::take(&mut st.dynamic_standalone_requests);
        let r = self.process_waiting_requests(st, &mut standalone);
        st.dynamic_standalone_requests = standalone;
        r
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:566 ResourceManager.processWaitingRequests
    fn process_waiting_requests(
        &self,
        st: &mut State,
        requests: &mut VecDeque<WaitingRequest>,
    ) -> Result<(), AcquireResourcesError> {
        if requests.is_empty() {
            return Ok(());
        }

        let mut i = 0;
        while i < requests.len() {
            let latch = requests[i]
                .resource_latch
                .latch
                .clone()
                .expect("waiting requests have a latch");
            if latch.get_count() != 0 {
                if self.are_resources_available_locked(
                    st,
                    &requests[i].resource_request.resource_set,
                )? {
                    let request = requests[i].resource_request.clone();
                    let worker = self.increment_resources(st, &request)?;
                    *requests[i].resource_latch.worker.0.lock() = worker;
                    latch.count_down();
                    requests.remove(i);
                    continue;
                }
            } else {
                // Cancelled by other side.
                requests.remove(i);
                continue;
            }
            i += 1;
        }
        Ok(())
    }

    /// Throws an exception if requested extra resource isn't being tracked.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:590 ResourceManager.assertResourcesTracked
    fn assert_resources_tracked(&self, resources: &ResourceSet) -> Result<(), ExecException> {
        let st = self.state.lock();
        let available = st
            .available_resources
            .as_ref()
            .expect("NullPointerException: availableResources");
        for key in resources.get_resources().keys() {
            if !available.get_resources().contains_key(key) {
                let mut message = String::new();
                message.push_str("Resource ");
                message.push_str(key);
                message.push_str(" is not being tracked by the resource manager.");
                message.push_str(" Available resources are: ");
                message.push_str(
                    &available
                        .get_resources()
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                return Err(local_execution_exception(
                    message,
                    local_execution::Code::UntrackedResource,
                ));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:612 ResourceManager.isAvailable
    fn is_available(
        st: &State,
        available: f64,
        used: f64,
        requested: f64,
        resource_name: &str,
    ) -> Result<bool, ExecException> {
        if !st.allow_one_action_on_resource_unavailable && available + used < requested {
            return Err(local_execution_exception(
                format!(
                    "The `{resource_name}` resources are not enough to fulfill the request. To allow Bazel to bypass this limitation, please adjust the --local_resources flag or specify --allow_one_action_on_resource_unavailable in your Bazel command."
                ),
                local_execution::Code::NotEnoughLocalResource,
            ));
        }
        // Resources are considered available if any one of the conditions below is true:
        // 1) If resource is not requested at all, it is available.
        // 2) If resource is not used at the moment and the flag
        // "allow_one_action_on_resource_unavailable" is enabled, it is considered to be
        // available regardless of how much is requested. This is necessary to
        // ensure that at any given time, at least one thread is able to acquire
        // resources even if it requests more than available.
        // 3) If used resource amount is less than total available resource amount.
        Ok(requested == 0.0
            || (st.allow_one_action_on_resource_unavailable && used == 0.0)
            || used + requested <= available)
    }

    /// Method will return true if all requested resources are considered to be available.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:646 ResourceManager.areResourcesAvailable
    pub fn are_resources_available(&self, resources: &ResourceSet) -> Result<bool, ExecException> {
        let st = self.state.lock();
        self.are_resources_available_locked(&st, resources)
    }

    fn are_resources_available_locked(
        &self,
        st: &State,
        resources: &ResourceSet,
    ) -> Result<bool, ExecException> {
        let available_resources = st
            .available_resources
            .as_ref()
            .expect("NullPointerException: availableResources");
        // Comparison below is robust, since any calculation errors will be fixed
        // by the release() method.

        if let Some(worker_key) = resources.get_worker_key()
            && !self.worker_pool().has_available_quota(worker_key)
        {
            return Ok(false);
        }

        if st.allow_one_action_on_resource_unavailable
            && st.used_resources.is_empty()
            && st.used_local_test_count == 0
            && resources.get_local_test_count() > 0
        {
            return Ok(true);
        }

        let available_local_test_count = available_resources.get_local_test_count();
        if !ResourceManager::is_available(
            st,
            available_local_test_count as f64,
            st.used_local_test_count as f64,
            resources.get_local_test_count() as f64,
            "local_test_count",
        )? {
            return Ok(false);
        }

        for (key, value) in resources.get_resources() {
            if key == ResourceSet::CPU {
                if !ResourceManager::is_cpu_available(st, key, *value)? {
                    return Ok(false);
                }
                continue;
            }
            // Use only MIN_NECESSARY_RATIO of the resource value to check for allocation. This is
            // necessary to account for the fact that most of the requested resource sets use
            // pessimistic estimations. Note that this ratio is used only during comparison - for
            // tracking we will actually mark whole requested amount as used.
            let requested = value * min_necessary_ratio(key);
            let used = st.used_resources.get(key).copied().unwrap_or(0.0);
            let available = available_resources.get(key);
            if !ResourceManager::is_available(st, available, used, requested, key)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:697 ResourceManager.isCpuAvailable
    fn is_cpu_available(st: &State, key: &str, value: f64) -> Result<bool, ExecException> {
        let available_resources = st
            .available_resources
            .as_ref()
            .expect("NullPointerException: availableResources");
        let requested = value * min_necessary_ratio(key);
        let available = available_resources.get(key);
        let used = st.used_resources.get(key).copied().unwrap_or(0.0);

        if st.cpu_load_scheduling {
            // Don't allow to run more than x3 of number cores actions simultaneously.
            if st.running_actions as f64
                >= MAX_ACTIONS_PER_CPU as f64 * available_resources.get(ResourceSet::CPU)
            {
                return Ok(false);
            }
            let _ = st.window_estimation_cpu;
            blaze_util::unported!("MachineLoadProvider.getCurrentCpuUsage");
        }

        ResourceManager::is_available(st, available, used, requested, key)
    }

    /// `getWaitCount()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:719 ResourceManager.getWaitCount
    pub fn get_wait_count(&self) -> i32 {
        let st = self.state.lock();
        (st.local_requests.len()
            + st.dynamic_standalone_requests.len()
            + st.dynamic_worker_requests.len()) as i32
    }
}

// Java: com/google/devtools/build/lib/actions/ResourceManager.java:600 ResourceManager.assertResourcesTracked
fn local_execution_exception(message: String, code: local_execution::Code) -> ExecException {
    UserExecException::new(FailureDetail {
        message,
        category: Some(failure_detail::Category::LocalExecution(
            failure_details::LocalExecution { code: code as i32 },
        )),
    })
    .into()
}

/// `LocalHostCapacity.getLocalHostCapacity()`: the local host's capacity as a `ResourceSet`
/// (`LocalHostResource.DEFAULT_RESOURCES`: memory, cpu, `Integer.MAX_VALUE` local tests).
// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:32 LocalHostCapacity.getLocalHostCapacity
pub fn get_local_host_capacity() -> ResourceSet {
    let capacity = blaze_optdefs::common::local_host_capacity::get_local_host_capacity();
    ResourceSet::create(capacity.memory_mb, capacity.cpu_usage, i32::MAX)
}

/// `ExecutionOptions.getLocalResources()`: cpu and memory (0.67 of the host's) overridden by
/// `--local_resources` (`ImmutableMap.Builder.buildKeepingLast`: first position, last value).
// Java: com/google/devtools/build/lib/exec/ExecutionOptions.java:356 ExecutionOptions.getLocalResources
pub fn get_local_resources(options: &ExecutionOptions) -> IndexMap<String, f64> {
    let capacity = get_local_host_capacity();
    let mut resources: IndexMap<String, f64> = IndexMap::new();
    resources.insert(ResourceSet::CPU.to_string(), capacity.get_cpu_usage());
    resources.insert(
        ResourceSet::MEMORY.to_string(),
        0.67 * capacity.get_memory_mb(),
    );
    for (k, v) in &options.local_resources {
        resources.insert(k.clone(), *v);
    }
    resources
}

/// `ExecutionTool.configureResourceManager(resourceMgr, request)`.
// Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:953 ExecutionTool.configureResourceManager
pub fn configure_resource_manager(resource_mgr: &ResourceManager, options: &ExecutionOptions) {
    resource_mgr.set_available_resources(ResourceSet::create_from_map(
        get_local_resources(options),
        // ExecutionOptions.usingLocalTestJobs()
        if options.local_test_jobs != 0 {
            options.local_test_jobs
        } else {
            i32::MAX
        },
    ));

    resource_mgr.initialize_cpu_load_functionality(
        options.experimental_cpu_load_scheduling,
        options.experimental_cpu_load_scheduling_window_size,
    );
    resource_mgr.schedule_cpu_load_window_update();

    resource_mgr.set_allow_one_action_on_resource_unavailable(
        options.allow_one_action_on_resource_unavailable,
    );
}
