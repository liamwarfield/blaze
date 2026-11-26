//! The signal behaviour of the JVM that runs Bazel's server (`BlazeRuntime.main`, server and
//! batch mode), as far as a client or `jvm.out` can observe it.
//!
//! PORT: Blaze only: the JVM installs these dispositions before `main`; Blaze's `main` is the
//! process entry, so [`install`] stands for them. The JVM installs handlers rather than ignoring
//! the signals, so a child process (an action, `run`'s binary in batch mode) starts with the
//! default dispositions after `exec`; the handlers here are installed the same way.
//! - `SIGPIPE`: the JVM's handler does nothing, so a write to a closed pipe or socket fails with
//!   `EPIPE` (an `IOException`) and never kills the process. This matters for the gRPC server,
//!   whose socket writes (hyper/h2, vectored without `MSG_NOSIGNAL`) would otherwise kill the
//!   server when a client goes away.
//! - `SIGQUIT`: HotSpot prints a thread dump to stdout (`jvm.out`) and continues. Blaze has no
//!   thread dump; the handler does nothing, so the process continues as the JVM does.
//! - `SIGTERM`, `SIGHUP`, `SIGINT` (each unless ignored when the process starts, as HotSpot
//!   leaves an ignored shutdown signal ignored): HotSpot's `Terminator` calls
//!   `Shutdown.exit(128 + signal)`:
//!   every shutdown hook runs (`ShutdownHooks` deletes the server's files; the worker processes'
//!   hooks destroy them; `VirtualCgroup`'s hook deletes the server's cgroup), then the process
//!   exits with status 143, 129 or 130. Bazel's own SIGINT handler (`crate::interrupt`,
//!   `BlazeRuntime.captureSigint` in batch mode) replaces the JVM's for a command and reverts to
//!   it after the second interrupt, so a third one is this `Shutdown.exit(130)` (review cgroups
//!   round 1, C1).

use std::sync::Once;
use std::sync::atomic::{AtomicI32, Ordering};

/// The write end of the pipe the terminating-signal handler writes to.
static TERMINATE_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn do_nothing(_signal: libc::c_int) {}

/// Wakes the `SIGTERM handler` thread with the signal number. Only async-signal-safe calls.
extern "C" fn on_terminate(signal: libc::c_int) {
    let fd = TERMINATE_PIPE_WRITE.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = signal as u8;
        // SAFETY: write(2) is async-signal-safe; the buffer is one valid byte.
        unsafe {
            libc::write(fd, (&raw const byte).cast::<libc::c_void>(), 1);
        }
    }
}

fn set_handler(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    // SAFETY: a zeroed sigaction is valid; the handler only calls async-signal-safe functions.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut());
    }
}

/// Installs the JVM's dispositions for `SIGPIPE`, `SIGQUIT`, `SIGTERM` and `SIGHUP` (once per
/// process).
pub fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        set_handler(libc::SIGPIPE, do_nothing);
        set_handler(libc::SIGQUIT, do_nothing);

        let mut fds: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `fds` is a valid array of two ints.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            panic!(
                "IllegalStateException: pipe2: {}",
                std::io::Error::last_os_error()
            );
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        TERMINATE_PIPE_WRITE.store(write_fd, Ordering::SeqCst);
        std::thread::Builder::new()
            .name("SIGTERM handler".to_owned())
            .spawn(move || {
                // An exception of the shutdown hooks reaches the default uncaught exception
                // handler.
                blaze_util::uncaught_exception::run_catching(|| {
                    loop {
                        let mut byte: u8 = 0;
                        // SAFETY: reads one byte into a valid buffer.
                        let n = unsafe {
                            libc::read(read_fd, (&raw mut byte).cast::<libc::c_void>(), 1)
                        };
                        if n < 0
                            && std::io::Error::last_os_error().kind()
                                == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        if n <= 0 {
                            return;
                        }
                        // Shutdown.exit(128 + signal): the shutdown hooks, then halt; blocks
                        // for good if a normal exit is already running the hooks.
                        blaze_exec::worker::shutdown_hooks::begin_shutdown();
                        run_all_shutdown_hooks();
                        let _ = blaze_events::OutErr::system_out_err()
                            .get_error_stream()
                            .flush();
                        let _ = blaze_events::OutErr::system_out_err()
                            .get_output_stream()
                            .flush();
                        // SAFETY: _exit ends the process, as Runtime.halt does after the hooks.
                        unsafe { libc::_exit(128 + byte as libc::c_int) }
                    }
                });
            })
            .unwrap_or_else(|e| panic!("OutOfMemoryError: unable to create native thread: {e}"));
        // java.lang.Terminator.setup registers HUP, INT and TERM; HotSpot's JVM_RegisterSignal
        // leaves a shutdown signal that is ignored at startup ignored (the server started by
        // `daemonize` inherits SIGHUP ignored, so Bazel's server survives a SIGHUP).
        // Java: java/lang/Terminator.java Terminator.setup (JDK)
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
            if !is_ignored(signal) {
                set_handler(signal, on_terminate);
            }
        }
    });
}

/// `os::Posix::is_sig_ignored(sig)`: whether the current disposition of `signal` is `SIG_IGN`.
pub(crate) fn is_ignored(signal: libc::c_int) -> bool {
    // SAFETY: a zeroed sigaction is valid output storage; a null new action only queries.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        libc::sigaction(signal, std::ptr::null(), &mut old) == 0
            && old.sa_sigaction == libc::SIG_IGN
    }
}

/// Every hook `Runtime.addShutdownHook` would hold in Java: the server's `ShutdownHooks` (its
/// files) and the hooks of the worker processes and of `VirtualCgroup` (registered in
/// `blaze_exec`, which an `atexit` handler runs on a normal exit; `_exit` skips that handler, so
/// they run here).
///
/// PORT: the JVM starts all hooks at once, in no order; here the `blaze_exec` hooks run first
/// (newest first, see `blaze_exec::worker::shutdown_hooks::run_shutdown_hooks`), then the
/// server's files are deleted.
// Java: java/lang/Shutdown.java Shutdown.exit (JDK)
pub fn run_all_shutdown_hooks() {
    // A panic in one list does not keep the other from running (each Java hook is a thread).
    let exec_hooks =
        std::panic::catch_unwind(blaze_exec::worker::shutdown_hooks::run_shutdown_hooks);
    let server_hooks =
        std::panic::catch_unwind(blaze_server::shutdown_hooks::run_registered_shutdown_hooks);
    for result in [exec_hooks, server_hooks] {
        if let Err(payload) = result {
            blaze_exec::worker::shutdown_hooks::report_hook_panic(&*payload);
        }
    }
}
