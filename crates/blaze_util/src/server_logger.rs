//! The `java.util.logging` loggers of the server's own classes (`GoogleLogger.forEnclosingClass()`
//! in the code below the runtime), as far as their records reach the server log.
//!
//! PORT: Blaze has no `java.util.logging`. The runtime owns the server log (`blaze_runtime`'s
//! `server_log`); crates below it log through this sink, which the runtime sets when it opens
//! the log. Without a sink (tests, tools) a record is dropped, as with no handler in Java.

use std::sync::OnceLock;

/// `(source class name, source method name, message)` of an `INFO` record.
pub type InfoSink = fn(&str, &str, &str);

static INFO_SINK: OnceLock<InfoSink> = OnceLock::new();

/// Sets the sink of `INFO` records (once per process).
pub fn set_info_sink(sink: InfoSink) {
    let _ = INFO_SINK.set(sink);
}

/// `logger.atInfo().log(message)` of class `source_class_name`, method `source_method_name`.
pub fn info(source_class_name: &str, source_method_name: &str, message: &str) {
    if let Some(sink) = INFO_SINK.get() {
        sink(source_class_name, source_method_name, message);
    }
}
