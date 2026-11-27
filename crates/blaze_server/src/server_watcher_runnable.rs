//! `ServerWatcherRunnable`: checks to see if a server has been idle for too long and shuts down
//! the server if so.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_events::ui::Clock;
use blaze_util::{check_argument, check_state};

use crate::command_manager::CommandManagerWatch;
use crate::grpc::ServerShutdown;
use crate::proc_meminfo_parser::ProcMeminfoParser;

// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:38 ServerWatcherRunnable.IDLE_MEMORY_CHECK_INTERVAL
const IDLE_MEMORY_CHECK_INTERVAL: Duration = Duration::from_secs(5);
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:39 ServerWatcherRunnable.TIME_IDLE_BEFORE_MEMORY_CHECK
const TIME_IDLE_BEFORE_MEMORY_CHECK: Duration = Duration::from_secs(5 * 60);
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:40 ServerWatcherRunnable.FREE_MEMORY_KB_ABSOLUTE_THRESHOLD
const FREE_MEMORY_KB_ABSOLUTE_THRESHOLD: i64 = 1 << 20;
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:41 ServerWatcherRunnable.FREE_MEMORY_PERCENTAGE_THRESHOLD
const FREE_MEMORY_PERCENTAGE_THRESHOLD: f64 = 0.05;

/// The platform part of a [`LowMemoryChecker`] (`check()`).
pub trait LowMemoryCheck: Send {
    /// Returns true if the system has observed low memory conditions.
    fn check(&self) -> bool;
}

/// Generic abstraction to check for low memory conditions on different platforms.
///
/// PORT: Java's abstract class is a struct holding the platform check; `BlazeClock.instance()`
/// is the clock passed in (the server's clock), so that tests can use a manual clock without a
/// process-wide setting.
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:50 ServerWatcherRunnable.LowMemoryChecker
pub struct LowMemoryChecker {
    /// Timestamp of the moment the server went idle.
    last_idle_time_millis: i64,
    clock: Arc<dyn Clock>,
    check: Box<dyn LowMemoryCheck>,
}

impl LowMemoryChecker {
    pub fn new(clock: Arc<dyn Clock>, check: Box<dyn LowMemoryCheck>) -> LowMemoryChecker {
        LowMemoryChecker {
            last_idle_time_millis: -1,
            clock,
            check,
        }
    }

    /// Creates a memory checker that makes sense for the current platform.
    ///
    /// PORT: Linux only, so always the `/proc/meminfo` checker; `MemoryPressureLowMemoryChecker`
    /// (`SystemMemoryPressureMonitor`) is skipped, macOS/Windows only.
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:56 LowMemoryChecker.forCurrentOS
    pub fn for_current_os(clock: Arc<dyn Clock>) -> LowMemoryChecker {
        LowMemoryChecker::new(
            clock,
            Box::new(ProcMeminfoLowMemoryChecker::new(Box::new(
                ProcMeminfoParser::new,
            ))),
        )
    }

    /// Checks if the server should shut down due to a low memory condition.
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:67 LowMemoryChecker.shouldShutdown
    pub fn should_shutdown(&self) -> bool {
        check_state!(
            self.last_idle_time_millis >= 0,
            "reset() ought to have been called before this"
        );

        if self.clock.current_time_millis() - self.last_idle_time_millis
            < TIME_IDLE_BEFORE_MEMORY_CHECK.as_millis() as i64
        {
            // Only run memory check if the server has been idle for longer than
            // TIME_IDLE_BEFORE_MEMORY_CHECK.
            return false;
        }

        self.check.check()
    }

    /// Notifies the checker that the server went idle at the given timestamp.
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:84 LowMemoryChecker.reset
    pub fn reset(&mut self, last_idle_time_millis: i64) {
        self.last_idle_time_millis = last_idle_time_millis;
    }
}

/// Supplier for a [`ProcMeminfoParser`].
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:107 ProcMeminfoLowMemoryChecker.ProcMeminfoParserSupplier
pub type ProcMeminfoParserSupplier = Box<dyn Fn() -> io::Result<ProcMeminfoParser> + Send>;

/// A low memory condition checker that uses instantaneous data from `/proc/meminfo`.
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:104 ServerWatcherRunnable.ProcMeminfoLowMemoryChecker
pub struct ProcMeminfoLowMemoryChecker {
    supplier: ProcMeminfoParserSupplier,
}

impl ProcMeminfoLowMemoryChecker {
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:113 ProcMeminfoLowMemoryChecker.<init>
    pub fn new(supplier: ProcMeminfoParserSupplier) -> Self {
        ProcMeminfoLowMemoryChecker { supplier }
    }
}

