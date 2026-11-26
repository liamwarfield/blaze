//! `TimestampGranularityMonitor`: a utility for dealing with filesystem timestamp granularity
//! issues.
//!
//! Consider a sequence of commands such as
//!
//! ```text
//!     echo ... > foo/bar
//!     blaze query ...
//!     echo ... > foo/bar
//!     blaze query ...
//! ```
//!
//! If these commands all run very fast, it is possible that the timestamp on foo/bar is not
//! changed by the second command, even though some time has passed, because the times are the
//! same when rounded to the file system timestamp granularity (often 1 second). For performance,
//! we assume that files whose timestamps haven't changed can safely be cached without
//! reexamining their contents. But this assumption would be violated in the above scenario.
//!
//! To address this, we record the current time at the start of executing a Blaze command, and
//! whenever we check the timestamp of a source file or BUILD file, we check if the timestamp of
//! that source file matches the current time. If so, we set a flag. At the end of the command, if
//! the flag was set, then we wait until the clock has advanced, so that any file modifications
//! performed after the command exits will result in a different file timestamp.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use crate::profiler::Clock;

/// See the module documentation.
///
/// PORT: Java's fields are plain (the class is `@ThreadCompatible`, the two flags `volatile`);
/// here every field is atomic so that one monitor can be shared by the evaluator threads of a
/// command through an `Arc`.
// Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:68 TimestampGranularityMonitor
pub struct TimestampGranularityMonitor {
    /// The time of the start of the current Blaze command, in milliseconds.
    command_start_time_millis: AtomicI64,
    /// The time of the start of the current Blaze command, in milliseconds, rounded to one second
    /// granularity.
    command_start_time_millis_rounded: AtomicI64,
    /// True iff we detected a source file or BUILD file whose (unrounded) timestamp matched the
    /// time at the start of the current Blaze command rounded to the nearest second.
    wait_a_second: AtomicBool,
    /// True iff we detected a source file or BUILD file whose timestamp exactly matched the time
    /// at the start of the current Blaze command (measuring both in integral numbers of
    /// milliseconds).
    wait_a_millisecond: AtomicBool,
    clock: Arc<dyn Clock>,
}

impl TimestampGranularityMonitor {
    // Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:98 TimestampGranularityMonitor.<init>
    pub fn new(clock: Arc<dyn Clock>) -> TimestampGranularityMonitor {
        TimestampGranularityMonitor {
            command_start_time_millis: AtomicI64::new(0),
            command_start_time_millis_rounded: AtomicI64::new(0),
            wait_a_second: AtomicBool::new(false),
            wait_a_millisecond: AtomicBool::new(false),
            clock,
        }
    }

    /// Record the time at which the Blaze command started. This is needed for use by
    /// [`Self::wait_for_timestamp_granularity`].
    // Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:106 TimestampGranularityMonitor.setCommandStartTime
    pub fn set_command_start_time(&self) {
        let now = self.clock.current_time_millis();
        self.command_start_time_millis.store(now, Ordering::SeqCst);
        self.command_start_time_millis_rounded
            .store(Self::round_down(now), Ordering::SeqCst);
        self.wait_a_second.store(false, Ordering::SeqCst);
        self.wait_a_millisecond.store(false, Ordering::SeqCst);
    }

    /// Record that the output of this Blaze command depended on the contents of a build file or
    /// source file with the specified time stamp.
    ///
    /// PORT: the "Will have to wait ..." log lines (logger.atInfo) are not ported.
    // Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:118 TimestampGranularityMonitor.notifyDependenceOnFileTime
    pub fn notify_dependence_on_file_time(&self, ctime_millis: i64) {
        if !self.wait_a_millisecond.load(Ordering::SeqCst)
            && ctime_millis == self.command_start_time_millis.load(Ordering::SeqCst)
        {
            self.wait_a_millisecond.store(true, Ordering::SeqCst);
        }
        if !self.wait_a_second.load(Ordering::SeqCst)
            && ctime_millis
                == self
                    .command_start_time_millis_rounded
                    .load(Ordering::SeqCst)
        {
            self.wait_a_second.store(true, Ordering::SeqCst);
        }
    }

