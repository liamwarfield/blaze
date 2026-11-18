//! Port of `ExternalDepsException` and the `FailureDetails.ExternalDeps.Code` enum.

use std::fmt;
use std::sync::Arc;

/// `FailureDetails.ExternalDeps.Code` (src/main/protobuf/failure_details.proto).
// PORT: there is no blaze_protos crate yet; this mirrors the proto enum and its exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalDepsCode {
    ExternalDepsUnknown,
    ModuleNotFound,
    BadModule,
    VersionResolutionError,
    InvalidRegistryUrl,
    ErrorAccessingRegistry,
    InvalidExtensionImport,
    BadLockfile,
    ExtensionEvalError,
}

impl ExternalDepsCode {
    /// The proto enum number.
    pub fn number(self) -> i32 {
        match self {
            ExternalDepsCode::ExternalDepsUnknown => 0,
            ExternalDepsCode::ModuleNotFound => 1,
            ExternalDepsCode::BadModule => 2,
            ExternalDepsCode::VersionResolutionError => 3,
            ExternalDepsCode::InvalidRegistryUrl => 4,
            ExternalDepsCode::ErrorAccessingRegistry => 5,
            ExternalDepsCode::InvalidExtensionImport => 6,
            ExternalDepsCode::BadLockfile => 7,
            ExternalDepsCode::ExtensionEvalError => 8,
        }
    }

    /// The exit code from the proto's `metadata` option.
    pub fn exit_code(self) -> i32 {
        match self {
            ExternalDepsCode::ExternalDepsUnknown => 37,
            ExternalDepsCode::ErrorAccessingRegistry => 32,
            _ => 48,
        }
    }
}

/// Exception indicating that something went wrong while processing external dependencies. Port of
/// Java `ExternalDepsException` (its `DetailedExitCode` is the message plus the code).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:30 ExternalDepsException
#[derive(Clone)]
pub struct ExternalDepsException {
    message: String,
    code: ExternalDepsCode,
    cause: Option<Arc<dyn std::error::Error + Send + Sync>>,
}

impl ExternalDepsException {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:47 ExternalDepsException.withMessage
    pub fn with_message(code: ExternalDepsCode, message: impl Into<String>) -> Self {
        ExternalDepsException {
            message: message.into(),
            code,
            cause: None,
        }
    }

    /// `withCauseAndMessage`: the message is `<message>: <cause message>`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:66 ExternalDepsException.withCauseAndMessage
    pub fn with_cause_and_message<E>(code: ExternalDepsCode, cause: E, message: &str) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        ExternalDepsException {
            message: format!("{}: {}", message, cause),
            code,
            cause: Some(Arc::new(cause)),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ExternalDepsException.java:72 ExternalDepsException.withCause
    pub fn with_cause<E>(code: ExternalDepsCode, cause: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        ExternalDepsException {
            message: cause.to_string(),
            code,
            cause: Some(Arc::new(cause)),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// The `ExternalDeps.Code` of the failure detail.
    pub fn get_code(&self) -> ExternalDepsCode {
        self.code
    }

    /// Java `getCause()`.
    pub fn get_cause(&self) -> Option<&(dyn std::error::Error + Send + Sync)> {
        self.cause.as_deref()
    }
}

impl fmt::Debug for ExternalDepsException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalDepsException")
            .field("message", &self.message)
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for ExternalDepsException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExternalDepsException {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_deref()
            .map(|c| c as &(dyn std::error::Error + 'static))
    }
}

/// A plain message error, used as the cause of [`ExternalDepsException`]s built from Java
/// exceptions that have no Rust type of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageException(pub String);

impl fmt::Display for MessageException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessageException {}
