//! The package exceptions this crate throws through Skyframe: re-exported from
//! `blaze_packages::exceptions` (owner A2), plus constructors for the Java overloads it lacks.
//!
//! Java: com/google/devtools/build/lib/packages/{NoSuchThingException,NoSuchPackageException,
//! BuildFileNotFoundException,RepositoryFetchException,ErrorDeterminingRepositoryException}.java.

use blaze_cmdline::PackageIdentifier;

pub use blaze_packages::exceptions::{
    DetailedExitCode, ErrorDeterminingRepositoryException, FailureCode, NoSuchPackageException,
    NoSuchPackageKind, NoSuchTargetException, NoSuchThingException, PackageLoadingCode,
};

/// Java `new BuildFileNotFoundException(packageIdentifier, message, detailedExitCode)`.
// Java: com/google/devtools/build/lib/packages/BuildFileNotFoundException.java:35 BuildFileNotFoundException.<init>
pub fn build_file_not_found_with_detailed_exit_code(
    package_id: PackageIdentifier,
    message: impl Into<String>,
    detailed_exit_code: DetailedExitCode,
) -> NoSuchPackageException {
    NoSuchPackageException::of_kind(
        NoSuchPackageKind::BuildFileNotFound,
        package_id,
        message,
        None,
        Some(detailed_exit_code),
    )
}

/// A marker class for exceptions that are already reported. Once caught, these exceptions
/// shouldn't be reported again.
///
/// Java subclasses (e.g. `RepositoryFunctionException.AlreadyReportedRepositoryAccessException`)
/// are thrown as this type with their class name kept for `toString`, so that catchers of
/// `AlreadyReportedException.class` (PackageLookupFunction) see them.
// Java: com/google/devtools/build/lib/skyframe/AlreadyReportedException.java:20 AlreadyReportedException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlreadyReportedException {
    message: String,
    java_class: &'static str,
}

impl AlreadyReportedException {
    pub const JAVA_CLASS: &'static str =
        "com.google.devtools.build.lib.skyframe.AlreadyReportedException";

    // Java: com/google/devtools/build/lib/skyframe/AlreadyReportedException.java:21 AlreadyReportedException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        AlreadyReportedException {
            message: message.into(),
            java_class: Self::JAVA_CLASS,
        }
    }

    /// An instance of the Java subclass `java_class` (fully qualified binary name).
    pub fn of_subclass(message: impl Into<String>, java_class: &'static str) -> Self {
        AlreadyReportedException {
            message: message.into(),
            java_class,
        }
    }

    /// Java `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// The Java class name of the thrown exception.
    pub fn java_class(&self) -> &'static str {
        self.java_class
    }

    /// Java `Throwable.toString()`.
    pub fn to_java_string(&self) -> String {
        format!("{}: {}", self.java_class, self.message)
    }
}

impl std::fmt::Display for AlreadyReportedException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AlreadyReportedException {}
