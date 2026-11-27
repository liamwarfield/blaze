//! Port of `com/google/devtools/build/lib/profiler/Profiler.java`.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::counter_series::{
    CounterSeriesTask, CounterSeriesTaskColor, CounterSeriesTraceData, TimeSeries,
};
use super::java_hash_order::{Insertion, java_hash_map_order, string_hash_code};
use super::json_trace_file_writer::{JsonTraceFileWriter, ProfileOutputStream, QueuedTraceData};
use super::profile_phase::ProfilePhase;
use super::profiler_task::{ProfilerTask, ProfilerTaskSet};
use super::stat_recorder::{
    PredicateBasedStatRecorder, SingleStatRecorder, StatRecorder, vfs_type_heuristics,
};
use super::thread_metadata::{ThreadMetadata, current_thread_id};
use super::trace_data::TaskData;

/// Port of `com.google.devtools.build.lib.clock.Clock`, the clock the profiler reads.
///
/// PORT: `lib/clock` is ported in `blaze_events` (above this crate); the runtime hands the
/// profiler that clock through this trait so that profile timestamps and the times other code
/// passes in (`nanoTimeMaybe`, critical path components) come from the same origin.
// Java: com/google/devtools/build/lib/clock/Clock.java:19 Clock
pub trait Clock: Send + Sync {
    /// Returns the current time in milliseconds since the epoch.
    fn current_time_millis(&self) -> i64;
    /// Returns the current time in nanoseconds, for measuring elapsed time.
    fn nano_time(&self) -> i64;
}

/// Collector used by the profiler to collect local resources.
// Java: com/google/devtools/build/lib/profiler/LocalResourceCollector.java:19 LocalResourceCollector
pub trait LocalResourceCollector: Send + Sync {
    /// Start collecting.
    fn start(&self);
    /// Register a counter series collector.
    fn register_counter_series_collector(&self, collector: Box<dyn CounterSeriesCollector>);
    /// Stop collecting.
    fn stop(&self);
}

/// Interface for collecting counter series.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:696 Profiler.CounterSeriesCollector
pub trait CounterSeriesCollector: Send {
    fn collect(&mut self, delta_nanos: f64, consumer: &mut dyn FnMut(&CounterSeriesTask, f64));
}

/// A counter series map in insertion order (`LinkedHashMap<CounterSeriesTask, double[]>` /
/// `ImmutableMap`).
pub type CounterSeriesMap = Vec<(CounterSeriesTask, Vec<f64>)>;

/// File format enum.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:119 Profiler.Format
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    JSON_TRACE_FILE_FORMAT,
    JSON_TRACE_FILE_COMPRESSED_FORMAT,
}

/// A task that was very slow.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:125 Profiler.SlowTask
#[derive(Clone, Debug, PartialEq)]
pub struct SlowTask {
    duration_nanos: i64,
    description: String,
    type_: Option<ProfilerTask>,
}

impl SlowTask {
    fn new(task_data: &TaskData) -> Self {
        SlowTask {
            duration_nanos: task_data.duration_nanos,
            description: task_data.description.clone(),
            type_: task_data.type_,
        }
    }

    pub fn get_duration_nanos(&self) -> i64 {
        self.duration_nanos
    }

    pub fn get_description(&self) -> &str {
        &self.description
    }

    pub fn get_type(&self) -> Option<ProfilerTask> {
        self.type_
    }
}

/// `Extrema.max(SIZE)` of slow tasks: the largest elements, sorted from most extreme to least.
// Java: com/google/devtools/build/lib/collect/Extrema.java:28 Extrema
#[derive(Default)]
struct MaxExtrema {
    k: usize,
    elements: Vec<SlowTask>,
}

impl MaxExtrema {
    fn new(k: usize) -> Self {
        MaxExtrema {
            k,
            elements: Vec::new(),
        }
    }

    fn aggregate(&mut self, element: SlowTask) {
        if self.k == 0 {
            return;
        }
        if self.elements.len() == self.k
            && self.elements.last().unwrap().duration_nanos >= element.duration_nanos
        {
            return;
        }
        let pos = self
            .elements
            .partition_point(|e| e.duration_nanos >= element.duration_nanos);
        self.elements.insert(pos, element);
        self.elements.truncate(self.k);
    }

    fn clear(&mut self) {
        self.elements.clear();
    }
}

/// Aggregator class that keeps track of the slowest tasks of the specified type.
///
/// `extrema_aggregators` is sharded so that all threads need not compete for the same lock if
/// they do the same operation at the same time.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:301 Profiler.SlowestTaskAggregator
struct SlowestTaskAggregator {
    extrema_aggregators: Vec<Mutex<MaxExtrema>>,
}

const SHARDS: usize = 16;
const SIZE: usize = 30;

