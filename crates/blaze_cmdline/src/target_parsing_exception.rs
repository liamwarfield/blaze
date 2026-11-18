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
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct TargetParsingException {
    message: String,
    code: TargetPatternsCode,
    /// The `DetailedExitCode` given to the constructor (the cause's), if any; otherwise it is
    /// derived from `message` and `code`.
    // W4-analysis: needed so that `cquery|build //nosuchpkg:x` exits with the cause's code.
    detailed_exit_code: Option<blaze_util::DetailedExitCode>,
}

// `DetailedExitCode` holds a `FailureDetail` proto (no floats), which only derives `PartialEq`.
impl Eq for TargetParsingException {}

impl TargetParsingException {
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:29 TargetParsingException.<init>
    pub fn new(message: impl Into<String>, code: TargetPatternsCode) -> Self {
        TargetParsingException {
            message: message.into(),
            code,
            detailed_exit_code: None,
        }
    }

    /// `new TargetParsingException(message, cause, detailedExitCode)` and
    /// `new TargetParsingException(message, detailedExitCode)`: the exception keeps the given
    /// `DetailedExitCode` (usually the cause's, e.g. a `NoSuchPackageException`'s).
    ///
    /// PORT: [`TargetParsingException::get_code`] has no `TargetPatterns.Code` to return for
    /// such an exception (its failure detail is the cause's category); it returns
    /// `TARGET_PATTERNS_UNKNOWN`. Use [`TargetParsingException::get_detailed_exit_code`].
    // W4-analysis: additive (see the field).
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:39 TargetParsingException.<init>
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:45 TargetParsingException.<init>
    pub fn with_detailed_exit_code(
        message: impl Into<String>,
        detailed_exit_code: blaze_util::DetailedExitCode,
    ) -> Self {
        TargetParsingException {
            message: message.into(),
            code: TargetPatternsCode::TargetPatternsUnknown,
            detailed_exit_code: Some(detailed_exit_code),
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

    /// Returns the detailed exit code that contains the failure detail associated with the
    /// error during parsing.
    // W4-analysis: additive (see the field).
    // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:76 TargetParsingException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> blaze_util::DetailedExitCode {
        match &self.detailed_exit_code {
            Some(d) => d.clone(),
            // Java: com/google/devtools/build/lib/cmdline/TargetParsingException.java:65 TargetParsingException.createFailureDetail
            None => {
                blaze_util::DetailedExitCode::of(blaze_protos::failure_details::FailureDetail {
                    message: self.message.clone(),
                    category: Some(
                        blaze_protos::failure_details::failure_detail::Category::TargetPatterns(
                            blaze_protos::failure_details::TargetPatterns {
                                code: self.code as i32,
                            },
                        ),
                    ),
                })
            }
        }
    }
}

#[cfg(test)]
mod detailed_exit_code_tests {
    use super::*;
    use blaze_protos::failure_details::failure_detail::Category;
    use blaze_protos::failure_details::{FailureDetail, PackageLoading, package_loading};

    // Java: TargetParsingException(String, TargetPatterns.Code): the detailed exit code is built
    // from the message and the code; PACKAGE_NOT_FOUND has exit code 1 in failure_details.proto.
    #[test]
    fn code_constructor_derives_the_detailed_exit_code() {
        let e = TargetParsingException::new("no such package", TargetPatternsCode::PackageNotFound);
        let d = e.get_detailed_exit_code();
        assert_eq!(d.get_exit_code().get_numeric_exit_code(), 1);
        let fd = d.get_failure_detail().unwrap();
        assert_eq!(fd.message, "no such package");
        assert!(matches!(&fd.category, Some(Category::TargetPatterns(t)) if t.code == 4));
    }

    // Java: TargetParsingException(String, Throwable, DetailedExitCode) keeps the given code
    // (TargetParsingException.java:39-43), e.g. PackageLoading.BUILD_FILE_MISSING (exit code 1)
    // of a NoSuchPackageException, where TARGET_PATTERNS_UNKNOWN would be exit code 37.
    #[test]
    fn detailed_exit_code_constructor_keeps_the_given_code() {
        let cause = blaze_util::DetailedExitCode::of(FailureDetail {
            message: "BUILD file not found".to_owned(),
            category: Some(Category::PackageLoading(PackageLoading {
                code: package_loading::Code::BuildFileMissing as i32,
            })),
        });
        let e =
            TargetParsingException::with_detailed_exit_code("no such package 'x'", cause.clone());
        assert_eq!(e.get_message(), "no such package 'x'");
        assert_eq!(e.get_detailed_exit_code(), cause);
        assert_eq!(
            e.get_detailed_exit_code()
                .get_exit_code()
                .get_numeric_exit_code(),
            1
        );
        assert_eq!(
            TargetParsingException::new("x", TargetPatternsCode::TargetPatternsUnknown)
                .get_detailed_exit_code()
                .get_exit_code()
                .get_numeric_exit_code(),
            37
        );
    }
}
