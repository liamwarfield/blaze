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
//! - `SIGTERM`, `SIGHUP`: HotSpot's `Terminator` calls `Shutdown.exit(128 + signal)`: the
//!   shutdown hooks run (`ShutdownHooks` deletes the server's files), then the process exits
//!   with status 143 or 129.
//! - `SIGINT` is Bazel's own `InterruptSignalHandler` (`crate::interrupt`).

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
                        // Shutdown.exit(128 + signal): the shutdown hooks, then halt.
                        blaze_server::shutdown_hooks::run_registered_shutdown_hooks();
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
        set_handler(libc::SIGTERM, on_terminate);
        set_handler(libc::SIGHUP, on_terminate);
    });
}
