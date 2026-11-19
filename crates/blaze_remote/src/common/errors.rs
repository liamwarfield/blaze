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

/// `new IOException(new StatusRuntimeException(status))`; for the status of a call that never
/// started ([`call_not_started`]), the `IOException` that stopped it, unchanged.
pub fn status_to_io_error(status: tonic::Status) -> io::Error {
    if let Some(not_started) =
        Error::source(&status).and_then(|s| s.downcast_ref::<CallNotStarted>())
    {
        return shared(&not_started.0);
    }
    StatusRuntimeException::new(status).into()
}

/// The `IOException` that kept a call from starting (the channel's connection could not be
/// obtained: `ChannelFactory.newChannel` failed, or the server capabilities check did), carried
/// as the source of the call's `tonic::Status` so that [`status_to_io_error`] gives it back.
///
/// PORT: in Java the future of `withChannelFuture` fails with that exception itself; the
/// channel methods of the port report failures as a `tonic::Status`.
#[derive(Debug, Clone)]
pub struct CallNotStarted(pub Arc<io::Error>);

impl fmt::Display for CallNotStarted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Error for CallNotStarted {}

/// Whether `status` is the status of a call that never started ([`call_not_started`]).
///
/// In Java the future of `withChannelFuture` fails with the `IOException` itself (for a failed
/// capabilities check, the `RemoteExecutionCapabilitiesException`): no `StreamObserver` callback
/// of the call runs, and a `catching(.., StatusRuntimeException.class, ..)` does not catch it.
/// A site that branches on the code of a call's `tonic::Status` where Java branches inside such
/// a callback or catch tests this first.
pub fn is_call_not_started(status: &tonic::Status) -> bool {
    Error::source(status).is_some_and(|s| s.downcast_ref::<CallNotStarted>().is_some())
}

/// The status of a call that fails with `e` before it starts (see [`CallNotStarted`]). Its code
/// is `Status.fromThrowable(e).getCode()`, which the callers that look at the code of a failed
/// call (`Status.fromThrowable(e)` in Java) see.
pub fn call_not_started(e: io::Error) -> tonic::Status {
    let code = status_from_throwable(&e)
        .map(|s| s.code())
        .unwrap_or(tonic::Code::Unknown);
    let mut status = tonic::Status::new(code, e.to_string());
    status.set_source(Arc::new(CallNotStarted(Arc::new(e))));
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
/// `getCause()` with `instanceof T` finds. The chain runs through [`IoExceptionWithCause`] and
/// `RemoteExecutionCapabilitiesException` (`new IOException(cause)`).
pub fn find_cause<T: Error + Send + Sync + 'static>(e: &io::Error) -> Option<&T> {
    let mut current = e;
    loop {
        if let Some(t) = downcast_ref::<T>(current) {
            return Some(t);
        }
        current = match downcast_ref::<IoExceptionWithCause>(current) {
            Some(e) => e.get_cause(),
            None => downcast_ref::<
                crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException,
            >(current)?
            .get_cause(),
        };
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

/// An exception of a class the port has no type for (a transport's exception such as Netty's
/// `AnnotatedConnectException`), carried with its Java class name so that `Throwable.toString()`
/// ([`java_to_string`]) can be reproduced; it displays its message.
#[derive(Debug, Clone)]
pub struct JavaException {
    /// `getClass().getName()`.
    pub class_name: &'static str,
    /// `getMessage()`.
    pub message: String,
}

impl fmt::Display for JavaException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for JavaException {}

/// `Throwable.toString()` of the exception `e` stands for: `<class name>: <message>`. Known
/// classes: `RemoteExecutionCapabilitiesException` (whose message is its cause's `toString()`),
/// [`JavaException`], and the `StatusRuntimeException` a capabilities failure has as its cause;
/// every other error is an `IOException`.
// Java: java/lang/Throwable.java Throwable.toString
pub fn java_to_string(e: &io::Error) -> String {
    use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
    if let Some(rece) = downcast_ref::<RemoteExecutionCapabilitiesException>(e) {
        // new IOException(cause): getMessage() is cause.toString().
        return format!(
            "com.google.devtools.build.lib.remote.common.RemoteExecutionCapabilitiesException: {}",
            java_cause_to_string(rece.get_cause())
        );
    }
    if let Some(java) = downcast_ref::<JavaException>(e) {
        return format!("{}: {}", java.class_name, java.message);
    }
    format!("java.io.IOException: {e}")
}

/// [`java_to_string`] of a cause: the `StatusRuntimeException` of a failed `GetCapabilities`
/// is carried as an [`IoExceptionWithCause`] holding it (see
/// `remote_module::capabilities_failure_cause`).
fn java_cause_to_string(cause: &io::Error) -> String {
    if let Some(with_cause) = downcast_ref::<IoExceptionWithCause>(cause)
        && downcast_ref::<StatusRuntimeException>(with_cause.get_cause()).is_some()
    {
        return format!("io.grpc.StatusRuntimeException: {with_cause}");
    }
    java_to_string(cause)
}

/// Not from a Java test: `Throwable.toString()` texts of review 2026-11-19 N7; the expected
/// line is the oracle's ("Unable to write all BEP events to file due to '...'", run
/// `tc-bep-o`: an unreachable `--remote_cache` with `--build_event_json_file`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;

    #[test]
    fn capabilities_exception_of_a_connect_exception() {
        let cause = io::Error::new(
            io::ErrorKind::ConnectionRefused,
            JavaException {
                class_name: "io.netty.channel.AbstractChannel$AnnotatedConnectException",
                message: "finishConnect(..) failed: Connection refused: localhost/127.0.0.1:9499"
                    .to_string(),
            },
        );
        let e: io::Error = RemoteExecutionCapabilitiesException::new(cause).into();
        assert_eq!(
            java_to_string(&e),
            "com.google.devtools.build.lib.remote.common.RemoteExecutionCapabilitiesException: io.netty.channel.AbstractChannel$AnnotatedConnectException: finishConnect(..) failed: Connection refused: localhost/127.0.0.1:9499"
        );
    }

    #[test]
    fn capabilities_exception_of_a_status_runtime_exception() {
        let sre: io::Error =
            StatusRuntimeException::new(tonic::Status::not_found("instance not found")).into();
        let cause = io_exception_with_cause("NOT_FOUND: instance not found", sre);
        let e: io::Error = RemoteExecutionCapabilitiesException::new(cause).into();
        assert_eq!(
            java_to_string(&e),
            "com.google.devtools.build.lib.remote.common.RemoteExecutionCapabilitiesException: io.grpc.StatusRuntimeException: NOT_FOUND: instance not found"
        );
    }

    #[test]
    fn other_errors_are_io_exceptions() {
        let sre: io::Error =
            StatusRuntimeException::new(tonic::Status::unavailable("io exception")).into();
        assert_eq!(
            java_to_string(&sre),
            "java.io.IOException: io.grpc.StatusRuntimeException: UNAVAILABLE: io exception"
        );
        assert_eq!(java_to_string(&io_exception("x")), "java.io.IOException: x");
    }
}
