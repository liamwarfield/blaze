//! Exceptions of the options framework.

use std::fmt;

// Java: com/google/devtools/common/options/OptionsParsingException.java:21 OptionsParsingException
/// Thrown when the options parser fails. `Display` prints `getMessage()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionsParsingException {
    message: String,
    invalid_argument: Option<String>,
    /// `getCause().getMessage()`, when the Java exception was constructed with a cause.
    cause_message: Option<String>,
}

impl OptionsParsingException {
    // Java: com/google/devtools/common/options/OptionsParsingException.java:24 OptionsParsingException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        OptionsParsingException {
            message: message.into(),
            invalid_argument: None,
            cause_message: None,
        }
    }

    // Java: com/google/devtools/common/options/OptionsParsingException.java:28 OptionsParsingException.<init>
    pub fn with_argument(message: impl Into<String>, argument: impl Into<String>) -> Self {
        OptionsParsingException {
            message: message.into(),
            invalid_argument: Some(argument.into()),
            cause_message: None,
        }
    }

    // Java: com/google/devtools/common/options/OptionsParsingException.java:33 OptionsParsingException.<init>
    pub fn with_cause(message: impl Into<String>, cause_message: impl Into<String>) -> Self {
        OptionsParsingException {
            message: message.into(),
            invalid_argument: None,
            cause_message: Some(cause_message.into()),
        }
    }

    // Java: com/google/devtools/common/options/OptionsParsingException.java:38 OptionsParsingException.<init>
    /// `new OptionsParsingException(message, argument, cause)`; the cause is kept as its
    /// `getMessage()`.
    pub fn with_argument_and_cause(
        message: impl Into<String>,
        argument: impl Into<String>,
        cause_message: impl Into<String>,
    ) -> Self {
        OptionsParsingException {
            message: message.into(),
            invalid_argument: Some(argument.into()),
            cause_message: Some(cause_message.into()),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/common/options/OptionsParsingException.java:46 OptionsParsingException.getInvalidArgument
    pub fn get_invalid_argument(&self) -> Option<&str> {
        self.invalid_argument.as_deref()
    }

    pub fn get_cause_message(&self) -> Option<&str> {
        self.cause_message.as_deref()
    }
}

impl fmt::Display for OptionsParsingException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OptionsParsingException {}

// Java: com/google/devtools/common/options/OptionsParser.java:87 OptionsParser.ConstructionException
/// An error constructing a parser (an invalid options class). In Java this is unchecked; here it
/// is returned from [`crate::OptionsParser`]'s builder, and raised as a panic (with this message)
/// where Java throws it from deep inside parsing (bad default values).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructionException {
    message: String,
    /// `getCause().getMessage()`.
    cause_message: Option<String>,
}

impl ConstructionException {
    pub fn new(message: impl Into<String>) -> Self {
        ConstructionException {
            message: message.into(),
            cause_message: None,
        }
    }

    /// `new ConstructionException(Throwable cause)`: the message is `cause.toString()`.
    pub fn from_cause(cause_class: &str, cause_message: impl Into<String>) -> Self {
        let cause_message = cause_message.into();
        ConstructionException {
            message: format!("{}: {}", cause_class, cause_message),
            cause_message: Some(cause_message),
        }
    }

    pub fn with_cause(message: impl Into<String>, cause_message: impl Into<String>) -> Self {
        ConstructionException {
            message: message.into(),
            cause_message: Some(cause_message.into()),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_cause_message(&self) -> Option<&str> {
        self.cause_message.as_deref()
    }

    /// Raises this exception as a panic (Java: unchecked throw).
    pub fn throw(self) -> ! {
        panic!("ConstructionException: {}", self.message)
    }
}

impl fmt::Display for ConstructionException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConstructionException {}

/// Class name used for `DuplicateOptionDeclarationException` causes.
pub const DUPLICATE_OPTION_DECLARATION_EXCEPTION: &str =
    "com.google.devtools.common.options.DuplicateOptionDeclarationException";
