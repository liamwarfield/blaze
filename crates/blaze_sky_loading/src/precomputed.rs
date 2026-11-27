//! Port of `PrecomputedValue`: variables injected into Skyframe before an evaluation.
//!
//! Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java. Only the variables the
//! loading functions read are declared here; later crates declare their own `Precomputed<T>`
//! constants with the same mechanism. The key and value types are `blaze_skyframe`'s, shared by
//! every crate (one graph node per variable).

pub use blaze_skyframe::precomputed::{Precomputed, PrecomputedKey, PrecomputedValue};
use blaze_starlark::StarlarkSemantics;

use crate::pkgcache::PathPackageLocator;

/// `RepositoryDirectoryValue.VENDOR_DIRECTORY`: the absolute vendor directory (`--vendor_dir`).
///
/// PORT: also declared as `blaze_bzlmod::skyframe::precomputed::VENDOR_DIRECTORY` (that crate
/// does not depend on this one); both name the key `vendor_directory` with the same value type,
/// so they are one Skyframe node.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:48 RepositoryDirectoryValue.VENDOR_DIRECTORY
pub static VENDOR_DIRECTORY: Precomputed<Option<std::path::PathBuf>> =
    Precomputed::new("vendor_directory");

/// The id of the current build (the command id). Actions that must have a chance to run in every
/// build (`Actions.dependsOnBuildId`) and the workspace status depend on it, so that an injection
/// per command dirties them.
///
/// PORT: Java's value is the command's `UUID`; here it is its string form.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:88 PrecomputedValue.BUILD_ID
pub static BUILD_ID: Precomputed<String> = Precomputed::new("build_id");

// PORT: skipped, registered at the pin but no function or command requests ACTION_ENVIRONMENT_VARIABLE (ActionEnvironmentFunction.key and getEnvironmentView have no caller): ActionEnvironmentFunction.compute, ActionEnvironmentFunction.key, ActionEnvironmentFunction.Key.<init>, ActionEnvironmentFunction.Key.create, ActionEnvironmentFunction.Key.intern, ActionEnvironmentFunction.Key.functionName, ActionEnvironmentFunction.Key.getSkyKeyInterner, ActionEnvironmentFunction.getEnvironmentView
/// The `--action_env` variables, in order; `None` for a variable inherited from the client
/// (Java's `LinkedHashMap` with null values). Injected by every command (`SkyframeExecutor.sync`);
/// Java's `ActionEnvironmentFunction` reads it.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:91 PrecomputedValue.ACTION_ENV
pub static ACTION_ENV: Precomputed<indexmap::IndexMap<String, Option<String>>> =
    Precomputed::new("action_env");

// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:84 PrecomputedValue.STARLARK_SEMANTICS
pub static STARLARK_SEMANTICS: Precomputed<StarlarkSemantics> =
    Precomputed::new("starlark_semantics");

// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:93 PrecomputedValue.PATH_PACKAGE_LOCATOR
pub static PATH_PACKAGE_LOCATOR: Precomputed<PathPackageLocator> =
    Precomputed::new("path_package_locator");

/// `--override_repository` values by canonical repo name.
///
/// PORT: move to blaze_bzlmod (Java `RepoDefinitionFunction`) when that crate exists;
/// REPOSITORY_MAPPING reads it.
// Java: com/google/devtools/build/lib/bazel/repository/RepoDefinitionFunction.java:58 RepoDefinitionFunction.REPOSITORY_OVERRIDES
pub static REPOSITORY_OVERRIDES: Precomputed<indexmap::IndexMap<String, blaze_vfs::PathFragment>> =
    Precomputed::new("repository_overrides");

/// Configuration of `--incompatible_autoload_externally`.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:99 AutoloadSymbols.AUTOLOAD_SYMBOLS
pub static AUTOLOAD_SYMBOLS: Precomputed<blaze_packages::builtins::AutoloadSymbols> =
    Precomputed::new("autoload_symbols");

/// `--default_visibility`.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:79 PrecomputedValue.DEFAULT_VISIBILITY
pub static DEFAULT_VISIBILITY: Precomputed<blaze_packages::package::RuleVisibility> =
    Precomputed::new("default_visibility");

/// How `config_setting` visibility is enforced.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:82 PrecomputedValue.CONFIG_SETTING_VISIBILITY_POLICY
pub static CONFIG_SETTING_VISIBILITY_POLICY: Precomputed<
    blaze_packages::package::ConfigSettingVisibilityPolicy,
> = Precomputed::new("config_setting_visibility_policy");

/// `--experimental_lazy_macro_expansion_packages` (Java `PackageOptions.LazyMacroExpansionPackages`).
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:107 PrecomputedValue.LAZY_MACRO_EXPANSION_PACKAGES
pub static LAZY_MACRO_EXPANSION_PACKAGES: Precomputed<LazyMacroExpansionPackages> =
    Precomputed::new("lazy_macro_expansion_packages");

/// The packages whose symbolic macros are expanded lazily (Java
/// `PackageOptions.LazyMacroExpansionPackages`: `ALL` for `*`, else the listed set).
// Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:266 PackageOptions.LazyMacroExpansionPackages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LazyMacroExpansionPackages {
    /// Java `LazyMacroExpansionPackages.ALL`.
    All,
    /// Java `LazyMacroExpansionPackagesSet`.
    Set(std::collections::HashSet<blaze_cmdline::PackageIdentifier>),
}

impl LazyMacroExpansionPackages {
    // Java: com/google/devtools/build/lib/pkgcache/PackageOptions.java:278 PackageOptions.LazyMacroExpansionPackages.contains
    pub fn contains(&self, package_id: &blaze_cmdline::PackageIdentifier) -> bool {
        match self {
            LazyMacroExpansionPackages::All => true,
            LazyMacroExpansionPackages::Set(ids) => ids.contains(package_id),
        }
    }
}
