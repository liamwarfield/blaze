//! Ports of `lib/worker/WorkerProcessMetrics.java`, `WorkerProcessMetricsCollector.java`, and
//! the parts of `lib/metrics/PsInfoCollector.java` / `ResourceSnapshot.java` the collector uses.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use blaze_events::{Clock, JavaClock};
use blaze_protos::build_event_stream::build_metrics::WorkerMetrics;
use blaze_protos::build_event_stream::build_metrics::worker_metrics::{WorkerStats, WorkerStatus};
use parking_lot::Mutex;

use super::worker_process_status::{Status, WorkerProcessStatus};

/// Contains data about worker statistics during execution. This class contains data for
/// `BuildMetrics.WorkerMetrics`.
// Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:31 WorkerProcessMetrics
#[derive(Debug)]
pub struct WorkerProcessMetrics {
    worker_ids: Mutex<Vec<i32>>,
    process_id: i64,
    mnemonic: String,
    is_multiplex: bool,
    is_sandbox: bool,
    worker_key_hash: i32,
    status: Arc<WorkerProcessStatus>,
    actions_executed: AtomicI32,
    mutable: Mutex<MetricsState>,
}

#[derive(Debug, Default)]
struct MetricsState {
    is_measurable: bool,
    memory_in_kb: i32,
    // Memory usage prior to this invocation, useful to calculate memory deltas as a result of a
    // particular invocations.
    prior_memory_in_kb: i32,
    /// Epoch milliseconds.
    last_call_time: Option<i64>,
    /// Epoch milliseconds.
    last_collected_time: Option<i64>,
    newly_created: bool,
    prior_actions_executed: i32,
}

