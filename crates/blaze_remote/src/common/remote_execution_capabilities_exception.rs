//! Port of `lib/remote/common/RemoteExecutionCapabilitiesException.java`.

use std::fmt;
use std::io;
use std::sync::Arc;

/// An exception to indicate that Bazel failed to query remote execution capabilities.
// Java: com/google/devtools/build/lib/remote/common/RemoteExecutionCapabilitiesException.java:19 RemoteExecutionCapabilitiesException
#[derive(Debug, Clone)]
pub struct RemoteExecutionCapabilitiesException {
    cause: Arc<io::Error>,
}

impl RemoteExecutionCapabilitiesException {
    /// `new RemoteExecutionCapabilitiesException(cause)`.
    // Java: com/google/devtools/build/lib/remote/common/RemoteExecutionCapabilitiesException.java:20 RemoteExecutionCapabilitiesException.<init>
    pub fn new(cause: io::Error) -> RemoteExecutionCapabilitiesException {
        RemoteExecutionCapabilitiesException {
            cause: Arc::new(cause),
        }
    }

    /// `getCause()`.
    pub fn get_cause(&self) -> &io::Error {
        &self.cause
    }
}

impl fmt::Display for RemoteExecutionCapabilitiesException {
    /// `new IOException(cause).getMessage()` is `cause.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.cause, f)
    }
}

impl std::error::Error for RemoteExecutionCapabilitiesException {}

impl From<RemoteExecutionCapabilitiesException> for io::Error {
    fn from(e: RemoteExecutionCapabilitiesException) -> io::Error {
        io::Error::other(e)
    }
}
