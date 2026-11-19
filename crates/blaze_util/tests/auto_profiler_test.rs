//! Port of `src/test/java/com/google/devtools/build/lib/profiler/AutoProfilerTest.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_util::profiler::{AutoProfiler, Clock};

/// `com.google.devtools.build.lib.testutil.ManualClock`.
struct ManualClock(AtomicI64);

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        self.0.load(Ordering::SeqCst) * 1_000_000 + 1000 * 1_000_000_000
    }
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/AutoProfilerTest.java:40 AutoProfilerTest.simple
#[test]
fn simple() {
    let clock = Arc::new(ManualClock(AtomicI64::new(0)));
    AutoProfiler::set_clock(clock.clone());
    let elapsed_time = Arc::new(AtomicI64::new(0));
    let receiver = {
        let elapsed_time = elapsed_time.clone();
        move |elapsed_time_nanos: i64| elapsed_time.store(elapsed_time_nanos, Ordering::SeqCst)
    };
    {
        let _profiler = AutoProfiler::create(Box::new(receiver));
        clock.0.fetch_add(42, Ordering::SeqCst);
    }
    assert_eq!(elapsed_time.load(Ordering::SeqCst), 42 * 1000 * 1000);
}
