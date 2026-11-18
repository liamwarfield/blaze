//! The client's signal handling in client/server mode (`SignalHandler` of
//! `src/main/cpp/blaze_util_platform.h`, POSIX implementation in `blaze_util_posix.cc`) and the
//! cancel thread of `BlazeServer` (`blaze.cc`).
//!
//! Bazel's client installs this handler before it sends the command to the server: a Ctrl-C
//! (SIGINT) or SIGTERM prints a line and cancels the command over gRPC (the server interrupts the
//! command thread: `GrpcServerImpl.cancel`, `CommandManager.doCancel`); the third SIGINT kills
//! the server and exits with 1. The server is a daemon in a session of its own, so the terminal's
//! signals reach only the client.
//!
//! PORT: the server runs in this process (`InProcessServer`), so "cancel the command" calls the
//! in-process server's cancel ([`crate::server::InProcessServer::canceller`]) on the cancel
//! thread instead of sending a `CancelRequest`, and the processes the server starts are put in
//! sessions of their own by the server (`blaze_shell::java_process::
//! set_start_children_in_new_session`), so that they do not get the terminal's SIGINT either.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::util::path::Path;

/// The messages the handler writes, formatted before the handler is installed (a signal
/// handler may not format or allocate).
struct Messages {
    third_interrupt: Vec<u8>,
    interrupt: Vec<u8>,
    terminate: Vec<u8>,
    quit: Vec<u8>,
}

static MESSAGES: OnceLock<Messages> = OnceLock::new();

/// The number of the last received signal that should cause the client to shutdown. This is
/// saved so that the client's WTERMSIG can be set correctly. (Currently only SIGPIPE uses this
/// mechanism.)
// Java: src/main/cpp/blaze_util_posix.cc:141 signal_handler_received_signal
static SIGNAL_HANDLER_RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

// Java: src/main/cpp/blaze_util_posix.cc:147 handler sigint_count
static SIGINT_COUNT: AtomicI32 = AtomicI32::new(0);

/// The write end of the cancel thread's pipe (-1: none).
static CANCEL_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// `BlazeServer::CancelThreadAction`.
// Java: src/main/cpp/blaze.cc:246 BlazeServer::CancelThreadAction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum CancelThreadAction {
    Nothing = 0,
    Join = 1,
    Cancel = 2,
}

/// `SigPrintf`: writes to stderr with `write(2)` (async-signal-safe).
// Java: src/main/cpp/blaze_util_posix.cc:610 SigPrintf
fn sig_print(buf: &[u8]) {
    // SAFETY: write(2) on a valid buffer; the result is ignored as in C++.
    unsafe {
        libc::write(libc::STDERR_FILENO, buf.as_ptr().cast(), buf.len());
    }
}

/// `BlazeServer::SendAction`: one byte into the cancel thread's pipe.
// Java: src/main/cpp/blaze.cc:1873 BlazeServer::SendAction
fn send_action(action: CancelThreadAction) {
    let fd = CANCEL_PIPE_WRITE.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let byte = action as u8;
    // SAFETY: write(2) of one valid byte (async-signal-safe).
    unsafe {
        libc::write(fd, (&raw const byte).cast(), 1);
    }
}

/// Signal handler.
// Java: src/main/cpp/blaze_util_posix.cc:144 handler
extern "C" fn handler(signum: libc::c_int) {
    // SAFETY: errno location of this thread.
    let saved_errno = unsafe { *libc::__errno_location() };
    let Some(messages) = MESSAGES.get() else {
        return;
    };
    match signum {
        libc::SIGINT => {
            if SIGINT_COUNT.fetch_add(1, Ordering::SeqCst) + 1 >= 3 {
                sig_print(&messages.third_interrupt);
                // PORT: KillServerProcess(server_pid_, output_base) kills the server's process
                // group with SIGKILL. The server is this process, so `_exit` below ends it; its
                // linux-sandbox children die with it (PR_SET_PDEATHSIG).
                // SAFETY: _exit is async-signal-safe.
                unsafe { libc::_exit(1) };
            }
            sig_print(&messages.interrupt);
            // SignalHandler::Get().CancelServer() -> BlazeServer::Cancel()
            send_action(CancelThreadAction::Cancel);
        }
        libc::SIGTERM => {
            sig_print(&messages.terminate);
            send_action(CancelThreadAction::Cancel);
        }
        libc::SIGPIPE => {
            SIGNAL_HANDLER_RECEIVED_SIGNAL.store(libc::SIGPIPE, Ordering::SeqCst);
        }
        libc::SIGQUIT => {
            sig_print(&messages.quit);
            // PORT: kill(server_pid_, SIGQUIT) asks the JVM for a thread dump in jvm.out. The
            // server is this process and has no JVM thread dump; sending the signal to ourselves
            // would only re-enter this handler.
        }
        _ => {}
    }
    // SAFETY: errno location of this thread.
    unsafe { *libc::__errno_location() = saved_errno };
}

