//! Port of `src/main/cpp/util/logging.{h,cc}` and `src/main/cpp/util/bazel_log_handler.{h,cc}`.
//!
//! `BAZEL_LOG(LEVEL) << ...` is [`bazel_log`] and `BAZEL_DIE(code) << ...` is [`bazel_die`]; both
//! take the C++ `__FILE__` (e.g. `src/main/cpp/blaze.cc`) and `__LINE__` of the pinned source,
//! which the debug log format prints.
//!
//! C++ runs the log handler's destructor from static destruction at `exit()`; that destructor
//! dumps the buffered debug log when the logging detail was never set. Here [`exit_process`]
//! (used by fatal messages) and [`close_at_exit`] (called when `Main` returns) drop the handler,
//! which does the same.

use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

use super::exit_code;

// Java: src/main/cpp/util/logging.h:27 LogLevel
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Info = 0,
    User = 1,
    Warning = 2,
    Error = 3,
    Fatal = 4,
}

// Java: src/main/cpp/util/logging.cc:24 LogLevelName
pub fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Info => "INFO",
        LogLevel::User => "USER",
        LogLevel::Warning => "WARNING",
        LogLevel::Error => "ERROR",
        LogLevel::Fatal => "FATAL",
    }
}

// Java: src/main/cpp/util/logging.h:144 LoggingDetail
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoggingDetail {
    /// Only errors.
    Quiet,
    /// Messages intended for the user (level USER and above).
    User,
    /// Debug logging.
    Debug,
}

/// An output stream (`std::ostream*`).
pub type Stream = Box<dyn Write + Send>;

// Java: src/main/cpp/util/logging.h:151 LogHandler
pub trait LogHandler: Send {
    /// Handles one message. Returns `Some(exit_code)` when the process must exit (FATAL); the
    /// caller then destroys the handler and exits, like C++'s `std::exit` inside the handler.
    fn handle_message(
        &mut self,
        level: LogLevel,
        filename: &str,
        line: u32,
        message: &str,
        exit_code: i32,
    ) -> Option<i32>;
    fn set_logging_detail(&mut self, detail: LoggingDetail, debug_stream: Option<Stream>);
    fn close(&mut self);
}

static LOG_HANDLER: Mutex<Option<Box<dyn LogHandler>>> = Mutex::new(None);

fn handler_lock() -> MutexGuard<'static, Option<Box<dyn LogHandler>>> {
    LOG_HANDLER.lock().unwrap_or_else(|e| e.into_inner())
}

// Java: src/main/cpp/util/logging.cc:92 SetLogHandler
/// Replaces the global handler; the old one is dropped (C++ destroys it in the unique_ptr
/// assignment, which may dump its buffer to stderr).
pub fn set_log_handler(new_handler: Option<Box<dyn LogHandler>>) {
    let old = std::mem::replace(&mut *handler_lock(), new_handler);
    drop(old);
}

// Java: src/main/cpp/util/logging.cc:96 SetLoggingDetail
pub fn set_logging_detail(detail: LoggingDetail, debug_stream: Option<Stream>) {
    if let Some(h) = handler_lock().as_mut() {
        h.set_logging_detail(detail, debug_stream);
    }
}

// Java: src/main/cpp/util/logging.cc:102 CloseLogging
pub fn close_logging() {
    if let Some(h) = handler_lock().as_mut() {
        h.close();
    }
}

/// Static destruction at process exit: destroys the global handler.
pub fn close_at_exit() {
    set_log_handler(None);
}

/// `std::exit(code)`: runs the log handler's destructor, flushes stdout, exits.
pub fn exit_process(code: i32) -> ! {
    close_at_exit();
    let _ = std::io::stdout().flush();
    std::process::exit(code)
}

// Java: src/main/cpp/util/logging.cc:68 LogMessage::Finish
/// Writes the message for the no-handler case; returns whether the process must exit.
pub fn print_without_handler(stream: &mut dyn Write, level: LogLevel, message: &str) {
    // If no custom handler was provided, never print INFO messages, but USER should always be
    // printed, as should warnings and errors.
    if level == LogLevel::User {
        let _ = writeln!(stream, "{}", message);
    } else if level > LogLevel::User {
        let _ = writeln!(stream, "{}: {}", log_level_name(level), message);
    }
}

// Java: src/main/cpp/util/logging.cc:68 LogMessage::Finish
pub fn log_message(level: LogLevel, filename: &str, line: u32, message: &str, exit_code: i32) {
    let exit = {
        let mut guard = handler_lock();
        match guard.as_mut() {
            Some(h) => h.handle_message(level, filename, line, message, exit_code),
            None => {
                print_without_handler(&mut std::io::stderr(), level, message);
                if level == LogLevel::Fatal {
                    // Exit for fatal calls after handling the message.
                    Some(exit_code)
                } else {
                    None
                }
            }
        }
    };
    if let Some(code) = exit {
        exit_process(code);
    }
}

