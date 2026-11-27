//! PACKAGE_LOOKUP: `PackageLookupFunction` and `PackageLookupValue`.
//!
//! Java: com/google/devtools/build/lib/skyframe/{PackageLookupFunction,PackageLookupValue}.java.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::label_constants::EXTERNAL_PACKAGE_IDENTIFIER;
use blaze_cmdline::label_validator;
use blaze_cmdline::{Label, PackageIdentifier, RepositoryName};
use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Interrupted, Key, SkyFunctionException,
    SkyFunctionName, Transience,
};
use blaze_vfs::{PathFragment, Root, RootedPath};
use parking_lot::RwLock;

use crate::actions::FileValue;
use crate::build_file_name::BuildFileName;
use crate::exceptions::{
    DetailedExitCode, ErrorDeterminingRepositoryException, FailureCode, NoSuchPackageException,
    PackageLoadingCode, build_file_not_found_with_detailed_exit_code,
};
use crate::file::FileKey;
use crate::ignored_subdirectories::IgnoredSubdirectoriesKey;
use crate::io::{IOException, IOExceptionKind};
use crate::local_repository_lookup::LocalRepositoryLookupKey;
use crate::pkgcache::PathPackageLocator;
use crate::precomputed::PATH_PACKAGE_LOCATOR;
use crate::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use crate::sky_functions;

/// Why a package lookup failed.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:51 PackageLookupValue.ErrorReason
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorReason {
    /// There is no BUILD file.
    NoBuildFile,
    /// The package name is invalid.
    InvalidPackageName,
    /// The package is considered deleted because of --deleted_packages.
    DeletedPackage,
    /// The repository was not found.
    RepositoryNotFound,
}

/// A value that represents a package lookup result.
///
/// Package lookups will always produce a value. On success, `get_root` returns the package path
/// root under which the package resides and the package's BUILD file is guaranteed to exist. On
/// failure, `get_error_reason` and `get_error_msg` describe why the package doesn't exist.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:42 PackageLookupValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PackageLookupValue {
    /// Successful lookup value.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:161 PackageLookupValue.SuccessfulPackageLookupValue
    Success {
        root: Root,
        build_file_name: BuildFileName,
    },
    /// Marker value for no build file found.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:240 PackageLookupValue.NoBuildFilePackageLookupValue
    NoBuildFile,
    /// Value indicating the package name was in error.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:258 PackageLookupValue.InvalidNamePackageLookupValue
    InvalidName { error_msg: String },
    /// Value indicating the package name was in error (it crosses into another repository).
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:296 PackageLookupValue.IncorrectRepositoryReferencePackageLookupValue
    IncorrectRepositoryReference {
        invalid_package_identifier: PackageIdentifier,
        corrected_package_identifier: PackageIdentifier,
    },
    /// Marker value for a deleted package.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:360 PackageLookupValue.DeletedPackageLookupValue
    DeletedPackage,
    /// Value for repository we could not find. This can happen when looking for a label that
    /// specifies a non-existent repository.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:380 PackageLookupValue.NoRepositoryPackageLookupValue
    NoRepository {
        repository_name: RepositoryName,
        reason: String,
    },
}

