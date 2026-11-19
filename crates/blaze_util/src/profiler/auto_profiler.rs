//! Port of `com/google/devtools/build/lib/profiler/AutoProfiler.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::RwLock;

use super::profiler::{Clock, Profiler};
use super::profiler_task::ProfilerTask;

static CLOCK_REF: RwLock<Option<Arc<dyn Clock>>> = RwLock::new(None);

/// A opaque receiver of elapsed time information.
///
/// Note that `System#nanoTime` isn't guaranteed to be non-decreasing, so implementations should
/// check for non-positive `elapsed_time_nanos` if they care about this sort of thing.
// Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:66 AutoProfiler.ElapsedTimeReceiver
pub trait ElapsedTimeReceiver: Send + Sync {
    fn accept(&self, elapsed_time_nanos: i64);
}

impl<F: Fn(i64) + Send + Sync> ElapsedTimeReceiver for F {
    fn accept(&self, elapsed_time_nanos: i64) {
        self(elapsed_time_nanos)
    }
}

/// A convenient way to actively get access to timing information (e.g. for logging and/or
/// profiling purposes) with minimal boilerplate. Completes when dropped (`close()`).
// Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:50 AutoProfiler
#[must_use = "the profiling ends when this is dropped"]
pub struct AutoProfiler {
    elapsed_time_receiver: Box<dyn ElapsedTimeReceiver>,
    start_time_nanos: i64,
    closed: AtomicBool,
}

/// `BlazeClock.instance().nanoTime()` when no clock was set.
///
/// PORT: `BlazeClock` is `blaze_events`' (above this crate); without a clock set by the runtime
/// (`AutoProfiler.setClock(runtime.getClock())` in `BlazeRuntime`), the monotonic clock of the
/// process, from an arbitrary origin.
fn default_nano_time() -> i64 {
    static ORIGIN: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    ORIGIN.elapsed().as_nanos() as i64
}

impl AutoProfiler {
    /// Sets the clock to use. May be called before trying to call any of the other static
    /// methods; otherwise `BlazeClock#instance` will be used.
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:83 AutoProfiler.setClock
    pub fn set_clock(clock: Arc<dyn Clock>) {
        *CLOCK_REF.write() = Some(clock);
    }

    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:87 AutoProfiler.nanoTime
    fn nano_time() -> i64 {
        match &*CLOCK_REF.read() {
            Some(clock) => clock.nano_time(),
            None => default_nano_time(),
        }
    }

    /// Returns an `AutoProfiler` that, when closed, records the elapsed time using `Profiler`.
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:98 AutoProfiler.profiled
    pub fn profiled(description: impl Into<String>, profiler_task_type: ProfilerTask) -> Self {
        Self::create(Box::new(ProfilingElapsedTimeReceiver {
            start_time_nanos: Profiler::nano_time_maybe(),
            description: description.into(),
            profiler_task_type,
        }))
    }

    /// Returns an `AutoProfiler` that, when closed, invokes the given `ElapsedTimeReceiver`.
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:108 AutoProfiler.create
    pub fn create(elapsed_time_receiver: Box<dyn ElapsedTimeReceiver>) -> Self {
        AutoProfiler {
            elapsed_time_receiver,
            start_time_nanos: Self::nano_time(),
            closed: AtomicBool::new(false),
        }
    }

    /// Manually completes the profiling (useful to trigger the underlying action on completion).
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:118 AutoProfiler.complete
    pub fn complete(self) {}

    /// Drops the profiler without completing it: Java leaves such an `AutoProfiler` unclosed
    /// (`ResourceManager.acquireResources` completes it only after waiting), so nothing is
    /// recorded.
    ///
    /// PORT: Blaze-only; a Rust value completes when dropped.
    pub fn abandon(self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Manually completes the profiling and returns the elapsed time in nanoseconds.
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:128 AutoProfiler.completeAndGetElapsedTimeNanos
    pub fn complete_and_get_elapsed_time_nanos(self) -> i64 {
        let elapsed_time_nanos = Self::nano_time() - self.start_time_nanos;
        crate::check_state!(
            self.closed
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        );
        self.elapsed_time_receiver.accept(elapsed_time_nanos);
        elapsed_time_nanos
    }
}

impl Drop for AutoProfiler {
    /// Automatically completes the profiling (`close()`).
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:142 AutoProfiler.close
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            // completeAndGetElapsedTimeNanos already completed it.
            return;
        }
        let elapsed_time_nanos = Self::nano_time() - self.start_time_nanos;
        self.elapsed_time_receiver.accept(elapsed_time_nanos);
    }
}

// Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:148 AutoProfiler.ProfilingElapsedTimeReceiver
struct ProfilingElapsedTimeReceiver {
    start_time_nanos: i64,
    description: String,
    profiler_task_type: ProfilerTask,
}

impl ElapsedTimeReceiver for ProfilingElapsedTimeReceiver {
    // Java: com/google/devtools/build/lib/profiler/AutoProfiler.java:160 AutoProfiler.ProfilingElapsedTimeReceiver.accept
    fn accept(&self, elapsed_time_nanos: i64) {
        if elapsed_time_nanos > 0 {
            Profiler::instance().log_simple_task_duration(
                self.start_time_nanos,
                elapsed_time_nanos,
                self.profiler_task_type,
                &self.description,
            );
        }
    }
}
