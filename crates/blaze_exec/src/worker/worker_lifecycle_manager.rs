//! Port of `lib/worker/WorkerLifecycleManager.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use blaze_events::{Event, EventHandler};
use blaze_optdefs::misc::worker_options::WorkerOptions;
use indexmap::IndexSet;
use parking_lot::Mutex;

use super::worker::java_sleep;
use super::worker_pool::WorkerPool;
use super::worker_process_metrics::{WorkerProcessMetrics, WorkerProcessMetricsCollector};
use super::worker_process_status::Status;

/// `String.format("%,d", n)` in the root locale.
fn java_grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// `List.toString()` / `Set.toString()` of integers.
fn java_list(ids: impl IntoIterator<Item = i32>) -> String {
    let v: Vec<String> = ids.into_iter().map(|i| i.to_string()).collect();
    format!("[{}]", v.join(", "))
}

/// This class kills idle persistent workers at intervals, if the total worker resource usage is
/// above a specified limit. Must be used as singleton.
// Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:39 WorkerLifecycleManager
pub struct WorkerLifecycleManager {
    is_working: AtomicBool,
    empty_eviction_was_logged: Mutex<bool>,
    worker_pool: Arc<dyn WorkerPool>,
    total_worker_memory_limit_kb: i32,
    worker_memory_limit_kb: i32,
    shrink_worker_pool: bool,
    worker_verbose: bool,
    worker_metrics_poll_interval: Duration,
    reporter: Option<Arc<dyn EventHandler>>,
    collector: &'static WorkerProcessMetricsCollector,
    /// The running thread's interrupt status (`Thread.interrupt()`).
    thread: Mutex<Option<blaze_shell::ThreadInterrupt>>,
}

impl WorkerLifecycleManager {
    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:54 WorkerLifecycleManager.<init>
    pub fn new(
        worker_pool: Arc<dyn WorkerPool>,
        options: &WorkerOptions,
        reporter: Option<Arc<dyn EventHandler>>,
    ) -> WorkerLifecycleManager {
        WorkerLifecycleManager {
            is_working: AtomicBool::new(false),
            empty_eviction_was_logged: Mutex::new(false),
            worker_pool,
            total_worker_memory_limit_kb: options.total_worker_memory_limit_mb.wrapping_mul(1000),
            worker_memory_limit_kb: options.worker_memory_limit_mb.wrapping_mul(1000),
            shrink_worker_pool: options.shrink_worker_pool,
            worker_verbose: options.worker_verbose,
            worker_metrics_poll_interval: options.worker_metrics_poll_interval,
            reporter,
            collector: WorkerProcessMetricsCollector::instance(),
            thread: Mutex::new(None),
        }
    }

    /// `setDaemon(true); start()`.
    pub fn start(self: &Arc<Self>) {
        let this = self.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("worker-lifecycle-manager".to_string())
            .spawn(move || {
                let _ = tx.send(blaze_shell::ThreadInterrupt::current());
                // A Java Thread: an exception reaches the default uncaught exception handler.
                blaze_util::uncaught_exception::run_catching(|| this.run());
            });
        *self.thread.lock() = rx.recv().ok();
    }

