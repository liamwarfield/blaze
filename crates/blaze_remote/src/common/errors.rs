//! How the Java exception hierarchy of the remote package maps to `std::io::Error`.
//!
//! Java signals almost every remote failure with an `IOException` subclass
//! (`CacheNotFoundException`, `BulkTransferException`, `OutputDigestMismatchException`, ...) or
//! with an `IOException` whose cause is a gRPC `StatusRuntimeException`. The port keeps
//! `io::Error` as the one error type and carries the typed exception as its payload:
//! `X::into()` builds the error, [`downcast_ref`] is Java's `e instanceof X` (looking through
//! [`SharedIoError`]), and [`status_from_throwable`] is `io.grpc.Status.fromThrowable(e)`.
//! `InterruptedException` is an error of kind `Interrupted` ([`interrupted`]).

use std::error::Error;
use std::fmt;
use std::io;
use std::sync::Arc;

/// An `io::Error` shared between the waiters of one deduplicated task (Java hands every waiter
/// the same exception object).
#[derive(Debug, Clone)]
pub struct SharedIoError(pub Arc<io::Error>);

impl fmt::Display for SharedIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Error for SharedIoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

/// Wraps a shared error into an `io::Error` of the same kind.
pub fn shared(e: &Arc<io::Error>) -> io::Error {
    io::Error::new(e.kind(), SharedIoError(e.clone()))
}

/// `e instanceof T` for an `IOException` subclass carried as the payload of `e`.
pub fn downcast_ref<T: Error + Send + Sync + 'static>(e: &io::Error) -> Option<&T> {
    let mut inner: &(dyn Error + Send + Sync + 'static) = e.get_ref()?;
    loop {
        if let Some(t) = inner.downcast_ref::<T>() {
            return Some(t);
        }
        inner = inner.downcast_ref::<SharedIoError>()?.0.get_ref()?;
    }
}

/// Takes the payload out of `e` (`(T) e`), or gives `e` back. A shared error is cloned out.
pub fn downcast<T: Error + Send + Sync + Clone + 'static>(e: io::Error) -> Result<T, io::Error> {
    match downcast_ref::<T>(&e) {
        Some(t) => Ok(t.clone()),
        None => Err(e),
    }
}

/// A gRPC `StatusRuntimeException`.
// Java: io/grpc/StatusRuntimeException.java
#[derive(Debug, Clone)]
pub struct StatusRuntimeException {
    /// `getStatus()` (with the trailers).
    pub status: tonic::Status,
}

impl StatusRuntimeException {
    /// `new StatusRuntimeException(status)`.
    pub fn new(status: tonic::Status) -> StatusRuntimeException {
        StatusRuntimeException { status }
    }

    /// `Status.formatThrowableMessage(status)`: `CODE` or `CODE: description`.
    // Java: io/grpc/Status.java Status.formatThrowableMessage
    pub fn format_throwable_message(status: &tonic::Status) -> String {
        let code = code_name(status.code());
        if status.message().is_empty() {
            code.to_string()
        } else {
            format!("{code}: {}", status.message())
        }
    }
}

impl fmt::Display for StatusRuntimeException {
    /// `new IOException(statusRuntimeException).getMessage()`, which is the cause's `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "io.grpc.StatusRuntimeException: {}",
            StatusRuntimeException::format_throwable_message(&self.status)
        )
    }
}

impl Error for StatusRuntimeException {}

impl From<StatusRuntimeException> for io::Error {
    /// `new IOException(statusRuntimeException)`.
    fn from(e: StatusRuntimeException) -> io::Error {
        io::Error::other(e)
    }
}

/// `new IOException(new StatusRuntimeException(status))`.
pub fn status_to_io_error(status: tonic::Status) -> io::Error {
    // A call that failed before it started with an `IOException` (see [`io_error_status`]) fails
    // with that exception, as the Java future does.
    if let Some(source) = Error::source(&status).and_then(|s| s.downcast_ref::<CallIoError>()) {
        return shared(&source.0);
    }
    StatusRuntimeException::new(status).into()
}

