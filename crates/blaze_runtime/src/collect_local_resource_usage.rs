//! Port of `com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java`: the thread
//! that samples local resource usage into counter series of the JSON profile.
//!
//! PORT: lives in `blaze_runtime` (docs/research/06 §5: runtime holds
//! `lib/profiler/CollectLocalResourceUsage`), because it needs the worker metrics, the resource
//! manager and `/proc/meminfo` parsing of crates above `blaze_util`'s profiler.

use std::sync::Arc;
use std::time::{Duration, Instant};

use blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector;
use blaze_util::profiler::java_hash_order::{Insertion, java_hash_map_order, string_hash_code};
use blaze_util::profiler::network_metrics_collector::NetworkMetricsCollector;
use blaze_util::profiler::profiler::get_process_cpu_time_nanos;
use blaze_util::profiler::{
    Clock, CounterSeriesCollector, CounterSeriesMap, CounterSeriesTask, CounterSeriesTaskColor,
    LocalResourceCollector, Profiler, TimeSeries,
};
use parking_lot::{Condvar, Mutex};

// TODO(twerth): Make these configurable.
const BUCKET_DURATION: Duration = Duration::from_secs(1);
const LOCAL_RESOURCES_COLLECT_SLEEP_INTERVAL: Duration = Duration::from_millis(200);

/// `ResourceEstimator` (the resource manager's estimation of used CPU and memory).
// Java: com/google/devtools/build/lib/actions/ResourceEstimator.java:21 ResourceEstimator
pub trait ResourceEstimator: Send + Sync {
    fn get_used_cpu(&self) -> f64;
    fn get_used_memory_in_mb(&self) -> f64;
}

impl ResourceEstimator for blaze_exec::actions::resource_manager::ResourceManager {
    fn get_used_cpu(&self) -> f64 {
        blaze_exec::actions::resource_manager::ResourceManager::get_used_cpu(self)
    }

    fn get_used_memory_in_mb(&self) -> f64 {
        blaze_exec::actions::resource_manager::ResourceManager::get_used_memory_in_mb(self)
    }
}

/// The state guarded by `synchronized (CollectLocalResourceUsage.this)`.
struct State {
    collectors: Vec<Box<dyn CounterSeriesCollector>>,
    /// `LinkedHashMap<CounterSeriesTask, TimeSeries>`, null before the thread runs and after
    /// `logCollectedData`.
    time_series: Option<Vec<(CounterSeriesTask, TimeSeries)>>,
    stop_local_usage_collection: bool,
    profiling_started: bool,
    stopwatch: Option<Instant>,
    collector_started: bool,
    collector_thread: Option<std::thread::JoinHandle<()>>,
}

/// Thread to collect local resource usage data and log into JSON profile.
// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:58 CollectLocalResourceUsage
pub struct CollectLocalResourceUsage {
    worker_process_metrics_collector: &'static WorkerProcessMetricsCollector,
    resource_estimator: Option<Arc<dyn ResourceEstimator>>,
    collect_worker_data_in_profiler: bool,
    collect_load_average: bool,
    collect_system_network_usage: bool,
    collect_resource_manager_estimation: bool,
    collect_pressure_stall_indicators: bool,
    collect_skyframe_counts: bool,
    /// The profiler's clock: `logCollectedData` reads `System.nanoTime()`, the clock the
    /// profiler uses in production (`BlazeClock`).
    clock: Arc<dyn Clock>,
    state: Arc<Mutex<State>>,
    wake: Arc<Condvar>,
}

