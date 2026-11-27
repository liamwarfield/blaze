//! The `java.util.logging` loggers of the server's own classes (`GoogleLogger.forEnclosingClass()`
//! in the code below the runtime), as far as their records reach the server log.
//!
//! PORT: Blaze has no `java.util.logging`. The runtime owns the server log (`blaze_runtime`'s
//! `server_log`); crates below it log through this sink, which the runtime sets when it opens
//! the log. Without a sink (tests, tools) a record is dropped, as with no handler in Java.

use std::sync::OnceLock;

/// The `java.util.logging.Level` of a record (`intValue()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Severe = 1000,
    Warning = 900,
    Info = 800,
}

/// `(level, source class name, source method name, message, cause)` of a record; the cause is
/// the `toString()` of the throwable Java attaches with `withCause`.
pub type Sink = fn(Level, &str, &str, &str, Option<&str>);

static SINK: OnceLock<Sink> = OnceLock::new();

/// Sets the sink of the records (once per process).
pub fn set_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

/// `logger.at<level>().withCause(cause).log(message)` of class `source_class_name`, method
/// `source_method_name` (`cause`: `None` without `withCause`).
pub fn log(
    level: Level,
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    if let Some(sink) = SINK.get() {
        sink(level, source_class_name, source_method_name, message, cause);
    }
}

/// `logger.atInfo().log(message)` of class `source_class_name`, method `source_method_name`.
pub fn info(source_class_name: &str, source_method_name: &str, message: &str) {
    log(
        Level::Info,
        source_class_name,
        source_method_name,
        message,
        None,
    );
}

/// `logger.atWarning().withCause(cause).log(message)`.
pub fn warning(
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    log(
        Level::Warning,
        source_class_name,
        source_method_name,
        message,
        cause,
    );
}

/// `logger.atSevere().withCause(cause).log(message)`.
pub fn severe(
    source_class_name: &str,
    source_method_name: &str,
    message: &str,
    cause: Option<&str>,
) {
    log(
        Level::Severe,
        source_class_name,
        source_method_name,
        message,
        cause,
    );
}