/// `BAZEL_LOG(level) << message` at `filename:line`. `BAZEL_LOG(FATAL)` exits with
/// INTERNAL_ERROR.
pub fn bazel_log(level: LogLevel, filename: &str, line: u32, message: impl AsRef<str>) {
    log_message(
        level,
        filename,
        line,
        message.as_ref(),
        exit_code::INTERNAL_ERROR,
    );
}

/// `BAZEL_DIE(exit_code) << message` at `filename:line`.
pub fn bazel_die(exit_code: i32, filename: &str, line: u32, message: impl AsRef<str>) -> ! {
    log_message(LogLevel::Fatal, filename, line, message.as_ref(), exit_code);
    // log_message exits for FATAL messages.
    unreachable!("BAZEL_DIE returned")
}

// Java: src/main/cpp/util/bazel_log_handler.cc:51 PrintUserLevelMessageToStream
/// Messages intended for the user (level USER, along with WARNINGs an ERRORs) should be printed
/// even if debug level logging was not requested.
pub fn print_user_level_message_to_stream(stream: &mut dyn Write, level: LogLevel, message: &str) {
    if level == LogLevel::User {
        let _ = writeln!(stream, "{}", message);
    } else if level > LogLevel::User {
        let _ = writeln!(stream, "{}: {}", log_level_name(level), message);
    }
    // If level < USER, this is an INFO message. It's useful for debugging but should not be
    // printed to the user unless the user has asked for debugging output. We ignore it here.
}

// Java: src/main/cpp/util/bazel_log_handler.cc:63 Timestamp
fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let s = now.as_secs() as libc::time_t;
    let ms = now.as_millis() % 1000;
    // SAFETY: zeroed tm is a valid out-parameter for localtime_r.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&s, &mut tm);
        tm
    };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        tm.tm_hour, tm.tm_min, tm.tm_sec, ms
    )
}

// Java: src/main/cpp/util/bazel_log_handler.cc:81 PrintDebugLevelMessageToStream
/// For debug logs, print all logs, both debug logging and USER logs and above, along with
/// information about where the log message came from.
pub fn print_debug_level_message_to_stream(
    stream: &mut dyn Write,
    filename: &str,
    line: u32,
    level: LogLevel,
    message: &str,
) {
    let _ = writeln!(
        stream,
        "[{} {} {}:{}] {}",
        log_level_name(level),
        timestamp(),
        filename,
        line,
        message
    );
}

// Java: src/main/cpp/util/bazel_log_handler.h:29 BazelLogHandler
/// Handles logging for the Bazel client. In order to control where the log output goes based on
/// the flags received at startup, logs are buffered until SetOutputStream is called. At that
/// point, all past log statements are dumped in the appropriate stream, and all following
/// statements are logged directly.
pub struct BazelLogHandler {
    /// The stream to which debug logs are sent (if logging detail is not LOGGINGDETAIL_DEBUG,
    /// everything goes to stderr).
    debug_stream: Option<Stream>,
    debug_stream_set: bool,
    detail: LoggingDetail,
    /// Buffers for messages received before the logging detail was determined.
    user_messages: Vec<(LogLevel, String)>,
    debug_buffer_stream: Option<Vec<u8>>,
    /// `std::cerr`.
    stderr: Stream,
}

impl BazelLogHandler {
    // Java: src/main/cpp/util/bazel_log_handler.cc:28 BazelLogHandler::BazelLogHandler
    pub fn new() -> Self {
        Self::with_stderr(Box::new(std::io::stderr()))
    }

    /// A handler whose `std::cerr` is `stderr` (for tests).
    pub fn with_stderr(stderr: Stream) -> Self {
        BazelLogHandler {
            debug_stream: None,
            debug_stream_set: false,
            detail: LoggingDetail::Debug,
            user_messages: Vec::new(),
            debug_buffer_stream: Some(Vec::new()),
            stderr,
        }
    }

    // Java: src/main/cpp/util/bazel_log_handler.cc:173 FlushBufferToNewStreamAndSet
    fn flush_buffer_to_new_stream_and_set(
        &mut self,
        buffer: Vec<u8>,
        mut new_debug_stream: Stream,
    ) {
        // Flush the buffer to the new stream, and print new log lines to it.
        let _ = new_debug_stream.write_all(&buffer);
        let _ = new_debug_stream.flush();
        self.debug_stream = Some(new_debug_stream);
    }
}

