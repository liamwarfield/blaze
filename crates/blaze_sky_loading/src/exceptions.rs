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