impl WorkerProcessMetrics {
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:67 WorkerProcessMetrics.<init>
    pub fn new(
        worker_ids: Vec<i32>,
        process_id: i64,
        status: Arc<WorkerProcessStatus>,
        mnemonic: &str,
        is_multiplex: bool,
        is_sandbox: bool,
        worker_key_hash: i32,
    ) -> WorkerProcessMetrics {
        WorkerProcessMetrics {
            worker_ids: Mutex::new(worker_ids),
            process_id,
            mnemonic: mnemonic.to_string(),
            is_multiplex,
            is_sandbox,
            worker_key_hash,
            status,
            actions_executed: AtomicI32::new(0),
            mutable: Mutex::new(MetricsState {
                newly_created: true,
                ..Default::default()
            }),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:105 WorkerProcessMetrics.maybeAddWorkerId
    pub fn maybe_add_worker_id(&self, worker_id: i32, _status: &WorkerProcessStatus) {
        let mut ids = self.worker_ids.lock();
        // Multiplex workers have multiple worker ids, make sure not to include duplicate worker
        // ids.
        if ids.contains(&worker_id) {
            return;
        }
        ids.push(worker_id);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:113 WorkerProcessMetrics.addCollectedMetrics
    pub fn add_collected_metrics(&self, memory_in_kb: i32, collection_time_millis: i64) {
        let mut m = self.mutable.lock();
        m.memory_in_kb = memory_in_kb;
        m.is_measurable = true;
        m.last_collected_time = Some(collection_time_millis);
    }

    /// Reset relevant internal states before each command.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:120 WorkerProcessMetrics.onBeforeCommand
    pub fn on_before_command(&self) {
        let mut m = self.mutable.lock();
        m.newly_created = false;
        m.prior_actions_executed = self.actions_executed.load(Ordering::SeqCst);
        m.prior_memory_in_kb = m.memory_in_kb;
    }

    /// Whether the worker process was created during the current invocation.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:127 WorkerProcessMetrics.isNewlyCreated
    pub fn is_newly_created(&self) -> bool {
        self.mutable.lock().newly_created
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:131 WorkerProcessMetrics.incrementActionsExecuted
    pub fn increment_actions_executed(&self) {
        self.actions_executed.fetch_add(1, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:135 WorkerProcessMetrics.getActionsExecuted
    pub fn get_actions_executed(&self) -> i32 {
        self.actions_executed.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:139 WorkerProcessMetrics.getLastCallTime
    pub fn get_last_call_time(&self) -> Option<i64> {
        self.mutable.lock().last_call_time
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:143 WorkerProcessMetrics.getLastCollectedTime
    pub fn get_last_collected_time(&self) -> Option<i64> {
        self.mutable.lock().last_collected_time
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:147 WorkerProcessMetrics.setLastCallTime
    pub fn set_last_call_time(&self, last_call_time_millis: i64) {
        self.mutable.lock().last_call_time = Some(last_call_time_millis);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:151 WorkerProcessMetrics.isMeasurable
    pub fn is_measurable(&self) -> bool {
        self.mutable.lock().is_measurable
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:155 WorkerProcessMetrics.getWorkerIds
    pub fn get_worker_ids(&self) -> Vec<i32> {
        self.worker_ids.lock().clone()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:159 WorkerProcessMetrics.getProcessId
    pub fn get_process_id(&self) -> i64 {
        self.process_id
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:163 WorkerProcessMetrics.getMnemonic
    pub fn get_mnemonic(&self) -> &str {
        &self.mnemonic
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:167 WorkerProcessMetrics.isMultiplex
    pub fn is_multiplex(&self) -> bool {
        self.is_multiplex
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:171 WorkerProcessMetrics.isSandboxed
    pub fn is_sandboxed(&self) -> bool {
        self.is_sandbox
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:175 WorkerProcessMetrics.getWorkerKeyHash
    pub fn get_worker_key_hash(&self) -> i32 {
        self.worker_key_hash
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:179 WorkerProcessMetrics.getUsedMemoryInKb
    pub fn get_used_memory_in_kb(&self) -> i32 {
        self.mutable.lock().memory_in_kb
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:183 WorkerProcessMetrics.getStatus
    pub fn get_status(&self) -> &Arc<WorkerProcessStatus> {
        &self.status
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java:187 WorkerProcessMetrics.toProto
    pub fn to_proto(&self) -> WorkerMetrics {
        let m = self.mutable.lock();
        let mut stats = WorkerStats {
            worker_memory_in_kb: m.memory_in_kb,
            prior_worker_memory_in_kb: m.prior_memory_in_kb,
            ..Default::default()
        };
        if let Some(t) = m.last_collected_time {
            stats.collect_time_in_ms = t;
        }
        if let Some(t) = m.last_call_time {
            stats.last_action_start_time_in_ms = t;
        }

        let mut builder = WorkerMetrics {
            // addAllWorkerIds: uint32 field.
            worker_ids: self.worker_ids.lock().iter().map(|i| *i as u32).collect(),
            // setProcessId((int) processId): uint32 field.
            process_id: (self.process_id as i32) as u32,
            mnemonic: self.mnemonic.clone(),
            is_sandbox: self.is_sandbox,
            is_multiplex: self.is_multiplex,
            is_measurable: m.is_measurable,
            worker_key_hash: self.worker_key_hash as i64,
            worker_status: self.status.to_worker_status() as i32,
            actions_executed: self.actions_executed.load(Ordering::SeqCst) as i64,
            prior_actions_executed: m.prior_actions_executed as i64,
            worker_stats: vec![stats],
            ..Default::default()
        };

        if let Some(code) = self.status.get_worker_code() {
            builder.code = Some(code as i32);
        }

        builder
    }
}

/// `ResourceSnapshot`: memory per pid and the collection time (epoch milliseconds).
// Java: com/google/devtools/build/lib/metrics/ResourceSnapshot.java:23 ResourceSnapshot
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub pid_to_memory_in_kb: HashMap<i64, i32>,
    pub collection_time_millis: i64,
}

/// `CgroupsInfoCollector.instance().collectResourceUsage(pidToCgroups, clock)`: the memory of
/// each existing cgroup.
// Java: com/google/devtools/build/lib/metrics/CgroupsInfoCollector.java:36 CgroupsInfoCollector.collectResourceUsage
fn cgroups_info_collector_collect_resource_usage(
    pid_to_cgroups: &BTreeMap<i64, Arc<dyn crate::sandbox::cgroups::Cgroup>>,
    clock: &dyn Clock,
) -> ResourceSnapshot {
    let mut pid_to_memory_in_kb = HashMap::new();
    for (pid, cgroup) in pid_to_cgroups {
        if cgroup.exists() {
            pid_to_memory_in_kb.insert(*pid, cgroup.get_memory_usage_in_kb());
        }
    }
    ResourceSnapshot {
        pid_to_memory_in_kb,
        collection_time_millis: clock.current_time_millis(),
    }
}

impl ResourceSnapshot {
    // Java: com/google/devtools/build/lib/metrics/ResourceSnapshot.java:40 ResourceSnapshot.createEmpty
    pub fn create_empty(collection_time_millis: i64) -> ResourceSnapshot {
        ResourceSnapshot {
            pid_to_memory_in_kb: HashMap::new(),
            collection_time_millis,
        }
    }
}

/// `PsInfoCollector.PsInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PsInfo {
    pid: i64,
    parent_pid: i64,
    memory_in_kb: i32,
}

/// `PsInfoCollector.PsSnapshot`.
struct PsSnapshot {
    pid_to_ps_info: HashMap<i64, PsInfo>,
    pid_to_children_ps_info: HashMap<i64, Vec<PsInfo>>,
    collection_time_millis: i64,
    process_pids_hash: i32,
}

// Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:42 PsInfoCollector.MIN_COLLECTION_INTERVAL
const MIN_COLLECTION_INTERVAL: Duration = Duration::from_millis(500);

/// Collects the memory of process trees with `ps`.
// Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:40 PsInfoCollector
pub struct PsInfoCollector {
    current_ps_snapshot: Mutex<Option<PsSnapshot>>,
}

/// `ImmutableSet<Long>.hashCode()`: the sum of `Long.hashCode` of the elements.
fn java_long_set_hash(pids: &[i64]) -> i32 {
    let mut h: i32 = 0;
    for p in pids {
        h = h.wrapping_add((*p ^ ((*p as u64) >> 32) as i64) as i32);
    }
    h
}

impl PsInfoCollector {
    // Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:43 PsInfoCollector.instance
    pub fn instance() -> &'static PsInfoCollector {
        static INSTANCE: LazyLock<PsInfoCollector> = LazyLock::new(|| PsInfoCollector {
            current_ps_snapshot: Mutex::new(None),
        });
        &INSTANCE
    }

    /// If ps snapshot was outdated will update it, and then returns resource consumption
    /// snapshot of processes subtrees based on collected ps snapshot.
    // Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:57 PsInfoCollector.collectResourceUsage
    pub fn collect_resource_usage(
        &self,
        process_ids: &[i64],
        clock: &dyn Clock,
    ) -> ResourceSnapshot {
        let now = clock.current_time_millis();
        let mut current = self.current_ps_snapshot.lock();
        let pids_hash = java_long_set_hash(process_ids);
        let outdated = match current.as_ref() {
            None => true,
            Some(s) => {
                now - s.collection_time_millis > MIN_COLLECTION_INTERVAL.as_millis() as i64
                    || s.process_pids_hash != pids_hash
            }
        };
        if outdated {
            *current = Some(PsInfoCollector::update_ps_snapshot(clock, pids_hash));
        }
        let snapshot = current.as_ref().unwrap();

        let mut pid_to_memory_in_kb = HashMap::new();
        for pid in process_ids {
            let Some(ps_info) = snapshot.pid_to_ps_info.get(pid) else {
                continue;
            };
            pid_to_memory_in_kb.insert(
                *pid,
                PsInfoCollector::collect_memory_usage_of_descendants(ps_info, snapshot, 0),
            );
        }
        ResourceSnapshot {
            pid_to_memory_in_kb,
            collection_time_millis: snapshot.collection_time_millis,
        }
    }

    /// Updates current snapshot of all processes state, using ps command.
    // Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:83 PsInfoCollector.updatePsSnapshot
    fn update_ps_snapshot(clock: &dyn Clock, process_pids_hash: i32) -> PsSnapshot {
        let pid_to_ps_info = PsInfoCollector::collect_data_from_ps();
        let mut pid_to_children_ps_info: HashMap<i64, Vec<PsInfo>> = HashMap::new();
        for info in pid_to_ps_info.values() {
            pid_to_children_ps_info
                .entry(info.parent_pid)
                .or_default()
                .push(*info);
        }
        PsSnapshot {
            pid_to_ps_info,
            pid_to_children_ps_info,
            collection_time_millis: clock.current_time_millis(),
            process_pids_hash,
        }
    }

    /// Collects memory usage for every process.
    // Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:96 PsInfoCollector.collectDataFromPs
    fn collect_data_from_ps() -> HashMap<i64, PsInfo> {
        // new ProcessBuilder("ps", "-e", "-o", "pid,ppid,rss").start()
        let child = Command::new("ps")
            .args(["-e", "-o", "pid,ppid,rss"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            // Error while executing command ps (logged).
            return HashMap::new();
        };
        let mut ps_infos = HashMap::new();
        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            let mut is_first = true;
            for line in reader.lines() {
                let Ok(output) = line else { break };
                if is_first {
                    is_first = false;
                    continue;
                }
                let line: Vec<&str> = output
                    .split(' ')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                if line.len() != 3 {
                    // Unexpected length of split line (logged).
                    continue;
                }
                let (Ok(pid), Ok(parent_pid), Ok(memory_in_kb)) = (
                    line[0].parse::<i64>(),
                    line[1].parse::<i64>(),
                    line[2].parse::<i32>(),
                ) else {
                    // Error while parsing psOutput (logged); Java stops at the first bad line.
                    break;
                };
                ps_infos.insert(
                    pid,
                    PsInfo {
                        pid,
                        parent_pid,
                        memory_in_kb,
                    },
                );
            }
        }
        let _ = child.wait();
        ps_infos
    }

    /// Recursively collects total memory usage of all descendants of the process.
    // Java: com/google/devtools/build/lib/metrics/PsInfoCollector.java:154 PsInfoCollector.collectMemoryUsageOfDescendants
    fn collect_memory_usage_of_descendants(
        ps_info: &PsInfo,
        ps_snapshot: &PsSnapshot,
        depth: usize,
    ) -> i32 {
        let mut current_memory_in_kb = ps_info.memory_in_kb;
        // PORT: `ps` can report a pid as its own ancestor only for pid 0; the depth guard keeps a
        // malformed listing from recursing forever (Java would overflow the stack).
        if depth > 4096 {
            return current_memory_in_kb;
        }
        if let Some(children) = ps_snapshot.pid_to_children_ps_info.get(&ps_info.pid) {
            for child in children {
                if child.pid == ps_info.pid {
                    continue;
                }
                current_memory_in_kb = current_memory_in_kb.wrapping_add(
                    PsInfoCollector::collect_memory_usage_of_descendants(
                        child,
                        ps_snapshot,
                        depth + 1,
                    ),
                );
            }
        }
        current_memory_in_kb
    }
}

// Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:238 WorkerProcessMetricsCollector.MAX_PUBLISHED_WORKER_METRICS
pub const MAX_PUBLISHED_WORKER_METRICS: usize = 50;

/// Collects and populates system metrics about persistent workers.
// Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:44 WorkerProcessMetricsCollector
pub struct WorkerProcessMetricsCollector {
    clock: Mutex<Arc<dyn Clock>>,
    /// Mapping of worker process ids to their process metrics. This contains all workers that
    /// have been alive at any point during the build.
    ///
    /// PORT: java-hash-order. Java iterates a `ConcurrentHashMap<Long, ...>`; for the few
    /// workers of a build (fewer buckets than pids are apart) that is ascending pid order modulo
    /// the table size. The map is ordered by pid; `limitWorkerMetricsToPublish` sorts what
    /// reaches the build event stream by worker id anyway.
    pid_to_worker_process_metrics: Mutex<BTreeMap<i64, Arc<WorkerProcessMetrics>>>,
    use_cgroups_on_linux: Mutex<bool>,
    /// `pidToCgroups`: the cgroup of each worker process that has one.
    pid_to_cgroups: Mutex<BTreeMap<i64, Arc<dyn crate::sandbox::cgroups::Cgroup>>>,
    /// For tests: the memory snapshot to report instead of asking `ps`.
    fake_resource_usage: Mutex<Option<Arc<dyn Fn(&[i64]) -> ResourceSnapshot + Send + Sync>>>,
}

impl WorkerProcessMetricsCollector {
    /// The metrics collector (a static singleton instance).
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:47 WorkerProcessMetricsCollector.instance
    pub fn instance() -> &'static WorkerProcessMetricsCollector {
        static INSTANCE: LazyLock<WorkerProcessMetricsCollector> =
            LazyLock::new(WorkerProcessMetricsCollector::new);
        &INSTANCE
    }

    /// A separate collector (`@VisibleForTesting` constructor).
    pub fn new() -> WorkerProcessMetricsCollector {
        WorkerProcessMetricsCollector {
            clock: Mutex::new(Arc::new(JavaClock)),
            pid_to_worker_process_metrics: Mutex::new(BTreeMap::new()),
            use_cgroups_on_linux: Mutex::new(false),
            pid_to_cgroups: Mutex::new(BTreeMap::new()),
            fake_resource_usage: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:79 WorkerProcessMetricsCollector.setClock
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        *self.clock.lock() = clock;
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:83 WorkerProcessMetricsCollector.setUseCgroupsOnLinux
    pub fn set_use_cgroups_on_linux(&self, use_cgroups_on_linux: bool) {
        *self.use_cgroups_on_linux.lock() = use_cgroups_on_linux;
    }

    /// For tests: replaces `ps` by `f`.
    pub fn set_resource_usage_for_testing(
        &self,
        f: Option<Arc<dyn Fn(&[i64]) -> ResourceSnapshot + Send + Sync>>,
    ) {
        *self.fake_resource_usage.lock() = f;
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:87 WorkerProcessMetricsCollector.collectResourceUsage
    fn collect_resource_usage(&self) -> ResourceSnapshot {
        // Only collect for process we know are alive.
        let alive_pids: Vec<i64> = self
            .pid_to_worker_process_metrics
            .lock()
            .iter()
            .filter(|(_, m)| !m.get_status().is_killed())
            .map(|(pid, _)| *pid)
            .collect();
        self.collect_resource_usage_for(&alive_pids)
    }

    /// Collects memory usage of all ancestors of processes by pid. If a pid does not allow
    /// collecting memory usage, it is silently ignored.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:103 WorkerProcessMetricsCollector.collectResourceUsage
    pub fn collect_resource_usage_for(&self, alive_pids: &[i64]) -> ResourceSnapshot {
        let clock = self.clock.lock().clone();
        // TODO(b/181317827): Support Windows.
        if alive_pids.is_empty() {
            return ResourceSnapshot::create_empty(clock.current_time_millis());
        }
        if let Some(fake) = self.fake_resource_usage.lock().clone() {
            return fake(alive_pids);
        }
        // OS.LINUX
        if *self.use_cgroups_on_linux.lock() {
            // Remove the killed pids so that we only collect from the cgroups that are alive.
            let mut pid_to_cgroups = self.pid_to_cgroups.lock();
            pid_to_cgroups.retain(|pid, _| alive_pids.contains(pid));
            return cgroups_info_collector_collect_resource_usage(&pid_to_cgroups, &*clock);
        }
        // Default to using ps if cgroups is not enabled.
        PsInfoCollector::instance().collect_resource_usage(alive_pids, &*clock)
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:128 WorkerProcessMetricsCollector.getLiveWorkerProcessMetrics
    pub fn get_live_worker_process_metrics(&self) -> Vec<Arc<WorkerProcessMetrics>> {
        self.collect_metrics()
            .into_iter()
            .filter(|m| !m.get_status().is_killed())
            .collect()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:134 WorkerProcessMetricsCollector.collectMetrics
    pub fn collect_metrics(&self) -> Vec<Arc<WorkerProcessMetrics>> {
        let resource_snapshot = self.collect_resource_usage();

        let pid_to_memory_in_kb = &resource_snapshot.pid_to_memory_in_kb;
        let collection_time = resource_snapshot.collection_time_millis;

        let mut worker_metrics = Vec::new();
        let entries: Vec<Arc<WorkerProcessMetrics>> = self
            .pid_to_worker_process_metrics
            .lock()
            .values()
            .cloned()
            .collect();
        for worker_metric in entries {
            if worker_metric.get_status().is_killed() {
                // If it was previously killed by Bazel, we don't do anything.
                worker_metrics.push(worker_metric);
                continue;
            }

            let pid = worker_metric.get_process_id();
            let memory_in_kb = pid_to_memory_in_kb.get(&pid).copied().unwrap_or(0);

            if memory_in_kb == 0 {
                // If it is not measurable, not killed by Bazel but has executed actions, then we
                // assume that something has happened to the worker process that is not accounted
                // for by Bazel and set this to KILLED_UNKNOWN. If a separate thread comes along to
                // update the status with a more specific reason why it is killed, then we allow
                // such an update.
                if worker_metric.get_actions_executed() > 0 {
                    worker_metric
                        .get_status()
                        .maybe_update_status(Status::KilledUnknown);
                }
                // We want to add the worker metric even if it is not measurable.
                worker_metrics.push(worker_metric);
                continue;
            }

            // If it is measurable, we want to update the collected metrics.
            worker_metric.add_collected_metrics(memory_in_kb, collection_time);
            worker_metrics.push(worker_metric);
        }

        worker_metrics
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:177 WorkerProcessMetricsCollector.onWorkerFinishExecution
    pub fn on_worker_finish_execution(&self, process_id: i64) {
        let wpm = self
            .pid_to_worker_process_metrics
            .lock()
            .get(&process_id)
            .cloned();
        let Some(wpm) = wpm else {
            return;
        };
        wpm.increment_actions_executed();
    }

    /// Returns a prioritized and limited list of WorkerMetrics to be published to the BEP.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:188 WorkerProcessMetricsCollector.limitWorkerMetricsToPublish
    pub fn limit_worker_metrics_to_publish(
        metrics: Vec<WorkerMetrics>,
        limit: usize,
    ) -> Vec<WorkerMetrics> {
        let mut sorted = metrics;
        // Stream.sorted is stable.
        sorted.sort_by(worker_metrics_publish_comparator);
        sorted.truncate(limit);
        sorted.sort_by_key(|m| m.worker_ids[0] as i32);
        sorted
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:228 WorkerProcessMetricsCollector.getLiveWorkerMetrics
    pub fn get_live_worker_metrics(&self) -> Vec<WorkerMetrics> {
        self.get_live_worker_process_metrics()
            .iter()
            .map(|m| m.to_proto())
            .collect()
    }

    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:234 WorkerProcessMetricsCollector.clear
    pub fn clear(&self) {
        self.pid_to_worker_process_metrics.lock().clear();
    }

    /// `getPidToWorkerProcessMetrics()` (`@VisibleForTesting`).
    pub fn get_pid_to_worker_process_metrics(&self) -> BTreeMap<i64, Arc<WorkerProcessMetrics>> {
        self.pid_to_worker_process_metrics.lock().clone()
    }

    /// Initializes workerIdToWorkerProperties for workers. If worker metrics already exists for
    /// this worker, only updates the last call time and maybe adds the multiplex worker id.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:247 WorkerProcessMetricsCollector.registerWorker
    pub fn register_worker(
        &self,
        worker_id: i32,
        process_id: i64,
        status: &Arc<WorkerProcessStatus>,
        mnemonic: &str,
        is_multiplex: bool,
        is_sandboxed: bool,
        worker_key_hash: i32,
        cgroup: Option<Arc<dyn crate::sandbox::cgroups::Cgroup>>,
    ) {
        let worker_metric = {
            let mut map = self.pid_to_worker_process_metrics.lock();
            map.entry(process_id)
                .or_insert_with(|| {
                    Arc::new(WorkerProcessMetrics::new(
                        vec![worker_id],
                        process_id,
                        status.clone(),
                        mnemonic,
                        is_multiplex,
                        is_sandboxed,
                        worker_key_hash,
                    ))
                })
                .clone()
        };
        if let Some(cgroup) = cgroup {
            self.pid_to_cgroups
                .lock()
                .entry(process_id)
                .or_insert(cgroup);
        }
        worker_metric.set_last_call_time(self.clock.lock().current_time_millis());
        worker_metric.maybe_add_worker_id(worker_id, status);
    }

    /// Removes all WorkerProcessMetrics that were marked as killed.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:275 WorkerProcessMetricsCollector.clearKilledWorkerProcessMetrics
    pub fn clear_killed_worker_process_metrics(&self) {
        let mut map = self.pid_to_worker_process_metrics.lock();
        let pids_to_remove: HashSet<i64> = map
            .iter()
            .filter(|(_, m)| m.get_status().is_killed())
            .map(|(pid, _)| *pid)
            .collect();
        map.retain(|pid, _| !pids_to_remove.contains(pid));
    }

    /// To reset states in each WorkerProcessMetric before each command where applicable.
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:285 WorkerProcessMetricsCollector.beforeCommand
    pub fn before_command(&self) {
        for m in self.pid_to_worker_process_metrics.lock().values() {
            m.on_before_command();
        }
    }
}

/// Because we log all worker processes that have been alive at any point during the build, the
/// size of this list might grow out of hand if there is some issue with the build (e.g.
/// kill-create cycles). As such, we enforce rules to prioritize WorkerMetrics before limiting:
/// (1) Prioritize WorkerStatuses ALIVE, then KILLED_DUE_TO_MEMORY_PRESSURE, then all remaining
/// worker statuses. (2) Then prioritize by decreasing memory usage and (3) limit to a fixed
/// number.
// Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:206 WorkerProcessMetricsCollector.WorkerMetricsPublishComparator
pub fn worker_metrics_publish_comparator(
    m1: &WorkerMetrics,
    m2: &WorkerMetrics,
) -> std::cmp::Ordering {
    // Java: com/google/devtools/build/lib/worker/WorkerProcessMetricsCollector.java:208 WorkerProcessMetricsCollector.WorkerMetricsPublishComparator.getWorkerStatusPriority
    fn get_worker_status_priority(status: i32) -> i32 {
        // Lower value is prioritized.
        if status == WorkerStatus::Alive as i32 {
            0
        } else if status == WorkerStatus::KilledDueToMemoryPressure as i32 {
            1
        } else {
            2
        }
    }
    let s1 = get_worker_status_priority(m1.worker_status);
    let s2 = get_worker_status_priority(m2.worker_status);
    if s1 != s2 {
        return s1.cmp(&s2);
    }
    m2.worker_stats[0]
        .worker_memory_in_kb
        .cmp(&m1.worker_stats[0].worker_memory_in_kb)
}
