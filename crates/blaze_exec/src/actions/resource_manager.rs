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
    // PORT: the worker handed over through the latch is not ported (workers are unported).
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
#[derive(Debug)]
pub struct ResourceManager {
    state: Mutex<State>,
}

/// A handle returned by [`ResourceManager::acquire_resources`] that must be closed (or dropped)
/// in order to free the resources again.
// Java: com/google/devtools/build/lib/actions/ResourceManager.java:90 ResourceManager.ResourceHandle
#[derive(Debug)]
pub struct ResourceHandle {
    manager: Option<Arc<ResourceManager>>,
    request: Option<ResourceRequest>,
}

impl ResourceHandle {
    /// `getRequest()`.
    pub fn get_request(&self) -> Option<&ResourceRequest> {
        self.request.as_ref()
    }

    /// Closing the ResourceHandle releases the resources associated with it.
    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:115 ResourceManager.ResourceHandle.close
    pub fn close(mut self) -> Result<(), ExecException> {
        match (self.manager.take(), self.request.take()) {
            (Some(manager), Some(request)) => manager.release_resources(&request),
            _ => Ok(()),
        }
        // PORT: profiling dropped.
    }

    /// `getWorker()` / `invalidateAndClose(e)`: persistent workers are not ported.
    pub fn invalidate_and_close(self) -> Result<(), ExecException> {
        blaze_util::unported!("ResourceManager.ResourceHandle.invalidateAndClose")
    }
}

impl Drop for ResourceHandle {
    fn drop(&mut self) {
        if let (Some(manager), Some(request)) = (self.manager.take(), self.request.take()) {
            if std::thread::panicking() {
                let _ = manager.release_locked_and_unlock_thread(&request);
            } else {
                let _ = manager.release_resources(&request);
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
        })
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
        // PORT: logging dropped.
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

        // PORT: profiling dropped.
        let resource_latch = self.acquire(&request)?;
        if let Some(latch) = &resource_latch.latch
            && latch.await_interruptibly().is_err()
        {
            // Synchronize on this to avoid any racing with #processWaitingRequests
            let mut st = self.state.lock();
            if latch.get_count() == 0 {
                // Resources already acquired by other side. Release them.
                let _ = ResourceManager::release_locked(&mut st, &request);
            } else {
                // Inform other side that resources shouldn't be acquired.
                latch.count_down();
            }
            return Err(ExecException::Interrupted);
        }

        self.set_thread_locked(true);

        Ok(ResourceHandle {
            manager: Some(self.clone()),
            request: Some(request),
        })
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:423 ResourceManager.incrementResources
    fn increment_resources(st: &mut State, request: &ResourceRequest) {
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
        // PORT: resources.getWorkerKey() is always null (workers are not ported).

        st.running_actions += 1;
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
        blaze_util::check_state!(
            self.thread_has_resources(),
            "releaseResources without resource lock during {}",
            request.owner
        );
        self.release_locked_and_unlock_thread(request)
    }

    fn release_locked_and_unlock_thread(
        &self,
        request: &ResourceRequest,
    ) -> Result<(), ExecException> {
        let result = {
            let mut st = self.state.lock();
            ResourceManager::release_locked(&mut st, request)
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
    fn acquire(&self, request: &ResourceRequest) -> Result<ResourceLatch, ExecException> {
        let mut st = self.state.lock();
        if ResourceManager::are_resources_available_locked(&st, &request.resource_set)? {
            ResourceManager::increment_resources(&mut st, request);
            return Ok(ResourceLatch { latch: None });
        }
        let waiting_request = WaitingRequest {
            resource_request: request.clone(),
            resource_latch: ResourceLatch {
                latch: Some(Arc::new(CountDownLatch::new(1))),
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
    fn release_locked(st: &mut State, request: &ResourceRequest) -> Result<(), ExecException> {
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

        ResourceManager::process_all_waiting_requests(st)
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:559 ResourceManager.processAllWaitingRequests
    fn process_all_waiting_requests(st: &mut State) -> Result<(), ExecException> {
        let mut local = std::mem::take(&mut st.local_requests);
        let r = ResourceManager::process_waiting_requests(st, &mut local);
        st.local_requests = local;
        r?;
        let mut worker = std::mem::take(&mut st.dynamic_worker_requests);
        let r = ResourceManager::process_waiting_requests(st, &mut worker);
        st.dynamic_worker_requests = worker;
        r?;
        let mut standalone = std::mem::take(&mut st.dynamic_standalone_requests);
        let r = ResourceManager::process_waiting_requests(st, &mut standalone);
        st.dynamic_standalone_requests = standalone;
        r
    }

    // Java: com/google/devtools/build/lib/actions/ResourceManager.java:566 ResourceManager.processWaitingRequests
    fn process_waiting_requests(
        st: &mut State,
        requests: &mut VecDeque<WaitingRequest>,
    ) -> Result<(), ExecException> {
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
                if ResourceManager::are_resources_available_locked(
                    st,
                    &requests[i].resource_request.resource_set,
                )? {
                    let request = requests[i].resource_request.clone();
                    ResourceManager::increment_resources(st, &request);
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
        ResourceManager::are_resources_available_locked(&st, resources)
    }

    fn are_resources_available_locked(
        st: &State,
        resources: &ResourceSet,
    ) -> Result<bool, ExecException> {
        let available_resources = st
            .available_resources
            .as_ref()
            .expect("NullPointerException: availableResources");
        // Comparison below is robust, since any calculation errors will be fixed
        // by the release() method.

        // PORT: resources.getWorkerKey() is always null (workers are not ported).

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