    /// If needed, wait until the next "tick" of the filesystem timestamp clock. This is done to
    /// ensure that files created after the current Blaze command finishes will have timestamps
    /// different than files created before the current Blaze command started. Otherwise a
    /// sequence of commands such as
    ///
    /// ```text
    ///     echo ... > foo/BUILD
    ///     blaze query ...
    ///     echo ... > foo/BUILD
    ///     blaze query ...
    /// ```
    ///
    /// could return wrong results, due to the contents of package foo being cached even though
    /// foo/BUILD changed.
    ///
    /// PORT: a Rust sleep is not interrupted, so the "INFO: Hang on a second..." and "Hang on a
    /// millisecond..." lines Java prints when the sleeping thread is interrupted cannot occur;
    /// the profiler task and the log line are not ported.
    // Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:149 TimestampGranularityMonitor.waitForTimestampGranularity
    pub fn wait_for_timestamp_granularity(&self) {
        let wait_a_second = self.wait_a_second.load(Ordering::SeqCst);
        if wait_a_second || self.wait_a_millisecond.load(Ordering::SeqCst) {
            if wait_a_second {
                let rounded = self
                    .command_start_time_millis_rounded
                    .load(Ordering::SeqCst);
                // 50ms slack after the whole-second boundary
                while self.clock.current_time_millis() < rounded + 1050 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            } else {
                let start = self.command_start_time_millis.load(Ordering::SeqCst);
                while self.clock.current_time_millis() == start {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    /// Rounds the specified time, in milliseconds, down to the nearest second, and returns the
    /// result in milliseconds.
    // Java: com/google/devtools/build/lib/util/io/TimestampGranularityMonitor.java:202 TimestampGranularityMonitor.roundDown
    fn round_down(millis: i64) -> i64 {
        millis / 1000 * 1000
    }

    /// Whether the command will wait a second at its end (test accessor).
    pub fn will_wait_a_second_for_testing(&self) -> bool {
        self.wait_a_second.load(Ordering::SeqCst)
    }

    /// Whether the command will wait a millisecond at its end (test accessor).
    pub fn will_wait_a_millisecond_for_testing(&self) -> bool {
        self.wait_a_millisecond.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    // Bazel has no JUnit class for TimestampGranularityMonitor; these tests follow its methods.
    use super::*;
    use std::sync::atomic::AtomicI64;

    struct ManualClock(AtomicI64);

    impl Clock for ManualClock {
        fn current_time_millis(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
        fn nano_time(&self) -> i64 {
            self.0.load(Ordering::SeqCst) * 1_000_000
        }
    }

    #[test]
    fn no_wait_when_no_file_has_the_start_time() {
        let clock = Arc::new(ManualClock(AtomicI64::new(1_234_567)));
        let tsgm = TimestampGranularityMonitor::new(clock.clone());
        tsgm.set_command_start_time();
        tsgm.notify_dependence_on_file_time(1_000);
        assert!(!tsgm.will_wait_a_second_for_testing());
        assert!(!tsgm.will_wait_a_millisecond_for_testing());
        tsgm.wait_for_timestamp_granularity();
    }

    #[test]
    fn same_millisecond_waits_a_millisecond() {
        let clock = Arc::new(ManualClock(AtomicI64::new(1_234_567)));
        let tsgm = TimestampGranularityMonitor::new(clock.clone());
        tsgm.set_command_start_time();
        tsgm.notify_dependence_on_file_time(1_234_567);
        assert!(tsgm.will_wait_a_millisecond_for_testing());
        assert!(!tsgm.will_wait_a_second_for_testing());
        clock.0.store(1_234_568, Ordering::SeqCst);
        tsgm.wait_for_timestamp_granularity();
    }

    #[test]
    fn rounded_second_waits_a_second() {
        let clock = Arc::new(ManualClock(AtomicI64::new(1_234_567)));
        let tsgm = TimestampGranularityMonitor::new(clock.clone());
        tsgm.set_command_start_time();
        tsgm.notify_dependence_on_file_time(1_234_000);
        assert!(tsgm.will_wait_a_second_for_testing());
        clock.0.store(1_235_050, Ordering::SeqCst);
        tsgm.wait_for_timestamp_granularity();
    }

    #[test]
    fn set_command_start_time_resets_the_flags() {
        let clock = Arc::new(ManualClock(AtomicI64::new(5_000)));
        let tsgm = TimestampGranularityMonitor::new(clock.clone());
        tsgm.set_command_start_time();
        tsgm.notify_dependence_on_file_time(5_000);
        assert!(tsgm.will_wait_a_second_for_testing());
        assert!(tsgm.will_wait_a_millisecond_for_testing());
        clock.0.store(7_001, Ordering::SeqCst);
        tsgm.set_command_start_time();
        assert!(!tsgm.will_wait_a_second_for_testing());
        assert!(!tsgm.will_wait_a_millisecond_for_testing());
    }
}
