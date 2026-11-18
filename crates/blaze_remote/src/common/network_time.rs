//! Port of `lib/remote/common/NetworkTime.java`.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Reentrant wall clock stopwatch and grpc interceptor for network waits.
// Java: com/google/devtools/build/lib/remote/common/NetworkTime.java:23 NetworkTime
#[derive(Debug, Default)]
pub struct NetworkTime {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Guava `Stopwatch`: accumulated time and the start of the running lap.
    elapsed: Duration,
    running_since: Option<Instant>,
    outstanding: i32,
}

impl NetworkTime {
    /// `new NetworkTime()`.
    pub fn new() -> NetworkTime {
        NetworkTime::default()
    }

    /// `start()`.
    // Java: com/google/devtools/build/lib/remote/common/NetworkTime.java:29 NetworkTime.start
    pub fn start(&self) {
        let mut inner = self.inner.lock();
        if inner.running_since.is_none() {
            inner.running_since = Some(Instant::now());
        }
        inner.outstanding += 1;
    }

    /// `stop()`.
    // Java: com/google/devtools/build/lib/remote/common/NetworkTime.java:36 NetworkTime.stop
    pub fn stop(&self) {
        let mut inner = self.inner.lock();
        inner.outstanding -= 1;
        if inner.outstanding == 0
            && let Some(since) = inner.running_since.take()
        {
            inner.elapsed += since.elapsed();
        }
    }

    /// `getDuration()`.
    // Java: com/google/devtools/build/lib/remote/common/NetworkTime.java:42 NetworkTime.getDuration
    pub fn get_duration(&self) -> Duration {
        let inner = self.inner.lock();
        match inner.running_since {
            Some(since) => inner.elapsed + since.elapsed(),
            None => inner.elapsed,
        }
    }
}
