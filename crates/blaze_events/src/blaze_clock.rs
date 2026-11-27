//! Port of `lib/clock/BlazeClock.java`: the process clock and conversions between its
//! `nanoTime` and milliseconds since the epoch.

use std::sync::{Arc, LazyLock};

use parking_lot::RwLock;

use crate::ui::{Clock, JavaClock};

// PORT: skipped, a private constructor of a static-only class: BlazeClock.BlazeClock

static INSTANCE: LazyLock<RwLock<Arc<dyn Clock>>> =
    LazyLock::new(|| RwLock::new(Arc::new(JavaClock)));

/// The process clock (a `JavaClock` unless [`set_clock`] replaced it).
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:39 BlazeClock.instance
pub fn instance() -> Arc<dyn Clock> {
    INSTANCE.read().clone()
}

/// Replaces the process clock.
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:44 BlazeClock.setClock
pub fn set_clock(clock: Arc<dyn Clock>) {
    *INSTANCE.write() = clock;
}

/// `instance().nanoTime()`.
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:48 BlazeClock.nanoTime
pub fn nano_time() -> i64 {
    instance().nano_time()
}

/// Converts a `nanoTime` value of `clock` to milliseconds since the epoch, anchored at the
/// moment of creation.
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:68 BlazeClock.createNanosToMillisSinceEpochConverter
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:63 BlazeClock.createNanosToMillisSinceEpochConverter
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:56 BlazeClock.NanosToMillisSinceEpochConverter.toEpochMillis
pub fn create_nanos_to_millis_since_epoch_converter(
    clock: &dyn Clock,
) -> impl Fn(i64) -> i64 + Send + Sync + use<> {
    let now_in_millis = clock.current_time_millis();
    let now_in_nanos = clock.nano_time();
    // `TimeUnit.NANOSECONDS.toMillis(d)`: truncating division.
    move |time_nanos: i64| now_in_millis - (now_in_nanos - time_nanos) / 1_000_000
}

/// Converts milliseconds since the epoch to a `nanoTime` value of `clock`, anchored at the
/// moment of creation.
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:92 BlazeClock.createMillisSinceEpochToNanosConverter
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:87 BlazeClock.createMillisSinceEpochToNanosConverter
// Java: com/google/devtools/build/lib/clock/BlazeClock.java:80 BlazeClock.MillisSinceEpochToNanosConverter.toNanos
pub fn create_millis_since_epoch_to_nanos_converter(
    clock: &dyn Clock,
) -> impl Fn(i64) -> i64 + Send + Sync + use<> {
    let now_in_millis = clock.current_time_millis();
    let now_in_nanos = clock.nano_time();
    // `TimeUnit.MILLISECONDS.toNanos(d)`: saturating multiplication.
    move |time_millis: i64| now_in_nanos - (now_in_millis - time_millis).saturating_mul(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedClock {
        millis: i64,
        nanos: i64,
    }

    impl Clock for FixedClock {
        fn current_time_millis(&self) -> i64 {
            self.millis
        }
        fn nano_time(&self) -> i64 {
            self.nanos
        }
    }

    /// Not a Java test (Bazel has no `BlazeClockTest`): the conversions of the Java code.
    #[test]
    fn converters_anchor_at_creation() {
        let clock = FixedClock {
            millis: 1_000_000,
            nanos: 5_000_000_000,
        };
        let to_millis = create_nanos_to_millis_since_epoch_converter(&clock);
        assert_eq!(to_millis(5_000_000_000), 1_000_000);
        assert_eq!(to_millis(4_000_000_000), 999_000);
        // Truncation toward zero of the difference.
        assert_eq!(to_millis(4_999_999_999), 1_000_000);
        let to_nanos = create_millis_since_epoch_to_nanos_converter(&clock);
        assert_eq!(to_nanos(1_000_000), 5_000_000_000);
        assert_eq!(to_nanos(999_000), 4_000_000_000);
    }
}
