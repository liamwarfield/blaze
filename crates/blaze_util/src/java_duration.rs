//! `java.time.Duration`: a signed amount of time, seconds plus a nanosecond adjustment in
//! `0..1_000_000_000`. `std::time::Duration` cannot be negative; Java's can, and Bazel carries
//! negative values (a negative `timeout` execution requirement, a negative
//! `--local_termination_grace_seconds`) to the tools it runs.

use std::cmp::Ordering;
use std::time::Duration;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_MILLI: i64 = 1_000_000;

/// `java.time.Duration`.
// Java (JDK): java/time/Duration.java Duration
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct JavaDuration {
    seconds: i64,
    /// `0..1_000_000_000`.
    nanos: i32,
}

impl JavaDuration {
    /// `Duration.ZERO`.
    pub const ZERO: JavaDuration = JavaDuration {
        seconds: 0,
        nanos: 0,
    };

    /// `Duration.ofSeconds(seconds)`.
    // Java (JDK): java/time/Duration.java Duration.ofSeconds(long)
    pub fn of_seconds(seconds: i64) -> JavaDuration {
        JavaDuration { seconds, nanos: 0 }
    }

    /// `Duration.ofMillis(millis)`.
    // Java (JDK): java/time/Duration.java Duration.ofMillis
    pub fn of_millis(millis: i64) -> JavaDuration {
        JavaDuration {
            seconds: millis.div_euclid(1000),
            nanos: (millis.rem_euclid(1000) * NANOS_PER_MILLI) as i32,
        }
    }

    /// The (non-negative) `std::time::Duration` as a Java `Duration`.
    pub fn from_std(d: Duration) -> JavaDuration {
        JavaDuration {
            seconds: i64::try_from(d.as_secs()).expect("ArithmeticException: long overflow"),
            nanos: d.subsec_nanos() as i32,
        }
    }

    /// The `std::time::Duration` of a non-negative duration; `None` for a negative one.
    pub fn to_std(self) -> Option<Duration> {
        u64::try_from(self.seconds)
            .ok()
            .map(|s| Duration::new(s, self.nanos as u32))
    }

    /// `isZero()`.
    pub fn is_zero(self) -> bool {
        self.seconds == 0 && self.nanos == 0
    }

    /// `isNegative()`.
    pub fn is_negative(self) -> bool {
        self.seconds < 0
    }

    /// `getSeconds()` / `toSeconds()`: the seconds part (rounded towards negative infinity).
    pub fn to_seconds(self) -> i64 {
        self.seconds
    }

    /// `getNano()`.
    pub fn get_nano(self) -> i32 {
        self.nanos
    }

    /// `toMillis()`, failing as `ArithmeticException` on overflow.
    // Java (JDK): java/time/Duration.java Duration.toMillis
    pub fn to_millis(self) -> i64 {
        let mut temp_seconds = self.seconds;
        let mut temp_nanos = self.nanos as i64;
        if temp_seconds < 0 {
            // change the seconds and nano value to handle Long.MIN_VALUE case
            temp_seconds += 1;
            temp_nanos -= NANOS_PER_SECOND;
        }
        temp_seconds
            .checked_mul(1000)
            .and_then(|m| m.checked_add(temp_nanos / NANOS_PER_MILLI))
            .expect("ArithmeticException: long overflow")
    }
}

impl PartialOrd for JavaDuration {
    fn partial_cmp(&self, other: &JavaDuration) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JavaDuration {
    /// `compareTo(other)`: by seconds, then by nanos.
    fn cmp(&self, other: &JavaDuration) -> Ordering {
        self.seconds
            .cmp(&other.seconds)
            .then(self.nanos.cmp(&other.nanos))
    }
}

impl From<Duration> for JavaDuration {
    fn from(d: Duration) -> JavaDuration {
        JavaDuration::from_std(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_millis_round_trip() {
        let d = JavaDuration::of_millis(-1500);
        assert_eq!(d.to_seconds(), -2);
        assert_eq!(d.get_nano(), 500_000_000);
        assert_eq!(d.to_millis(), -1500);
        assert!(d.is_negative());
        assert!(!d.is_zero());
    }

    #[test]
    fn seconds() {
        let d = JavaDuration::of_seconds(-1);
        assert_eq!(d.to_seconds(), -1);
        assert_eq!(d.to_millis(), -1000);
        assert_eq!(d.to_std(), None);
        assert_eq!(
            JavaDuration::of_seconds(3).to_std(),
            Some(Duration::from_secs(3))
        );
        assert!(JavaDuration::of_seconds(-1) < JavaDuration::ZERO);
        assert!(JavaDuration::ZERO.is_zero());
    }
}
