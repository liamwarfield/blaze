//! Port of `com/google/devtools/build/lib/profiler` (the profiler that writes
//! `command.profile.gz`; decisions.md 2026-11-19, profiling ruling (3)).
//!
//! `MemoryProfiler` is ported with the D15 process figures (`memory_profiler`). PORT: skipped,
//! memory instrumentation: `profiler/memory` (the allocation tracker). Skipped, JVM only:
//! `GoogleAutoProfilerUtils` (Google-internal logging) and the JVM heap and GC collectors of
//! `CollectLocalResourceUsage`. `CollectLocalResourceUsage` itself needs the worker metrics, the
//! resource manager and `/proc/meminfo` parsing of higher crates and lives in `blaze_runtime`.

// PORT: skipped, no caller at the pin: ConcurrencyMeter.<init>, ConcurrencyMeter.queueSize, ConcurrencyMeter.request, ConcurrencyMeter.maybeLease, ConcurrencyMeter.setTicket, ConcurrencyMeter.getStats, ConcurrencyMeter.release, ConcurrencyMeter.releaseAndSchedule, ConcurrencyMeter.schedule, ConcurrencyMeter.ReleasingTicket.<init>, ConcurrencyMeter.ReleasingTicket.done, ConcurrencyMeter.PendingJob.<init>, ConcurrencyMeter.PendingJob.compareTo, ConcurrencyMeter.Ticket.done
// PORT: skipped, no caller at the pin (the text form of a critical path, used by no command): CriticalPathText.<init>, CriticalPathText.printCriticalPaths, TextPrinter.<init>, TextPrinter.printLn, TextPrinter.lnPrint, TextPrinter.lnPrintf, TextPrinter.prettyPercentage, CriticalPathStatistics.<init>, CriticalPathStatistics.getTotalDuration, CriticalPathStatistics.getCriticalPathEntries
// PORT: skipped, exports the syscall caches to a metrics registry that nothing in Bazel reads (no output): LatestObjectMetricExporter.CallbackRegistration.register, LatestObjectMetricExporter.Strength.makeRef, LatestObjectMetricExporter.<init>, LatestObjectMetricExporter.setLatestInstance

pub mod auto_profiler;
pub mod counter_series;
pub mod java_hash_order;
pub mod json_profile;
pub mod json_trace_file_writer;
pub mod json_writer;
pub mod memory_profiler;
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
