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
//! With `BLAZE_SERVER_MODE=daemon` or `jvm` ([`crate::server::ServerMode`]) the server is a
//! separate process and this is the C++ handler: [`install_for_server_process`], the cancel
//! thread of `BlazeServer` (`blaze.rs`) reading [`CancelThreadAction`]s from the pipe.
//!
//! PORT: in the default in-process mode the server runs in this process (`InProcessServer`), so
//! "cancel the command" calls the in-process server's cancel ([`crate::server::InProcessServer::canceller`]) on the cancel
//! thread instead of sending a `CancelRequest`, and the processes the server starts are put in
//! sessions of their own by the server (`blaze_shell::java_process::
//! set_start_children_in_new_session`), so that they do not get the terminal's SIGINT either.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::util::path::Path;
use crate::util::strings::console_bytes;

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

/// Whether the handler was installed by [`install_for_server_process`]: the server is a separate
/// process (`SignalHandler::Get().GetServerProcessInfo()` is that process).
static SERVER_PROCESS: AtomicBool = AtomicBool::new(false);

/// `SignalHandler::Get().GetServerProcessInfo()->server_pid_` (server-process mode).
static SERVER_PID: AtomicI32 = AtomicI32::new(-1);

/// `SignalHandler::Get().GetOutputBase()` (server-process mode).
static OUTPUT_BASE: OnceLock<Path> = OnceLock::new();

/// `BlazeServer::CancelThreadAction`.
// Java: src/main/cpp/blaze.cc:246 BlazeServer::CancelThreadAction
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CancelThreadAction {
    Nothing = 0,
    Join = 1,
    Cancel = 2,
    CommandIdReceived = 3,
    TerminalSizeChanged = 4,
}

impl CancelThreadAction {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(CancelThreadAction::Nothing),
            1 => Some(CancelThreadAction::Join),
            2 => Some(CancelThreadAction::Cancel),
            3 => Some(CancelThreadAction::CommandIdReceived),
            4 => Some(CancelThreadAction::TerminalSizeChanged),
            _ => None,
        }
    }
}

/// Makes `fd` the pipe [`send_action`] writes to (`BlazeServer::pipe_`, created in the
/// `BlazeServer` constructor).
pub(crate) fn set_cancel_pipe(fd: libc::c_int) {
    CANCEL_PIPE_WRITE.store(fd, Ordering::SeqCst);
}

/// `SigPrintf`: writes to stderr with `write(2)` (async-signal-safe).
// Java: src/main/cpp/blaze_util_posix.cc:610 SigPrintf
pub(crate) fn sig_print(buf: &[u8]) {
    // SAFETY: write(2) on a valid buffer; the result is ignored as in C++.
    unsafe {
        libc::write(libc::STDERR_FILENO, buf.as_ptr().cast(), buf.len());
    }
}

/// `BlazeServer::SendAction`: one byte into the cancel thread's pipe.
// Java: src/main/cpp/blaze.cc:2208 BlazeServer::SendAction
pub(crate) fn send_action(action: CancelThreadAction) {
    let fd = CANCEL_PIPE_WRITE.load(Ordering::SeqCst);
    if fd < 0 {
        return;
    }
    let byte = action as u8;
    // SAFETY: write(2) of one valid byte (async-signal-safe).
    let written = unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    // PosixPipe::Send: write(2) of the whole buffer.
    if written != 1 {
        sig_print(b"\nCould not interrupt server (cannot write to client pipe)\n\n");
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
                if SERVER_PROCESS.load(Ordering::SeqCst) {
                    let server_pid = SERVER_PID.load(Ordering::SeqCst);
                    if server_pid != -1
                        && let Some(output_base) = OUTPUT_BASE.get()
                    {
                        crate::platform::kill_server_process(server_pid, output_base);
                    }
                }
                // PORT (in-process mode only): KillServerProcess(server_pid_, output_base) kills the server's process
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
        // PORT: `case SIGPIPE: signal_handler_received_signal = SIGPIPE;` is
        // [`note_broken_pipe`]: SIGPIPE stays ignored in this process (see [`install`]).
        libc::SIGQUIT => {
            sig_print(&messages.quit);
            if SERVER_PROCESS.load(Ordering::SeqCst) {
                // SAFETY: kill(2) is async-signal-safe.
                unsafe { libc::kill(SERVER_PID.load(Ordering::SeqCst), libc::SIGQUIT) };
            }
            // PORT (in-process mode only): kill(server_pid_, SIGQUIT) asks the JVM for a thread dump in jvm.out. The
            // server is this process and has no JVM thread dump; sending the signal to ourselves
            // would only re-enter this handler.
        }
        libc::SIGWINCH => {
            // SignalHandler::Get().TerminalSizeChanged() -> BlazeServer::TerminalSizeChanged()
            send_action(CancelThreadAction::TerminalSizeChanged);
        }
        _ => {}
    }
    // SAFETY: errno location of this thread.
    unsafe { *libc::__errno_location() = saved_errno };
}

