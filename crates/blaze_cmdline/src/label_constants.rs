//! Port of `LabelConstants`: constants associated with labels.

use std::sync::LazyLock;

use blaze_vfs::PathFragment;

use crate::package_identifier::PackageIdentifier;

/// The subdirectory under the output base which contains external repositories.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:21 LabelConstants.EXTERNAL_REPOSITORY_LOCATION
pub static EXTERNAL_REPOSITORY_LOCATION: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("external"));

/// The subdirectory under the output base which contains temporary working directories for
/// module extensions.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:27 LabelConstants.MODULE_EXTENSION_WORKING_DIRECTORY_LOCATION
pub static MODULE_EXTENSION_WORKING_DIRECTORY_LOCATION: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("modextwd"));

/// The name of the package that contains the targets representing external repositories.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:34 LabelConstants.EXTERNAL_PACKAGE_NAME
pub static EXTERNAL_PACKAGE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("external"));

/// The identifier of the package that contains the targets representing external repositories.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:40 LabelConstants.EXTERNAL_PACKAGE_IDENTIFIER
pub static EXTERNAL_PACKAGE_IDENTIFIER: LazyLock<PackageIdentifier> =
    LazyLock::new(|| PackageIdentifier::create_in_main_repo(EXTERNAL_PACKAGE_NAME.clone()));

// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:43 LabelConstants.WORKSPACE_FILE_NAME
pub static WORKSPACE_FILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("WORKSPACE"));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:44 LabelConstants.WORKSPACE_DOT_BAZEL_FILE_NAME
pub static WORKSPACE_DOT_BAZEL_FILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("WORKSPACE.bazel"));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:46 LabelConstants.MODULE_DOT_BAZEL_FILE_NAME
pub static MODULE_DOT_BAZEL_FILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("MODULE.bazel"));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:47 LabelConstants.REPO_FILE_NAME
pub static REPO_FILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("REPO.bazel"));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:48 LabelConstants.VENDOR_FILE_NAME
pub static VENDOR_FILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("VENDOR.bazel"));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:50 LabelConstants.MODULE_LOCKFILE_NAME
pub static MODULE_LOCKFILE_NAME: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("MODULE.bazel.lock"));

/// The prefix of external repository exec paths ("external").
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:54 LabelConstants.EXTERNAL_PATH_PREFIX
pub static EXTERNAL_PATH_PREFIX: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create("external"));
/// The prefix of external repository exec paths under --experimental_sibling_repository_layout.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:57 LabelConstants.EXPERIMENTAL_EXTERNAL_PATH_PREFIX
pub static EXPERIMENTAL_EXTERNAL_PATH_PREFIX: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create(".."));

/// The prefix of external repository runfiles paths.
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:64 LabelConstants.EXTERNAL_RUNFILES_PATH_PREFIX
pub static EXTERNAL_RUNFILES_PATH_PREFIX: LazyLock<PathFragment> =
    LazyLock::new(|| PathFragment::create(".."));
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:65 LabelConstants.COMMAND_LINE_OPTION_PREFIX
pub const COMMAND_LINE_OPTION_PREFIX: &str = "//command_line_option:";
// Java: com/google/devtools/build/lib/cmdline/LabelConstants.java:66 LabelConstants.COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER
pub static COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER: LazyLock<PackageIdentifier> =
    LazyLock::new(|| {
        PackageIdentifier::create_in_main_repo(PathFragment::create("command_line_option"))
    });
