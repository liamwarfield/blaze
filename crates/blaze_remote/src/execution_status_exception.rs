//! Port of `lib/remote/ExecutionStatusException.java`.

use std::fmt;
use std::io;

use blaze_protos::JavaMessage;

use crate::common::errors::{self, StatusRuntimeException};
use crate::reapi::ExecuteResponse;
use crate::reapi::rpc::Status;

/// Exception to signal that a remote execution has failed with a certain status received from
/// the server, and other details, such as the action result and the server logs. The exception
/// may be retriable or not, depending on the status/details.
///
/// PORT: Java's class extends the unchecked `StatusRuntimeException` and travels as the cause of
/// an `IOException`. Here it is an `io::Error` payload of its own (`e.into()` is
/// `new IOException(e)`, `errors::downcast_ref::<ExecutionStatusException>(&e)` is
/// `e.getCause() instanceof ExecutionStatusException`). `errors::status_from_throwable` only
/// looks for a `StatusRuntimeException` payload, so `io.grpc.Status.fromThrowable(e)` for an
/// error that may carry this exception is [`status_from_throwable`] of this module.
// Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:28 ExecutionStatusException
#[derive(Debug, Clone)]
pub struct ExecutionStatusException {
    /// `super(e.getStatus(), e.getTrailers())`.
    status_runtime_exception: StatusRuntimeException,
    status: Status,
    response: Option<ExecuteResponse>,
}

impl ExecutionStatusException {
    /// `new ExecutionStatusException(e, original, response)` (package-private in Java).
    // Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:32 ExecutionStatusException.<init>
    pub(crate) fn from_status_runtime_exception(
        e: StatusRuntimeException,
        original: Status,
        response: Option<ExecuteResponse>,
    ) -> ExecutionStatusException {
        ExecutionStatusException {
            status_runtime_exception: e,
            status: original,
            response,
        }
    }

    /// `new ExecutionStatusException(status, response)`.
    // Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:39 ExecutionStatusException.<init>
    pub fn new(status: Status, response: Option<ExecuteResponse>) -> ExecutionStatusException {
        let e = to_status_runtime_exception(&convert_status(&status, response.as_ref()));
        ExecutionStatusException::from_status_runtime_exception(e, status, response)
    }

    /// `isExecutionTimeout()`.
    // Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:58 ExecutionStatusException.isExecutionTimeout
    pub fn is_execution_timeout(&self) -> bool {
        is_execution_timeout(&self.status, self.response.as_ref())
    }

    /// `getResponse()`.
    // Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:63 ExecutionStatusException.getResponse
    pub fn get_response(&self) -> Option<&ExecuteResponse> {
        self.response.as_ref()
    }

    /// `getOriginalStatus()`.
    // Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:67 ExecutionStatusException.getOriginalStatus
    pub fn get_original_status(&self) -> &Status {
        &self.status
    }

    /// `StatusRuntimeException.getStatus()` (the gRPC status, after the timeout conversion).
    // Java: io/grpc/StatusRuntimeException.java StatusRuntimeException.getStatus
    pub fn get_status(&self) -> &tonic::Status {
        &self.status_runtime_exception.status
    }

    /// The `StatusRuntimeException` this exception is (Java: the superclass part).
    pub fn as_status_runtime_exception(&self) -> &StatusRuntimeException {
        &self.status_runtime_exception
    }
}

// Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:43 ExecutionStatusException.convertStatus
fn convert_status(status: &Status, response: Option<&ExecuteResponse>) -> Status {
    let mut result = status.clone();
    if is_execution_timeout(status, response) {
        // Hack: convert to non-retriable exception on timeouts.
        result.code = tonic::Code::FailedPrecondition as i32;
    }
    result
}

// Java: com/google/devtools/build/lib/remote/ExecutionStatusException.java:52 ExecutionStatusException.isExecutionTimeout
fn is_execution_timeout(status: &Status, response: Option<&ExecuteResponse>) -> bool {
    match response {
        // `response.getStatus()` is the default instance when the field is not set.
        Some(response) => {
            response.status.clone().unwrap_or_default() == *status
                && status.code == tonic::Code::DeadlineExceeded as i32
        }
        None => false,
    }
}

/// `StatusProto.toStatusRuntimeException(statusProto)`: the gRPC status with the code and
/// message of the proto, and the proto itself in the `grpc-status-details-bin` trailer.
// Java: io/grpc/protobuf/StatusProto.java StatusProto.toStatusRuntimeException
fn to_status_runtime_exception(status_proto: &Status) -> StatusRuntimeException {
    // Java: io/grpc/protobuf/StatusProto.java StatusProto.toStatus
    let code = tonic::Code::from_i32(status_proto.code);
    blaze_util::check_argument!(code as i32 == status_proto.code, "invalid status code");
    // Java: io/grpc/protobuf/StatusProto.java StatusProto.toMetadata
    // (`metadata.put(STATUS_DETAILS_KEY, statusProto)`; tonic keeps that trailer as the details).
    StatusRuntimeException::new(tonic::Status::with_details(
        code,
        status_proto.message.clone(),
        bytes::Bytes::from(status_proto.to_byte_array()),
    ))
}

impl fmt::Display for ExecutionStatusException {
    /// `new IOException(executionStatusException).getMessage()`, which is the cause's
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "com.google.devtools.build.lib.remote.ExecutionStatusException: {}",
            StatusRuntimeException::format_throwable_message(&self.status_runtime_exception.status)
        )
    }
}

impl std::error::Error for ExecutionStatusException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.status_runtime_exception)
    }
}

impl From<ExecutionStatusException> for io::Error {
    /// `new IOException(executionStatusException)`.
    fn from(e: ExecutionStatusException) -> io::Error {
        io::Error::other(e)
    }
}

/// `io.grpc.Status.fromThrowable(e)` for an error that may carry an
/// [`ExecutionStatusException`] (a `StatusRuntimeException` subclass in Java) as well as a plain
/// `StatusRuntimeException`. `None` is Java's `Status.UNKNOWN.withCause(e)`.
// Java: io/grpc/Status.java Status.fromThrowable
pub fn status_from_throwable(e: &io::Error) -> Option<&tonic::Status> {
    if let Some(execution_status_exception) = errors::downcast_ref::<ExecutionStatusException>(e) {
        return Some(execution_status_exception.get_status());
    }
    errors::status_from_throwable(e)
}
