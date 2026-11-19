//! Precomputed values read by repository fetching and module extension evaluation.
//!
//! PORT: these use `blaze_bzlmod`'s `Precomputed` (key type `blaze_bzlmod::PrecomputedKey`).
//! `STARLARK_SEMANTICS`, `PATH_PACKAGE_LOCATOR` and `REPOSITORY_OVERRIDES` are read from
//! `blaze_sky_loading::precomputed` (its own key type), and `LOCKFILE_MODE` / `VENDOR_DIRECTORY`
//! from `blaze_bzlmod::skyframe::precomputed`; the runtime injects each once.

use blaze_bzlmod::skyframe::precomputed::Precomputed;

/// `--nofetch`.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:42 RepositoryDirectoryValue.FETCH_DISABLED
pub static FETCH_DISABLED: Precomputed<bool> = Precomputed::new("fetch_disabled");

/// A UUID when every repository must be refetched (`bazel fetch --force`), else empty.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:44 RepositoryDirectoryValue.FORCE_FETCH
pub static FORCE_FETCH: Precomputed<String> =
    Precomputed::new("dependency_for_force_fetching_repository");

/// A UUID when every `configure` repository must be refetched, else empty.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:45 RepositoryDirectoryValue.FORCE_FETCH_CONFIGURE
pub static FORCE_FETCH_CONFIGURE: Precomputed<String> =
    Precomputed::new("dependency_for_force_fetching_configure_repositories");

/// Whether the command is `vendor`.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:47 RepositoryDirectoryValue.IS_VENDOR_COMMAND
pub static IS_VENDOR_COMMAND: Precomputed<bool> = Precomputed::new("is_vendor_command");

/// `--incompatible_require_repo_extension_metadata`.
// Java: com/google/devtools/build/lib/bazel/repository/RepoMetadataRequirements.java:22 RepoMetadataRequirements.REQUIRE_REPO_EXTENSION_METADATA
pub static REQUIRE_REPO_EXTENSION_METADATA: Precomputed<RequireRepoExtensionMetadataMode> =
    Precomputed::new("require_repo_extension_metadata");

/// Whether remote execution is enabled.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:96 PrecomputedValue.REMOTE_EXECUTION_ENABLED
pub static REMOTE_EXECUTION_ENABLED: Precomputed<bool> =
    Precomputed::new("remote_execution_enabled");

/// Values of `--incompatible_require_repo_extension_metadata`.
// Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java RepositoryOptions.RequireRepoExtensionMetadataMode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequireRepoExtensionMetadataMode {
    False,
    Root,
    All,
}

impl std::fmt::Display for RequireRepoExtensionMetadataMode {
    // Java: com/google/devtools/build/lib/bazel/repository/RepositoryOptions.java:421 RequireRepoExtensionMetadataMode.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RequireRepoExtensionMetadataMode::False => "false",
            RequireRepoExtensionMetadataMode::Root => "root",
            RequireRepoExtensionMetadataMode::All => "all",
        })
    }
}
