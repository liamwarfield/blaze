//! Error types standing in for the Java exceptions thrown by the fetch machinery.
//!
//! PORT: Java distinguishes `IOException` subclasses by type (callers catch
//! `FileNotFoundException`, `SocketTimeoutException`, ... and messages print `e.getClass()`).
//! Rust keeps one [`IoException`] struct carrying an [`IoExceptionKind`] that names the Java
//! class, so both the type tests and the printed class names survive.

use std::fmt;
use std::io;

/// The Java class of an [`IoException`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IoExceptionKind {
    /// `java.io.IOException`
    IOException,
    /// `java.io.FileNotFoundException`
    FileNotFoundException,
    /// `java.io.EOFException`
    EOFException,
    /// `java.io.InterruptedIOException`
    InterruptedIOException,
    /// `java.net.SocketTimeoutException` (a subclass of `InterruptedIOException`)
    SocketTimeoutException,
    /// `java.net.SocketException`
    SocketException,
    /// `java.net.ConnectException` (a subclass of `SocketException`)
    ConnectException,
    /// `java.net.UnknownHostException`
    UnknownHostException,
    /// `javax.net.ssl.SSLException`
    SSLException,
    /// `java.util.zip.ZipException`
    ZipException,
    /// `UnrecoverableHttpException`
    UnrecoverableHttpException,
    /// `ContentLengthMismatchException`
    ContentLengthMismatchException,
    /// `Checksum.MissingChecksumException`
    MissingChecksumException,
    /// `DecompressorValue.Decompressor.CouldNotFindPrefixException`
    CouldNotFindPrefixException,
}

impl IoExceptionKind {
    /// The fully qualified Java class name, as `Class.getName()` returns it.
    pub fn java_class_name(self) -> &'static str {
        match self {
            IoExceptionKind::IOException => "java.io.IOException",
            IoExceptionKind::FileNotFoundException => "java.io.FileNotFoundException",
            IoExceptionKind::EOFException => "java.io.EOFException",
            IoExceptionKind::InterruptedIOException => "java.io.InterruptedIOException",
            IoExceptionKind::SocketTimeoutException => "java.net.SocketTimeoutException",
            IoExceptionKind::SocketException => "java.net.SocketException",
            IoExceptionKind::ConnectException => "java.net.ConnectException",
            IoExceptionKind::UnknownHostException => "java.net.UnknownHostException",
            IoExceptionKind::SSLException => "javax.net.ssl.SSLException",
            IoExceptionKind::ZipException => "java.util.zip.ZipException",
            IoExceptionKind::UnrecoverableHttpException => {
                "com.google.devtools.build.lib.bazel.repository.downloader.UnrecoverableHttpException"
            }
            IoExceptionKind::ContentLengthMismatchException => {
                "com.google.devtools.build.lib.bazel.repository.downloader.ContentLengthMismatchException"
            }
            IoExceptionKind::MissingChecksumException => {
                "com.google.devtools.build.lib.bazel.repository.downloader.Checksum$MissingChecksumException"
            }
            IoExceptionKind::CouldNotFindPrefixException => {
                "com.google.devtools.build.lib.bazel.repository.decompressor.DecompressorValue$Decompressor$CouldNotFindPrefixException"
            }
        }
    }

    /// Java `e instanceof InterruptedIOException`.
    pub fn is_interrupted_io(self) -> bool {
        matches!(
            self,
            IoExceptionKind::InterruptedIOException | IoExceptionKind::SocketTimeoutException
        )
    }

    /// Java `e instanceof SocketException`.
    pub fn is_socket_exception(self) -> bool {
        matches!(
            self,
            IoExceptionKind::SocketException | IoExceptionKind::ConnectException
        )
    }
}

/// A Java `IOException` (or subclass): a message, the class, and suppressed exceptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoException {
    pub kind: IoExceptionKind,
    /// `Throwable.getMessage()`; `None` is Java `null`.
    pub message: Option<String>,
    /// `Throwable.getSuppressed()`.
    pub suppressed: Vec<IoException>,
}

impl IoException {
    /// `new IOException(message)`.
    pub fn new(message: impl Into<String>) -> IoException {
        IoException::of(IoExceptionKind::IOException, message)
    }

    /// An exception of the given class with a message.
    pub fn of(kind: IoExceptionKind, message: impl Into<String>) -> IoException {
        IoException {
            kind,
            message: Some(message.into()),
            suppressed: Vec::new(),
        }
    }

    /// An exception of the given class with a `null` message.
    pub fn without_message(kind: IoExceptionKind) -> IoException {
        IoException {
            kind,
            message: None,
            suppressed: Vec::new(),
        }
    }

    /// `Throwable.getMessage()`, with Java's `"null"` string conversion for a missing message.
    pub fn get_message(&self) -> String {
        self.message.clone().unwrap_or_else(|| "null".to_string())
    }

