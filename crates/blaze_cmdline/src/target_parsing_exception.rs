//! Port of `TargetParsingException` and the `FailureDetails.TargetPatterns.Code` enum it uses.

/// `failure_details.proto` `TargetPatterns.Code` (with the exit codes from its metadata).
// PORT: the FailureDetails protos are not ported yet; this enum carries the codes needed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetPatternsCode {
    TargetPatternsUnknown = 0,
    TargetPatternFileWithCommandLinePattern = 1,
    TargetPatternFileReadFailure = 2,
    TargetPatternParseFailure = 3,
    PackageNotFound = 4,
    TargetFormatInvalid = 5,
    AbsoluteTargetPatternInvalid = 6,
    CannotDetermineTargetFromFilename = 7,
    LabelSyntaxError = 8,
    TargetCannotBeEmptyString = 9,
    PackagePartCannotEndInSlash = 10,
    Cycle = 11,
    CannotPreloadTarget = 12,
    TargetsMissing = 13,
    RecursiveTargetPatternsNotAllowed = 14,
    UpLevelReferencesNotAllowed = 15,
    NegativeTargetPatternNotAllowed = 16,
    TargetMustBeAFile = 17,
    DependencyNotFound = 18,
    PackageNameInvalid = 19,
}

impl TargetPatternsCode {
    /// The process exit code from the proto's `(metadata) = { exit_code: N }`.
    pub fn exit_code(self) -> i32 {
        match self {
            TargetPatternsCode::TargetPatternsUnknown => 37,
            TargetPatternsCode::TargetPatternFileWithCommandLinePattern
            | TargetPatternsCode::TargetPatternFileReadFailure => 2,
            _ => 1,
        }
    }
}

/// A target label or pattern that cannot be parsed (Java `TargetParsingException`).
///
/// Java stores a `DetailedExitCode` built from the message and code; here the code is kept and
/// the message is the failure detail's message.
// Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:25 TargetParsingException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct TargetParsingException {
    message: String,
    code: TargetPatternsCode,
}

impl TargetParsingException {
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:29 TargetParsingException.<init>
    pub fn new(message: impl Into<String>, code: TargetPatternsCode) -> Self {
        TargetParsingException {
            message: message.into(),
            code,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// The `TargetPatterns.Code` of the failure detail.
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:76 TargetParsingException.getDetailedExitCode
    pub fn get_code(&self) -> TargetPatternsCode {
        self.code
    }
}
