//! The precomputed values and client environment variables the bzlmod SkyFunctions read.
//!
//! The precomputed key and value types are `blaze_skyframe::precomputed`'s, shared with
//! `blaze_sky_loading` (one graph node per variable name, as Java's interned keys).
//!
//! PORT: move `ClientEnvironmentFunction` to blaze_sky_loading (`lib/skyframe`); this crate
//! cannot depend on it (it would pull in the package loading crates). The local copy keeps the
//! function name `CLIENT_ENVIRONMENT_VARIABLE` and the key format.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_bzlmod_model::{
    BazelCompatibilityMode, CheckDirectDepsMode, LockfileMode, ModuleOverride,
};
use blaze_skyframe::EvaluatorBuilder;
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

pub use blaze_skyframe::precomputed::{PRECOMPUTED, Precomputed, PrecomputedKey, PrecomputedValue};

/// The client environment variable key and value moved to blaze_skyframe (I2 item X2: action
/// execution, in a crate that cannot depend on this one, requests them too).
pub use blaze_skyframe::precomputed::{
    CLIENT_ENVIRONMENT_VARIABLE, ClientEnvironmentKey, EnvironmentVariableValue,
};

// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:85 PrecomputedValue.STARLARK_SEMANTICS
pub static STARLARK_SEMANTICS: Precomputed<StarlarkSemantics> =
    Precomputed::new("starlark_semantics");

/// Never empty.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:106 ModuleFileFunction.REGISTRIES
pub static REGISTRIES: Precomputed<IndexSet<String>> = Precomputed::new("registries");

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:108 ModuleFileFunction.IGNORE_DEV_DEPS
pub static IGNORE_DEV_DEPS: Precomputed<bool> = Precomputed::new("ignore_dev_dependency");

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:111 ModuleFileFunction.MODULE_OVERRIDES
pub static MODULE_OVERRIDES: Precomputed<IndexMap<String, ModuleOverride>> =
    Precomputed::new("module_overrides");

// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:113 ModuleFileFunction.INJECTED_REPOSITORIES
pub static INJECTED_REPOSITORIES: Precomputed<IndexMap<String, PathFragment>> =
    Precomputed::new("repository_injections");

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:64 BazelModuleResolutionFunction.CHECK_DIRECT_DEPENDENCIES
pub static CHECK_DIRECT_DEPENDENCIES: Precomputed<CheckDirectDepsMode> =
    Precomputed::new("check_direct_dependency");

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:66 BazelModuleResolutionFunction.BAZEL_COMPATIBILITY_MODE
pub static BAZEL_COMPATIBILITY_MODE: Precomputed<BazelCompatibilityMode> =
    Precomputed::new("bazel_compatibility_mode");

// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsUtil.java:28 YankedVersionsUtil.ALLOWED_YANKED_VERSIONS
pub static ALLOWED_YANKED_VERSIONS: Precomputed<Vec<String>> =
    Precomputed::new("allowed_yanked_versions");

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileFunction.java:51 BazelLockFileFunction.LOCKFILE_MODE
pub static LOCKFILE_MODE: Precomputed<LockfileMode> = Precomputed::new("lockfile_mode");

// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFunction.java:45 RegistryFunction.MODULE_MIRRORS
pub static MODULE_MIRRORS: Precomputed<IndexMap<String, IndexSet<String>>> =
    Precomputed::new("module_mirrors");

/// Set to the current time by `BazelRepositoryModule` after `INVALIDATION_INTERVAL` has passed;
/// used to refresh the mutable registry contents cached in memory from time to time.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryFunction.java:42 RegistryFunction.LAST_INVALIDATION
pub static LAST_INVALIDATION: Precomputed<SystemTime> =
    Precomputed::new("last_registry_invalidation");

/// `Optional<Path>`: the absolute vendor directory.
// Java: com/google/devtools/build/lib/rules/repository/RepositoryDirectoryValue.java:48 RepositoryDirectoryValue.VENDOR_DIRECTORY
pub static VENDOR_DIRECTORY: Precomputed<Option<PathBuf>> = Precomputed::new("vendor_directory");

/// Registers `ClientEnvironmentFunction`: the value of a variable in `client_env`.
// Java: com/google/devtools/build/lib/skyframe/ClientEnvironmentFunction.java:73 ClientEnvironmentFunction.compute
pub fn register_client_environment_function(
    builder: EvaluatorBuilder,
    client_env: Arc<IndexMap<String, String>>,
) -> EvaluatorBuilder {
    builder.typed_function::<ClientEnvironmentKey, _>(move |key, _env| {
        Ok(Some(EnvironmentVariableValue(
            client_env.get(&key.0).cloned(),
        )))
    })
}