/// `SignalHandler::Install`: unblocks all signals and installs the handler for SIGINT, SIGTERM,
/// SIGPIPE and SIGQUIT.
///
/// PORT: SIGWINCH is not handled: the client forwards terminal size changes to the server only
/// because the JVM is not attached to a terminal; the in-process server is.
// Java: src/main/cpp/blaze_util_posix.cc:195 SignalHandler::Install
pub fn install(product_name: &str, _output_base: &Path, server_pid: i32, jvm_log_file: &Path) {
    let _ = MESSAGES.set(Messages {
        third_interrupt: format!(
            "\n{product_name} caught third interrupt signal; server killed. (This may be \
             expensive, see https://bazel.build/advanced/performance/\
             iteration-speed#avoid-ctrl-c.)\n\n"
        )
        .into_bytes(),
        interrupt: format!(
            "\n{product_name} caught interrupt signal; cancelling pending invocation.\n\n"
        )
        .into_bytes(),
        terminate: format!(
            "\n{product_name} caught terminate signal; cancelling pending invocation.\n\n"
        )
        .into_bytes(),
        quit: format!(
            "\nSending SIGQUIT to JVM process {server_pid} (see {}).\n\n",
            jvm_log_file.as_native_path()
        )
        .into_bytes(),
    });

    // SAFETY: plain libc calls with valid arguments; `handler` is async-signal-safe.
    unsafe {
        // Unblock all signals.
        let mut sigset: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut sigset);
        libc::sigprocmask(libc::SIG_SETMASK, &sigset, std::ptr::null_mut());

        let h = handler as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
        libc::signal(libc::SIGPIPE, h);
        libc::signal(libc::SIGQUIT, h);
    }
}

/// `SignalHandler::PropagateSignalOrExit`: kills this process with the signal that should end
/// it (SIGPIPE), or returns `exit_code` for the caller to exit with.
// Java: src/main/cpp/blaze_util_posix.cc:221 SignalHandler::PropagateSignalOrExit
pub fn propagate_signal_or_exit(exit_code: i32) -> i32 {
    let signal = SIGNAL_HANDLER_RECEIVED_SIGNAL.load(Ordering::SeqCst);
    if signal != 0 {
        // Kill ourselves with the same signal, so that callers see the right WTERMSIG value.
        // SAFETY: plain libc calls.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        std::process::exit(1); // (in case raise didn't kill us for some reason)
    }
    exit_code
}

/// Runs `command` while a cancel thread turns the handler's cancel requests into calls of
/// `cancel` (`BlazeServer`'s cancel thread, started with the server connection and joined at
/// the end of `Communicate`).
// Java: src/main/cpp/blaze.cc:1842 BlazeServer::CancelThread
pub fn with_cancel_thread<R>(
    cancel: std::sync::Arc<dyn Fn() + Send + Sync>,
    command: impl FnOnce() -> R,
) -> R {
    let mut fds: [libc::c_int; 2] = [-1, -1];
    // SAFETY: fds is a valid array of two ints.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        // Java: src/main/cpp/blaze.cc:1728 BlazeServer::BlazeServer
        crate::util::logging::bazel_die(
            crate::util::exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            "src/main/cpp/blaze.cc",
            1730,
            format!(
                "Couldn't create pipe: {}",
                crate::util::errors::get_last_error_string()
            ),
        );
    }
    let (read_fd, write_fd) = (fds[0], fds[1]);
    CANCEL_PIPE_WRITE.store(write_fd, Ordering::SeqCst);
    /// Closes the pipe once the cancel thread is gone.
    struct ClosePipe(libc::c_int, libc::c_int);
    impl Drop for ClosePipe {
        fn drop(&mut self) {
            let _ =
                CANCEL_PIPE_WRITE.compare_exchange(self.1, -1, Ordering::SeqCst, Ordering::SeqCst);
            // SAFETY: descriptors created above, no longer used.
            unsafe {
                libc::close(self.0);
                libc::close(self.1);
            }
        }
    }
    let _close = ClosePipe(read_fd, write_fd);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            loop {
                let mut buf: u8 = 0;
                // SAFETY: reads one byte into a valid buffer.
                let n = unsafe { libc::read(read_fd, (&raw mut buf).cast(), 1) };
                if n < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                } else if n != 1 {
                    crate::util::logging::bazel_die(
                        crate::util::exit_code::INTERNAL_ERROR,
                        "src/main/cpp/blaze.cc",
                        1865,
                        format!(
                            "Cannot communicate with cancel thread: {}",
                            crate::util::errors::get_last_error_string()
                        ),
                    );
                }
                match buf {
                    b if b == CancelThreadAction::Nothing as u8 => {}
                    b if b == CancelThreadAction::Join as u8 => return,
                    b if b == CancelThreadAction::Cancel as u8 => {
                        // PORT: the command id is known at once in-process (the command runs
                        // as soon as the request is handed over), so a cancel is never pending
                        // for COMMAND_ID_RECEIVED; SendCancelMessage is the server's cancel.
                        cancel();
                    }
                    _ => {}
                }
            }
        });
        // The cancel thread is joined at the end of the execution of the command, also when the
        // command panics (the scope waits for the thread before the panic propagates).
        struct JoinOnDrop(libc::c_int);
        impl Drop for JoinOnDrop {
            fn drop(&mut self) {
                // Into this call's own pipe (the handler writes to the latest one).
                let byte = CancelThreadAction::Join as u8;
                // SAFETY: write(2) of one valid byte; the descriptor is open until the thread
                // has joined.
                unsafe {
                    libc::write(self.0, (&raw const byte).cast(), 1);
                }
            }
        }
        let _join = JoinOnDrop(write_fd);
        command()
    })
}
