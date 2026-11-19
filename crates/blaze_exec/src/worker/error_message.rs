//! Port of `lib/worker/ErrorMessage.java`, and [`JavaException`], the stand-in for the Java
//! exceptions whose stack traces `ErrorMessage` prints.

use std::fmt;
use std::io;

use blaze_vfs::Path;

/// A Java exception as `Throwables.getStackTraceAsString` prints it: `toString()` of the
/// exception (`<class name>: <message>`, or the class name alone without a message) and of each
/// cause, introduced by `Caused by: `.
///
/// PORT: Blaze has no JVM stack frames; the `\tat ...` lines between the exception and its
/// causes are not printed. Everything else of the trace is Java's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaException {
    /// The fully qualified class name (`java.io.IOException`).
    pub class_name: String,
    /// `getMessage()`, `None` where Java's is null.
    pub message: Option<String>,
    /// `getCause()`.
    pub cause: Option<Box<JavaException>>,
}

impl JavaException {
    /// `new <class_name>(message)`.
    pub fn new(class_name: &str, message: impl Into<String>) -> JavaException {
        JavaException {
            class_name: class_name.to_string(),
            message: Some(message.into()),
            cause: None,
        }
    }

    /// `new <class_name>(message, cause)`. Java's `Throwable(String, Throwable)` keeps the
    /// message as given.
    pub fn with_cause(class_name: &str, message: impl Into<String>, cause: JavaException) -> Self {
        JavaException {
            class_name: class_name.to_string(),
            message: Some(message.into()),
            cause: Some(Box::new(cause)),
        }
    }

    /// `new IOException(message)`.
    pub fn io(message: impl Into<String>) -> JavaException {
        JavaException::new("java.io.IOException", message)
    }

    /// The Java exception of an `io::Error` raised by Blaze's file system or process layer.
    ///
    /// PORT: Java's exception class depends on the call that failed (`FileNotFoundException`,
    /// `NoSuchFileException`, ...); Blaze knows only the `io::Error`, so the class is
    /// `java.io.IOException` unless the error wraps a [`JavaException`] (see
    /// [`JavaException::into_io_error`]).
    pub fn from_io_error(e: &io::Error) -> JavaException {
        if let Some(inner) = e.get_ref()
            && let Some(java) = inner.downcast_ref::<JavaIoError>()
        {
            return java.0.clone();
        }
        JavaException::io(e.to_string())
    }

    /// Wraps this exception in an `io::Error` whose `Display` is `getMessage()`, so that it can
    /// travel through `io::Result` and come back out with [`JavaException::from_io_error`].
    pub fn into_io_error(self) -> io::Error {
        io::Error::other(JavaIoError(self))
    }

    /// `getMessage()`, or `""` when null.
    pub fn get_message(&self) -> &str {
        self.message.as_deref().unwrap_or("")
    }

    /// `Throwables.getStackTraceAsString(this)` without the frames.
    pub fn stack_trace_as_string(&self) -> String {
        let mut sb = self.to_string();
        sb.push('\n');
        let mut cause = self.cause.as_deref();
        while let Some(c) = cause {
            sb.push_str("Caused by: ");
            sb.push_str(&c.to_string());
            sb.push('\n');
            cause = c.cause.as_deref();
        }
        sb
    }
}

impl fmt::Display for JavaException {
    // Java: java/lang/Throwable.java Throwable.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.message {
            Some(m) => write!(f, "{}: {}", self.class_name, m),
            None => f.write_str(&self.class_name),
        }
    }
}

/// The payload of an `io::Error` made by [`JavaException::into_io_error`].
#[derive(Debug)]
struct JavaIoError(JavaException);

impl fmt::Display for JavaIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.get_message())
    }
}

impl std::error::Error for JavaIoError {}

/// A well-formatted error message that is easy to read and easy to create.
// Java: com/google/devtools/build/lib/worker/ErrorMessage.java:27 ErrorMessage
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMessage {
    message: String,
}

impl ErrorMessage {
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:37 ErrorMessage.builder
    pub fn builder() -> ErrorMessageBuilder {
        ErrorMessageBuilder::new()
    }

    /// `message()`.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ErrorMessage {
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:33 ErrorMessage.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// `ErrorMessage.Builder`.
// Java: com/google/devtools/build/lib/worker/ErrorMessage.java:41 ErrorMessage.Builder
#[derive(Clone, Debug)]
pub struct ErrorMessageBuilder {
    message: String,
    log_file: Option<Path>,
    log_text: String,
    log_size_limit: i32,
    exception: Option<JavaException>,
}

/// Java `String.trim()`: removes leading and trailing chars `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

impl ErrorMessageBuilder {
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:48 ErrorMessage.Builder.<init>
    fn new() -> ErrorMessageBuilder {
        ErrorMessageBuilder {
            message: "Unknown error".to_string(),
            log_file: None,
            log_text: String::new(),
            log_size_limit: i32::MAX,
            exception: None,
        }
    }

