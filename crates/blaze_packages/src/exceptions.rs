//! Ports `packages/NoSuchThingException.java`, `NoSuchPackageException.java`,
//! `NoSuchTargetException.java`, `BuildFileNotFoundException.java`,
//! `BuildFileContainsErrorsException.java`, `InvalidPackageNameException.java`,
//! `ErrorDeterminingRepositoryException.java` and `RepositoryFetchException.java`.
//!
//! Java's subclasses of `NoSuchPackageException` are a [`NoSuchPackageKind`] on one struct (the
//! kind decides `getMessage()`); `catch (BuildFileNotFoundException e)` becomes a kind check.

use std::fmt;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;

/// Java `FailureDetails.PackageLoading.Code` (src/main/protobuf/failure_details.proto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackageLoadingCode {
    PackageLoadingUnknown = 0,
    WorkspaceFileError = 1,
    MaxComputationStepsExceeded = 2,
    BuildFileMissing = 3,
    RepositoryMissing = 4,
    PersistentInconsistentFilesystemError = 5,
    TransientInconsistentFilesystemError = 6,
    InvalidName = 7,
    EvalGlobsSymlinkError = 9,
    ImportStarlarkFileError = 10,
    PackageMissing = 11,
    TargetMissing = 12,
    NoSuchThing = 13,
    GlobIoException = 14,
    DuplicateLabel = 15,
    InvalidPackageSpecification = 16,
    SyntaxError = 17,
    EnvironmentInDifferentPackage = 18,
    DefaultEnvironmentUndeclared = 19,
    EnvironmentInMultipleGroups = 20,
    EnvironmentDoesNotExist = 21,
    EnvironmentInvalid = 22,
    EnvironmentNotInGroup = 23,
    PackageNameInvalid = 24,
    StarlarkEvalError = 25,
    LicenseParseFailure = 26,
    DistributionsParseFailure = 27,
    LabelCrossesPackageBoundary = 28,
    BuiltinsInjectionFailure = 29,
    SymlinkCycleOrInfiniteExpansion = 30,
    OtherIoException = 31,
    BadRepoFile = 32,
    BadIgnoredDirectories = 33,
}

/// The category-specific code of a failure detail.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FailureCode {
    PackageLoading(PackageLoadingCode),
    /// Another `FailureDetail` category, as `<Category>.<CODE>` (ported with its crate).
    Other(String),
}

/// Java `util/DetailedExitCode` reduced to its `FailureDetail` message and code (the exit
/// code follows from the code; ported with blaze_runtime).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DetailedExitCode {
    pub message: String,
    pub code: FailureCode,
}

impl DetailedExitCode {
    pub fn package_loading(message: impl Into<String>, code: PackageLoadingCode) -> Self {
        DetailedExitCode {
            message: message.into(),
            code: FailureCode::PackageLoading(code),
        }
    }
}

/// The Java subclass of a [`NoSuchPackageException`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoSuchPackageKind {
    /// Plain `NoSuchPackageException`.
    NoSuchPackage,
    // Java: com/google/devtools/build/lib/packages/BuildFileNotFoundException.java BuildFileNotFoundException
    BuildFileNotFound,
    // Java: com/google/devtools/build/lib/packages/BuildFileContainsErrorsException.java BuildFileContainsErrorsException
    BuildFileContainsErrors,
    // Java: com/google/devtools/build/lib/packages/InvalidPackageNameException.java InvalidPackageNameException
    InvalidPackageName,
    // Java: com/google/devtools/build/lib/packages/RepositoryFetchException.java RepositoryFetchException
    RepositoryFetch,
}

/// Java `NoSuchPackageException` (and subclasses, see [`NoSuchPackageKind`]).
// Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:27 NoSuchPackageException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSuchPackageException {
    kind: NoSuchPackageKind,
    package_id: PackageIdentifier,
    /// Java `super.getMessage()` (the raw message).
    message: String,
    /// The message of the Java `cause`, if any.
    cause: Option<String>,
    detailed_exit_code: Option<DetailedExitCode>,
}

impl NoSuchPackageException {
    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:31 NoSuchPackageException.<init>
    pub fn new(package_id: PackageIdentifier, message: impl Into<String>) -> Self {
        NoSuchPackageException {
            kind: NoSuchPackageKind::NoSuchPackage,
            package_id,
            message: message.into(),
            cause: None,
            detailed_exit_code: None,
        }
    }

    /// The general constructor: any kind, optional cause message and detailed exit code.
    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:45 NoSuchPackageException.<init>
    pub fn of_kind(
        kind: NoSuchPackageKind,
        package_id: PackageIdentifier,
        message: impl Into<String>,
        cause: Option<String>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        NoSuchPackageException {
            kind,
            package_id,
            message: message.into(),
            cause,
            detailed_exit_code,
        }
    }

    /// Java `new NoSuchPackageException(packageId, message, detailedExitCode)`.
    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:39 NoSuchPackageException.<init>
    pub fn with_detailed_exit_code(
        package_id: PackageIdentifier,
        message: impl Into<String>,
        detailed_exit_code: DetailedExitCode,
    ) -> Self {
        Self::of_kind(
            NoSuchPackageKind::NoSuchPackage,
            package_id,
            message,
            None,
            Some(detailed_exit_code),
        )
    }

