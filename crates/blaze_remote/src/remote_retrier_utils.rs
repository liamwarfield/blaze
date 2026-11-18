//! Port of `lib/remote/RemoteRetrierUtils.java`: methods useful when using the `RemoteRetrier`.

use std::io;

use crate::common::errors;
use crate::execution_status_exception::{self, ExecutionStatusException};

/// Whether `e` is, or is caused by, a gRPC `StatusRuntimeException` / `StatusException` whose
/// status code is `expected`.
///
/// PORT: Java walks `getCause()`; here the status exception is the payload of the `io::Error`.
/// `ExecutionStatusException` is a `StatusRuntimeException` in Java, hence
/// `execution_status_exception::status_from_throwable`.
// Java: com/google/devtools/build/lib/remote/RemoteRetrierUtils.java:24 RemoteRetrierUtils.causedByStatus
pub fn caused_by_status(e: &io::Error, expected: tonic::Code) -> bool {
    match execution_status_exception::status_from_throwable(e) {
        Some(status) => status.code() == expected,
        None => false,
    }
}

/// Whether `e` is, or is caused by, an `ExecutionStatusException` that is an execution timeout.
// Java: com/google/devtools/build/lib/remote/RemoteRetrierUtils.java:35 RemoteRetrierUtils.causedByExecTimeout
pub fn caused_by_exec_timeout(e: &io::Error) -> bool {
    match errors::downcast_ref::<ExecutionStatusException>(e) {
        Some(execution_status_exception) => execution_status_exception.is_execution_timeout(),
        None => false,
    }
}