impl SlowestTaskAggregator {
    fn new() -> Self {
        SlowestTaskAggregator {
            extrema_aggregators: (0..SHARDS)
                .map(|_| Mutex::new(MaxExtrema::new(SIZE)))
                .collect(),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:315 Profiler.SlowestTaskAggregator.add
    fn add(&self, task_data: &TaskData) {
        let shard = task_data.thread_id.rem_euclid(SHARDS as i64) as usize;
        self.extrema_aggregators[shard]
            .lock()
            .aggregate(SlowTask::new(task_data));
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:323 Profiler.SlowestTaskAggregator.clear
    fn clear(&self) {
        for extrema in &self.extrema_aggregators {
            extrema.lock().clear();
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:333 Profiler.SlowestTaskAggregator.getSlowestTasks
    fn get_slowest_tasks(&self) -> Vec<SlowTask> {
        // This is slow, but since it only happens during the end of the invocation, it's OK.
        let mut merged_extrema = MaxExtrema::new(SIZE);
        for extrema in &self.extrema_aggregators {
            for task in &extrema.lock().elements {
                merged_extrema.aggregate(task.clone());
            }
        }
        merged_extrema.elements
    }
}

const HISTOGRAM_BUCKETS: i32 = 20;

const ACTION_COUNT_BUCKET_DURATION: Duration = Duration::from_millis(200);

/// The state `start` sets and `stop` / `clear` reset.
#[derive(Default)]
struct Inner {
    clock: Option<Arc<dyn Clock>>,
    /// The reference to the current writer, if any. If the referenced writer is null, then disk
    /// writes are disabled. This can happen when slowest task recording is enabled.
    writer: Option<Arc<JsonTraceFileWriter>>,
    profile_cpu_start_time: i64,
    profile_cpu_end_time: i64,
    profile_start_time: i64,
    profile_end_time: i64,
    /// Collects local cpu usage data (if enabled).
    local_resource_collector: Option<Arc<dyn LocalResourceCollector>>,
    action_count_time_series: Option<Arc<TimeSeries>>,
    action_cache_count_time_series: Option<Arc<TimeSeries>>,
    local_action_count_time_series: Option<Arc<TimeSeries>>,
    /// `ConcurrentHashMap<String, TimeSeries>`, with its insertion order (iteration follows
    /// `java_hash_map_order`).
    inflight_rpc_time_series_map: Option<Arc<Mutex<Vec<(String, Arc<TimeSeries>)>>>>,
    action_count_start_time: i64,
    collect_task_histograms: bool,
    include_primary_output: bool,
    include_target_label: bool,
    include_configuration: bool,
}

/// Blaze internal profiler. Provides facility to report various Blaze tasks and store them
/// (asynchronously) in the file for future analysis.
///
/// Implemented as singleton so any caller should use `Profiler::instance()` to obtain reference.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:110 Profiler
pub struct Profiler {
    active: AtomicBool,
    record_all_durations: AtomicBool,
    /// `profiledTasks`, as a bit set (low and high 64 tasks).
    profiled_tasks_lo: AtomicU64,
    profiled_tasks_hi: AtomicU64,
    inner: RwLock<Inner>,
    /// The monitor of Java's `synchronized` methods.
    sync: Mutex<()>,
    slowest_tasks: Vec<Option<SlowestTaskAggregator>>,
    tasks_histograms: RwLock<Vec<Option<Box<dyn StatRecorder>>>>,
    next_lane_id: AtomicI64,
    multi_lane_generator: MultiLaneGenerator,
}

static INSTANCE: OnceLock<Profiler> = OnceLock::new();

/// `getProcessCpuTime()`: `OperatingSystemMXBean.getProcessCpuTime()`, the CPU time of the
/// process in nanoseconds.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:452 Profiler.getProcessCpuTime
pub fn get_process_cpu_time_nanos() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes into the timespec only.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    if rc != 0 {
        return -1;
    }
    ts.tv_sec as i64 * 1_000_000_000 + ts.tv_nsec as i64
}

/// The returned scope of [`Profiler::profile`]; ends the task when dropped or closed.
// Java: com/google/devtools/build/lib/profiler/SilentCloseable.java:17 SilentCloseable
#[must_use = "the task ends when this is dropped"]
pub struct SilentCloseable {
    scope: Option<Box<Scope>>,
}

enum Scope {
    Task {
        start_time_nanos: i64,
        type_: ProfilerTask,
        description: String,
    },
    Action {
        start_time_nanos: i64,
        type_: ProfilerTask,
        description: String,
        mnemonic: Option<String>,
        primary_output: Option<String>,
        target_label: Option<String>,
        configuration: Option<String>,
    },
    LaneTask {
        lane_id: i64,
        start_time_nanos: i64,
        type_: ProfilerTask,
        description: String,
    },
}

impl SilentCloseable {
    /// `NOP`.
    pub const fn nop() -> Self {
        SilentCloseable { scope: None }
    }

    /// `close()`.
    pub fn close(self) {}
}

impl Drop for SilentCloseable {
    fn drop(&mut self) {
        let Some(scope) = self.scope.take() else {
            return;
        };
        let profiler = Profiler::instance();
        match *scope {
            Scope::Task {
                start_time_nanos,
                type_,
                description,
            } => {
                // Java: com/google/devtools/build/lib/profiler/Profiler.java:829 Profiler.reallyProfile
                profiler.complete_task_on_lane(
                    current_thread_id(),
                    start_time_nanos,
                    type_,
                    description,
                );
            }
            Scope::Action {
                start_time_nanos,
                type_,
                description,
                mnemonic,
                primary_output,
                target_label,
                configuration,
            } => {
                // Java: com/google/devtools/build/lib/profiler/Profiler.java:908 Profiler.profileAction
                profiler.complete_action(
                    current_thread_id(),
                    start_time_nanos,
                    type_,
                    description,
                    mnemonic,
                    primary_output,
                    target_label,
                    configuration,
                );
            }
            Scope::LaneTask {
                lane_id,
                start_time_nanos,
                type_,
                description,
            } => {
                profiler.complete_task_on_lane(lane_id, start_time_nanos, type_, description);
            }
        }
    }
}

impl Profiler {
    fn new() -> Self {
        let slowest_tasks = ProfilerTask::VALUES
            .iter()
            .map(|task| {
                if task.collects_slowest_instances() {
                    Some(SlowestTaskAggregator::new())
                } else {
                    None
                }
            })
            .collect();
        let profiler = Profiler {
            active: AtomicBool::new(false),
            record_all_durations: AtomicBool::new(false),
            profiled_tasks_lo: AtomicU64::new(0),
            profiled_tasks_hi: AtomicU64::new(0),
            inner: RwLock::new(Inner::default()),
            sync: Mutex::new(()),
            slowest_tasks,
            tasks_histograms: RwLock::new(Vec::new()),
            next_lane_id: AtomicI64::new(1_000_000),
            multi_lane_generator: MultiLaneGenerator::default(),
        };
        profiler.init_histograms();
        profiler
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:396 Profiler.initHistograms
    fn init_histograms(&self) {
        let mut histograms: Vec<Option<Box<dyn StatRecorder>>> = Vec::new();
        for task in ProfilerTask::VALUES {
            if task.is_vfs() {
                let vfs_heuristics = vfs_type_heuristics();
                let mut recorders: Vec<(
                    Box<dyn StatRecorder>,
                    super::stat_recorder::StringPredicate,
                )> = Vec::with_capacity(vfs_heuristics.len());
                for (key, predicate) in vfs_heuristics {
                    recorders.push((
                        Box::new(SingleStatRecorder::new(
                            Some(format!("{task} {key}")),
                            HISTOGRAM_BUCKETS,
                        )),
                        predicate,
                    ));
                }
                histograms.push(Some(Box::new(PredicateBasedStatRecorder::new(recorders))));
            } else {
                histograms.push(Some(Box::new(SingleStatRecorder::new(
                    Some(task.to_string()),
                    HISTOGRAM_BUCKETS,
                ))));
            }
        }
        *self.tasks_histograms.write() = histograms;
    }

    /// Runs `f` on the task histograms. This must be called between calls to `start` and
    /// `stop`, or the returned recorders are all empty. The stat recorders are indexed by
    /// `ProfilerTask#ordinal`.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:425 Profiler.getTasksHistograms
    pub fn with_tasks_histograms<R>(
        &self,
        f: impl FnOnce(&[Option<Box<dyn StatRecorder>>]) -> R,
    ) -> R {
        let _sync = self.sync.lock();
        crate::check_state!(self.is_active());
        f(&self.tasks_histograms.read())
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:430 Profiler.instance
    pub fn instance() -> &'static Profiler {
        INSTANCE.get_or_init(Profiler::new)
    }

    fn clock(&self) -> Option<Arc<dyn Clock>> {
        self.inner.read().clock.clone()
    }

    fn now(&self) -> i64 {
        self.inner
            .read()
            .clock
            .as_ref()
            .expect("NullPointerException: Profiler.clock")
            .nano_time()
    }

    /// Returns the nanoTime of the current profiler instance, or an arbitrary constant if not
    /// active.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:437 Profiler.nanoTimeMaybe
    pub fn nano_time_maybe() -> i64 {
        let instance = Self::instance();
        if instance.is_active() {
            if let Some(clock) = instance.clock() {
                return clock.nano_time();
            }
        }
        -1
    }

    /// Returns the elapsed wall clock time since the profile has been started.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:445 Profiler.getProfileElapsedTime
    pub fn get_profile_elapsed_time_nanos() -> i64 {
        let instance = Self::instance();
        let inner = instance.inner.read();
        let end_time = if instance.is_active() {
            inner.clock.as_ref().map_or(0, |c| c.nano_time())
        } else {
            inner.profile_end_time
        };
        end_time - inner.profile_start_time
    }

    /// Returns the CPU time since the profile has been started.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:459 Profiler.getServerProcessCpuTime
    pub fn get_server_process_cpu_time_nanos() -> i64 {
        let instance = Self::instance();
        let inner = instance.inner.read();
        let cpu_end_time = if instance.is_active() {
            get_process_cpu_time_nanos()
        } else {
            inner.profile_cpu_end_time
        };
        cpu_end_time - inner.profile_cpu_start_time
    }

    /// Enable profiling.
    ///
    /// Subsequent calls to beginTask/endTask will be recorded in the provided output stream.
    ///
    /// `record_all_durations`: iff true, record all tasks regardless of their duration;
    /// otherwise some tasks may get aggregated if they finished quick enough.
    /// `exec_start_time_nanos`: execution start time in nanos obtained from `clock.nano_time()`.
    ///
    /// PORT: `bazel_version` is what Java's writer reads from `BlazeVersionInfo` (see
    /// [`JsonTraceFileWriter::new`]); `build_id` is the command id's `toString()`.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:478 Profiler.start
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        profiled_tasks: ProfilerTaskSet,
        stream: Option<Box<dyn io::Write + Send>>,
        format: Option<Format>,
        output_base: String,
        build_id: String,
        record_all_durations: bool,
        clock: Arc<dyn Clock>,
        exec_start_time_nanos: i64,
        slim_profile: bool,
        include_primary_output: bool,
        include_target_label: bool,
        include_configuration: bool,
        collect_task_histograms: bool,
        local_resource_collector: Arc<dyn LocalResourceCollector>,
        bazel_version: String,
    ) -> io::Result<()> {
        let _sync = self.sync.lock();
        crate::check_state!(
            !self.active.load(Ordering::SeqCst),
            "Profiler already active"
        );

        self.init_histograms();

        let bits = profiled_tasks.bits();
        self.profiled_tasks_lo.store(bits as u64, Ordering::SeqCst);
        self.profiled_tasks_hi
            .store((bits >> 64) as u64, Ordering::SeqCst);
        let action_count_start_time = clock.nano_time();
        let new_series = || {
            Some(Arc::new(TimeSeries::new_nanos(
                action_count_start_time,
                ACTION_COUNT_BUCKET_DURATION,
            )))
        };

        let mut writer = None;
        if let (Some(stream), Some(format)) = (stream, format) {
            // BlazeClock.createNanosToMillisSinceEpochConverter(clock).toEpochMillis(nanos).
            let profile_start_epoch_millis = clock.current_time_millis()
                - (clock.nano_time() - exec_start_time_nanos).div_euclid(1_000_000);
            let out = match format {
                Format::JSON_TRACE_FILE_FORMAT => ProfileOutputStream::Plain(stream),
                Format::JSON_TRACE_FILE_COMPRESSED_FORMAT => ProfileOutputStream::gzip(stream),
            };
            let w = Arc::new(JsonTraceFileWriter::new(
                out,
                exec_start_time_nanos,
                profile_start_epoch_millis,
                slim_profile,
                output_base,
                build_id,
                bazel_version,
            ));
            w.start();
            writer = Some(w);
        }

        {
            let mut inner = self.inner.write();
            inner.clock = Some(clock);
            inner.action_count_start_time = action_count_start_time;
            inner.action_count_time_series = new_series();
            inner.action_cache_count_time_series = new_series();
            inner.local_action_count_time_series = new_series();
            inner.inflight_rpc_time_series_map = Some(Arc::new(Mutex::new(Vec::new())));
            inner.collect_task_histograms = collect_task_histograms;
            inner.include_primary_output = include_primary_output;
            inner.include_target_label = include_target_label;
            inner.include_configuration = include_configuration;
            inner.writer = writer;
            // Activate profiler.
            inner.profile_start_time = exec_start_time_nanos;
            inner.profile_cpu_start_time = get_process_cpu_time_nanos();
            inner.local_resource_collector = Some(local_resource_collector.clone());
        }
        self.record_all_durations
            .store(record_all_durations, Ordering::SeqCst);
        self.active.store(true, Ordering::SeqCst);

        // Start collecting Bazel and system-wide CPU metric collection.
        local_resource_collector.start();
        Ok(())
    }

    /// Returns the slowest tasks. This must be called between calls to `start` and `stop`, or
    /// the returned list is empty.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:557 Profiler.getSlowestTasks
    pub fn get_slowest_tasks(&self) -> Vec<SlowTask> {
        let _sync = self.sync.lock();
        let mut slowest_tasks_by_type = Vec::new();
        for aggregator in self.slowest_tasks.iter().flatten() {
            slowest_tasks_by_type.extend(aggregator.get_slowest_tasks());
        }
        slowest_tasks_by_type
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:569 Profiler.collectActionCounts
    fn collect_action_counts(&self) {
        let (clock, action_count_start_time) = {
            let inner = self.inner.read();
            (inner.clock.clone().unwrap(), inner.action_count_start_time)
        };
        let end_time = clock.nano_time();
        let bucket = ACTION_COUNT_BUCKET_DURATION.as_nanos() as i64;
        let len = ((end_time - action_count_start_time) / bucket) as i32 + 1;
        let len = len.max(0) as usize;
        let mut counter_series_map: CounterSeriesMap = Vec::new();
        let action_count_time_series = self.inner.write().action_count_time_series.take();
        if let Some(series) = action_count_time_series {
            counter_series_map.push((
                CounterSeriesTask::new("action count", "action", None),
                series.to_double_array(len),
            ));
        }
        let action_cache_count_time_series =
            self.inner.write().action_cache_count_time_series.take();
        if let Some(series) = action_cache_count_time_series {
            counter_series_map.push((
                CounterSeriesTask::new("action count", "local action cache", None),
                series.to_double_array(len),
            ));
        }
        if !counter_series_map.is_empty() {
            self.log_counters(
                counter_series_map,
                action_count_start_time,
                ACTION_COUNT_BUCKET_DURATION,
            );
        }

        let mut local_counter_series_map: CounterSeriesMap = Vec::new();
        let local_action_count_time_series =
            self.inner.write().local_action_count_time_series.take();
        if let Some(series) = local_action_count_time_series {
            local_counter_series_map.push((
                CounterSeriesTask::new(
                    "action count (local)",
                    "local action",
                    Some(CounterSeriesTaskColor::DETAILED_MEMORY_DUMP),
                ),
                series.to_double_array(len),
            ));
        }
        if has_non_zero_values(&local_counter_series_map) {
            self.log_counters(
                local_counter_series_map,
                action_count_start_time,
                ACTION_COUNT_BUCKET_DURATION,
            );
        }

        let inflight_rpc_time_series_map = self.inner.write().inflight_rpc_time_series_map.take();
        if let Some(map) = inflight_rpc_time_series_map {
            // PORT: java-hash-order (a ConcurrentHashMap<String, TimeSeries>).
            let entries = map.lock().clone();
            let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
            for name in java_hash_map_order(&names, |s| string_hash_code(s), Insertion::Put) {
                let time_series = &entries.iter().find(|(n, _)| *n == name).unwrap().1;
                let values = time_series.to_double_array(len);
                self.log_counters(
                    vec![(
                        CounterSeriesTask::new(format!("Inflight RPCs - {name}"), name, None),
                        values,
                    )],
                    action_count_start_time,
                    ACTION_COUNT_BUCKET_DURATION,
                );
            }
        }
    }

    /// Disable profiling and complete profile file creation. Subsequent calls to
    /// beginTask/endTask will no longer be recorded in the profile.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:632 Profiler.stop
    pub fn stop(&self) -> io::Result<()> {
        let _sync = self.sync.lock();
        if !self.active.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.collect_action_counts();
        let collector = self.inner.read().local_resource_collector.clone();
        if let Some(collector) = collector {
            collector.stop();
        }

        // Log a final event to update the duration of ProfilePhase.FINISH.
        self.log_event(ProfilerTask::INFO, "Finishing");
        let writer = self.inner.write().writer.take();
        let result = match writer {
            Some(writer) => writer.shutdown(),
            None => Ok(()),
        };
        {
            let mut inner = self.inner.write();
            inner.profile_cpu_end_time = get_process_cpu_time_nanos();
            inner.profile_end_time = inner.clock.as_ref().unwrap().nano_time();
        }
        self.active.store(false, Ordering::SeqCst);
        result
    }

    /// Clears the records the profiler instance keeps.
    ///
    /// Should always be called between a `stop()` and a subsequent `start`.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:660 Profiler.clear
    pub fn clear(&self) {
        let _sync = self.sync.lock();
        crate::check_state!(!self.active.load(Ordering::SeqCst));

        for h in self.tasks_histograms.write().iter_mut() {
            *h = None;
        }
        {
            let mut inner = self.inner.write();
            inner.profile_start_time = 0;
            inner.profile_end_time = 0;
            inner.profile_cpu_start_time = 0;
            inner.profile_cpu_end_time = 0;
        }

        for aggregator in self.slowest_tasks.iter().flatten() {
            aggregator.clear();
        }

        self.multi_lane_generator.reset();
    }

    /// Returns true iff profiling is currently enabled.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:679 Profiler.isActive
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:683 Profiler.isProfiling
    #[inline]
    pub fn is_profiling(&self, type_: ProfilerTask) -> bool {
        let ordinal = type_.ordinal();
        if ordinal < 64 {
            self.profiled_tasks_lo.load(Ordering::Relaxed) & (1u64 << ordinal) != 0
        } else {
            self.profiled_tasks_hi.load(Ordering::Relaxed) & (1u64 << (ordinal - 64)) != 0
        }
    }

    /// Unless --record_full_profiler_data is given we drop small tasks and add their time to the
    /// parents duration.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:691 Profiler.wasTaskSlowEnoughToRecord
    fn was_task_slow_enough_to_record(&self, type_: ProfilerTask, duration: i64) -> bool {
        self.record_all_durations.load(Ordering::Relaxed) || duration >= type_.min_duration()
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:700 Profiler.registerCounterSeriesCollector
    pub fn register_counter_series_collector(&self, collector: Box<dyn CounterSeriesCollector>) {
        if !self.is_active() {
            return;
        }

        let local_resource_collector = self.inner.read().local_resource_collector.clone();
        if let Some(c) = local_resource_collector {
            c.register_counter_series_collector(collector);
        }
    }

    /// Adds a whole action count series to the writer bypassing histogram and subtask creation.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:709 Profiler.logCounters
    pub fn log_counters(
        &self,
        counter_series_map: CounterSeriesMap,
        profile_start_nanos: i64,
        bucket_duration: Duration,
    ) {
        let current_writer = self.inner.read().writer.clone();
        if self.is_active() {
            if let Some(current_writer) = current_writer {
                let counter_series_trace_data = CounterSeriesTraceData::new(
                    counter_series_map,
                    profile_start_nanos,
                    bucket_duration,
                );
                current_writer.enqueue(QueuedTraceData::Other(Box::new(counter_series_trace_data)));
            }
        }
    }

    /// Adds task directly to the main queue bypassing task stack. Used for simple tasks that are
    /// known to not have any subtasks.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:730 Profiler.logTask
    fn log_task(
        &self,
        start_time_nanos: i64,
        mut duration: i64,
        type_: ProfilerTask,
        description: &str,
    ) {
        // PORT: borrowLane() is null on a platform thread; Rust threads are never virtual.
        crate::check_state!(!description.is_empty(), "No description -> not helpful");
        if duration < 0 {
            // See note in Clock#nanoTime, which is used by Profiler#nanoTimeMaybe.
            duration = 0;
        }

        let (collect_task_histograms, current_writer) = {
            let inner = self.inner.read();
            (inner.collect_task_histograms, inner.writer.clone())
        };
        if collect_task_histograms {
            if let Some(Some(stat_recorder)) = self.tasks_histograms.read().get(type_.ordinal()) {
                stat_recorder.add_stat((duration / 1_000_000) as i32, description);
            }
        }

        if self.is_active() && start_time_nanos >= 0 && self.is_profiling(type_) {
            // Store instance fields as local variables so they are not nulled out from under us
            // by #clear.
            if self.was_task_slow_enough_to_record(type_, duration) {
                let data = TaskData::new(
                    current_thread_id(),
                    start_time_nanos,
                    duration,
                    Some(type_),
                    description.to_string(),
                );
                if let Some(aggregator) = &self.slowest_tasks[type_.ordinal()] {
                    aggregator.add(&data);
                }
                if let Some(current_writer) = current_writer {
                    current_writer.enqueue(QueuedTraceData::Task(data));
                }
            }
        }
    }

    /// Used externally to submit simple task (one that does not have any subtasks). Depending on
    /// the minDuration attribute of the task type, task may be just aggregated into the parent
    /// task and not stored directly.
    ///
    /// `start_time_nanos`: task start time (obtained through [`Profiler::nano_time_maybe`]).
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:777 Profiler.logSimpleTask
    pub fn log_simple_task(&self, start_time_nanos: i64, type_: ProfilerTask, description: &str) {
        if let Some(clock) = self.clock() {
            self.log_task(
                start_time_nanos,
                clock.nano_time() - start_time_nanos,
                type_,
                description,
            );
        }
    }

    /// Note that start and stop time must both be acquired from the same clock instance.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:795 Profiler.logSimpleTask
    pub fn log_simple_task_start_stop(
        &self,
        start_time_nanos: i64,
        stop_time_nanos: i64,
        type_: ProfilerTask,
        description: &str,
    ) {
        self.log_task(
            start_time_nanos,
            stop_time_nanos - start_time_nanos,
            type_,
            description,
        );
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:810 Profiler.logSimpleTaskDuration
    pub fn log_simple_task_duration(
        &self,
        start_time_nanos: i64,
        duration_nanos: i64,
        type_: ProfilerTask,
        description: &str,
    ) {
        self.log_task(start_time_nanos, duration_nanos, type_, description);
    }

    /// Used to log "events" happening at a specific time - tasks with zero duration.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:816 Profiler.logEventAtTime
    pub fn log_event_at_time(&self, at_time_nanos: i64, type_: ProfilerTask, description: &str) {
        self.log_task(at_time_nanos, 0, type_, description);
    }

    /// Used to log "events" - tasks with zero duration.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:822 Profiler.logEvent
    pub fn log_event(&self, type_: ProfilerTask, description: &str) {
        self.log_event_at_time(self.now(), type_, description);
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:826 Profiler.reallyProfile
    fn really_profile(&self, type_: ProfilerTask, description: String) -> SilentCloseable {
        let start_time_nanos = self.now();
        SilentCloseable {
            scope: Some(Box::new(Scope::Task {
                start_time_nanos,
                type_,
                description,
            })),
        }
    }

    /// Records the beginning of a task as specified, and returns a [`SilentCloseable`] that ends
    /// the task when dropped.
    ///
    /// Use of this method allows to support nested task monitoring. For tasks that are known to
    /// not have any subtasks, `log_simple_task()` should be used instead.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:857 Profiler.profile
    #[inline]
    pub fn profile(&self, type_: ProfilerTask, description: &str) -> SilentCloseable {
        if self.is_active() && self.is_profiling(type_) {
            self.really_profile(type_, description.to_string())
        } else {
            SilentCloseable::nop()
        }
    }

    /// Version of [`Profiler::profile`] that avoids creating string unless actually profiling.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:865 Profiler.profile
    #[inline]
    pub fn profile_with(
        &self,
        type_: ProfilerTask,
        description: impl FnOnce() -> String,
    ) -> SilentCloseable {
        if self.is_active() && self.is_profiling(type_) {
            self.really_profile(type_, description())
        } else {
            SilentCloseable::nop()
        }
    }

    /// Convenience method that uses [`ProfilerTask::INFO`].
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:889 Profiler.profile
    #[inline]
    pub fn profile_info(&self, description: &str) -> SilentCloseable {
        self.profile(ProfilerTask::INFO, description)
    }

    /// Similar to [`Profiler::profile`], but specific to action-related events. Takes an extra
    /// argument: primaryOutput.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:897 Profiler.profileAction
    pub fn profile_action(
        &self,
        type_: ProfilerTask,
        mnemonic: Option<&str>,
        description: &str,
        primary_output: Option<&str>,
        target_label: Option<&str>,
        configuration: Option<&str>,
    ) -> SilentCloseable {
        if self.is_active() && self.is_profiling(type_) {
            let start_time_nanos = self.now();
            let (include_primary_output, include_target_label, include_configuration) = {
                let inner = self.inner.read();
                (
                    inner.include_primary_output,
                    inner.include_target_label,
                    inner.include_configuration,
                )
            };
            SilentCloseable {
                scope: Some(Box::new(Scope::Action {
                    start_time_nanos,
                    type_,
                    description: description.to_string(),
                    mnemonic: mnemonic.map(str::to_string),
                    primary_output: if include_primary_output {
                        primary_output.map(str::to_string)
                    } else {
                        None
                    },
                    target_label: if include_target_label {
                        target_label.map(str::to_string)
                    } else {
                        None
                    },
                    configuration: if include_configuration {
                        configuration.map(str::to_string)
                    } else {
                        None
                    },
                })),
            }
        } else {
            SilentCloseable::nop()
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:930 Profiler.countAction
    fn count_action(type_: ProfilerTask) -> bool {
        type_ == ProfilerTask::ACTION || type_ == ProfilerTask::DISCOVER_INPUTS
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:934 Profiler.completeTask
    pub fn complete_task(&self, start_time_nanos: i64, type_: ProfilerTask, description: &str) {
        self.complete_task_on_lane(
            current_thread_id(),
            start_time_nanos,
            type_,
            description.to_string(),
        );
    }

    /// Records the end of the task.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:944 Profiler.completeTask
    fn complete_task_on_lane(
        &self,
        lane_id: i64,
        start_time_nanos: i64,
        type_: ProfilerTask,
        description: String,
    ) {
        if self.is_active() {
            let end_time_nanos = self.now();
            let duration = end_time_nanos - start_time_nanos;
            let rpc_map = if type_ == ProfilerTask::RPC {
                let inner = self.inner.read();
                inner
                    .inflight_rpc_time_series_map
                    .clone()
                    .map(|m| (m, inner.action_count_start_time))
            } else {
                None
            };
            if let Some((inflight_rpc_time_serie_map, action_count_start_time)) = rpc_map {
                let time_series = {
                    let mut map = inflight_rpc_time_serie_map.lock();
                    match map.iter().find(|(n, _)| *n == description) {
                        Some((_, ts)) => ts.clone(),
                        None => {
                            let ts = Arc::new(TimeSeries::new_nanos(
                                action_count_start_time,
                                ACTION_COUNT_BUCKET_DURATION,
                            ));
                            map.push((description.clone(), ts.clone()));
                            ts
                        }
                    }
                };
                time_series.add_range_nanos(start_time_nanos, end_time_nanos, 1.0);
            }
            if self.was_task_slow_enough_to_record(type_, duration) {
                self.record_task(TaskData::new(
                    lane_id,
                    start_time_nanos,
                    duration,
                    Some(type_),
                    description,
                ));
            }
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:966 Profiler.completeAction
    #[allow(clippy::too_many_arguments)]
    fn complete_action(
        &self,
        thread_id: i64,
        start_time_nanos: i64,
        type_: ProfilerTask,
        description: String,
        mnemonic: Option<String>,
        primary_output: Option<String>,
        target_label: Option<String>,
        configuration: Option<String>,
    ) {
        if self.is_active() {
            let end_time_nanos = self.now();
            let duration = end_time_nanos - start_time_nanos;
            let should_record_task = self.was_task_slow_enough_to_record(type_, duration);
            if should_record_task {
                self.record_task(TaskData::new_action(
                    thread_id,
                    start_time_nanos,
                    duration,
                    type_,
                    mnemonic,
                    description,
                    primary_output,
                    target_label,
                    configuration,
                ));
            }
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:995 Profiler.recordTask
    fn record_task(&self, data: TaskData) {
        let (writer, action_count, action_cache_count, local_action_count) = {
            let inner = self.inner.read();
            (
                inner.writer.clone(),
                inner.action_count_time_series.clone(),
                inner.action_cache_count_time_series.clone(),
                inner.local_action_count_time_series.clone(),
            )
        };
        let start_time_nanos = data.start_time_nanos;
        let end_time_nanos = data.start_time_nanos + data.duration_nanos;
        let type_ = data.type_.expect("type");
        if let Some(aggregator) = &self.slowest_tasks[type_.ordinal()] {
            aggregator.add(&data);
        }
        if let Some(writer) = writer {
            writer.enqueue(QueuedTraceData::Task(data));
        }
        if let Some(series) = action_count {
            if Self::count_action(type_) {
                series.add_range_nanos(start_time_nanos, end_time_nanos, 1.0);
            }
        }
        if let Some(series) = action_cache_count {
            if type_ == ProfilerTask::ACTION_CHECK {
                series.add_range_nanos(start_time_nanos, end_time_nanos, 1.0);
            }
        }

        if let Some(series) = local_action_count {
            if type_ == ProfilerTask::LOCAL_ACTION_COUNTS {
                series.add_range_nanos(start_time_nanos, end_time_nanos, 1.0);
            }
        }
    }

    /// Convenience method to log phase marker tasks.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1024 Profiler.markPhase
    pub fn mark_phase(&self, phase: ProfilePhase) {
        crate::profiler::memory_profiler::MemoryProfiler::instance().mark_phase(phase);
        if self.is_active() && self.is_profiling(ProfilerTask::PHASE) {
            self.log_event(ProfilerTask::PHASE, phase.description());
        }
    }

    /// Profiles an asynchronous operation (`profileFuture`): returns the scope the operation's
    /// completion closes, on a virtual lane named after `prefix`.
    ///
    /// PORT: Java adds a listener to a `ListenableFuture`; the caller drops the returned scope
    /// where the future completes.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1187 Profiler.profileFuture
    pub fn profile_future(
        &self,
        prefix: &str,
        type_: ProfilerTask,
        description: &str,
    ) -> LaneScope {
        let Some(lane) = self.multi_lane_generator.acquire(self, prefix) else {
            return LaneScope { inner: None };
        };
        let start_time_nanos = self.now();
        LaneScope {
            inner: Some((lane, start_time_nanos, type_, description.to_string())),
        }
    }

    /// Creates a profiler that can be used to profile async operations of a task.
    ///
    /// `prefix`: the prefix of the virtual lanes. Similar to the thread name prefix.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1286 Profiler.profileAsync
    pub fn profile_async(&self, prefix: &str, description: &str) -> AsyncProfiler {
        AsyncProfiler {
            lane: self.multi_lane_generator.acquire(self, prefix),
            start_time_nanos: self.clock().map_or(0, |c| c.nano_time()),
            description: description.to_string(),
        }
    }
}

// Java: com/google/devtools/build/lib/profiler/Profiler.java:622 Profiler.hasNonZeroValues
fn has_non_zero_values(counters_series_map: &CounterSeriesMap) -> bool {
    counters_series_map
        .iter()
        .flat_map(|(_, v)| v.iter())
        .any(|v| *v != 0.0)
}

/// A virtual lane (`Profiler.Lane`).
// Java: com/google/devtools/build/lib/profiler/Profiler.java:1060 Profiler.Lane
struct Lane {
    generator: Arc<LaneGenerator>,
    id: i64,
}

// Java: com/google/devtools/build/lib/profiler/Profiler.java:1076 Profiler.LaneGenerator
struct LaneGenerator {
    prefix: String,
    available_lanes: Mutex<VecDeque<i64>>,
    count: AtomicI32,
}

impl LaneGenerator {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1087 Profiler.LaneGenerator.acquire
    fn acquire(self: &Arc<Self>, profiler: &Profiler) -> Lane {
        let polled = self.available_lanes.lock().pop_front();
        // It might create more virtual lanes, but it's fine for our purpose.
        let id = match polled {
            Some(id) => id,
            None => {
                let id = profiler.next_lane_id.fetch_add(1, Ordering::SeqCst);
                let new_lane_index = self.count.fetch_add(1, Ordering::SeqCst);
                let new_lane_name = if self.prefix.ends_with('-') {
                    format!("{}{} (Virtual)", self.prefix, new_lane_index)
                } else {
                    format!("{}-{} (Virtual)", self.prefix, new_lane_index)
                };
                let thread_metadata = ThreadMetadata::new(new_lane_name, id);
                let writer = profiler.inner.read().writer.clone();
                if let Some(writer) = writer {
                    writer.enqueue(QueuedTraceData::Other(Box::new(thread_metadata)));
                }
                id
            }
        };
        Lane {
            generator: self.clone(),
            id,
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1109 Profiler.LaneGenerator.release
    fn release(&self, lane_id: i64) {
        self.available_lanes.lock().push_back(lane_id);
    }
}

// Java: com/google/devtools/build/lib/profiler/Profiler.java:1034 Profiler.MultiLaneGenerator
#[derive(Default)]
struct MultiLaneGenerator {
    lane_generators: Mutex<Vec<(String, Arc<LaneGenerator>)>>,
}

impl MultiLaneGenerator {
    /// Returns the lane if the profiler is active, otherwise `None`.
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1041 Profiler.MultiLaneGenerator.acquire
    fn acquire(&self, profiler: &Profiler, prefix: &str) -> Option<Lane> {
        if !profiler.is_active() {
            return None;
        }
        let lane_generator = {
            let mut generators = self.lane_generators.lock();
            match generators.iter().find(|(p, _)| p == prefix) {
                Some((_, g)) => g.clone(),
                None => {
                    let g = Arc::new(LaneGenerator {
                        prefix: prefix.to_string(),
                        available_lanes: Mutex::new(VecDeque::new()),
                        count: AtomicI32::new(0),
                    });
                    generators.push((prefix.to_string(), g.clone()));
                    g
                }
            }
        };
        Some(lane_generator.acquire(profiler))
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1055 Profiler.MultiLaneGenerator.reset
    fn reset(&self) {
        self.lane_generators.lock().clear();
    }
}

/// The scope [`Profiler::profile_future`] returns: completes the task on its lane and releases
/// the lane when dropped.
#[must_use = "the task ends when this is dropped"]
pub struct LaneScope {
    inner: Option<(Lane, i64, ProfilerTask, String)>,
}

impl Drop for LaneScope {
    fn drop(&mut self) {
        if let Some((lane, start_time_nanos, type_, description)) = self.inner.take() {
            Profiler::instance().complete_task_on_lane(
                lane.id,
                start_time_nanos,
                type_,
                description,
            );
            lane.generator.release(lane.id);
        }
    }
}

/// A profiler that can be used to profile async operations.
///
/// This profiler is thread-compatible but not thread-safe. You should create one profiler per
/// task.
// Java: com/google/devtools/build/lib/profiler/Profiler.java:1213 Profiler.AsyncProfiler
pub struct AsyncProfiler {
    lane: Option<Lane>,
    start_time_nanos: i64,
    description: String,
}

impl AsyncProfiler {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1224 Profiler.AsyncProfiler.profile
    pub fn profile(&self, type_: ProfilerTask, description: &str) -> SilentCloseable {
        let profiler = Profiler::instance();
        match &self.lane {
            Some(lane) if profiler.is_profiling(type_) => {
                let start_time_nanos = profiler.now();
                SilentCloseable {
                    scope: Some(Box::new(Scope::LaneTask {
                        lane_id: lane.id,
                        start_time_nanos,
                        type_,
                        description: description.to_string(),
                    })),
                }
            }
            _ => SilentCloseable::nop(),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1232 Profiler.AsyncProfiler.profile
    pub fn profile_info(&self, description: &str) -> SilentCloseable {
        self.profile(ProfilerTask::INFO, description)
    }
}

impl Drop for AsyncProfiler {
    // Java: com/google/devtools/build/lib/profiler/Profiler.java:1274 Profiler.AsyncProfiler.close
    fn drop(&mut self) {
        // PORT: Java dereferences a null lane (an inactive profiler) here; an AsyncProfiler made
        // while the profiler was inactive records nothing.
        if let Some(lane) = self.lane.take() {
            Profiler::instance().complete_task_on_lane(
                lane.id,
                self.start_time_nanos,
                ProfilerTask::INFO,
                std::mem::take(&mut self.description),
            );
            lane.generator.release(lane.id);
        }
    }
}