    /// Java `new NoSuchPackageException(packageId, message, cause)`.
    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:34 NoSuchPackageException.<init>
    pub fn with_cause(
        package_id: PackageIdentifier,
        message: impl Into<String>,
        cause: impl Into<String>,
    ) -> Self {
        Self::of_kind(
            NoSuchPackageKind::NoSuchPackage,
            package_id,
            message,
            Some(cause.into()),
            None,
        )
    }

    /// Java `new BuildFileNotFoundException(packageIdentifier, message, detailedExitCode)`.
    // Java: com/google/devtools/build/lib/packages/BuildFileNotFoundException.java:37 BuildFileNotFoundException.<init>
    pub fn build_file_not_found_with_detailed_exit_code(
        package_id: PackageIdentifier,
        message: impl Into<String>,
        detailed_exit_code: DetailedExitCode,
    ) -> Self {
        Self::of_kind(
            NoSuchPackageKind::BuildFileNotFound,
            package_id,
            message,
            None,
            Some(detailed_exit_code),
        )
    }

    /// Java `new BuildFileNotFoundException(packageIdentifier, message)`.
    // Java: com/google/devtools/build/lib/packages/BuildFileNotFoundException.java:27 BuildFileNotFoundException.<init>
    pub fn build_file_not_found(package_id: PackageIdentifier, message: impl Into<String>) -> Self {
        Self::of_kind(
            NoSuchPackageKind::BuildFileNotFound,
            package_id,
            message,
            None,
            None,
        )
    }

    /// Java `new BuildFileContainsErrorsException(packageIdentifier)`: "Package '<name>' contains
    /// errors".
    // Java: com/google/devtools/build/lib/packages/BuildFileContainsErrorsException.java:28 BuildFileContainsErrorsException.<init>
    pub fn build_file_contains_errors(package_id: PackageIdentifier) -> Self {
        let message = format!(
            "Package '{}' contains errors",
            package_id.get_package_fragment().get_path_string()
        );
        Self::of_kind(
            NoSuchPackageKind::BuildFileContainsErrors,
            package_id,
            message,
            None,
            None,
        )
    }

    /// Java `new BuildFileContainsErrorsException(packageIdentifier, message)`.
    // Java: com/google/devtools/build/lib/packages/BuildFileContainsErrorsException.java:36 BuildFileContainsErrorsException.<init>
    pub fn build_file_contains_errors_with_message(
        package_id: PackageIdentifier,
        message: impl Into<String>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        Self::of_kind(
            NoSuchPackageKind::BuildFileContainsErrors,
            package_id,
            message,
            None,
            detailed_exit_code,
        )
    }

    /// Java `new InvalidPackageNameException(packageIdentifier, message[, detailedExitCode])`.
    // Java: com/google/devtools/build/lib/packages/InvalidPackageNameException.java:23 InvalidPackageNameException.<init>
    pub fn invalid_package_name(
        package_id: PackageIdentifier,
        message: impl Into<String>,
        detailed_exit_code: Option<DetailedExitCode>,
    ) -> Self {
        Self::of_kind(
            NoSuchPackageKind::InvalidPackageName,
            package_id,
            message,
            None,
            detailed_exit_code,
        )
    }

    /// Java `new RepositoryFetchException(packageIdentifier, message)`.
    // Java: com/google/devtools/build/lib/packages/RepositoryFetchException.java:23 RepositoryFetchException.<init>
    pub fn repository_fetch(package_id: PackageIdentifier, message: impl Into<String>) -> Self {
        Self::of_kind(
            NoSuchPackageKind::RepositoryFetch,
            package_id,
            message,
            None,
            None,
        )
    }

    pub fn kind(&self) -> NoSuchPackageKind {
        self.kind
    }