    /// `Throwable.addSuppressed`.
    pub fn add_suppressed(&mut self, e: IoException) {
        self.suppressed.push(e);
    }

    /// Java `e.getClass()` printed via string concatenation: `class java.io.IOException`.
    pub fn class_string(&self) -> String {
        format!("class {}", self.kind.java_class_name())
    }

    /// Java `new IOException(cause)`: the message is `cause.toString()`.
    // PORT: the cause chain itself is not kept; only its text, which is what Java prints.
    pub fn wrapping(cause: &IoException) -> IoException {
        IoException::new(cause.to_java_string())
    }

    /// Java `Throwable.toString()`: `<class name>: <message>` or just the class name.
    pub fn to_java_string(&self) -> String {
        match &self.message {
            Some(m) => format!("{}: {}", self.kind.java_class_name(), m),
            None => self.kind.java_class_name().to_string(),
        }
    }
}

impl fmt::Display for IoException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message.as_deref().unwrap_or("null"))
    }
}

impl std::error::Error for IoException {}

impl From<io::Error> for IoException {
    /// Converts an IO error from `blaze_vfs` / `std`. The VFS already formats messages the way
    /// Java's `UnixFileSystem` does (`<path> (<strerror>)`).
    fn from(e: io::Error) -> IoException {
        let kind = match e.kind() {
            io::ErrorKind::NotFound => IoExceptionKind::FileNotFoundException,
            io::ErrorKind::Interrupted => IoExceptionKind::InterruptedIOException,
            io::ErrorKind::TimedOut => IoExceptionKind::SocketTimeoutException,
            io::ErrorKind::UnexpectedEof => IoExceptionKind::EOFException,
            io::ErrorKind::ConnectionRefused => IoExceptionKind::ConnectException,
            io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe => IoExceptionKind::SocketException,
            _ => IoExceptionKind::IOException,
        };
        // An IoException tunnelled through std::io (see `into_io_error`) comes back unchanged.
        if let Some(inner) = e.get_ref().and_then(|i| i.downcast_ref::<IoException>()) {
            return inner.clone();
        }
        IoException::of(kind, e.to_string())
    }
}

impl IoException {
    /// Tunnels this exception through a `std::io::Error` (for `Read` implementations).
    pub fn into_io_error(self) -> io::Error {
        let kind = match self.kind {
            IoExceptionKind::FileNotFoundException => io::ErrorKind::NotFound,
            IoExceptionKind::SocketTimeoutException => io::ErrorKind::TimedOut,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, self)
    }
}

/// Port of `SkyFunctionException.Transience`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transience {
    TRANSIENT,
    PERSISTENT,
}

/// The wrapped cause of a [`RepositoryFunctionException`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoryFunctionCause {
    Io(IoException),
    /// A Starlark `EvalException` (from `Starlark.errorf`), by message.
    Eval(String),
}

/// Port of `RepositoryFunctionException`: a `SkyFunctionException` wrapping an `IOException` or
/// `EvalException`.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryFunctionException.java:27 RepositoryFunctionException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryFunctionException {
    pub cause: RepositoryFunctionCause,
    pub transience: Transience,
}

impl RepositoryFunctionException {
    pub fn io(e: IoException, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionCause::Io(e),
            transience,
        }
    }

    pub fn eval(message: impl Into<String>, transience: Transience) -> Self {
        RepositoryFunctionException {
            cause: RepositoryFunctionCause::Eval(message.into()),
            transience,
        }
    }

    /// The message of the wrapped exception.
    pub fn get_message(&self) -> String {
        match &self.cause {
            RepositoryFunctionCause::Io(e) => e.get_message(),
            RepositoryFunctionCause::Eval(m) => m.clone(),
        }
    }
}

impl fmt::Display for RepositoryFunctionException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for RepositoryFunctionException {}

/// Errors from a decompressor: Java's `IOException | RepositoryFunctionException`.
///
/// PORT: `InterruptedException` has no counterpart; Blaze does not interrupt fetch threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecompressorError {
    Io(IoException),
    Repository(RepositoryFunctionException),
}

impl From<IoException> for DecompressorError {
    fn from(e: IoException) -> Self {
        DecompressorError::Io(e)
    }
}

impl From<io::Error> for DecompressorError {
    fn from(e: io::Error) -> Self {
        DecompressorError::Io(e.into())
    }
}

impl From<RepositoryFunctionException> for DecompressorError {
    fn from(e: RepositoryFunctionException) -> Self {
        DecompressorError::Repository(e)
    }
}

impl fmt::Display for DecompressorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecompressorError::Io(e) => e.fmt(f),
            DecompressorError::Repository(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DecompressorError {}
