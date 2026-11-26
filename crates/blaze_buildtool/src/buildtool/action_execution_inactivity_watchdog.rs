//! Port of `skyframe/ActionExecutionInactivityWatchdog.java` and
//! `actions/ActionExecutionInactivityEvent.java`.
//!
//! PORT: Java keeps the watchdog in `lib/skyframe` (the `SkyframeExecutor` owns it); its monitor
//! and reporter are the `ExecutionProgressReceiver`'s, in this crate, so it lives here too.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use parking_lot::{Condvar, Mutex};

use crate::buildtool::action_execution_status_reporter::ActionExecutionStatusReporter;

/// An object used in monitoring action execution inactivity.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:33 ActionExecutionInactivityWatchdog.InactivityMonitor
pub trait InactivityMonitor: Send + Sync {
    /// Returns whether action execution has started.
    fn has_started(&self) -> bool;

    /// Returns the number of enqueued but not yet completed actions.
    fn get_pending(&self) -> usize;

    /// Waits for any action to complete, or the timeout to elapse.
    ///
    /// The thread must wait at least for the specified timeout, unless some action completes in
    /// the meantime. It's not allowed to return 0 too early.
    ///
    /// Note that it's acceptable to return (any value) later than specified by the timeout.
    ///
    /// Returns the number of actions completed during the wait, `Err` when interrupted.
    fn wait_for_next_completion(
        &self,
        timeout_seconds: i32,
        sleeper: &Sleeper,
    ) -> Result<usize, Interrupted>;
}

/// An object that the watchdog can report inactivity to.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:56 ActionExecutionInactivityWatchdog.InactivityReporter
pub trait InactivityReporter: Send + Sync {
    /// Report that actions are not getting completed in a timely manner.
    ///
    /// Inactivity is typically not reported if tests with streaming output are being run.
    fn maybe_report_inactivity(&self, last_action_completed_at: SystemTime);
}

/// `Thread.interrupt()` of the watchdog thread, seen by its sleeps.
#[derive(Debug)]
pub struct Interrupted;

/// `Thread.sleep` that [`ActionExecutionInactivityWatchdog::stop`] interrupts.
pub struct Sleeper {
    interrupted: Mutex<bool>,
    wakeup: Condvar,
}

impl Sleeper {
    fn new() -> Sleeper {
        Sleeper {
            interrupted: Mutex::new(false),
            wakeup: Condvar::new(),
        }
    }

    /// `Thread.sleep(durationMilliseconds)`.
    pub fn sleep(&self, duration_milliseconds: u64) -> Result<(), Interrupted> {
        let deadline = std::time::Instant::now() + Duration::from_millis(duration_milliseconds);
        let mut interrupted = self.interrupted.lock();
        while !*interrupted {
            if self
                .wakeup
                .wait_until(&mut interrupted, deadline)
                .timed_out()
            {
                break;
            }
        }
        if *interrupted {
            return Err(Interrupted);
        }
        Ok(())
    }