    /// `interrupt()`.
    pub fn interrupt(&self) {
        if let Some(t) = &*self.thread.lock() {
            t.interrupt();
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:65 WorkerLifecycleManager.run
    pub fn run(&self) {
        if self.total_worker_memory_limit_kb == 0 && self.worker_memory_limit_kb == 0 {
            return;
        }

        let msg = format!(
            "Worker Lifecycle Manager starts work with (total limit: {} KB, individual limit: {} KB, shrinking: {})",
            self.total_worker_memory_limit_kb,
            self.worker_memory_limit_kb,
            if self.shrink_worker_pool {
                "enabled"
            } else {
                "disabled"
            }
        );
        if self.worker_verbose
            && let Some(reporter) = &self.reporter
        {
            reporter.handle(&Event::info(&msg));
        }

        self.is_working.store(true, Ordering::SeqCst);

        // This loop works until method stopProcessing() called by WorkerModule.
        while self.is_working.load(Ordering::SeqCst) {
            if java_sleep(self.worker_metrics_poll_interval.as_millis() as u64).is_err() {
                // received interrupt in worker life cycle manager (logged)
                break;
            }

            let worker_process_metrics = self.collector.get_live_worker_process_metrics();

            if self.total_worker_memory_limit_kb > 0 {
                self.evict_workers(&worker_process_metrics);
            }

            if self.worker_memory_limit_kb > 0 {
                self.kill_large_workers(&worker_process_metrics, self.worker_memory_limit_kb);
            }
        }

        self.is_working.store(false, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:112 WorkerLifecycleManager.stopProcessing
    pub fn stop_processing(&self) {
        self.is_working.store(false, Ordering::SeqCst);
    }

    /// Kills any worker that uses more than `limit_kb` KB of memory.
    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:117 WorkerLifecycleManager.killLargeWorkers
    pub fn kill_large_workers(
        &self,
        worker_process_metrics: &[Arc<WorkerProcessMetrics>],
        limit_kb: i32,
    ) {
        let large: Vec<&Arc<WorkerProcessMetrics>> = worker_process_metrics
            .iter()
            .filter(|m| m.get_used_memory_in_kb() > limit_kb)
            .collect();

        for l in large {
            let worker_ids = l.get_worker_ids();
            // ProcessHandle.of(pid): present while the process exists.
            let pid = l.get_process_id() as libc::pid_t;
            // SAFETY: signal 0 only checks for existence.
            let ph_present = pid > 0 && unsafe { libc::kill(pid, 0) } == 0;
            if ph_present {
                let msg = format!(
                    "Killing {} worker {} (pid {}) because it is using more memory than the limit ({} KB > {} KB)",
                    l.get_mnemonic(),
                    if worker_ids.len() == 1 {
                        worker_ids[0].to_string()
                    } else {
                        java_list(worker_ids.iter().copied())
                    },
                    l.get_process_id(),
                    java_grouped(l.get_used_memory_in_kb() as i64),
                    java_grouped(limit_kb as i64)
                );
                // TODO(b/310640400): Converge APIs in killing workers, rather than killing via the
                //  process handle here (resulting in errors in execution), perhaps we want to wait
                //  till the worker is returned before killing it.
                // ph.get().destroyForcibly()
                // SAFETY: plain kill(2).
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                l.get_status()
                    .maybe_update_status(Status::KilledDueToMemoryPressure);
                // We want to always report this as this is a potential source of build failure.
                if let Some(reporter) = &self.reporter {
                    reporter.handle(&Event::warn(&msg));
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:152 WorkerLifecycleManager.evictWorkers
    pub fn evict_workers(&self, worker_process_metrics: &[Arc<WorkerProcessMetrics>]) {
        if self.total_worker_memory_limit_kb == 0 {
            return;
        }

        let worker_memory_usage_kb: i32 = worker_process_metrics
            .iter()
            .map(|m| m.get_used_memory_in_kb())
            .fold(0i32, |a, b| a.wrapping_add(b));

        // TODO: Remove after b/274608075 is fixed. (total worker memory ... logged)

        if worker_memory_usage_kb <= self.total_worker_memory_limit_kb {
            return;
        }

        let candidates = self.collect_eviction_candidates(
            worker_process_metrics,
            self.total_worker_memory_limit_kb,
            worker_memory_usage_kb,
        );

        let mut empty_eviction_was_logged = self.empty_eviction_was_logged.lock();
        if !candidates.is_empty() || !*empty_eviction_was_logged {
            let msg = if candidates.is_empty() {
                format!(
                    "Could not find any worker eviction candidates. Worker memory usage: {} KB, Memory limit: {} KB",
                    worker_memory_usage_kb, self.total_worker_memory_limit_kb
                )
            } else {
                let worker_ids_to_evict: IndexSet<i32> =
                    candidates.iter().flat_map(|m| m.get_worker_ids()).collect();
                format!(
                    "Attempting eviction of {} workers with ids: {}",
                    worker_ids_to_evict.len(),
                    java_list(worker_ids_to_evict.iter().copied())
                )
            };

            if self.worker_verbose
                && let Some(reporter) = &self.reporter
            {
                reporter.handle(&Event::info(&msg));
            }
        }

        let evicted_workers = evict_candidates(&*self.worker_pool, &candidates);

        if !evicted_workers.is_empty() || !*empty_eviction_was_logged {
            let msg = format!(
                "Total evicted idle workers {}. With ids: {}",
                evicted_workers.len(),
                java_list(evicted_workers.iter().copied())
            );
            if self.worker_verbose
                && let Some(reporter) = &self.reporter
            {
                reporter.handle(&Event::info(&msg));
            }

            *empty_eviction_was_logged = candidates.is_empty();
        }
        drop(empty_eviction_was_logged);

        // TODO(b/300067854): Shrinking of the worker pool happens on worker keys that are active
        //  at the time of polling, but doesn't shrink the pools of idle workers.
        if self.shrink_worker_pool {
            let not_evicted_worker_process_metrics: Vec<Arc<WorkerProcessMetrics>> =
                worker_process_metrics
                    .iter()
                    .filter(|metric| {
                        !metric
                            .get_worker_ids()
                            .iter()
                            .all(|id| evicted_workers.contains(id))
                    })
                    .cloned()
                    .collect();

            let not_evicted_worker_memory_usage_kb: i32 = not_evicted_worker_process_metrics
                .iter()
                .map(|m| m.get_used_memory_in_kb())
                .fold(0i32, |a, b| a.wrapping_add(b));

            if not_evicted_worker_memory_usage_kb <= self.total_worker_memory_limit_kb {
                return;
            }

            self.postpone_invalidation(
                not_evicted_worker_process_metrics,
                not_evicted_worker_memory_usage_kb,
            );
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:248 WorkerLifecycleManager.postponeInvalidation
    fn postpone_invalidation(
        &self,
        worker_process_metrics: Vec<Arc<WorkerProcessMetrics>>,
        not_evicted_worker_memory_usage_kb: i32,
    ) {
        let potential_candidates = get_candidates(
            worker_process_metrics,
            self.total_worker_memory_limit_kb,
            not_evicted_worker_memory_usage_kb,
        );

        if !potential_candidates.is_empty() {
            let msg = format!(
                "Postponing eviction of worker ids: {}",
                java_list(potential_candidates.iter().flat_map(|m| m.get_worker_ids()))
            );
            if self.worker_verbose
                && let Some(reporter) = &self.reporter
            {
                reporter.handle(&Event::info(&msg));
            }
            for m in &potential_candidates {
                m.get_status()
                    .maybe_update_status(Status::PendingKillDueToMemoryPressure);
            }
        }
    }

    /// Collects worker candidates to evict. Chooses workers with the largest memory consumption.
    // Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:287 WorkerLifecycleManager.collectEvictionCandidates
    pub fn collect_eviction_candidates(
        &self,
        worker_process_metrics: &[Arc<WorkerProcessMetrics>],
        memory_limit_kb: i32,
        worker_memory_usage_kb: i32,
    ) -> Vec<Arc<WorkerProcessMetrics>> {
        let idle_workers = self.worker_pool.get_idle_workers();

        let idle_worker_process_metrics: Vec<Arc<WorkerProcessMetrics>> = worker_process_metrics
            .iter()
            .filter(|metric| {
                metric
                    .get_worker_ids()
                    .iter()
                    .any(|id| idle_workers.contains(id))
            })
            .cloned()
            .collect();

        get_candidates(
            idle_worker_process_metrics,
            memory_limit_kb,
            worker_memory_usage_kb,
        )
    }
}

/// Applies eviction police for candidates. Returns the worker ids of evicted workers.
// Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:276 WorkerLifecycleManager.evictCandidates
fn evict_candidates(
    pool: &dyn WorkerPool,
    candidates: &[Arc<WorkerProcessMetrics>],
) -> IndexSet<i32> {
    pool.evict_workers(&candidates.iter().flat_map(|w| w.get_worker_ids()).collect())
}

/// Chooses the WorkerProcessMetrics of workers with the most usage of memory. Selects workers
/// until total memory usage is less than memoryLimitKb. (Returns an `ImmutableSet` in insertion
/// order; the metrics are distinct objects.)
// Java: com/google/devtools/build/lib/worker/WorkerLifecycleManager.java:312 WorkerLifecycleManager.getCandidates
fn get_candidates(
    mut worker_process_metrics: Vec<Arc<WorkerProcessMetrics>>,
    memory_limit_kb: i32,
    used_memory_kb: i32,
) -> Vec<Arc<WorkerProcessMetrics>> {
    // MemoryComparator: descending memory (List.sort is stable).
    worker_process_metrics.sort_by(|m1, m2| {
        (m2.get_used_memory_in_kb()
            .wrapping_sub(m1.get_used_memory_in_kb()))
        .cmp(&0)
    });
    let mut candidates = Vec::new();
    let mut free_memory_kb: i32 = 0;
    for metric in worker_process_metrics {
        free_memory_kb = free_memory_kb.wrapping_add(metric.get_used_memory_in_kb());
        candidates.push(metric);

        if used_memory_kb.wrapping_sub(free_memory_kb) <= memory_limit_kb {
            break;
        }
    }

    candidates
}
