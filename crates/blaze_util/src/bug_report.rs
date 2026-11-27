//! `BugReport.sendBugReport` for the code below the runtime: an unexpected state that is not a
//! crash is reported and the program continues.
//!
//! PORT: Bazel reports to a remote logging service only from a Google-internal release; in a
//! Bazel release (a binary with a build label, `BlazeVersionInfo.isReleasedBlaze`) the report is
//! the server log records of `logException`, and in a binary without one a single INFO record.
//! In a test (`TestType.isInTest`, unless `ENABLE_BUG_REPORT_LOGGING_IN_TEST` is set) a bug
//! report crashes: Java rethrows the exception, Blaze panics with its text.

use std::sync::OnceLock;

use crate::server_logger;

static RELEASED: OnceLock<bool> = OnceLock::new();

/// `SHOULD_NOT_SEND_BUG_REPORT_BECAUSE_IN_TEST`.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:78 BugReport.SHOULD_NOT_SEND_BUG_REPORT_BECAUSE_IN_TEST
fn should_not_send_bug_report_because_in_test() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        crate::test_type::TestType::is_in_test()
            && std::env::var_os("ENABLE_BUG_REPORT_LOGGING_IN_TEST").is_none()
    })
}

/// Records `BlazeVersionInfo.instance().isReleasedBlaze()` (set once by the runtime).
pub fn set_released(released: bool) {
    let _ = RELEASED.set(released);
}

const CLASS: &str = "com.google.devtools.build.lib.bugreport.BugReport";

/// `BugReport.sendBugReport(String message, Object... args)`: reports a new
/// `IllegalStateException(message)`.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:195 BugReport.sendBugReport
pub fn send_bug_report(message: &str) {
    send_bug_report_internal(&format!("java.lang.IllegalStateException: {message}"));
}

// Java: com/google/devtools/build/lib/bugreport/BugReport.java:223 BugReport.sendBugReportInternal
fn send_bug_report_internal(exception: &str) {
    if should_not_send_bug_report_because_in_test() {
        // `Throwables.throwIfUnchecked(exception)`: the exception (an IllegalStateException) is
        // rethrown. PORT: a panic with its text.
        panic!("{exception}");
    }
    if !RELEASED.get().copied().unwrap_or(false) {
        server_logger::info(
            CLASS,
            "sendBugReportInternal",
            "(Not a released binary; not logged.)",
        );
        return;
    }
    log_exception(exception);
}

/// PORT: `LoggingUtil.logToRemote` has no remote logger in Bazel (`installRemoteLogger` is
/// called only by Google-internal modules), so only the local records remain.
// Java: com/google/devtools/build/lib/bugreport/BugReport.java:447 BugReport.logException
fn log_exception(exception: &str) {
    server_logger::severe(CLASS, "logException", "Exception", Some(exception));
    server_logger::info(CLASS, "logException", "Calling logToRemote");
    server_logger::info(CLASS, "logException", "Call to logToRemote complete");
}