impl LogHandler for BazelLogHandler {
    // Java: src/main/cpp/util/bazel_log_handler.cc:90 BazelLogHandler::HandleMessage
    fn handle_message(
        &mut self,
        level: LogLevel,
        filename: &str,
        line: u32,
        message: &str,
        exit_code: i32,
    ) -> Option<i32> {
        if self.detail != LoggingDetail::Debug {
            // If debugging was explicitly disabled, never print INFO messages, but messages of
            // level USER and above should always be printed, as should warnings and errors.
            // Omit the debug-level file and line number information, though.
            let min_log_level = if self.detail == LoggingDetail::Quiet {
                LogLevel::Error
            } else {
                LogLevel::User
            };
            if level >= min_log_level {
                print_user_level_message_to_stream(&mut self.stderr, level, message);
            }
            if level == LogLevel::Fatal {
                return Some(exit_code);
            }
            return None;
        }

        match self.debug_stream.as_mut() {
            None => {
                // If we haven't decided whether messages should be logged to debug levels or
                // not, buffer each version. This is redundant for USER levels and above, but is
                // to make sure we can provide the right output to the user once we know that
                // they do or do not want debug level information.
                self.user_messages.push((level, message.to_string()));
                if let Some(buf) = self.debug_buffer_stream.as_mut() {
                    print_debug_level_message_to_stream(buf, filename, line, level, message);
                }
            }
            Some(stream) => {
                // If an output stream has been specifically set, it is for the full suite of log
                // messages. We don't print the user messages separately here as they are
                // included.
                print_debug_level_message_to_stream(stream, filename, line, level, message);
            }
        }

        // If we have a fatal message, exit with the provided error code.
        if level == LogLevel::Fatal {
            if self.debug_stream.is_some() {
                print_user_level_message_to_stream(&mut self.stderr, level, message);
            }
            return Some(exit_code);
        }
        None
    }

    // Java: src/main/cpp/util/bazel_log_handler.cc:134 BazelLogHandler::SetLoggingDetail
    fn set_logging_detail(&mut self, detail: LoggingDetail, debug_stream: Option<Stream>) {
        // Disallow second calls to this, we only intend to support setting the output once,
        // otherwise the buffering will not work as intended and the log will be fragmented.
        if self.debug_stream_set {
            panic!("CHECK failed: !debug_stream_set_: Tried to set log output a second time");
        }
        self.debug_stream_set = true;
        self.debug_stream = None;
        self.detail = detail;

        if detail == LoggingDetail::Debug {
            // The user asked for debug level information, which includes the user messages. We
            // can discard the separate buffer at this point.
            self.user_messages.clear();
            let buffer = self.debug_buffer_stream.take().unwrap_or_default();
            match debug_stream {
                Some(stream) => self.flush_buffer_to_new_stream_and_set(buffer, stream),
                None => {
                    // PORT: C++ would dereference a null stream here (undefined behaviour);
                    // no caller passes null with LOGGINGDETAIL_DEBUG.
                    ::blaze_util::unported!(
                        "BazelLogHandler::SetLoggingDetail",
                        "LOGGINGDETAIL_DEBUG with a null stream"
                    )
                }
            }
            return;
        }

        self.debug_stream = debug_stream;
        self.debug_buffer_stream = None;
        let min_log_level = if detail == LoggingDetail::Quiet {
            LogLevel::Error
        } else {
            LogLevel::User
        };
        let messages = std::mem::take(&mut self.user_messages);
        for (level, message) in &messages {
            if *level >= min_log_level {
                print_user_level_message_to_stream(&mut self.stderr, *level, message);
            }
        }
        self.user_messages = messages;
    }

    // Java: src/main/cpp/util/bazel_log_handler.cc:165 BazelLogHandler::Close
    fn close(&mut self) {
        if let Some(s) = self.debug_stream.as_mut() {
            let _ = s.flush();
        }
        self.debug_stream = None;
    }
}

impl Drop for BazelLogHandler {
    // Java: src/main/cpp/util/bazel_log_handler.cc:34 BazelLogHandler::~BazelLogHandler
    fn drop(&mut self) {
        if self.detail == LoggingDetail::Debug {
            // If SetLoggingOutputStream was never called, dump the buffer to stderr, otherwise,
            // flush the stream.
            if let Some(s) = self.debug_stream.as_mut() {
                let _ = s.flush();
            } else if let Some(buf) = self.debug_buffer_stream.as_ref() {
                let _ = self.stderr.write_all(buf);
            } else {
                let _ = writeln!(
                    self.stderr,
                    "Illegal state - neither a logfile nor a logbuffer existed at program end."
                );
            }
        }
        let _ = self.stderr.flush();
    }
}

/// A cloneable in-memory stream (a `std::stringstream` the test keeps a pointer to).
#[derive(Clone, Default)]
pub struct SharedBuffer(pub Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