impl LowMemoryCheck for ProcMeminfoLowMemoryChecker {
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:118 ProcMeminfoLowMemoryChecker.check
    fn check(&self) -> bool {
        let result = (|| -> io::Result<bool> {
            let meminfo_parser = (self.supplier)()?;
            let free_ram_kb = meminfo_parser.get_free_ram_kb().map_err(io::Error::other)?;
            let used_ram_kb = meminfo_parser.get_total_kb().map_err(io::Error::other)?;
            let fraction_ram_free = (free_ram_kb as f64) / (used_ram_kb as f64);

            // Shutdown when both the absolute amount and percentage of free RAM is lower than
            // the set thresholds.
            Ok(fraction_ram_free < FREE_MEMORY_PERCENTAGE_THRESHOLD
                && free_ram_kb < FREE_MEMORY_KB_ABSOLUTE_THRESHOLD)
        })();
        match result {
            Ok(should_shutdown) => should_shutdown,
            Err(e) => {
                // PORT: the cause is the I/O error's text (Java prints the exception's stack
                // trace).
                blaze_util::server_logger::warning(
                    "com.google.devtools.build.lib.server.ServerWatcherRunnable$ProcMeminfoLowMemoryChecker",
                    "check",
                    "Unable to read memory info.",
                    Some(&e.to_string()),
                );
                false
            }
        }
    }
}

/// Checks to see if a server has been idle for too long and shuts down the server if so.
// Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:36 ServerWatcherRunnable
pub struct ServerWatcherRunnable {
    server: Arc<dyn ServerShutdown>,
    max_idle_seconds: i64,
    command_manager: Arc<dyn CommandManagerWatch>,
    low_memory_checker: LowMemoryChecker,
    shutdown_on_low_sys_mem: bool,
    clock: Arc<dyn Clock>,
}

impl ServerWatcherRunnable {
    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:136 ServerWatcherRunnable.<init>
    pub fn new(
        server: Arc<dyn ServerShutdown>,
        max_idle_seconds: i64,
        shutdown_on_low_sys_mem: bool,
        command_manager: Arc<dyn CommandManagerWatch>,
        clock: Arc<dyn Clock>,
    ) -> ServerWatcherRunnable {
        let checker = LowMemoryChecker::for_current_os(clock.clone());
        ServerWatcherRunnable::with_checker(
            server,
            max_idle_seconds,
            shutdown_on_low_sys_mem,
            command_manager,
            checker,
            clock,
        )
    }

    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:150 ServerWatcherRunnable.<init>
    pub fn with_checker(
        server: Arc<dyn ServerShutdown>,
        max_idle_seconds: i64,
        shutdown_on_low_sys_mem: bool,
        command_manager: Arc<dyn CommandManagerWatch>,
        low_memory_checker: LowMemoryChecker,
        clock: Arc<dyn Clock>,
    ) -> ServerWatcherRunnable {
        check_argument!(
            max_idle_seconds > 0,
            "Expected to only check idleness when --max_idle_secs > 0 but it was {}",
            max_idle_seconds
        );
        ServerWatcherRunnable {
            server,
            max_idle_seconds,
            command_manager,
            low_memory_checker,
            shutdown_on_low_sys_mem,
            clock,
        }
    }

    // Java: com/google/devtools/build/lib/server/ServerWatcherRunnable.java:168 ServerWatcherRunnable.run
    pub fn run(mut self) {
        let mut idle = self.command_manager.is_empty();
        let mut was_idle = false;
        let mut shutdown_time_millis: i64 = -1;

        loop {
            if !was_idle && idle {
                let now = self.clock.current_time_millis();
                shutdown_time_millis = now + self.max_idle_seconds * 1000;
                self.low_memory_checker.reset(now);
            }

            // An InterruptedException is dealt with by checking the current time below.
            let _ = if idle {
                // Verify.verify(shutdownTimeMillis > 0)
                if shutdown_time_millis <= 0 {
                    panic!("VerifyException");
                }
                if self.shutdown_on_low_sys_mem && self.low_memory_checker.should_shutdown() {
                    blaze_util::server_logger::severe(
                        "com.google.devtools.build.lib.server.ServerWatcherRunnable",
                        "run",
                        "Available RAM is low. Shutting down idle server...",
                        None,
                    );
                    break;
                }
                // Re-run the check every 5 seconds if no other commands have been sent to the
                // server.
                self.command_manager
                    .wait_for_change_with_timeout(IDLE_MEMORY_CHECK_INTERVAL.as_millis() as i64)
            } else {
                self.command_manager.wait_for_change()
            };

            was_idle = idle;
            idle = self.command_manager.is_empty();
            if was_idle && idle && self.clock.current_time_millis() >= shutdown_time_millis {
                blaze_util::server_logger::info(
                    "com.google.devtools.build.lib.server.ServerWatcherRunnable",
                    "run",
                    "About to shutdown due to idleness",
                );
                break;
            }
        }
        self.server.shutdown();
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/server/ServerWatcherRunnableTest.java
    use super::*;
    use blaze_shell::thread_interrupt::InterruptedException;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};

    /// `ManualClock` from lib/testutil.
    #[derive(Default)]
    struct ManualClock(AtomicI64);

