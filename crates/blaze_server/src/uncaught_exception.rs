//! What happens when a panic escapes one of the server's threads: Java's default uncaught
//! exception handler.
//!
//! PORT: Bazel installs `BugReport`'s handler (`BlazeRuntime.setupUncaughtHandlerAtStartup`,
//! then `newRuntime`), whose `handleCrash` logs, notifies the modules (`blazeShutdownOnCrash`),
//! writes the exit code for the client and halts the JVM. `blaze_runtime` installs that handler
//! with [`set_uncaught_exception_handler`]; until it does (and in this crate's own tests and
//! probe), the default handler prints the panic to stderr (the server's `jvm.out`) and halts
//! with `BLAZE_INTERNAL_ERROR` (37), never leaving a server whose command died behind.
//!
//! A Rust thread has no uncaught-exception handler: the threads of this crate whose Java
//! counterparts are plain threads or a `ThreadPoolExecutor` (whose `execute`d tasks' exceptions
//! reach the thread's handler) run their body through [`run_catching`]: the `grpc-command-N`
//! threads, `grpc-timeout-and-memory` and `interrupt-watcher-N`. The threads Java runs on a
//! `ScheduledExecutorService` (`pid-file-watcher-N`, `idle-server-tasks-N`) keep an exception in
//! their future instead, so they are not wrapped.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use blaze_util::exit_code::ExitCode;
use parking_lot::RwLock;

/// A handler for a panic that escaped a server thread. It must not return normally if the
/// process cannot continue.
pub type UncaughtExceptionHandler = Arc<dyn Fn(&str) + Send + Sync>;

static HANDLER: LazyLock<RwLock<Option<UncaughtExceptionHandler>>> =
    LazyLock::new(|| RwLock::new(None));

/// `Thread.setDefaultUncaughtExceptionHandler` for the threads this crate starts.
pub fn set_uncaught_exception_handler(handler: UncaughtExceptionHandler) {
    *HANDLER.write() = Some(handler);
}

/// The panic message of `payload`.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Runs `body`, the whole body of a thread; a panic that escapes it is an uncaught exception
/// ([`handle_uncaught_exception`]).
pub fn run_catching(body: impl FnOnce()) {
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        handle_uncaught_exception(&*payload);
    }
}

/// Handles a panic that escaped a server thread.
pub fn handle_uncaught_exception(payload: &(dyn Any + Send)) {
    let message = panic_message(payload);
    let handler = HANDLER.read().clone();
    match handler {
        Some(handler) => handler(&message),
        None => {
            let thread = std::thread::current();
            eprintln!(
                "FATAL: uncaught panic in thread {}: {}",
                thread.name().unwrap_or("<unnamed>"),
                message
            );
            // SAFETY: _exit ends the process at once, as Runtime.halt does.
            unsafe { libc::_exit(ExitCode::BLAZE_INTERNAL_ERROR.get_numeric_exit_code()) }
        }
    }
}
