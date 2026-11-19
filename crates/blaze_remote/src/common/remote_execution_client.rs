//! Port of `lib/remote/common/RemoteExecutionClient.java` and `OperationObserver.java`.

use std::io;

use super::remote_action_execution_context::RemoteActionExecutionContext;
use crate::reapi::longrunning::Operation;
use crate::reapi::{ExecuteRequest, ExecuteResponse, ServerCapabilities};

/// Receives `Operation` updates from an stream of messages.
// Java: com/google/devtools/build/lib/remote/common/OperationObserver.java:20 OperationObserver
pub trait OperationObserver: Send + Sync {
    /// Receives a value from the stream.
    fn on_next(&self, o: &Operation) -> io::Result<()>;
}

/// A no-op implementation (`OperationObserver.NO_OP`).
// Java: com/google/devtools/build/lib/remote/common/OperationObserver.java:25 OperationObserver.NO_OP
pub struct NoOpOperationObserver;

impl OperationObserver for NoOpOperationObserver {
    fn on_next(&self, _o: &Operation) -> io::Result<()> {
        Ok(())
    }
}

impl<F: Fn(&Operation) -> io::Result<()> + Send + Sync> OperationObserver for F {
    fn on_next(&self, o: &Operation) -> io::Result<()> {
        self(o)
    }
}

/// An interface for the remote execution client.
///
/// Implementations must be thread-safe.
// Java: com/google/devtools/build/lib/remote/common/RemoteExecutionClient.java:27 RemoteExecutionClient
pub trait RemoteExecutionClient: Send + Sync {
    /// Returns the cache capabilities of the remote execution server.
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities>;

    /// The client as `Any`, for the tests' `(GrpcRemoteExecutor) client` casts (`None` for
    /// clients that do not support it).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Execute an action remotely using Remote Execution API (blocking).
    fn execute_remotely(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        request: ExecuteRequest,
        observer: &dyn OperationObserver,
    ) -> io::Result<ExecuteResponse>;

    /// Close resources associated with the remote execution client.
    fn close(&self);
}