    /// Sets the main text of this error message.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:52 ErrorMessage.Builder.message
    pub fn message(mut self, message: &str) -> Self {
        self.message = if message.is_empty() {
            "Unknown error".to_string()
        } else {
            java_trim(message).to_string()
        };
        self
    }

    /// Sets the log file that should be printed as part of the error message.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:60 ErrorMessage.Builder.logFile
    pub fn log_file(mut self, log_file: &Path) -> Self {
        self.log_file = Some(log_file.clone());
        match crate::analysis::actions::file_system_utils::read_content(log_file) {
            // FileSystemUtils.readContent(logFile, UTF_8).
            // PORT: Java decodes the log as UTF-8 into a Java String (malformed input becomes
            // U+FFFD); so does String::from_utf8_lossy. The result is a Unicode string, mixed into
            // the message as in Java.
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                self.log_text(&text)
            }
            Err(e) => {
                self = self.log_size_limit(i32::MAX);
                // FileSystemUtils.readContent opens a FileInputStream: a missing file is its
                // FileNotFoundException, "<path> (No such file or directory)".
                let exception = if e.kind() == std::io::ErrorKind::NotFound {
                    JavaException::new(
                        "java.io.FileNotFoundException",
                        format!("{} (No such file or directory)", log_file.get_path_string()),
                    )
                } else {
                    JavaException::from_io_error(&e)
                };
                let text = format!(
                    "ERROR: IOException while trying to read log file:\n{}",
                    exception.stack_trace_as_string()
                );
                self.log_text(&text)
            }
        }
    }

    /// Sets additional text, which is to be presented as a log file in the error message.
    ///
    /// If the log originally comes from a file, it is recommended to use [`Self::log_file`]
    /// instead, because then the path to the log file can be printed together with the message.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:80 ErrorMessage.Builder.logText
    pub fn log_text(mut self, log_text: &str) -> Self {
        // Set the log text to "(empty)" when the passed in string is empty, otherwise error
        // messages like "Something failed. Check below log for details:" would be pretty confusing
        // for users.
        self.log_text = if log_text.is_empty() {
            "(empty)".to_string()
        } else {
            java_trim(log_text).to_string()
        };
        self
    }

    /// If the log file or text of this error message is longer than the character limit set via
    /// this method, it will be truncated so that only the last X characters of the log are
    /// printed.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:93 ErrorMessage.Builder.logSizeLimit
    pub fn log_size_limit(mut self, log_size_limit: i32) -> Self {
        blaze_util::check_argument!(log_size_limit > 0, "logSizeLimit must be positive");
        self.log_size_limit = log_size_limit;
        self
    }

    /// Lets the error message contain the details of an exception.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:101 ErrorMessage.Builder.exception
    pub fn exception(mut self, e: JavaException) -> Self {
        self.exception = Some(e);
        self
    }

    /// Builds and returns the formatted error message.
    // Java: com/google/devtools/build/lib/worker/ErrorMessage.java:108 ErrorMessage.Builder.build
    pub fn build(self) -> ErrorMessage {
        let mut sb = self.message.clone();

        if let Some(exception) = &self.exception {
            sb.push_str("\n\n---8<---8<--- Exception details ---8<---8<---\n");
            sb.push_str(java_trim(&exception.stack_trace_as_string()));
            sb.push_str("\n---8<---8<--- End of exception details ---8<---8<---");
        }

        if !self.log_text.is_empty() {
            // Java's String.length() counts UTF-16 code units.
            let units: Vec<u16> = self.log_text.encode_utf16().collect();
            let length = units.len() as i64;
            let limit = self.log_size_limit as i64;
            sb.push_str("\n\n---8<---8<--- Start of log");
            if length > limit {
                sb.push_str(" snippet");
            }
            if let Some(log_file) = &self.log_file {
                sb.push_str(", file at ");
                sb.push_str(log_file.get_path_string());
            }
            sb.push_str(" ---8<---8<---\n");

            // If the length of the log is longer than the limit, print only the last part.
            if length > limit {
                sb.push_str("[... truncated ...]\n");
                sb.push_str(&String::from_utf16_lossy(
                    &units[(length - limit) as usize..length as usize],
                ));
                sb.push_str("\n---8<---8<--- End of log snippet, ");
                sb.push_str(&(length - limit).to_string());
                sb.push_str(" chars omitted ---8<---8<---");
            } else {
                sb.push_str(&self.log_text);
                sb.push_str("\n---8<---8<--- End of log ---8<---8<---");
            }
        }

        ErrorMessage { message: sb }
    }
}
