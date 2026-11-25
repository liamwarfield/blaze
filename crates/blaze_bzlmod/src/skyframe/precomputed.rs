//! The precomputed values and client environment variables the bzlmod SkyFunctions read.
//!
//! The precomputed key and value types are `blaze_skyframe::precomputed`'s, shared with
//! `blaze_sky_loading` (one graph node per variable name, as Java's interned keys).
//!
//! PORT: move `ClientEnvironmentFunction` to blaze_sky_loading (`lib/skyframe`); this crate
//! cannot depend on it (it would pull in the package loading crates). The local copy keeps the
//! function name `CLIENT_ENVIRONMENT_VARIABLE` and the key format.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_bzlmod_model::{
    BazelCompatibilityMode, CheckDirectDepsMode, LockfileMode, ModuleOverride,
};
use blaze_skyframe::{EvaluatorBuilder, FunctionHermeticity, Key, SkyFunctionName};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

pub use blaze_skyframe::precomputed::{PRECOMPUTED, Precomputed, PrecomputedKey, PrecomputedValue};

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:24 SkyFunctions.CLIENT_ENVIRONMENT_VARIABLE
pub const CLIENT_ENVIRONMENT_VARIABLE: SkyFunctionName =
    SkyFunctionName("CLIENT_ENVIRONMENT_VARIABLE");

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

/// `ClientEnvironmentFunction.Key`: the name of a client environment variable. `Debug` is
/// `CLIENT_ENVIRONMENT_VARIABLE:<name>` (Java `AbstractSkyKey.toString`).
///
/// PORT: move to blaze_sky_loading (`lib/skyframe/ClientEnvironmentFunction`).
// Java: com/google/devtools/build/lib/skyframe/ClientEnvironmentFunction.java:37 ClientEnvironmentFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ClientEnvironmentKey(pub String);

impl ClientEnvironmentKey {
    // Java: com/google/devtools/build/lib/skyframe/ClientEnvironmentFunction.java:30 ClientEnvironmentFunction.key
    pub fn key(key_string: &str) -> ClientEnvironmentKey {
        ClientEnvironmentKey(key_string.to_string())
    }
}

impl fmt::Debug for ClientEnvironmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", CLIENT_ENVIRONMENT_VARIABLE, self.0)
    }
}

impl Key for ClientEnvironmentKey {
    type Value = EnvironmentVariableValue;
    const FUNCTION: SkyFunctionName = CLIENT_ENVIRONMENT_VARIABLE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:24 SkyFunctions.CLIENT_ENVIRONMENT_VARIABLE (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: `EnvironmentVariableValue` is a record of one nullable string.
    // Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
    fn values_equal(old: &EnvironmentVariableValue, new: &EnvironmentVariableValue) -> bool {
        old == new
    }
}

/// The value of a client environment variable (`None`: unset). Port of
/// `EnvironmentVariableValue`.
///
/// PORT: move to blaze_sky_loading (`lib/skyframe/EnvironmentVariableValue`).
// Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariableValue(pub Option<String>);

impl EnvironmentVariableValue {
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

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
