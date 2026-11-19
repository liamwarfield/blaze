//! Port of `WallTicker`.

use std::sync::Arc;

use blaze_events::ui::Clock;

use super::credential_cache::Ticker;
use super::unchecked_exception::UncheckedException;

/// WallTicker is a [`Ticker`] which reports wall time since the unix epoch.
///
/// We use this instead of Caffeine's `Ticker.systemTicker()` because the latter uses monotonic
/// time (which doesn't increment the time source while the system is asleep) with an
/// unspecified reference point (which is unhelpful when computing the cache duration for
/// credentials whose expiry is a fixed point in time, not a fixed duration).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/WallTicker.java:29 WallTicker
pub struct WallTicker {
    clock: Arc<dyn Clock>,
}

impl WallTicker {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/WallTicker.java:32 WallTicker.<init>
    pub fn new(clock: Arc<dyn Clock>) -> WallTicker {
        WallTicker { clock }
    }
}

impl Ticker for WallTicker {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/WallTicker.java:37 WallTicker.read
    fn read(&self) -> i64 {
        // Documented to return a value in nanoseconds.
        // Duration.ofMillis(millis).toNanos()
        self.clock
            .current_time_millis()
            .checked_mul(1_000_000)
            .unwrap_or_else(|| {
                UncheckedException::throw("java.lang.ArithmeticException", "long overflow")
            })
    }
}
