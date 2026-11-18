//! Port of `query2/engine/QuerySyntaxException.java` and `SkyframeRestartQueryException.java`.

/// Indicates an unrecoverable problem with query syntax.
// Java: com/google/devtools/build/lib/query2/engine/QuerySyntaxException.java:18 QuerySyntaxException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct QuerySyntaxException {
    message: String,
}

impl QuerySyntaxException {
    // Java: com/google/devtools/build/lib/query2/engine/QuerySyntaxException.java:19 QuerySyntaxException.<init>
    pub fn new(message: impl Into<String>) -> QuerySyntaxException {
        QuerySyntaxException {
            message: message.into(),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// This exception is thrown when a query operation was unable to complete because of a Skyframe
/// missing dependency.
///
/// PORT: a Java `RuntimeException`; Rust code that would throw it panics with
/// [`SkyframeRestartQueryException::MESSAGE`] or returns this value, as its caller requires.
// Java: com/google/devtools/build/lib/query2/engine/SkyframeRestartQueryException.java:20 SkyframeRestartQueryException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("need skyframe retry. missing dep")]
pub struct SkyframeRestartQueryException;

impl SkyframeRestartQueryException {
    pub const MESSAGE: &'static str = "need skyframe retry. missing dep";
}