/// `SignalHandler::Install`: unblocks all signals and installs the handler for SIGINT, SIGTERM
/// and SIGQUIT; SIGPIPE is ignored ([`note_broken_pipe`]).
///
/// PORT: SIGWINCH is not handled: the client forwards terminal size changes to the server only
/// because the JVM is not attached to a terminal; the in-process server is.
// Java: src/main/cpp/blaze_util_posix.cc:195 SignalHandler::Install
// The C++ writes the raw bytes of these strings.
pub fn install(product_name: &str, _output_base: &Path, server_pid: i32, jvm_log_file: &Path) {
    let _ = MESSAGES.set(Messages {
        third_interrupt: console_bytes(&format!(
            "\n{product_name} caught third interrupt signal; server killed. (This may be \
             expensive, see https://bazel.build/advanced/performance/\
             iteration-speed#avoid-ctrl-c.)\n\n"
        ))
        .into_owned(),
        interrupt: console_bytes(&format!(
            "\n{product_name} caught interrupt signal; cancelling pending invocation.\n\n"
        ))
        .into_owned(),
        terminate: console_bytes(&format!(
            "\n{product_name} caught terminate signal; cancelling pending invocation.\n\n"
        ))
        .into_owned(),
        quit: console_bytes(&format!(
            "\nSending SIGQUIT to JVM process {server_pid} (see {}).\n\n",
            jvm_log_file.as_native_path()
        ))
        .into_owned(),
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
        // PORT: `signal(SIGPIPE, handler)`. In Bazel the client's only pipes are its stdout and
        // stderr; the sockets (remote cache, executor, BES) belong to the server, a JVM, which
        // ignores SIGPIPE. Here the server runs in this process, and a `writev` on one of its
        // sockets whose peer has reset raises SIGPIPE (std's vectored socket writes do not pass
        // MSG_NOSIGNAL), which the handler would turn into a death by SIGPIPE at the end of a
        // command that has nothing to do with the client's output. So SIGPIPE stays ignored
        // (Rust's startup disposition, also the JVM's), and the flag is set by
        // [`note_broken_pipe`] at the writes that are Bazel's client writes: the command's
        // output to this process's stdout and stderr. The same holds for the client process of
        // the server-process modes, whose only sockets are the gRPC connection to the server:
        // its EPIPE on stdout/stderr (`write_to_std_out_err`) sets the flag.
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, h);
    }
}

/// What the C++ handler does on SIGPIPE: records the signal, so that
/// [`propagate_signal_or_exit`] ends the process with it. Called when a write of the command's
/// output to this process's stdout or stderr fails with EPIPE (the write at which the C++
/// client's `fwrite` gets the signal); does nothing before [`install`] (batch mode, where
/// Bazel's process is the JVM, has no handler).
// Java: src/main/cpp/blaze_util_posix.cc:175 handler
pub fn note_broken_pipe() {
    if MESSAGES.get().is_some() {
        SIGNAL_HANDLER_RECEIVED_SIGNAL.store(libc::SIGPIPE, Ordering::SeqCst);
    }
}

/// `SignalHandler::Install` for a server that is a separate process (`BLAZE_SERVER_MODE=daemon`
/// or `jvm`): the C++ handler, including SIGWINCH, the third SIGINT killing the server process
/// group and SIGQUIT forwarded to the server. Cancel requests go to the pipe the
/// `BlazeServer` created ([`set_cancel_pipe`]).
// Java: src/main/cpp/blaze_util_posix.cc:195 SignalHandler::Install
pub fn install_for_server_process(
    product_name: &str,
    output_base: &Path,
    server_pid: i32,
    jvm_log_file: &Path,
) {
    SERVER_PROCESS.store(true, Ordering::SeqCst);
    SERVER_PID.store(server_pid, Ordering::SeqCst);
    let _ = OUTPUT_BASE.set(output_base.clone());
    install(product_name, output_base, server_pid, jvm_log_file);
    // SIGWINCH is reserved for Bazel server internal use and cannot be passed to it. The JVM is
    // not attached to a terminal, so the client forwards terminal size changes over the command
    // RPC channel.
    // SAFETY: `handler` is async-signal-safe for SIGWINCH (one write(2)).
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            handler as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
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
