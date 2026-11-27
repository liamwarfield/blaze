//! `PidFileWatcher`: watches if the PID file changes and shuts down the server immediately if so.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use blaze_util::check_state;
use blaze_util::exit_code::ExitCode;
use blaze_util::string_encoding::latin1_decode;
use blaze_vfs::Path;
use parking_lot::Mutex;

use crate::interruptible::Monitor;

/// The scheduled executor of Java (`scheduleWithFixedDelay(iteration, 3, 3, SECONDS)`), as a
/// thread that `endWatch` stops.
struct Executor {
    /// `shutdownNow` was called.
    stopped: Monitor<bool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// A class that watches if the PID file changes and shuts down the server immediately if so.
// Java: com/google/devtools/build/lib/server/PidFileWatcher.java:33 PidFileWatcher
pub struct PidFileWatcher {
    pid_file: Path,
    server_pid: i32,
    on_pid_file_change: Box<dyn Fn() + Send + Sync>,
    shutting_down: AtomicBool,
    started: AtomicBool,
    executor: Executor,
}

impl PidFileWatcher {
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:45 PidFileWatcher.<init>
    pub fn new(pid_file: Path, server_pid: i32) -> PidFileWatcher {
        PidFileWatcher::with_callback(
            pid_file,
            server_pid,
            // Runtime.getRuntime().halt(ExitCode.BLAZE_INTERNAL_ERROR): no shutdown hooks.
            Box::new(|| {
                // SAFETY: _exit ends the process at once, as Runtime.halt does.
                unsafe { libc::_exit(ExitCode::BLAZE_INTERNAL_ERROR.get_numeric_exit_code()) }
            }),
        )
    }

    /// Test-only constructor allowing a more gentle `on_pid_file_change` callback.
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:54 PidFileWatcher.<init>
    pub fn with_callback(
        pid_file: Path,
        server_pid: i32,
        on_pid_file_change: Box<dyn Fn() + Send + Sync>,
    ) -> PidFileWatcher {
        PidFileWatcher {
            pid_file,
            server_pid,
            on_pid_file_change,
            shutting_down: AtomicBool::new(false),
            started: AtomicBool::new(false),
            executor: Executor {
                stopped: Monitor::new(false),
                thread: Mutex::new(None),
            },
        }
    }

    /// Starts the watching. Can only be called once per instance.
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:68 PidFileWatcher.start
    pub fn start(self: &Arc<Self>) {
        // Can only be called once
        check_state!(!self.started.swap(true, Ordering::SeqCst));
        let this = self.clone();
        // Java: new ThreadFactoryBuilder().setNameFormat("pid-file-watcher-%d").setDaemon(true)
        let handle = std::thread::Builder::new()
            .name("pid-file-watcher-0".to_string())
            .spawn(move || {
                loop {
                    {
                        let mut stopped = this.executor.stopped.lock();
                        if *stopped {
                            return;
                        }
                        // scheduleWithFixedDelay(this::iteration, 3, 3, SECONDS)
                        let _ = this
                            .executor
                            .stopped
                            .wait(&mut stopped, Some(Duration::from_secs(3)));
                        if *stopped {
                            return;
                        }
                    }
                    // iteration(): iteration.cancel(false) when the checks say stop.
                    if !this.run_pid_file_checks() {
                        return;
                    }
                }
            })
            .expect("failed to start the pid file watcher thread");
        *self.executor.thread.lock() = Some(handle);
    }

    /// Signals a shutdown is in progress, thus PID file changes can be expected and should not
    /// trigger an aggressive shutdown.
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:77 PidFileWatcher.signalShutdown
    pub fn signal_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
    }

    /// Stops the watching. Once this method returns, no more activity will happen.
    ///
    /// This should invoked before the PID file is deleted by the Blaze server on an orderly
    /// shutdown.
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:87 PidFileWatcher.endWatch
    pub fn end_watch(&self) {
        {
            let mut stopped = self.executor.stopped.lock();
            *stopped = true;
            self.executor.stopped.notify_all();
        }
        let handle = self.executor.thread.lock().take();
        if let Some(handle) = handle
            && handle.thread().id() != std::thread::current().id()
        {
            let _ = handle.join();
        }
    }

