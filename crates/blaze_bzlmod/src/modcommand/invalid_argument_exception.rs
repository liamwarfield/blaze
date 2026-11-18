//! Port of `InvalidArgumentException` and the `FailureDetails.ModCommand.Code` values it carries.

use std::fmt;

/// `FailureDetails.ModCommand.Code` (src/main/protobuf/failure_details.proto, message
/// `ModCommand`), with the exit code from each value's `metadata` option.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCommandCode {
    MOD_COMMAND_UNKNOWN,
    MISSING_ARGUMENTS,
    TOO_MANY_ARGUMENTS,
    INVALID_ARGUMENTS,
    BUILDOZER_FAILED,
    ERROR_DURING_GRAPH_INSPECTION,
}

impl ModCommandCode {
    /// The proto enum value name.
    pub fn name(self) -> &'static str {
        match self {
            ModCommandCode::MOD_COMMAND_UNKNOWN => "MOD_COMMAND_UNKNOWN",
            ModCommandCode::MISSING_ARGUMENTS => "MISSING_ARGUMENTS",
            ModCommandCode::TOO_MANY_ARGUMENTS => "TOO_MANY_ARGUMENTS",
            ModCommandCode::INVALID_ARGUMENTS => "INVALID_ARGUMENTS",
            ModCommandCode::BUILDOZER_FAILED => "BUILDOZER_FAILED",
            ModCommandCode::ERROR_DURING_GRAPH_INSPECTION => "ERROR_DURING_GRAPH_INSPECTION",
        }
    }

    /// The proto enum number.
    pub fn number(self) -> i32 {
        match self {
            ModCommandCode::MOD_COMMAND_UNKNOWN => 0,
            ModCommandCode::MISSING_ARGUMENTS => 1,
            ModCommandCode::TOO_MANY_ARGUMENTS => 2,
            ModCommandCode::INVALID_ARGUMENTS => 3,
            ModCommandCode::BUILDOZER_FAILED => 4,
            ModCommandCode::ERROR_DURING_GRAPH_INSPECTION => 5,
        }
    }

    /// The `(metadata) = { exit_code: N }` of the value (failure_details.proto:1410-1415).
    pub fn exit_code(self) -> i32 {
        match self {
            ModCommandCode::MOD_COMMAND_UNKNOWN => 37,
            ModCommandCode::MISSING_ARGUMENTS
            | ModCommandCode::TOO_MANY_ARGUMENTS
            | ModCommandCode::INVALID_ARGUMENTS
            | ModCommandCode::BUILDOZER_FAILED
            | ModCommandCode::ERROR_DURING_GRAPH_INSPECTION => 2,
        }
    }
}

impl fmt::Display for ModCommandCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Exception thrown when a user-input argument is invalid (wrong number of arguments or the
/// specified modules do not exist).
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/InvalidArgumentException.java:24 InvalidArgumentException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidArgumentException {
    message: String,
    code: ModCommandCode,
    /// The message of the wrapped cause, if any (Java keeps the cause object).
    cause_message: Option<String>,
}

impl InvalidArgumentException {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/InvalidArgumentException.java:27 InvalidArgumentException.<init>
    pub fn with_cause(
        message: impl Into<String>,
        code: ModCommandCode,
        cause_message: impl Into<String>,
    ) -> Self {
        InvalidArgumentException {
            message: message.into(),
            code,
            cause_message: Some(cause_message.into()),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/InvalidArgumentException.java:32 InvalidArgumentException.<init>
    pub fn with_code(message: impl Into<String>, code: ModCommandCode) -> Self {
        InvalidArgumentException {
            message: message.into(),
            code,
            cause_message: None,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/InvalidArgumentException.java:37 InvalidArgumentException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_code(message, ModCommandCode::INVALID_ARGUMENTS)
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/InvalidArgumentException.java:41 InvalidArgumentException.getCode
    pub fn get_code(&self) -> ModCommandCode {
        self.code
    }

    pub fn get_cause_message(&self) -> Option<&str> {
        self.cause_message.as_deref()
    }
}

impl fmt::Display for InvalidArgumentException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for InvalidArgumentException {}