    pub fn get_cause(&self) -> Option<&str> {
        self.cause.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:58 NoSuchPackageException.getPackageId
    pub fn get_package_id(&self) -> &PackageIdentifier {
        &self.package_id
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:62 NoSuchPackageException.getRawMessage
    pub fn get_raw_message(&self) -> &str {
        &self.message
    }

    /// Java `getMessage()`; `BuildFileContainsErrorsException` overrides the prefix.
    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:67 NoSuchPackageException.getMessage
    // Java: com/google/devtools/build/lib/packages/BuildFileContainsErrorsException.java:57 BuildFileContainsErrorsException.getMessage
    pub fn get_message(&self) -> String {
        match self.kind {
            NoSuchPackageKind::BuildFileContainsErrors => format!(
                "error loading package '{}': {}",
                self.package_id,
                self.get_raw_message()
            ),
            _ => format!(
                "no such package '{}': {}",
                self.package_id,
                self.get_raw_message()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:72 NoSuchPackageException.hasExplicitDetailedExitCode
    pub fn has_explicit_detailed_exit_code(&self) -> bool {
        self.detailed_exit_code.is_some()
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:77 NoSuchPackageException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        match &self.detailed_exit_code {
            Some(d) => d.clone(),
            // Java: com/google/devtools/build/lib/packages/NoSuchPackageException.java:84 NoSuchPackageException.defaultDetailedExitCode
            None => DetailedExitCode::package_loading(
                self.get_message(),
                PackageLoadingCode::PackageMissing,
            ),
        }
    }
}

impl fmt::Display for NoSuchPackageException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_message())
    }
}

impl std::error::Error for NoSuchPackageException {}

/// Java `NoSuchTargetException`.
// Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:27 NoSuchTargetException
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSuchTargetException {
    message: String,
    label: Option<Label>,
    has_target: bool,
    detailed_exit_code: Option<DetailedExitCode>,
}

impl NoSuchTargetException {
    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:29 NoSuchTargetException.createForParentPropagation
    pub fn create_for_parent_propagation(label: Label) -> Self {
        Self::for_target_in_error(label)
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:36 NoSuchTargetException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        NoSuchTargetException {
            message: message.into(),
            label: None,
            has_target: false,
            detailed_exit_code: None,
        }
    }

    /// Java `NoSuchTargetException(Label, String)`: "no such target '<label>': <message>".
    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:43 NoSuchTargetException.<init>
    pub fn with_label(label: Label, message: &str) -> Self {
        NoSuchTargetException {
            message: format!("no such target '{label}': {message}"),
            label: Some(label),
            has_target: false,
            detailed_exit_code: None,
        }
    }

    /// Java `NoSuchTargetException(Target targetInError)` / `(Label)`: the target exists but
    /// its package is in error.
    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:54 NoSuchTargetException.<init>
    pub fn for_target_in_error(label: Label) -> Self {
        NoSuchTargetException {
            message: format!("Target '{label}' contains an error and its package is in error"),
            label: Some(label),
            has_target: true,
            detailed_exit_code: None,
        }
    }

    /// Like [`NoSuchTargetException::with_label`] with an explicit detailed exit code.
    // Java: com/google/devtools/build/lib/packages/NoSuchThingException.java:46 NoSuchThingException.<init>
    pub fn with_detailed_exit_code(mut self, detailed_exit_code: DetailedExitCode) -> Self {
        self.detailed_exit_code = Some(detailed_exit_code);
        self
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:73 NoSuchTargetException.getLabel
    pub fn get_label(&self) -> Option<&Label> {
        self.label.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:77 NoSuchTargetException.hasTarget
    pub fn has_target(&self) -> bool {
        self.has_target
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchTargetException.java:82 NoSuchTargetException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        match &self.detailed_exit_code {
            Some(d) => d.clone(),
            None => DetailedExitCode::package_loading(
                self.message.clone(),
                PackageLoadingCode::TargetMissing,
            ),
        }
    }
}

impl fmt::Display for NoSuchTargetException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NoSuchTargetException {}

/// Java `ErrorDeterminingRepositoryException`.
// Java: com/google/devtools/build/lib/packages/ErrorDeterminingRepositoryException.java:20 ErrorDeterminingRepositoryException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ErrorDeterminingRepositoryException {
    pub message: String,
}

impl ErrorDeterminingRepositoryException {
    // Java: com/google/devtools/build/lib/packages/ErrorDeterminingRepositoryException.java:21 ErrorDeterminingRepositoryException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        ErrorDeterminingRepositoryException {
            message: message.into(),
        }
    }
}

/// Java `NoSuchThingException` and its subclasses.
// Java: com/google/devtools/build/lib/packages/NoSuchThingException.java:27 NoSuchThingException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoSuchThingException {
    #[error("{0}")]
    Package(#[from] NoSuchPackageException),
    #[error("{0}")]
    Target(#[from] NoSuchTargetException),
    #[error("{0}")]
    ErrorDeterminingRepository(#[from] ErrorDeterminingRepositoryException),
    /// A plain `NoSuchThingException(message[, detailedExitCode])`.
    #[error("{message}")]
    Other {
        message: String,
        detailed_exit_code: Option<DetailedExitCode>,
    },
}

impl NoSuchThingException {
    /// Java `getMessage()`.
    pub fn get_message(&self) -> String {
        self.to_string()
    }

    // Java: com/google/devtools/build/lib/packages/NoSuchThingException.java:54 NoSuchThingException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> DetailedExitCode {
        match self {
            NoSuchThingException::Package(e) => e.get_detailed_exit_code(),
            NoSuchThingException::Target(e) => e.get_detailed_exit_code(),
            // Java: com/google/devtools/build/lib/packages/NoSuchThingException.java:63 NoSuchThingException.defaultDetailedExitCode
            NoSuchThingException::ErrorDeterminingRepository(e) => {
                DetailedExitCode::package_loading(
                    e.message.clone(),
                    PackageLoadingCode::NoSuchThing,
                )
            }
            NoSuchThingException::Other {
                message,
                detailed_exit_code,
            } => detailed_exit_code.clone().unwrap_or_else(|| {
                DetailedExitCode::package_loading(message.clone(), PackageLoadingCode::NoSuchThing)
            }),
        }
    }
}