/// The `IOException` a call fails with before it reaches the server (Java: the
/// `Single<ChannelConnection>` of `ReferenceCountedChannel.withChannelConnection` fails, and so
/// does the call's future), carried as the source of a `tonic::Status` through the call API;
/// [`status_to_io_error`] returns it.
#[derive(Debug)]
pub struct CallIoError(pub Arc<io::Error>);

impl fmt::Display for CallIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Error for CallIoError {}

/// A `tonic::Status` (code UNKNOWN) that [`status_to_io_error`] turns back into `e`.
pub fn io_error_status(e: io::Error) -> tonic::Status {
    let mut status = tonic::Status::unknown(e.to_string());
    status.set_source(Arc::new(CallIoError(Arc::new(e))));
    status
}

/// `new IOException(message, cause)`: displays `message`; the cause stays reachable for
/// [`find_cause`] / [`status_from_throwable`] (`Status.fromThrowable` walks the cause chain)
/// but not for [`downcast_ref`] (`instanceof` does not).
#[derive(Debug, Clone)]
pub struct IoExceptionWithCause {
    message: String,
    cause: Arc<io::Error>,
}

impl IoExceptionWithCause {
    /// `getCause()`.
    pub fn get_cause(&self) -> &io::Error {
        &self.cause
    }
}

impl fmt::Display for IoExceptionWithCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for IoExceptionWithCause {}

/// `new IOException(message, cause)`.
pub fn io_exception_with_cause(message: impl Into<String>, cause: io::Error) -> io::Error {
    io::Error::other(IoExceptionWithCause {
        message: message.into(),
        cause: Arc::new(cause),
    })
}

/// The first `T` in the cause chain of `e` (`e` itself included): what a Java loop over
/// `getCause()` with `instanceof T` finds.
pub fn find_cause<T: Error + Send + Sync + 'static>(e: &io::Error) -> Option<&T> {
    let mut current = e;
    loop {
        if let Some(t) = downcast_ref::<T>(current) {
            return Some(t);
        }
        current = downcast_ref::<IoExceptionWithCause>(current)?.get_cause();
    }
}

/// `io.grpc.Status.fromThrowable(e)`: the status of the `StatusRuntimeException` in the cause
/// chain, else `UNKNOWN` (`None` here; Java returns `Status.UNKNOWN.withCause(e)`).
pub fn status_from_throwable(e: &io::Error) -> Option<&tonic::Status> {
    find_cause::<StatusRuntimeException>(e).map(|s| &s.status)
}

/// `Status.Code.name()`.
// Java: io/grpc/Status.java Status.Code
pub fn code_name(code: tonic::Code) -> &'static str {
    match code {
        tonic::Code::Ok => "OK",
        tonic::Code::Cancelled => "CANCELLED",
        tonic::Code::Unknown => "UNKNOWN",
        tonic::Code::InvalidArgument => "INVALID_ARGUMENT",
        tonic::Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        tonic::Code::NotFound => "NOT_FOUND",
        tonic::Code::AlreadyExists => "ALREADY_EXISTS",
        tonic::Code::PermissionDenied => "PERMISSION_DENIED",
        tonic::Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        tonic::Code::FailedPrecondition => "FAILED_PRECONDITION",
        tonic::Code::Aborted => "ABORTED",
        tonic::Code::OutOfRange => "OUT_OF_RANGE",
        tonic::Code::Unimplemented => "UNIMPLEMENTED",
        tonic::Code::Internal => "INTERNAL",
        tonic::Code::Unavailable => "UNAVAILABLE",
        tonic::Code::DataLoss => "DATA_LOSS",
        tonic::Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// An `InterruptedException` travelling through an `io::Result`.
pub fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "interrupted")
}

/// Whether `e` stands for an `InterruptedException`.
pub fn is_interrupted(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Interrupted
}

/// A plain `new IOException(message)`.
pub fn io_exception(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// `e instanceof FileSymlinkLoopException`.
///
/// PORT: `blaze_vfs`' Unix file system reports `ELOOP` as an error of the kind std derives from
/// it with the message as payload, which `FileSymlinkLoopException::is` does not recognise; the
/// kind is compared too.
pub fn is_file_symlink_loop(e: &io::Error) -> bool {
    blaze_vfs::FileSymlinkLoopException::is(e)
        || e.kind() == io::Error::from_raw_os_error(libc::ELOOP).kind()
}