    impl ManualClock {
        fn advance_millis(&self, millis: i64) {
            self.0.fetch_add(millis, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn current_time_millis(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
        fn nano_time(&self) -> i64 {
            self.0.load(Ordering::SeqCst) * 1_000_000
        }
    }

    /// `mock(Server.class)`: records `shutdown()` calls.
    #[derive(Default)]
    struct MockServer {
        shutdowns: AtomicUsize,
    }

    impl ServerShutdown for MockServer {
        fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// `mock(CommandManager.class)` with `isEmpty()` returning true and an answer for
    /// `waitForChange(long)`.
    struct MockCommands<F: Fn(i64) + Send + Sync> {
        on_wait_for_change: F,
    }

    impl<F: Fn(i64) + Send + Sync> CommandManagerWatch for MockCommands<F> {
        fn is_empty(&self) -> bool {
            true
        }
        fn wait_for_change(&self) -> Result<(), InterruptedException> {
            Ok(())
        }
        fn wait_for_change_with_timeout(&self, timeout: i64) -> Result<(), InterruptedException> {
            (self.on_wait_for_change)(timeout);
            Ok(())
        }
    }

    #[test]
    fn test_basic_idle_check() {
        let clock = Arc::new(ManualClock::default());
        let mock_server = Arc::new(MockServer::default());
        let check_idle_counter = Arc::new(AtomicUsize::new(0));
        let (c, s, k) = (
            clock.clone(),
            mock_server.clone(),
            check_idle_counter.clone(),
        );
        let mock_commands = Arc::new(MockCommands {
            on_wait_for_change: move |_| {
                k.fetch_add(1, Ordering::SeqCst);
                // verify(mockServer, never()).shutdown()
                assert_eq!(s.shutdowns.load(Ordering::SeqCst), 0);
                c.advance_millis(Duration::from_secs(5).as_millis() as i64);
            },
        });
        let under_test = ServerWatcherRunnable::new(
            mock_server.clone(),
            /* max_idle_seconds= */ 10,
            /* shutdown_on_low_sys_mem= */ false,
            mock_commands,
            clock.clone(),
        );
        let thread = std::thread::spawn(move || under_test.run());
        thread.join().unwrap();

        assert_eq!(mock_server.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(check_idle_counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn run_low_absolute_high_percentage_memory_check() {
        assert!(!does_idle_low_memory_check_shutdown(5000, 10000, true));
    }

    #[test]
    fn run_high_absolute_low_percentage_memory_check() {
        assert!(!does_idle_low_memory_check_shutdown(1 << 21, 1 << 30, true));
    }

    #[test]
    fn run_low_absolute_low_percentage_memory_check() {
        assert!(does_idle_low_memory_check_shutdown(5000, 1000000, true));
    }

    #[test]
    fn testshutdown_on_low_sys_mem_disabled() {
        assert!(!does_idle_low_memory_check_shutdown(5000, 1000000, false));
    }

    fn does_idle_low_memory_check_shutdown(
        free_ram_kb: i64,
        total_ram_kb: i64,
        shutdown_on_low_sys_mem: bool,
    ) -> bool {
        let clock = Arc::new(ManualClock::default());
        let mock_server = Arc::new(MockServer::default());
        let server_watcher_loop_counter = Arc::new(AtomicUsize::new(0));
        let wrong_timeout = Arc::new(AtomicBool::new(false));
        let (c, k, w) = (
            clock.clone(),
            server_watcher_loop_counter.clone(),
            wrong_timeout.clone(),
        );
        let mock_command_manager = Arc::new(MockCommands {
            on_wait_for_change: move |timeout| {
                // The answer is registered for waitForChange(Duration.ofSeconds(5).toMillis()).
                if timeout != Duration::from_secs(5).as_millis() as i64 {
                    w.store(true, Ordering::SeqCst);
                    return;
                }
                k.fetch_add(1, Ordering::SeqCst);
                c.advance_millis(Duration::from_secs(60).as_millis() as i64);
            },
        });
        // mock(ProcMeminfoParser.class) returning the given free and total RAM.
        let mock_parser = ProcMeminfoParser::from_content(&format!(
            "MemAvailable: {free_ram_kb} kB\nMemTotal: {total_ram_kb} kB\n"
        ));
        let under_test = ServerWatcherRunnable::with_checker(
            mock_server.clone(),
            // Shut down after an hour if we see no memory issues.
            /* max_idle_seconds= */
            Duration::from_secs(3600).as_secs() as i64,
            shutdown_on_low_sys_mem,
            mock_command_manager,
            LowMemoryChecker::new(
                clock.clone(),
                Box::new(ProcMeminfoLowMemoryChecker::new(Box::new(move || {
                    Ok(mock_parser.clone())
                }))),
            ),
            clock.clone(),
        );
        let thread = std::thread::spawn(move || under_test.run());
        thread.join().unwrap();
        assert_eq!(mock_server.shutdowns.load(Ordering::SeqCst), 1);
        assert!(!wrong_timeout.load(Ordering::SeqCst));

        // If we shut down due to memory pressure, it will only be after 5 minutes of being idle.
        server_watcher_loop_counter.load(Ordering::SeqCst) == 5
    }
}
