//! Exceptions of the C++ core.

use std::fmt;

/// Stand-in for Starlark's `net.starlark.java.eval.EvalException`, carrying the same message.
// PORT: the Starlark wrapper crate converts this into its own error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct EvalException {
    pub message: String,
}

impl EvalException {
    pub fn new(message: impl Into<String>) -> EvalException {
        EvalException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Thrown when a flag value cannot be expanded under a set of build variables.
///
/// This happens for example when a flag references a variable that is not provided by the
/// action, or when a flag group implicitly references multiple variables of sequence type.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:88 CcToolchainFeatures.ExpansionException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExpansionException {
    pub message: String,
}

impl ExpansionException {
    pub fn new(message: impl Into<String>) -> ExpansionException {
        ExpansionException {
            message: message.into(),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// Java: `ExpansionException extends EvalException`.
impl From<ExpansionException> for EvalException {
    fn from(e: ExpansionException) -> EvalException {
        EvalException { message: e.message }
    }
}

/// Thrown when multiple features provide the same string symbol.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainFeatures.java:99 CcToolchainFeatures.CollidingProvidesException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CollidingProvidesException {
    pub message: String,
}

/// Java: `new EvalException(Throwable cause)` takes the cause's message.
impl From<CollidingProvidesException> for EvalException {
    fn from(e: CollidingProvidesException) -> EvalException {
        EvalException { message: e.message }
    }
}

/// Java's `String.format` for `%s` of a `Collection<String>`: `[a, b]`.
pub(crate) struct JavaList<'a, T: fmt::Display>(pub &'a [T]);

impl<T: fmt::Display> fmt::Display for JavaList<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{e}")?;
        }
        f.write_str("]")
    }
}
