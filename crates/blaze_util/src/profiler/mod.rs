//! Port of `com/google/devtools/build/lib/profiler` (the profiler that writes
//! `command.profile.gz`; decisions.md 2026-11-19, profiling ruling (3)).
//!
//! PORT: skipped, memory instrumentation: `MemoryProfiler`, `profiler/memory`. Skipped, JVM only:
//! `GoogleAutoProfilerUtils` (Google-internal logging) and the JVM heap and GC collectors of
//! `CollectLocalResourceUsage`. `CollectLocalResourceUsage` itself needs the worker metrics, the
//! resource manager and `/proc/meminfo` parsing of higher crates and lives in `blaze_runtime`.

pub mod auto_profiler;
pub mod counter_series;
pub mod java_hash_order;
pub mod json_profile;
pub mod json_trace_file_writer;
pub mod json_writer;
pub mod network_metrics_collector;
pub mod profile_phase;
#[allow(clippy::module_inception)]
pub mod profiler;
pub mod profiler_task;
pub mod stat_recorder;
pub mod thread_metadata;
pub mod trace_data;

pub use auto_profiler::AutoProfiler;
pub use counter_series::{CounterSeriesTask, CounterSeriesTaskColor, TimeSeries};
pub use profile_phase::ProfilePhase;
pub use profiler::{
    AsyncProfiler, Clock, CounterSeriesCollector, CounterSeriesMap, Format, LocalResourceCollector,
    Profiler, SilentCloseable, SlowTask,
};
pub use profiler_task::{ProfilerTask, ProfilerTaskSet};