impl CollectLocalResourceUsage {
    /// PORT: Java's `BugReporter` is only used by the JVM heap collector (skipped). The Skyframe
    /// graph (`--experimental_collect_skyframe_counts_in_profiler`) and the resource estimator are
    /// `None` when the caller has none (see `start`).
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:92 CollectLocalResourceUsage.<init>
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        worker_process_metrics_collector: &'static WorkerProcessMetricsCollector,
        resource_estimator: Option<Arc<dyn ResourceEstimator>>,
        collect_worker_data_in_profiler: bool,
        collect_load_average: bool,
        collect_system_network_usage: bool,
        collect_resource_manager_estimation: bool,
        collect_pressure_stall_indicators: bool,
        collect_skyframe_counts: bool,
        clock: Arc<dyn Clock>,
    ) -> Self {
        if collect_skyframe_counts {
            // Preconditions.checkState(!collectSkyframeCounts || graph != null, ...) holds in
            // Java (the runtime passes the evaluator's graph); Blaze's evaluator has no
            // InMemoryGraph that outlives an evaluation.
            blaze_util::unported!(
                "CollectLocalResourceUsage.SkyframeCountsCollector",
                "--experimental_collect_skyframe_counts_in_profiler needs InMemoryGraph"
            );
        }
        CollectLocalResourceUsage {
            worker_process_metrics_collector,
            resource_estimator,
            collect_worker_data_in_profiler,
            collect_load_average,
            collect_system_network_usage,
            collect_resource_manager_estimation,
            collect_pressure_stall_indicators,
            collect_skyframe_counts,
            clock,
            state: Arc::new(Mutex::new(State {
                collectors: Vec::new(),
                time_series: None,
                stop_local_usage_collection: false,
                profiling_started: false,
                stopwatch: None,
                collector_started: false,
                collector_thread: None,
            })),
            wake: Arc::new(Condvar::new()),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:131 CollectLocalResourceUsage.createLocalCollectors
    fn create_local_collectors(&self) -> Vec<Box<dyn CounterSeriesCollector>> {
        let mut collectors: Vec<Box<dyn CounterSeriesCollector>> = Vec::new();
        collectors.push(Box::new(LocalCpuUsageCollector::new()));
        // LocalMemoryUsageCollector ("Memory usage (Bazel)") reports the JVM's heap and
        // non-heap usage from MemoryMXBean.
        // PORT: skipped, JVM only (see above): CollectLocalResourceUsage.LocalMemoryUsageCollector.<init>, CollectLocalResourceUsage.LocalMemoryUsageCollector.collect
        collectors.push(Box::new(SystemCpuUsageCollector::new()));
        collectors.push(Box::new(SystemMemoryUsageCollector));

        if self.collect_worker_data_in_profiler {
            // Enabling the WorkerMemoryUsageCollector will cause hangs on Windows. We should only
            // enable it on Linux and Darwin.
            collectors.push(Box::new(TotalWorkerMemoryUsageCollector {
                worker_process_metrics_collector: self.worker_process_metrics_collector,
            }));
            collectors.push(Box::new(PerMnemonicWorkerMemoryUsageCollector {
                worker_process_metrics_collector: self.worker_process_metrics_collector,
            }));
        }
        if self.collect_load_average {
            collectors.push(Box::new(SystemLoadAverageCollector));
        }
        if self.collect_system_network_usage {
            collectors.push(Box::new(SystemNetworkUsageCollector));
        }
        if self.collect_resource_manager_estimation {
            match &self.resource_estimator {
                Some(e) => collectors.push(Box::new(ResourceManagerEstimationCollector {
                    resource_estimator: e.clone(),
                })),
                None => blaze_util::unported!(
                    "CommandEnvironment.getLocalResourceManager",
                    "--experimental_collect_resource_estimation: Blaze creates the resource \
                     manager in the execution phase, after the profiler starts"
                ),
            }
        }
        // The pressure stall indicators are only available on Linux.
        if self.collect_pressure_stall_indicators {
            collectors.push(Box::new(PressureStallIndicatorCollector));
        }

        if self.collect_skyframe_counts {
            blaze_util::unported!("CollectLocalResourceUsage.SkyframeCountsCollector");
        }

        collectors
    }

    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:225 CollectLocalResourceUsage.logCollectedData
    fn log_collected_data(&self) {
        let mut state = self.state.lock();
        if !state.profiling_started {
            return;
        }
        blaze_util::check_argument!(state.stop_local_usage_collection);
        let end_time_nanos = self.clock.nano_time();
        let elapsed_nanos = state.stopwatch.expect("stopwatch").elapsed().as_nanos() as i64;
        let start_time_nanos = end_time_nanos - elapsed_nanos;
        let profile_start = start_time_nanos;
        let len = (elapsed_nanos / BUCKET_DURATION.as_nanos() as i64) as usize + 1;
        let profiler = Profiler::instance();

        // timeSeries.entrySet().stream().collect(groupingBy(e -> e.getKey().laneName())).
        // PORT: java-hash-order (groupingBy's HashMap, filled with computeIfAbsent).
        let time_series = state.time_series.take().unwrap_or_default();
        let mut lane_names: Vec<String> = Vec::new();
        for (task, _) in &time_series {
            if !lane_names.iter().any(|l| l == task.lane_name()) {
                lane_names.push(task.lane_name().to_string());
            }
        }
        let groups = java_hash_map_order(
            &lane_names,
            |s| string_hash_code(s),
            Insertion::ComputeIfAbsent,
        );
        state.collectors.clear();
        drop(state);

        for lane_name in groups {
            let mut stacked_counters: CounterSeriesMap = Vec::new();
            for (task, series) in &time_series {
                if task.lane_name() == lane_name {
                    stacked_counters.push((task.clone(), series.to_double_array(len)));
                }
            }
            profiler.log_counters(stacked_counters, profile_start, BUCKET_DURATION);
        }
    }
}

/// `addRange(type, startTime, previousElapsed, nextElapsed, value)` on the state.
// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:253 CollectLocalResourceUsage.addRange
fn add_range(
    time_series: &mut Option<Vec<(CounterSeriesTask, TimeSeries)>>,
    type_: &CounterSeriesTask,
    start_time: Duration,
    previous_elapsed: Duration,
    next_elapsed: Duration,
    value: f64,
) {
    let Some(time_series) = time_series else {
        return;
    };
    let index = match time_series.iter().position(|(t, _)| t == type_) {
        Some(i) => i,
        None => {
            time_series.push((type_.clone(), TimeSeries::new(start_time, BUCKET_DURATION)));
            time_series.len() - 1
        }
    };
    time_series[index]
        .1
        .add_range_value(previous_elapsed, next_elapsed, value);
}

impl LocalResourceCollector for CollectLocalResourceUsage {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:121 CollectLocalResourceUsage.start
    fn start(&self) {
        let local_collectors = self.create_local_collectors();
        let state = self.state.clone();
        let wake = self.wake.clone();
        // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:177 CollectLocalResourceUsage.Collector.run
        let run = move || {
            let stopwatch;
            let start_time;
            let mut previous_elapsed;
            {
                let mut st = state.lock();
                st.collectors.extend(local_collectors);
                st.time_series = Some(Vec::new());
                stopwatch = Instant::now();
                st.stopwatch = Some(stopwatch);
                start_time = stopwatch.elapsed();
                previous_elapsed = stopwatch.elapsed();
                st.profiling_started = true;
            }
            loop {
                {
                    let mut st = state.lock();
                    if st.stop_local_usage_collection {
                        return;
                    }
                    // Thread.sleep: an interrupt (stop) ends the thread at once.
                    let deadline = Instant::now() + LOCAL_RESOURCES_COLLECT_SLEEP_INTERVAL;
                    while !st.stop_local_usage_collection {
                        if wake.wait_until(&mut st, deadline).timed_out() {
                            break;
                        }
                    }
                    if st.stop_local_usage_collection {
                        return;
                    }
                }
                let next_elapsed = stopwatch.elapsed();
                let delta_nanos = (next_elapsed - previous_elapsed).as_nanos() as f64;
                let final_previous_elapsed = previous_elapsed;
                {
                    let mut st = state.lock();
                    let State {
                        collectors,
                        time_series,
                        ..
                    } = &mut *st;
                    for collector in collectors.iter_mut() {
                        collector.collect(delta_nanos, &mut |type_, value| {
                            add_range(
                                time_series,
                                type_,
                                start_time,
                                final_previous_elapsed,
                                next_elapsed,
                                value,
                            )
                        });
                    }
                }
                previous_elapsed = next_elapsed;
            }
        };
        // collector.setDaemon(true); collector.start().
        // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:172 CollectLocalResourceUsage.Collector.<init>
        let handle = std::thread::Builder::new()
            .name("collect-local-resources".to_string())
            // A plain Java thread: an exception reaches the default uncaught exception handler.
            .spawn(move || {
                blaze_util::uncaught_exception::run_catching(run);
            })
            .expect("failed to start collect-local-resources");
        let mut st = self.state.lock();
        st.collector_started = true;
        st.collector_thread = Some(handle);
    }

    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:127 CollectLocalResourceUsage.registerCounterSeriesCollector
    fn register_counter_series_collector(&self, collector: Box<dyn CounterSeriesCollector>) {
        self.state.lock().collectors.push(collector);
    }

    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:210 CollectLocalResourceUsage.stop
    fn stop(&self) {
        let handle = {
            let mut st = self.state.lock();
            if !st.collector_started {
                return;
            }
            blaze_util::check_argument!(!st.stop_local_usage_collection);
            st.stop_local_usage_collection = true;
            st.collector_started = false;
            st.collector_thread.take()
        };
        // collector.interrupt(); collector.join().
        self.wake.notify_all();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        self.log_collected_data();
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:269 CollectLocalResourceUsage.LocalCpuUsageCollector
struct LocalCpuUsageCollector {
    previous_cpu_time_nanos: i64,
}

fn local_cpu_usage() -> CounterSeriesTask {
    CounterSeriesTask::new(
        "CPU usage (Bazel)",
        "cpu",
        Some(CounterSeriesTaskColor::GOOD),
    )
}

impl LocalCpuUsageCollector {
    fn new() -> Self {
        LocalCpuUsageCollector {
            previous_cpu_time_nanos: get_process_cpu_time_nanos(),
        }
    }
}

impl CounterSeriesCollector for LocalCpuUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:282 CollectLocalResourceUsage.LocalCpuUsageCollector.collect
    fn collect(&mut self, delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let next_cpu_time_nanos = get_process_cpu_time_nanos();
        let cpu_level = (next_cpu_time_nanos - self.previous_cpu_time_nanos) as f64 / delta_nanos;
        self.previous_cpu_time_nanos = next_cpu_time_nanos;
        consumer(&local_cpu_usage(), cpu_level);
    }
}

/// `/proc/stat`'s first line as the JDK reads it for `OperatingSystemMXBean.getCpuLoad()`
/// (`get_totalticks`: used = user + nice, usedKernel = system + irq + softirq, total = those plus
/// idle and iowait).
#[derive(Clone, Copy, Default)]
struct Ticks {
    used: u64,
    used_kernel: u64,
    total: u64,
}

fn read_total_ticks() -> Option<Ticks> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let v: Vec<u64> = fields.take(7).filter_map(|f| f.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    let get = |i: usize| v.get(i).copied().unwrap_or(0);
    let (user, nice, system, idle, iow, irq, sirq) =
        (get(0), get(1), get(2), get(3), get(4), get(5), get(6));
    Some(Ticks {
        used: user + nice,
        used_kernel: system + irq + sirq,
        total: user + nice + system + idle + iow + irq + sirq,
    })
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:320 CollectLocalResourceUsage.SystemCpuUsageCollector
struct SystemCpuUsageCollector {
    num_processors: i32,
    /// The JDK's `counters.cpuTicks` (initialized by `perfInit` on the first call).
    previous: Option<Ticks>,
}

impl SystemCpuUsageCollector {
    fn new() -> Self {
        SystemCpuUsageCollector {
            // Runtime.getRuntime().availableProcessors() (affinity and cgroup quota aware).
            num_processors: std::thread::available_parallelism().map_or(1, |n| n.get() as i32),
            previous: None,
        }
    }

    /// `OperatingSystemMXBean.getCpuLoad()` on Linux: the share of non-idle ticks since the last
    /// call (JDK `get_cpuload_internal(-1)`), at most 1.0; negative when unavailable.
    ///
    /// PORT: the host's `/proc/stat`; the JDK switches to container metrics only when the
    /// cgroup has a CPU quota or shares, or a cpuset smaller than the host's.
    fn get_cpu_load(&mut self) -> f64 {
        let previous = match self.previous {
            Some(p) => p,
            None => match read_total_ticks() {
                Some(t) => t,
                None => return -1.0,
            },
        };
        let Some(now) = read_total_ticks() else {
            return -1.0;
        };
        self.previous = Some(now);
        let udiff = now.used.saturating_sub(previous.used) as f64;
        let kdiff = now.used_kernel.saturating_sub(previous.used_kernel) as f64;
        let mut tdiff = now.total.saturating_sub(previous.total) as f64;
        if tdiff == 0.0 {
            return 0.0;
        }
        if tdiff < udiff + kdiff {
            tdiff = udiff + kdiff;
        }
        let load = kdiff / tdiff + udiff / tdiff;
        load.min(1.0)
    }
}

fn system_cpu_usage() -> CounterSeriesTask {
    CounterSeriesTask::new(
        "CPU usage (total)",
        "system cpu",
        Some(CounterSeriesTaskColor::RAIL_LOAD),
    )
}

impl CounterSeriesCollector for SystemCpuUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:332 CollectLocalResourceUsage.SystemCpuUsageCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let mut system_cpu_load = self.get_cpu_load();
        if system_cpu_load.is_nan() {
            // Unlike advertised, on Mac the system CPU load is NaN sometimes.
            // There is no good way to handle this, so to avoid any downstream method crashing on
            // this, we reset the CPU value here.
            system_cpu_load = 0.0;
        }
        consumer(
            &system_cpu_usage(),
            system_cpu_load * self.num_processors as f64,
        );
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:353 CollectLocalResourceUsage.SystemMemoryUsageCollector
struct SystemMemoryUsageCollector;

impl CounterSeriesCollector for SystemMemoryUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:363 CollectLocalResourceUsage.SystemMemoryUsageCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let mut system_memory_usage_mb: i64 = -1;
        // On Linux we get a better estimate by using /proc/meminfo. See
        // https://www.linuxatemyram.com/ for more info on buffer caches.
        if let Ok(proc_meminfo_parser) =
            blaze_server::proc_meminfo_parser::ProcMeminfoParser::from_file("/proc/meminfo")
        {
            if let (Ok(total), Ok(free)) = (
                proc_meminfo_parser.get_total_kb(),
                proc_meminfo_parser.get_free_ram_kb(),
            ) {
                system_memory_usage_mb = (total - free) / 1024;
            }
            // Silently ignore and fallback.
        }
        if system_memory_usage_mb <= 0 {
            // In case we aren't running on Linux or /proc/meminfo parsing went wrong, fall back
            // to the OS bean.
            // SAFETY: sysconf has no preconditions.
            let (pages, avail, page_size) = unsafe {
                (
                    libc::sysconf(libc::_SC_PHYS_PAGES) as i64,
                    libc::sysconf(libc::_SC_AVPHYS_PAGES) as i64,
                    libc::sysconf(libc::_SC_PAGESIZE) as i64,
                )
            };
            system_memory_usage_mb = (pages * page_size - avail * page_size) / (1024 * 1024);
        }
        consumer(
            &CounterSeriesTask::new(
                "Memory usage (total)",
                "system memory",
                Some(CounterSeriesTaskColor::BAD),
            ),
            system_memory_usage_mb as f64,
        );
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:387 CollectLocalResourceUsage.TotalWorkerMemoryUsageCollector
struct TotalWorkerMemoryUsageCollector {
    worker_process_metrics_collector: &'static WorkerProcessMetricsCollector,
}

impl CounterSeriesCollector for TotalWorkerMemoryUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:399 CollectLocalResourceUsage.TotalWorkerMemoryUsageCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let worker_memory_usage_mb: i32 = {
            let _c = Profiler::instance().profile_info("Worker metrics collection");
            self.worker_process_metrics_collector
                .get_live_worker_process_metrics()
                .iter()
                .map(|m| m.get_used_memory_in_kb())
                .fold(0i32, |a, b| a.wrapping_add(b))
                / 1024
        };
        consumer(
            &CounterSeriesTask::new(
                "Total worker memory usage",
                "workers memory",
                Some(CounterSeriesTaskColor::RAIL_ANIMATION),
            ),
            worker_memory_usage_mb as f64,
        );
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:412 CollectLocalResourceUsage.PerMnemonicWorkerMemoryUsageCollector
struct PerMnemonicWorkerMemoryUsageCollector {
    worker_process_metrics_collector: &'static WorkerProcessMetricsCollector,
}

const SERIES_LANE_NAME: &str = "Per-mnemonic worker memory usage";

impl CounterSeriesCollector for PerMnemonicWorkerMemoryUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:432 CollectLocalResourceUsage.PerMnemonicWorkerMemoryUsageCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        for collector in self
            .worker_process_metrics_collector
            .get_live_worker_process_metrics()
        {
            let _c = Profiler::instance().profile_info(collector.get_mnemonic());
            consumer(
                // getWorkerMemoryUsageSeries(mnemonic): the static map only caches the task,
                // which is a value (equal for equal fields).
                // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:423 CollectLocalResourceUsage.PerMnemonicWorkerMemoryUsageCollector.getWorkerMemoryUsageSeries
                &CounterSeriesTask::new(
                    SERIES_LANE_NAME,
                    collector.get_mnemonic(),
                    Some(CounterSeriesTaskColor::RAIL_ANIMATION),
                ),
                collector.get_used_memory_in_kb() as f64 / 1024.0,
            );
        }
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:447 CollectLocalResourceUsage.SystemLoadAverageCollector
struct SystemLoadAverageCollector;

impl CounterSeriesCollector for SystemLoadAverageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:457 CollectLocalResourceUsage.SystemLoadAverageCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        // OperatingSystemMXBean.getSystemLoadAverage(): getloadavg(3), the last minute.
        let mut loadavg = [0f64; 1];
        // SAFETY: getloadavg writes at most one element into the array.
        let n = unsafe { libc::getloadavg(loadavg.as_mut_ptr(), 1) };
        let load_average = if n == 1 { loadavg[0] } else { -1.0 };
        if load_average > 0.0 {
            consumer(
                &CounterSeriesTask::new(
                    "System load average",
                    "load",
                    Some(CounterSeriesTaskColor::GENERIC_WORK),
                ),
                load_average,
            );
        }
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:465 CollectLocalResourceUsage.SystemNetworkUsageCollector
struct SystemNetworkUsageCollector;

impl CounterSeriesCollector for SystemNetworkUsageCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:478 CollectLocalResourceUsage.SystemNetworkUsageCollector.collect
    fn collect(&mut self, delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let system_network_usages =
            NetworkMetricsCollector::instance().collect_system_network_usages(delta_nanos);
        if let Some(usages) = system_network_usages {
            consumer(
                &CounterSeriesTask::new(
                    "Network Up usage (total)",
                    "system network up (Mbps)",
                    Some(CounterSeriesTaskColor::RAIL_RESPONSE),
                ),
                usages.megabits_sent_per_sec(),
            );
            consumer(
                &CounterSeriesTask::new(
                    "Network Down usage (total)",
                    "system network down (Mbps)",
                    Some(CounterSeriesTaskColor::RAIL_RESPONSE),
                ),
                usages.megabits_recv_per_sec(),
            );
        }
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:488 CollectLocalResourceUsage.ResourceManagerEstimationCollector
struct ResourceManagerEstimationCollector {
    resource_estimator: Arc<dyn ResourceEstimator>,
}

impl CounterSeriesCollector for ResourceManagerEstimationCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:505 CollectLocalResourceUsage.ResourceManagerEstimationCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        let estimated_cpu_usage = self.resource_estimator.get_used_cpu();
        let estimated_memory_usage_in_mb = self.resource_estimator.get_used_memory_in_mb();
        consumer(
            &CounterSeriesTask::new(
                "CPU usage estimation",
                "estimated cpu",
                Some(CounterSeriesTaskColor::CQ_BUILD_ATTEMPT_PASSED),
            ),
            estimated_cpu_usage,
        );
        consumer(
            &CounterSeriesTask::new(
                "Memory usage estimation",
                "estimated memory",
                Some(CounterSeriesTaskColor::RAIL_IDLE),
            ),
            estimated_memory_usage_in_mb,
        );
    }
}

// Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:513 CollectLocalResourceUsage.PressureStallIndicatorCollector
struct PressureStallIndicatorCollector;

impl CounterSeriesCollector for PressureStallIndicatorCollector {
    // Java: com/google/devtools/build/lib/profiler/CollectLocalResourceUsage.java:541 CollectLocalResourceUsage.PressureStallIndicatorCollector.collect
    fn collect(&mut self, _delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64)) {
        // The pressure stall indicator for full CPU metric is not defined.
        let pressure_stall_full_io = read_pressure_stall_indicator("io", "full");
        let pressure_stall_full_memory = read_pressure_stall_indicator("memory", "full");
        let pressure_stall_some_io = read_pressure_stall_indicator("io", "some");
        let pressure_stall_some_memory = read_pressure_stall_indicator("memory", "some");
        let pressure_stall_some_cpu = read_pressure_stall_indicator("cpu", "some");

        let task =
            |lane: &str, series: &str, color| CounterSeriesTask::new(lane, series, Some(color));
        consumer(
            &task(
                "I/O pressure stall level",
                "i/o pressure (full)",
                CounterSeriesTaskColor::RAIL_ANIMATION,
            ),
            pressure_stall_full_io,
        );
        consumer(
            &task(
                "Memory pressure stall level",
                "memory pressure (full)",
                CounterSeriesTaskColor::THREAD_STATE_UNKNOWN,
            ),
            pressure_stall_full_memory,
        );
        consumer(
            &task(
                "I/O pressure stall level",
                "i/o pressure (some)",
                CounterSeriesTaskColor::CQ_BUILD_ATTEMPT_FAILED,
            ),
            pressure_stall_some_io,
        );
        consumer(
            &task(
                "Memory pressure stall level",
                "memory pressure (some)",
                CounterSeriesTaskColor::RAIL_IDLE,
            ),
            pressure_stall_some_memory,
        );
        consumer(
            &task(
                "CPU pressure stall level",
                "cpu pressure (some)",
                CounterSeriesTaskColor::THREAD_STATE_RUNNING,
            ),
            pressure_stall_some_cpu,
        );
    }
}

/// Reads the Pressure Staller Indicator file for a given type and returns the double value for
/// `avg10`, or -1 if we couldn't read that value.
///
/// `lib/util/ResourceUsage.readPressureStallIndicator`, the only method of that class with a
/// caller at the pin (resource `memory` / `io` / `cpu`, metric `full` / `some`). The value is a
/// Java `float`.
// PORT: skipped, the resource and metric are passed as their strings: ResourceUsage.PressureStallIndicatorResource.<init>, ResourceUsage.PressureStallIndicatorResource.getResource, ResourceUsage.PressureStallIndicatorMetric.<init>, ResourceUsage.PressureStallIndicatorMetric.getMetric
// PORT: skipped, no caller at the pin: ResourceUsage.<init>, ResourceUsage.getAvailableProcessors, ResourceUsage.getTotalPhysicalMemorySize, ResourceUsage.getOsArchitecture, ResourceUsage.getOsName, ResourceUsage.getOsVersion, ResourceUsage.getHeapMemoryInit, ResourceUsage.getNonHeapMemoryInit, ResourceUsage.getHeapMemoryMax, ResourceUsage.getNonHeapMemoryMax, ResourceUsage.measureCurrentResourceUsage, ResourceUsage.getCurrentCpuUtilizationInMs, ResourceUsage.getAvailableMemory, ResourceUsage.Measurement.timeInMs, ResourceUsage.Measurement.cpuUtilizationInMs
// Java: com/google/devtools/build/lib/util/ResourceUsage.java:166 ResourceUsage.readPressureStallIndicator
fn read_pressure_stall_indicator(resource: &str, metric: &str) -> f64 {
    let file_name = format!("/proc/pressure/{resource}");
    let Ok(text) = std::fs::read(&file_name) else {
        return -1.0;
    };
    let text = blaze_util::string_encoding::latin1_decode(&text);
    // PSI_AVG10_START_* ("full avg10" / "some avg10") and PSI_AVG10_VALUE_PATTERN_*
    // ("^full avg10=([\\d.]+).*" / "^some avg10=([\\d.]+).*").
    let start = format!("{metric} avg10");
    for line in text.lines() {
        // Tries to find a line in file with the metric.
        if !line.starts_with(&start) {
            continue;
        }
        let Some(rest) = line.strip_prefix(&format!("{metric} avg10=")) else {
            return -1.0;
        };
        let value_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if value_end == 0 {
            return -1.0;
        }
        let value = &rest[..value_end];
        return match value.parse::<f32>() {
            Ok(v) => v as f64,
            Err(_) => panic!("NumberFormatException: For input string: \"{value}\""),
        };
    }
    -1.0
}
