//! Port of `packages/Type.java` `Type.ConversionException`.

use std::fmt;

use starlark::values::Value;

use crate::types::type_::Type;

/// Thrown when a type conversion fails; carries Java's explanatory message (Java
/// `Type.ConversionException`, an `EvalException`).
// Java: com/google/devtools/build/lib/packages/Type.java:296 Type.ConversionException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ConversionException {
    message: String,
}

impl ConversionException {
    // Java: com/google/devtools/build/lib/packages/Type.java:297 Type.ConversionException.message
    fn message(expected: &str, value: Value<'_>, what: Option<&dyn fmt::Display>) -> String {
        let mut s = String::new();
        s.push_str("expected ");
        s.push_str(expected);
        if let Some(what) = what {
            s.push_str(" for ");
            s.push_str(&what.to_string());
        }
        s.push_str(", but got ");
        s.push_str(&value.to_repr());
        s.push_str(" (");
        // PORT: Starlark.type(value) is starlark-rust's get_type() (same names for Bazel's
        // value types; see docs/decisions.md on Starlark-level text).
        s.push_str(value.get_type());
        s.push(')');
        s
    }

    /// Java `ConversionException(String expected, Object value, Object what)`.
    // Java: com/google/devtools/build/lib/packages/Type.java:310 Type.ConversionException.<init>
    pub fn expected(expected: &str, value: Value<'_>, what: Option<&dyn fmt::Display>) -> Self {
        ConversionException {
            message: Self::message(expected, value, what),
        }
    }

    /// Java `ConversionException(Type<?> type, Object value, Object what)`.
    // Java: com/google/devtools/build/lib/packages/Type.java:315 Type.ConversionException.<init>
    pub fn of_type(type_: Type, value: Value<'_>, what: Option<&dyn fmt::Display>) -> Self {
        ConversionException {
            message: Self::message(&format!("value of type '{type_}'"), value, what),
        }
    }

    /// Java `ConversionException(String message)`.
    // Java: com/google/devtools/build/lib/packages/Type.java:323 Type.ConversionException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        ConversionException {
            message: message.into(),
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}
