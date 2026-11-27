//! `SkyValue.debugPrint` (the Java `toString` that `blaze dump --skyframe=value` prints) of the
//! loading values whose Java text is deterministic. See `blaze_skyframe::dump::dump_values`.
//!
//! Not registered here, because Java prints an identity hash code (`Object.toString`):
//! - `PACKAGE_LOOKUP` (`SuccessfulPackageLookupValue`, `NoBuildFilePackageLookupValue`,
//!   `DeletedPackageLookupValue` do not override `toString`);
//! - `BZL_LOAD`, `BZL_COMPILE`, `STARLARK_BUILTINS`, `GLOBS`, `REPO_PACKAGE_ARGS`,
//!   `DIRECTORY_LISTING` (no `toString` of their own).
//!
//! Not ported: `PROJECT_FILES_LOOKUP` (`Object.toString` with the class's own `hashCode`, e.g.
//! `com.google.devtools.build.lib.skyframe.ProjectFilesLookupValue@1`).

use std::sync::Once;

use blaze_skyframe::dump::register_to_string;

use crate::actions::{FileStateValue, FileValue};
use crate::containing_package_lookup::ContainingPackageLookupValue;
use crate::directory_listing::DirectoryListingStateValue;
use crate::ignored_subdirectories::IgnoredSubdirectoriesValue;
use crate::local_repository_lookup::LocalRepositoryLookupValue;
use crate::package_function::PackageValue;
use crate::repos::RepositoryDirectoryValue;

/// Registers the loading values' `debugPrint` (once per process).
pub fn register_debug_prints() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        register_to_string::<PackageValue>(package_value_to_string);
        register_to_string::<LocalRepositoryLookupValue>(local_repository_lookup_value_to_string);
        register_to_string::<ContainingPackageLookupValue>(
            containing_package_lookup_value_to_string,
        );
        register_to_string::<DirectoryListingStateValue>(directory_listing_state_value_to_string);
        register_to_string::<IgnoredSubdirectoriesValue>(ignored_subdirectories_value_to_string);
        register_to_string::<FileStateValue>(file_state_value_to_string);
        register_to_string::<FileValue>(file_value_to_string);
        register_to_string::<RepositoryDirectoryValue>(repository_directory_value_to_string);
    });
}

// Java: com/google/devtools/build/lib/skyframe/PackageValue.java:56 PackageValue.toString
fn package_value_to_string(value: &PackageValue) -> String {
    format!("<PackageValue name={}>", value.get_package().get_name())
}

// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:126 LocalRepositoryLookupValue.MainRepositoryLookupValue.toString
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:169 LocalRepositoryLookupValue.SuccessfulLocalRepositoryLookupValue.toString
// Java: com/google/devtools/build/lib/skyframe/LocalRepositoryLookupValue.java:209 LocalRepositoryLookupValue.NotFoundLocalRepositoryLookupValue.toString
fn local_repository_lookup_value_to_string(value: &LocalRepositoryLookupValue) -> String {
    match value {
        LocalRepositoryLookupValue::MainRepository => "MainRepositoryLookupValue".to_owned(),
        LocalRepositoryLookupValue::Successful {
            repository_name, ..
        } => format!("SuccessfulLocalRepositoryLookupValue({repository_name})"),
        LocalRepositoryLookupValue::NotFound => "NotFoundLocalRepositoryLookupValue".to_owned(),
    }
}

// Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:174 ContainingPackageLookupValue.NoContainingPackage.toString
// Java: com/google/devtools/build/lib/skyframe/ContainingPackageLookupValue.java:228 ContainingPackageLookupValue.ContainingPackage.toString
fn containing_package_lookup_value_to_string(value: &ContainingPackageLookupValue) -> String {
    match value {
        // getClass().getName(): the binary name of the nested class.
        ContainingPackageLookupValue::NoContainingPackage { .. } => {
            "com.google.devtools.build.lib.skyframe.ContainingPackageLookupValue$NoContainingPackage"
                .to_owned()
        }
        // MoreObjects.toStringHelper(this): the simple class name.
        ContainingPackageLookupValue::ContainingPackage {
            containing_package,
            containing_package_root,
        } => format!(
            "ContainingPackage{{containingPackage={containing_package}, \
             containingPackageRoot={containing_package_root}}}"
        ),
    }
}

// Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:110 DirectoryListingStateValue.toString
fn directory_listing_state_value_to_string(value: &DirectoryListingStateValue) -> String {
    // Iterables.toString: "[" + elements joined by ", " + "]".
    let dirents: Vec<String> = value.get_dirents().iter().map(|d| d.to_string()).collect();
    format!(
        "DirectoryListingStateValue{{dirents=[{}]}}",
        dirents.join(", ")
    )
}

// Java: com/google/devtools/build/lib/skyframe/IgnoredSubdirectoriesValue.java:82 IgnoredSubdirectoriesValue.toString
fn ignored_subdirectories_value_to_string(value: &IgnoredSubdirectoriesValue) -> String {
    // Java: com/google/devtools/build/lib/cmdline/IgnoredSubdirectories.java:205 IgnoredSubdirectories.toString
    format!("{:?}", value.as_ignored_subdirectories())
}

/// The Java `toString` of a [`FileStateValue`], failing loudly where Java's text holds a
/// `FileArtifactValue` text that is not ported.
// Java: com/google/devtools/build/lib/actions/FileStateValue.java:294 FileStateValue.toString
fn file_state_value_to_string(value: &FileStateValue) -> String {
    match value {
        // Java: com/google/devtools/build/lib/actions/FileStateValue.java:505 RegularFileStateValueWithMetadata.toString
        FileStateValue::RegularFileWithMetadata(_) => blaze_util::unported!(
            "FileStateValue.RegularFileStateValueWithMetadata.toString",
            "the toString of the FileArtifactValue"
        ),
        // The other variants' Display is Java's toString (prettyPrint, or toStringHelper for
        // RegularFileStateValueWithDigest and RegularFileStateValueWithContentsProxy).
        _ => value.to_string(),
    }
}

// Java: com/google/devtools/build/lib/actions/FileValue.java:226 FileValue
fn file_value_to_string(value: &FileValue) -> String {
    // Every variant prints its real FileStateValue; check it as file_state_value_to_string does.
    file_state_value_to_string(value.real_file_state_value());
    value.to_string()
}

/// Records: `Success[root=..., excludeFromVendoring=...]`, `Failure[errorMsg=...]`.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:59 RepositoryDirectoryValue.Success
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:68 RepositoryDirectoryValue.Failure
fn repository_directory_value_to_string(value: &RepositoryDirectoryValue) -> String {
    match value {
        RepositoryDirectoryValue::Success {
            root,
            exclude_from_vendoring,
        } => format!("Success[root={root}, excludeFromVendoring={exclude_from_vendoring}]"),
        RepositoryDirectoryValue::Failure { error_msg } => format!("Failure[errorMsg={error_msg}]"),
    }
}