    fn interrupt(&self) {
        *self.interrupted.lock() = true;
        self.wakeup.notify_all();
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:74 ActionExecutionInactivityWatchdog.WaitTime
struct WaitTime {
    progress_interval_flag_value: i32,
    prev: i32,
}

impl WaitTime {
    fn reset(&mut self) {
        self.prev = 0;
    }

    fn next(&mut self) -> i32 {
        self.prev = ActionExecutionStatusReporter::get_wait_time(
            self.progress_interval_flag_value,
            self.prev,
        );
        self.prev
    }
}

/// A component that monitors the inactivity of actions during the execution phase. Inactivity
/// is when no action finishes for a while (the "Still waiting for ..." messages).
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:30 ActionExecutionInactivityWatchdog
pub struct ActionExecutionInactivityWatchdog {
    is_running: Arc<AtomicBool>,
    monitor: Arc<dyn InactivityMonitor>,
    reporter: Arc<dyn InactivityReporter>,
    sleeper: Arc<Sleeper>,
    progress_interval_flag_value: i32,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ActionExecutionInactivityWatchdog {
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:98 ActionExecutionInactivityWatchdog.<init>
    pub fn new(
        monitor: Arc<dyn InactivityMonitor>,
        reporter: Arc<dyn InactivityReporter>,
        progress_interval_flag_value: i32,
    ) -> ActionExecutionInactivityWatchdog {
        ActionExecutionInactivityWatchdog {
            is_running: Arc::new(AtomicBool::new(false)),
            monitor,
            reporter,
            sleeper: Arc::new(Sleeper::new()),
            progress_interval_flag_value,
            thread: Mutex::new(None),
        }
    }

    /// Starts the watchdog thread. This method should only be called once.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:131 ActionExecutionInactivityWatchdog.start
    pub fn start(&self) {
        blaze_util::check_state!(!self.is_running.swap(true, Ordering::SeqCst));
        let is_running = self.is_running.clone();
        let monitor = self.monitor.clone();
        let reporter = self.reporter.clone();
        let sleeper = self.sleeper.clone();
        let wait_time = WaitTime {
            progress_interval_flag_value: self.progress_interval_flag_value,
            prev: 0,
        };
        let handle = std::thread::Builder::new()
            .name("action-execution-watchdog".to_owned())
            .spawn(move || {
                enter_watchdog_loop(&is_running, &*monitor, &*reporter, &sleeper, wait_time)
            })
            .expect("spawn action-execution-watchdog");
        *self.thread.lock() = Some(handle);
    }

    /// Stops the watchdog thread. This method should only be called once.
    ///
    /// The method waits for the thread to terminate.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:143 ActionExecutionInactivityWatchdog.stop
    pub fn stop(&self) {
        blaze_util::check_state!(self.is_running.swap(false, Ordering::SeqCst));
        self.sleeper.interrupt();
        if let Some(handle) = self.thread.lock().take() {
            let _ = handle.join();
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/ActionExecutionInactivityWatchdog.java:154 ActionExecutionInactivityWatchdog.enterWatchdogLoop
fn enter_watchdog_loop(
    is_running: &AtomicBool,
    monitor: &dyn InactivityMonitor,
    reporter: &dyn InactivityReporter,
    sleeper: &Sleeper,
    mut wait_time: WaitTime,
) {
    // clock.now(): BlazeClock.instance(), the wall clock.
    let mut last_action_completed_at = SystemTime::now();
    while is_running.load(Ordering::SeqCst) {
        // Wait a while for any SkyFunction to finish. The returned number indicates how many
        // actions completed during the wait. It's possible that this is more than 1, since
        // this thread may not immediately regain control.
        let completed_actions = match monitor.wait_for_next_completion(wait_time.next(), sleeper) {
            Ok(n) => n,
            Err(Interrupted) => return,
        };
        if !is_running.load(Ordering::SeqCst) {
            break;
        }

        let pending = monitor.get_pending();
        if !monitor.has_started() || completed_actions > 0 || pending == 0 {
            // If no keys have been enqueued yet (execution hasn't started), or some actions
            // were completed since this thread was notified (we are making visible progress),
            // or there are currently no enqueued actions waiting to be processed (perhaps all
            // have completed and we are about to stop monitoring), then there's no need to
            // display any messages.
            wait_time.reset();

            last_action_completed_at = SystemTime::now();

            // Sleep a while before checking again. Actions might be executing at a nice rate, no
            // need to worry about inactivity. This extra sleep isn't required but it's nice to
            // have: without it we would, at times of high action completion rate, unnecessarily
            // put the monitor into a fast sleep-wake cycle --- not a big problem but wasteful.
            if sleeper.sleep(1000).is_err() {
                return;
            }
        } else {
            // If actions are executing but we haven't made any progress in a while (no new
            // action completion), then reassure the user that we're still running. Next time
            // wait a little longer.
            reporter.maybe_report_inactivity(last_action_completed_at);
        }
    }
}

/// An event that is fired after action execution stalls (no action finished for a while).
// Java: com/google/devtools/build/lib/actions/ActionExecutionInactivityEvent.java:20 ActionExecutionInactivityEvent
#[derive(Clone, Debug)]
pub struct ActionExecutionInactivityEvent {
    /// `lastActionCompletedAt()`.
    pub last_action_completed_at: SystemTime,
}

impl blaze_events::Postable for ActionExecutionInactivityEvent {}
