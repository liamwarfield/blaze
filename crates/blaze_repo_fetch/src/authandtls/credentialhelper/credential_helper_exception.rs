//! Port of `CredentialHelperException`.

use std::fmt;

use crate::errors::{IoException, IoExceptionKind};

/// An exception thrown while invoking a credential helper.
///
/// PORT: Java's class extends `IOException`; it converts into an [`IoException`] of kind
/// `IoExceptionKind::CredentialHelperException` with the same message, which is how callers
/// test `instanceof CredentialHelperException` (`Utils.grpcAwareErrorMessage`). The cause is
/// kept here only; Java prints it only in a `--verbose_failures` stack trace.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperException.java:20 CredentialHelperException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialHelperException {
    message: String,
    /// `getCause().toString()`, when there is a cause.
    cause: Option<String>,
}

impl CredentialHelperException {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperException.java:21 CredentialHelperException.<init>
    pub fn new(message: impl Into<String>) -> CredentialHelperException {
        CredentialHelperException {
            message: message.into(),
            cause: None,
        }
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperException.java:25 CredentialHelperException.<init>
    pub fn with_cause(
        message: impl Into<String>,
        cause: impl fmt::Display,
    ) -> CredentialHelperException {
        CredentialHelperException {
            message: message.into(),
            cause: Some(cause.to_string()),
        }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// `getCause()`, as text.
    pub fn get_cause(&self) -> Option<&str> {
        self.cause.as_deref()
    }
}

impl fmt::Display for CredentialHelperException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CredentialHelperException {}

impl From<CredentialHelperException> for IoException {
    fn from(e: CredentialHelperException) -> IoException {
        IoException::of(IoExceptionKind::CredentialHelperException, e.message)
    }
}
