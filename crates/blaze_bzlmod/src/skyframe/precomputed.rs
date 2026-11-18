//! The precomputed values and client environment variables the bzlmod SkyFunctions read.
//!
//! PORT: move to blaze_sky_loading. `PrecomputedValue` and `ClientEnvironmentFunction` live in
//! `lib/skyframe`; `blaze_sky_loading` has a `PrecomputedKey` of its own, but this crate cannot
//! depend on it (it would pull in the package loading crates). These local copies keep the same
//! function names (`PRECOMPUTED`, `CLIENT_ENVIRONMENT_VARIABLE`) and key formats. Both kinds of
//! `PrecomputedKey` can live in one evaluator: they are different Rust key types, so the graph
//! keeps them apart.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_bzlmod_model::{
    BazelCompatibilityMode, CheckDirectDepsMode, LockfileMode, ModuleOverride,
};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Interrupted, Key, SkyFunctionName,
};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:22 SkyFunctions.PRECOMPUTED
pub const PRECOMPUTED: SkyFunctionName = SkyFunctionName("PRECOMPUTED");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:24 SkyFunctions.CLIENT_ENVIRONMENT_VARIABLE
pub const CLIENT_ENVIRONMENT_VARIABLE: SkyFunctionName =
    SkyFunctionName("CLIENT_ENVIRONMENT_VARIABLE");

/// A value computed outside of Skyframe and injected into the graph. Port of `PrecomputedValue`.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:45 PrecomputedValue
#[derive(Clone)]
pub struct PrecomputedValue(pub Arc<dyn Any + Send + Sync>);

impl PrecomputedValue {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:113 PrecomputedValue.<init>
    pub fn new<T: Send + Sync + 'static>(value: T) -> PrecomputedValue {
        PrecomputedValue(Arc::new(value))
    }

    /// Returns the value of the variable.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:118 PrecomputedValue.get
    pub fn get(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.0
    }
}

impl fmt::Debug for PrecomputedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java prints "<BuildVariable " + value + ">"; the value is type-erased here.
        f.write_str("<BuildVariable>")
    }
}

/// `SkyKey` of a [`PrecomputedValue`]: the variable name. `Debug` is `PRECOMPUTED:<name>`
/// (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:191 PrecomputedValue.Key
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrecomputedKey(pub &'static str);

impl fmt::Debug for PrecomputedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", PRECOMPUTED, self.0)
    }
}

impl Key for PrecomputedKey {
    type Value = PrecomputedValue;
    const FUNCTION: SkyFunctionName = PRECOMPUTED;
}

/// A typed helper for one precomputed variable. Port of `PrecomputedValue.Precomputed`.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:145 PrecomputedValue.Precomputed
pub struct Precomputed<T> {
    key: PrecomputedKey,
    _value: PhantomData<fn() -> T>,
}

impl<T: Send + Sync + 'static> Precomputed<T> {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:148 Precomputed.<init>
    pub const fn new(key: &'static str) -> Precomputed<T> {
        Precomputed {
            key: PrecomputedKey(key),
            _value: PhantomData,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:156 Precomputed.getKey
    pub fn get_key(&self) -> PrecomputedKey {
        self.key
    }

    /// Retrieves the value of this variable from Skyframe (`None`: unavailable).
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:167 Precomputed.get
    pub fn get(&self, env: &mut Environment<'_>) -> Result<Option<Arc<T>>, Interrupted> {
        let Some(value) = env.get_value(self.key)? else {
            return Ok(None);
        };
        Ok(Some(self.downcast(&value)))
    }

    /// `Objects.requireNonNull(P.get(env))`, and Java's unboxing of a `Precomputed<Boolean>`:
    /// the variable must be injected.
    pub fn get_required(&self, env: &mut Environment<'_>) -> Result<Arc<T>, ComputeError> {
        match self.get(env)? {
            Some(v) => Ok(v),
            None => panic!("NullPointerException: precomputed value {:?}", self.key),
        }
    }

    fn downcast(&self, value: &PrecomputedValue) -> Arc<T> {
        value.0.clone().downcast::<T>().unwrap_or_else(|_| {
            panic!(
                "ClassCastException: precomputed value {:?} is not a {}",
                self.key,
                std::any::type_name::<T>()
            )
        })
    }

    /// Injects a value into an evaluator being built.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:176 Precomputed.set
    pub fn set(&self, builder: EvaluatorBuilder, value: T) -> EvaluatorBuilder {
        builder.inject(self.key, PrecomputedValue::new(value))
    }

    /// Injects a value between evaluations.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:176 Precomputed.set
    pub fn set_on(&self, evaluator: &blaze_skyframe::Evaluator, value: T) {
        evaluator.inject(self.key, PrecomputedValue::new(value));
    }
}

impl<T> fmt::Debug for Precomputed<T> {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:181 Precomputed.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Precomputed{{key={:?}, shareable=true}}", self.key)
    }
}

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
