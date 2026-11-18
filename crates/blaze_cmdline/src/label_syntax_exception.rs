//! Port of `LabelSyntaxException`.

/// Thrown by the parsing methods to indicate a bad label (Java `LabelSyntaxException`).
// Java: com/google/devtools/build/lib/cmdline/LabelSyntaxException.java:20 LabelSyntaxException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LabelSyntaxException {
    message: String,
}

impl LabelSyntaxException {
    // Java: com/google/devtools/build/lib/cmdline/LabelSyntaxException.java:21 LabelSyntaxException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        LabelSyntaxException {
            message: message.into(),
        }
    }

    /// The exception message (Java `getMessage()`).
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Minimal stand-in for Starlark's `EvalException` thrown by the few `lib/cmdline` methods that
/// are Starlark-facing. Carries the same message.
// PORT: net.starlark.java.eval.EvalException lives in the Starlark layer (starlark-rust); the
// wrapper crate converts this into its own error type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct EvalException {
    message: String,
}

impl EvalException {
    pub fn new(message: impl Into<String>) -> Self {
        EvalException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}
