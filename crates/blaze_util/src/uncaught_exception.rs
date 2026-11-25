//! The process's default uncaught exception handler (Java
//! `Thread.setDefaultUncaughtExceptionHandler`) for panics that escape the body of a thread.
//!
//! PORT: a Java exception that escapes a thread's `run` goes to the default uncaught exception
//! handler, which Bazel sets to `BugReport.handleCrash` (`BlazeRuntime
//! .setupUncaughtHandlerAtStartup`, then `newRuntime`): the crash is reported and the server
//! halts with the crash's exit code (37 for a bug). A Rust panic that escapes a thread nobody
//! joins only ends that thread. So the body of every thread whose Java counterpart is a plain
//! `Thread` (or a task `execute`d on an executor, whose exception also reaches the thread's
//! handler) runs through [`run_catching`], which hands an escaping panic to the handler
//! `blaze_runtime` installs ([`set_default_uncaught_exception_handler`]). Threads whose Java
//! counterpart keeps the exception in a `Future` (a `submit`ted task, a joined helper whose
//! result is read) are not wrapped: there the panic reaches whoever waits for the result.
//!
//! The handler lives here, below every crate that starts threads, because `BugReport` is in
//! `blaze_runtime`, above them.

use std::any::Any;
use std::sync::{Arc, LazyLock};

use parking_lot::RwLock;

/// How the handler reports the crash: Java's `CrashContext.halt()` (the default handler) or
/// `CrashContext.haltOrReturnIfCrashInProgress()` (threads that catch `Throwable` themselves and
/// must not block while another thread reports a crash, such as the UI's `cli-update-thread`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashContextKind {
    /// `CrashContext.halt()`.
    Halt,
    /// `CrashContext.haltOrReturnIfCrashInProgress()`.
    HaltOrReturnIfCrashInProgress,
}

/// A handler for a panic that escaped a thread, given the panic's message. With
/// [`CrashContextKind::Halt`] it must not return.
pub type UncaughtExceptionHandler = Arc<dyn Fn(&str, CrashContextKind) + Send + Sync>;

static HANDLER: LazyLock<RwLock<Option<UncaughtExceptionHandler>>> =
    LazyLock::new(|| RwLock::new(None));

/// `Thread.setDefaultUncaughtExceptionHandler(handler)`.
pub fn set_default_uncaught_exception_handler(handler: UncaughtExceptionHandler) {
    *HANDLER.write() = Some(handler);
}

/// `Thread.getDefaultUncaughtExceptionHandler()`.
pub fn get_default_uncaught_exception_handler() -> Option<UncaughtExceptionHandler> {
    HANDLER.read().clone()
}

/// The message of a panic payload (`panic!` with a message gives a `&str` or a `String`).
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_string()
    }
}

/// Runs `body`, the whole body of a thread; a panic that escapes it goes to the default uncaught
/// exception handler ([`handle_uncaught_exception`]).
pub fn run_catching<T>(body: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => Some(value),
        Err(payload) => {
            handle_uncaught_exception(&*payload, CrashContextKind::Halt);
            None
        }
    }
}

/// [`run_catching`] for a thread whose Java body ends in `catch (Throwable t) {
/// BugReport.handleCrash(Crash.from(t), CrashContext.haltOrReturnIfCrashInProgress()); }`.
pub fn run_catching_or_return_if_crash_in_progress<T>(body: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => Some(value),
        Err(payload) => {
            handle_uncaught_exception(&*payload, CrashContextKind::HaltOrReturnIfCrashInProgress);
            None
        }
    }
}

/// Hands a panic that escaped a thread's body to the default uncaught exception handler.
///
/// Without one (a unit test, or a process that never installed one) it does what the JVM's own
/// default does (`ThreadGroup.uncaughtException`): it prints `Exception in thread "<name>"` and
/// the message to stderr, and the thread ends.
pub fn handle_uncaught_exception(payload: &(dyn Any + Send), kind: CrashContextKind) {
    let message = panic_message(payload);
    match get_default_uncaught_exception_handler() {
        Some(handler) => handler(&message, kind),
        None => {
            let thread = std::thread::current();
            eprintln!(
                "Exception in thread \"{}\" {}",
                thread.name().unwrap_or("<unnamed>"),
                message
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[test]
    fn escaping_panic_reaches_the_handler_with_its_kind() {
        let seen: Arc<Mutex<Vec<(String, CrashContextKind)>>> = Arc::default();
        let recorder = seen.clone();
        set_default_uncaught_exception_handler(Arc::new(move |message, kind| {
            recorder.lock().push((message.to_owned(), kind));
        }));
        std::thread::spawn(|| run_catching(|| panic!("IllegalStateException: on a thread")))
            .join()
            .unwrap();
        std::thread::spawn(|| {
            run_catching_or_return_if_crash_in_progress(|| panic!("{}", "boom".to_owned()))
        })
        .join()
        .unwrap();
        assert_eq!(run_catching(|| 7), Some(7));
        assert_eq!(
            *seen.lock(),
            vec![
                (
                    "IllegalStateException: on a thread".to_owned(),
                    CrashContextKind::Halt
                ),
                (
                    "boom".to_owned(),
                    CrashContextKind::HaltOrReturnIfCrashInProgress
                ),
            ]
        );
    }
}