impl PackageLookupValue {
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:43 PackageLookupValue.NO_BUILD_FILE_VALUE
    pub const NO_BUILD_FILE_VALUE: PackageLookupValue = PackageLookupValue::NoBuildFile;
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:47 PackageLookupValue.DELETED_PACKAGE_VALUE
    pub const DELETED_PACKAGE_VALUE: PackageLookupValue = PackageLookupValue::DeletedPackage;

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:64 PackageLookupValue.success
    pub fn success(root: Root, build_file_name: BuildFileName) -> PackageLookupValue {
        PackageLookupValue::Success {
            root,
            build_file_name,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:68 PackageLookupValue.invalidPackageName
    pub fn invalid_package_name(error_msg: impl Into<String>) -> PackageLookupValue {
        PackageLookupValue::InvalidName {
            error_msg: error_msg.into(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:72 PackageLookupValue.incorrectRepositoryReference
    pub fn incorrect_repository_reference(
        invalid_package: PackageIdentifier,
        correct_package: PackageIdentifier,
    ) -> PackageLookupValue {
        PackageLookupValue::IncorrectRepositoryReference {
            invalid_package_identifier: invalid_package,
            corrected_package_identifier: correct_package,
        }
    }

    /// For a successful package lookup, returns the root (package path entry) that the package
    /// resides in. Panics otherwise.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:81 PackageLookupValue.getRoot
    pub fn get_root(&self) -> &Root {
        match self {
            PackageLookupValue::Success { root, .. } => root,
            _ => panic!("IllegalStateException"),
        }
    }

    /// For a successful package lookup, returns the build file name that the package uses.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:84 PackageLookupValue.getBuildFileName
    pub fn get_build_file_name(&self) -> BuildFileName {
        match self {
            PackageLookupValue::Success {
                build_file_name, ..
            } => *build_file_name,
            _ => panic!("IllegalStateException"),
        }
    }

    /// Returns whether the package lookup was successful.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:87 PackageLookupValue.packageExists
    pub fn package_exists(&self) -> bool {
        matches!(self, PackageLookupValue::Success { .. })
    }

    /// For a successful package lookup, returns the [`RootedPath`] for the build file that
    /// defines the package.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:93 PackageLookupValue.getRootedPath
    pub fn get_rooted_path(&self, package_identifier: &PackageIdentifier) -> RootedPath {
        RootedPath::to_rooted_path(
            self.get_root().clone(),
            &self
                .get_build_file_name()
                .get_build_file_fragment(package_identifier),
        )
    }

    /// For an unsuccessful package lookup, gets the reason why `package_exists` returns false.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:102 PackageLookupValue.getErrorReason
    pub fn get_error_reason(&self) -> ErrorReason {
        match self {
            PackageLookupValue::Success { .. } => panic!("IllegalStateException"),
            PackageLookupValue::NoBuildFile => ErrorReason::NoBuildFile,
            PackageLookupValue::InvalidName { .. }
            | PackageLookupValue::IncorrectRepositoryReference { .. } => {
                ErrorReason::InvalidPackageName
            }
            PackageLookupValue::DeletedPackage => ErrorReason::DeletedPackage,
            PackageLookupValue::NoRepository { .. } => ErrorReason::RepositoryNotFound,
        }
    }

    /// For an unsuccessful package lookup, gets a detailed error message for `get_error_reason`
    /// that is suitable for reporting to a user.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:108 PackageLookupValue.getErrorMsg
    pub fn get_error_msg(&self) -> String {
        match self {
            PackageLookupValue::Success { .. } => panic!(
                "IllegalStateException: attempted to retrieve unsuccessful lookup reason for \
                 successful lookup"
            ),
            PackageLookupValue::NoBuildFile => "BUILD file not found on package path".to_owned(),
            PackageLookupValue::InvalidName { error_msg } => error_msg.clone(),
            PackageLookupValue::IncorrectRepositoryReference {
                invalid_package_identifier,
                corrected_package_identifier,
            } => format!(
                "Invalid package reference {} crosses into repository {}: did you mean to use {} \
                 instead?",
                invalid_package_identifier,
                corrected_package_identifier.get_repository(),
                corrected_package_identifier
            ),
            PackageLookupValue::DeletedPackage => {
                "Package is considered deleted due to --deleted_packages".to_owned()
            }
            PackageLookupValue::NoRepository {
                repository_name,
                reason,
            } => format!(
                "The repository '{}' could not be resolved: {}",
                repository_name, reason
            ),
        }
    }

    /// Creates the error message for the input label has a subpackage crossing boundary.
    ///
    /// Returns `None` if no subpackage is discovered or the subpackage is marked as DELETED.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:405 PackageLookupValue.getErrorMessageForLabelCrossingPackageBoundary
    pub fn get_error_message_for_label_crossing_package_boundary(
        pkg_root: &Root,
        label: &Label,
        subpackage_identifier: &PackageIdentifier,
        package_lookup_value: &PackageLookupValue,
    ) -> Option<String> {
        let mut message = None;
        if package_lookup_value.package_exists() {
            let mut m = format!(
                "Label '{}' is invalid because '{}' is a subpackage",
                label, subpackage_identifier
            );
            let sub_package_root = package_lookup_value.get_root();
            if pkg_root == sub_package_root {
                let label_root_path_fragment = label.get_package_identifier().get_source_root();
                let subpackage_path_fragment = subpackage_identifier.get_source_root();
                if subpackage_path_fragment.starts_with(label_root_path_fragment) {
                    let label_name_in_subpackage = PathFragment::create(label.get_name())
                        .sub_fragment_from(
                            subpackage_path_fragment.segment_count()
                                - label_root_path_fragment.segment_count(),
                        );
                    m.push_str("; perhaps you meant to put the colon here: '");
                    if subpackage_identifier.get_repository().is_main() {
                        m.push_str("//");
                    }
                    m.push_str(&format!(
                        "{}:{}'?",
                        subpackage_identifier, label_name_in_subpackage
                    ));
                } else {
                    // TODO: Is this a valid case? How do we handle this case?
                }
            } else {
                m.push_str(&format!(
                    "; have you deleted {}/BUILD? If so, use the --deleted_packages={} option",
                    subpackage_identifier, subpackage_identifier
                ));
            }
            message = Some(m);
        } else if let PackageLookupValue::IncorrectRepositoryReference {
            corrected_package_identifier,
            ..
        } = package_lookup_value
        {
            message = Some(format!(
                "Label '{}' is invalid because '{}' is a subpackage",
                label, corrected_package_identifier
            ));
        }
        message
    }
}

impl fmt::Display for PackageLookupValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:284 InvalidNamePackageLookupValue.toString
            PackageLookupValue::InvalidName { error_msg } => {
                write!(f, "InvalidNamePackageLookupValue: {}", error_msg)
            }
            // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:347 IncorrectRepositoryReferencePackageLookupValue.toString
            PackageLookupValue::IncorrectRepositoryReference {
                invalid_package_identifier,
                corrected_package_identifier,
            } => write!(
                f,
                "IncorrectRepositoryReferencePackageLookupValue: invalidPackageIdentifier: {}, \
                 corrected: {}",
                invalid_package_identifier, corrected_package_identifier
            ),
            other => write!(f, "{:?}", other),
        }
    }
}

/// SkyKey for [`PackageLookupValue`] computation. `Debug` prints `PACKAGE_LOOKUP:<pkg>`.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:129 PackageLookupValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageLookupKey(pub PackageIdentifier);

impl PackageLookupKey {
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:117 PackageLookupValue.key
    pub fn key(pkg_identifier: PackageIdentifier) -> Self {
        PackageLookupKey(pkg_identifier)
    }
}

impl fmt::Debug for PackageLookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for PackageLookupKey {
    type Value = PackageLookupValue;
    const FUNCTION: SkyFunctionName = sky_functions::PACKAGE_LOOKUP;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:41 SkyFunctions.PACKAGE_LOOKUP (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: `SuccessfulPackageLookupValue` (root, build file name), `InvalidName` (message) and
    /// `IncorrectRepositoryReference` (both ids) compare by value; `NO_BUILD_FILE_VALUE` and
    /// `DELETED_PACKAGE_VALUE` are singletons; `NoRepositoryPackageLookupValue` has no `equals`
    /// and is made anew by each computation, so it is never equal to the last build's.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:207 PackageLookupValue.SuccessfulPackageLookupValue.equals
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupValue.java:371 PackageLookupValue.NoRepositoryPackageLookupValue
    fn values_equal(old: &PackageLookupValue, new: &PackageLookupValue) -> bool {
        !matches!(old, PackageLookupValue::NoRepository { .. }) && old == new
    }
}

/// Lists possible ways to handle a package label which crosses into a new repository.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:57 PackageLookupFunction.CrossRepositoryLabelViolationStrategy
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossRepositoryLabelViolationStrategy {
    /// Ignore the violation.
    Ignore,
    /// Generate an error.
    Error,
}

/// What PackageLookupFunction throws (Java `PackageLookupFunctionException`): a
/// `BuildFileNotFoundException` / `RepositoryFetchException` (both [`NoSuchPackageException`])
/// or an `InconsistentFilesystemException` ([`IOException`]).
// Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:404 PackageLookupFunction.PackageLookupFunctionException
fn package_lookup_function_exception(e: NoSuchPackageException, t: Transience) -> ComputeError {
    SkyFunctionException::new(e, t).into()
}

/// SkyFunction for [`PackageLookupValue`]s.
// Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:55 PackageLookupFunction
pub struct PackageLookupFunction {
    deleted_packages: Arc<RwLock<HashSet<PackageIdentifier>>>,
    cross_repository_label_violation_strategy: CrossRepositoryLabelViolationStrategy,
    build_files_by_priority: Vec<BuildFileName>,
}

impl PackageLookupFunction {
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:69 PackageLookupFunction.<init>
    pub fn new(
        deleted_packages: Arc<RwLock<HashSet<PackageIdentifier>>>,
        cross_repository_label_violation_strategy: CrossRepositoryLabelViolationStrategy,
        build_files_by_priority: Vec<BuildFileName>,
    ) -> Self {
        PackageLookupFunction {
            deleted_packages,
            cross_repository_label_violation_strategy,
            build_files_by_priority,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:84 PackageLookupFunction.compute
    pub fn compute(
        &self,
        sky_key: &PackageLookupKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackageLookupValue>, ComputeError> {
        let pkg_locator = PATH_PACKAGE_LOCATOR.get(env)?;

        let package_key = &sky_key.0;

        if let Some(package_name_error_msg) = label_validator::validate_package_name(
            package_key.get_package_fragment().get_path_string(),
        ) {
            return Ok(Some(PackageLookupValue::invalid_package_name(format!(
                "Invalid package name '{}': {}",
                package_key, package_name_error_msg
            ))));
        }

        if self.deleted_packages.read().contains(package_key) {
            return Ok(Some(PackageLookupValue::DELETED_PACKAGE_VALUE));
        }

        if !package_key.get_repository().is_main() {
            return self.compute_external_package_lookup_value(package_key, env);
        }

        if *package_key == *EXTERNAL_PACKAGE_IDENTIFIER {
            return Ok(Some(PackageLookupValue::NO_BUILD_FILE_VALUE));
        }

        // Check .bazelignore file under main repository.
        let Some(ignored_patterns_value) = env.get_value(IgnoredSubdirectoriesKey::key_main())?
        else {
            return Ok(None);
        };

        let package_fragment = package_key.get_package_fragment();
        if ignored_patterns_value
            .as_ignored_subdirectories()
            .matching_entry(package_fragment)
            .is_some()
        {
            return Ok(Some(PackageLookupValue::DELETED_PACKAGE_VALUE));
        }

        // Java reads PATH_PACKAGE_LOCATOR without a null check.
        let pkg_locator = pkg_locator.expect("PATH_PACKAGE_LOCATOR is injected");
        self.find_package_by_build_file(env, &pkg_locator, package_key)
    }

    /// For a package identifier `package_key` such that the compute for its PACKAGE_LOOKUP key
    /// returned `NO_BUILD_FILE_VALUE`, provide a human-readable error message with more details on
    /// where we searched for the package.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:129 PackageLookupFunction.explainNoBuildFileValue
    pub fn explain_no_build_file_value(
        package_key: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<String, Interrupted> {
        let educational_message = "Add a BUILD file to a directory to mark it as a package.";
        if package_key.get_repository().is_main() {
            let pkg_locator = PATH_PACKAGE_LOCATOR
                .get(env)?
                .expect("PATH_PACKAGE_LOCATOR is injected");
            let mut message = String::new();
            message.push_str("BUILD file not found in any of the following directories. ");
            message.push_str(educational_message);
            let entries = pkg_locator.get_path_entries();
            for root in entries {
                message.push_str("\n - ");
                if entries.len() == 1 {
                    message.push_str(package_key.get_package_fragment().get_path_string());
                } else {
                    message.push_str(
                        root.as_path()
                            .expect("package path entries are path roots")
                            .get_relative(package_key.get_package_fragment())
                            .get_path_string(),
                    );
                }
            }
            Ok(message)
        } else {
            Ok(format!(
                "BUILD file not found in directory '{}' of external repository {}. {}",
                package_key.get_package_fragment(),
                package_key.get_repository(),
                educational_message
            ))
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:157 PackageLookupFunction.findPackageByBuildFile
    fn find_package_by_build_file(
        &self,
        env: &mut Environment<'_>,
        pkg_locator: &PathPackageLocator,
        package_key: &PackageIdentifier,
    ) -> Result<Option<PackageLookupValue>, ComputeError> {
        // PORT: Java keeps the loop positions in a SkyKeyComputeState across restarts; the
        // blocking evaluator never restarts, so the loop runs once.
        for package_path_entry in pkg_locator.get_path_entries() {
            for build_file_name in &self.build_files_by_priority {
                let Some(result) = self.get_package_lookup_value(
                    env,
                    package_path_entry,
                    package_key,
                    *build_file_name,
                )?
                else {
                    return Ok(None);
                };
                if result != PackageLookupValue::NO_BUILD_FILE_VALUE {
                    return Ok(Some(result));
                }
            }
        }
        Ok(Some(PackageLookupValue::NO_BUILD_FILE_VALUE))
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:180 PackageLookupFunction.getFileValue
    fn get_file_value(
        file_rooted_path: &RootedPath,
        env: &mut Environment<'_>,
        package_identifier: &PackageIdentifier,
    ) -> Result<Option<Arc<FileValue>>, ComputeError> {
        let path = file_rooted_path.as_path();
        let basename = path.get_base_name();
        let file_sky_key = FileKey::key(file_rooted_path.clone());
        let e = match env.get_value_or_throw::<FileKey, IOException>(file_sky_key)? {
            Ok(file_value) => return Ok(file_value),
            Err(e) => e,
        };
        match e.kind() {
            IOExceptionKind::InconsistentFilesystem => {
                // This error is not transient from the perspective of the PackageLookupFunction.
                Err(SkyFunctionException::persistent(e).into())
            }
            IOExceptionKind::FileSymlinkCycle { .. }
            | IOExceptionKind::FileSymlinkInfiniteExpansion { .. } => {
                let message = format!(
                    "{} detected while trying to find {} file {}",
                    e.get_message(),
                    basename,
                    path
                );
                Err(package_lookup_function_exception(
                    build_file_not_found_with_detailed_exit_code(
                        package_identifier.clone(),
                        message.clone(),
                        DetailedExitCode::package_loading(
                            message,
                            PackageLoadingCode::SymlinkCycleOrInfiniteExpansion,
                        ),
                    ),
                    Transience::Persistent,
                ))
            }
            IOExceptionKind::Detailed(detailed) => {
                let message = format!(
                    "IO errors while looking for {} file reading {}: {}",
                    basename,
                    path,
                    e.get_message()
                );
                Err(package_lookup_function_exception(
                    build_file_not_found_with_detailed_exit_code(
                        package_identifier.clone(),
                        message.clone(),
                        DetailedExitCode {
                            message,
                            code: FailureCode::Other(format!(
                                "Filesystem.{}",
                                detailed.get_filesystem_code()
                            )),
                        },
                    ),
                    if detailed.is_transient() {
                        Transience::Transient
                    } else {
                        Transience::Persistent
                    },
                ))
            }
            IOExceptionKind::Plain => {
                let message = format!(
                    "IO errors while looking for {} file reading {}: {}",
                    basename,
                    path,
                    e.get_message()
                );
                Err(package_lookup_function_exception(
                    build_file_not_found_with_detailed_exit_code(
                        package_identifier.clone(),
                        message.clone(),
                        DetailedExitCode::package_loading(
                            message,
                            PackageLoadingCode::OtherIoException,
                        ),
                    ),
                    Transience::Persistent,
                ))
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:252 PackageLookupFunction.getPackageLookupValue
    fn get_package_lookup_value(
        &self,
        env: &mut Environment<'_>,
        package_path_entry: &Root,
        package_identifier: &PackageIdentifier,
        build_file_name: BuildFileName,
    ) -> Result<Option<PackageLookupValue>, ComputeError> {
        let build_file_fragment = build_file_name.get_build_file_fragment(package_identifier);

        if self.cross_repository_label_violation_strategy
            == CrossRepositoryLabelViolationStrategy::Error
        {
            // Is this path part of a local repository?
            let current_path = RootedPath::to_rooted_path(
                package_path_entry.clone(),
                &build_file_fragment
                    .get_parent_directory()
                    .expect("a build file has a parent directory"),
            );
            let repository_lookup_key = LocalRepositoryLookupKey::key(current_path.clone());

            let local_repository = match env
                .get_value_or_throw::<LocalRepositoryLookupKey, ErrorDeterminingRepositoryException>(
                    repository_lookup_key,
                )? {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(None),
                Err(_e) => {
                    // If the directory selected isn't part of a repository, that's an error.
                    // TODO(katre): Improve the error message given here.
                    return Err(package_lookup_function_exception(
                        NoSuchPackageException::build_file_not_found(
                            package_identifier.clone(),
                            format!(
                                "Unable to determine the local repository for directory {}",
                                current_path.as_path().get_path_string()
                            ),
                        ),
                        Transience::Persistent,
                    ));
                }
            };

            if local_repository.exists()
                && local_repository.get_repository() != package_identifier.get_repository()
            {
                // There is a repository mismatch, this is an error.
                // The correct package path is the one originally given, minus the part that is
                // the local repository.
                let mut path_to_requested_package = package_identifier.get_source_root().clone();
                let local_repository_path = local_repository.get_path();
                if local_repository_path.is_absolute() {
                    // We need the package path to also be absolute.
                    path_to_requested_package = package_path_entry
                        .get_relative(&path_to_requested_package)
                        .as_fragment()
                        .clone();
                }
                let remaining_path = path_to_requested_package.relative_to(&local_repository_path);
                let correct_package = PackageIdentifier::create(
                    local_repository.get_repository().clone(),
                    remaining_path,
                );
                return Ok(Some(PackageLookupValue::incorrect_repository_reference(
                    package_identifier.clone(),
                    correct_package,
                )));
            }

            // There's no local repository, keep going.
        } else {
            // Future-proof against adding future values to CrossRepositoryLabelViolationStrategy.
            blaze_util::check_state!(
                self.cross_repository_label_violation_strategy
                    == CrossRepositoryLabelViolationStrategy::Ignore,
                "{:?}",
                self.cross_repository_label_violation_strategy
            );
        }

        // Check for the existence of the build file.
        let build_file_rooted_path =
            RootedPath::to_rooted_path(package_path_entry.clone(), &build_file_fragment);
        let Some(file_value) =
            Self::get_file_value(&build_file_rooted_path, env, package_identifier)?
        else {
            return Ok(None);
        };

        if file_value.is_file() {
            return Ok(Some(PackageLookupValue::success(
                build_file_rooted_path.get_root().clone(),
                build_file_name,
            )));
        }

        Ok(Some(PackageLookupValue::NO_BUILD_FILE_VALUE))
    }

    /// Gets a PackageLookupValue from a different Bazel repository.
    ///
    /// To do this, it looks up the "external" package and finds a path mapping for the repository
    /// name.
    // Java: com/google/devtools/build/lib/skyframe/PackageLookupFunction.java:332 PackageLookupFunction.computeExternalPackageLookupValue
    fn compute_external_package_lookup_value(
        &self,
        id: &PackageIdentifier,
        env: &mut Environment<'_>,
    ) -> Result<Option<PackageLookupValue>, ComputeError> {
        let repository_key = RepositoryDirectoryKey::key(id.get_repository().clone());
        // Java: getValueOrThrow(repositoryKey, NoSuchPackageException.class, IOException.class,
        // EvalException.class, AlreadyReportedException.class).
        let repository_value = match env.get_value_or_throw_with(repository_key, |e| {
            if let Some(e) = e.downcast_ref::<NoSuchPackageException>() {
                Some(NoSuchPackageException::build_file_not_found(
                    id.clone(),
                    e.get_message(),
                ))
            } else if let Some(e) = e.downcast_ref::<IOException>() {
                Some(NoSuchPackageException::repository_fetch(
                    id.clone(),
                    e.get_message(),
                ))
            } else if let Some(e) = e.downcast_ref::<blaze_starlark::EvalError>() {
                Some(NoSuchPackageException::repository_fetch(
                    id.clone(),
                    e.message.clone(),
                ))
            } else {
                // catch (IOException | EvalException | AlreadyReportedException e)
                e.downcast_ref::<crate::exceptions::AlreadyReportedException>()
                    .map(|e| NoSuchPackageException::repository_fetch(id.clone(), e.get_message()))
            }
        })? {
            Err(e) => {
                return Err(package_lookup_function_exception(e, Transience::Persistent));
            }
            Ok(v) => v,
        };
        let Some(repository_value) = repository_value else {
            return Ok(None);
        };
        let root = match &*repository_value {
            RepositoryDirectoryValue::Failure { error_msg } => {
                return Ok(Some(PackageLookupValue::NoRepository {
                    repository_name: id.get_repository().clone(),
                    reason: error_msg.clone(),
                }));
            }
            RepositoryDirectoryValue::Success { root, .. } => root.clone(),
        };

        // Check .bazelignore file after fetching the external repository.
        let Some(ignored_patterns_value) =
            env.get_value(IgnoredSubdirectoriesKey::key(id.get_repository().clone()))?
        else {
            return Ok(None);
        };

        let package_fragment = id.get_package_fragment();
        if ignored_patterns_value
            .as_ignored_subdirectories()
            .matching_entry(package_fragment)
            .is_some()
        {
            return Ok(Some(PackageLookupValue::DELETED_PACKAGE_VALUE));
        }

        // This checks for the build file names in the correct precedence order.
        for build_file_name in &self.build_files_by_priority {
            let build_file_fragment = id
                .get_package_fragment()
                .get_relative(build_file_name.get_filename_fragment());
            let build_file_rooted_path =
                RootedPath::to_rooted_path(root.clone(), &build_file_fragment);
            let Some(file_value) = Self::get_file_value(&build_file_rooted_path, env, id)? else {
                return Ok(None);
            };

            if file_value.is_file() {
                return Ok(Some(PackageLookupValue::success(root, *build_file_name)));
            }
        }

        Ok(Some(PackageLookupValue::NO_BUILD_FILE_VALUE))
    }
}