    /// Runs one iteration of PID file checks. Returns whether or not the main loop should
    /// continue, or crashes the program via the `on_pid_file_change` callback.
    // Java: com/google/devtools/build/lib/server/PidFileWatcher.java:105 PidFileWatcher.runPidFileChecks
    pub fn run_pid_file_checks(&self) -> bool {
        if pid_file_valid(&self.pid_file, self.server_pid) {
            return true;
        }

        if self.shutting_down.load(Ordering::SeqCst) {
            blaze_util::server_logger::warning(
                "com.google.devtools.build.lib.server.PidFileWatcher",
                "runPidFileChecks",
                "PID file deleted or overwritten but shutdown is already in progress",
                None,
            );
            return false;
        }

        // Someone overwrote the PID file. Maybe it's another server, so shut down as quickly
        // as possible without even running the shutdown hooks (that would delete it)
        blaze_util::server_logger::severe(
            "com.google.devtools.build.lib.server.PidFileWatcher",
            "runPidFileChecks",
            "PID file deleted or overwritten, exiting as quickly as possible",
            None,
        );
        (self.on_pid_file_change)();
        panic!("IllegalStateException: Should have crashed.");
    }
}

// Java: com/google/devtools/build/lib/server/PidFileWatcher.java:122 PidFileWatcher.pidFileValid
fn pid_file_valid(pid_file: &Path, expected_pid: i32) -> bool {
    // FileSystemUtils.readContentAsLatin1
    let mut bytes = Vec::new();
    let read = pid_file
        .get_input_stream()
        .and_then(|mut input| input.read_to_end(&mut bytes));
    if let Err(e) = read {
        // PORT: e.getMessage() is the I/O error's text.
        blaze_util::server_logger::info(
            "com.google.devtools.build.lib.server.PidFileWatcher",
            "pidFileValid",
            &format!("Cannot read PID file: {e}"),
        );
        return false;
    }
    let pid_file_contents = latin1_decode(&bytes);

    // Integer.parseInt: an optional sign and ASCII digits (the only decimal digits in Latin-1).
    match pid_file_contents.parse::<i32>() {
        Ok(pid) => pid == expected_pid,
        Err(_) => {
            blaze_util::server_logger::warning(
                "com.google.devtools.build.lib.server.PidFileWatcher",
                "pidFileValid",
                &format!("Pid was invalid: {pid_file_contents}"),
                Some(&format!(
                    "java.lang.NumberFormatException: For input string: \"{pid_file_contents}\""
                )),
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/server/PidFileWatcherTest.java
    use super::*;
    use blaze_vfs::{DigestHashFunction, UnixFileSystem};
    use std::io::Write;

    const EXPECTED_PID: i32 = 42;
    const THROWN_ON_HALT: &str = "crash!";

    struct Fixture {
        _dir: tempfile::TempDir,
        pid_file: Path,
        under_test: PidFileWatcher,
    }

    fn set_up() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let pid_file = Path::create_from_str(&format!("{}/pid", dir.path().to_str().unwrap()), &fs);
        let under_test = PidFileWatcher::with_callback(
            pid_file.clone(),
            EXPECTED_PID,
            Box::new(|| std::panic::panic_any(THROWN_ON_HALT)),
        );
        Fixture {
            _dir: dir,
            pid_file,
            under_test,
        }
    }

    fn write_content(path: &Path, content: &[u8]) {
        let mut out = path.get_output_stream().unwrap();
        out.write_all(content).unwrap();
    }

    fn assert_pid_check_halts_program(under_test: &PidFileWatcher) {
        let expected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            under_test.run_pid_file_checks()
        }))
        .expect_err("expected the halt callback to throw");
        // isSameInstanceAs(THROWN_ON_HALT): the payload is the one the callback threw.
        assert_eq!(expected.downcast_ref::<&str>(), Some(&THROWN_ON_HALT));
    }

    #[test]
    fn test_missing_pid_file_halts_program() {
        let f = set_up();
        // Delete just in case.
        let _ = f.pid_file.delete();

        assert_pid_check_halts_program(&f.under_test);
    }

    #[test]
    fn test_empty_pid_file_counts_as_changed() {
        let f = set_up();
        write_content(&f.pid_file, b"");

        assert_pid_check_halts_program(&f.under_test);
    }

    #[test]
    fn test_garbage_pid_file_counts_as_changed() {
        let f = set_up();
        write_content(&f.pid_file, b"junk");

        assert_pid_check_halts_program(&f.under_test);
    }

    #[test]
    fn test_pid_file_continues_execution() {
        let f = set_up();
        write_content(&f.pid_file, b"42");

        assert!(f.under_test.run_pid_file_checks());
    }

    #[test]
    fn test_pid_file_trailing_whitespace_not_tolerated() {
        let f = set_up();
        write_content(&f.pid_file, b"42\n");

        assert_pid_check_halts_program(&f.under_test);
    }

    #[test]
    fn test_pid_file_change_after_shutdown_notification_stops_watcher() {
        let f = set_up();
        write_content(&f.pid_file, b"42\n");

        f.under_test.signal_shutdown();
        assert!(!f.under_test.run_pid_file_checks());
    }
}
