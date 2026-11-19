//! Port of `PrecomputedValue`: variables injected into Skyframe before an evaluation.
//!
//! Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java. Only the variables the
//! loading functions read are declared here; later crates declare their own `Precomputed<T>`
//! constants with the same mechanism.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use blaze_skyframe::{Environment, EvaluatorBuilder, Interrupted, Key, SkyFunctionName};
use blaze_starlark::StarlarkSemantics;

use crate::pkgcache::PathPackageLocator;
use crate::sky_functions;

/// A variable's value. Java: `PrecomputedValue` (the `Object value` it wraps).
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:44 PrecomputedValue
#[derive(Clone)]
pub struct PrecomputedValue {
    value: Arc<dyn Any + Send + Sync>,
}

impl PrecomputedValue {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:113 PrecomputedValue.<init>
    pub fn new<T: Send + Sync + 'static>(value: T) -> PrecomputedValue {
        PrecomputedValue {
            value: Arc::new(value),
        }
    }

    /// Returns the value of the variable.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:118 PrecomputedValue.get
    pub fn get(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.value
    }
}

impl fmt::Debug for PrecomputedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java prints "<BuildVariable " + value + ">"; the value is type-erased here.
        f.write_str("<BuildVariable>")
    }
}

/// `SkyKey` for a `PrecomputedValue`: the variable name. `Debug` prints `PRECOMPUTED:<name>`
/// (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:192 PrecomputedValue.Key
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrecomputedKey(pub &'static str);

impl fmt::Debug for PrecomputedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", sky_functions::PRECOMPUTED, self.0)
    }
}

impl Key for PrecomputedKey {
    type Value = PrecomputedValue;
    const FUNCTION: SkyFunctionName = sky_functions::PRECOMPUTED;
}

/// A helper object corresponding to a variable in Skyframe. Instances have no internal state.
///
/// PORT: Java's `shareable` flag (remote caching of values) is not represented.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:146 PrecomputedValue.Precomputed
pub struct Precomputed<T> {
    key: PrecomputedKey,
    _value: PhantomData<fn() -> T>,
}

impl<T: Send + Sync + 'static> Precomputed<T> {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:149 Precomputed.<init>
    pub const fn new(key: &'static str) -> Precomputed<T> {
        Precomputed {
            key: PrecomputedKey(key),
            _value: PhantomData,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:157 Precomputed.getKey
    pub fn get_key(&self) -> PrecomputedKey {
        self.key
    }

    /// Retrieves the value of this variable from Skyframe. If the value was not set, the
    /// evaluator fails (no PRECOMPUTED function is registered).
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:168 Precomputed.get
    pub fn get(&self, env: &mut Environment<'_>) -> Result<Option<Arc<T>>, Interrupted> {
        let Some(value) = env.get_value(self.key)? else {
            return Ok(None);
        };
        Ok(Some(value.value.clone().downcast::<T>().unwrap_or_else(
            |_| {
                panic!(
                    "ClassCastException: precomputed value {:?} has the wrong type",
                    self.key
                )
            },
        )))
    }

    /// Injects a new variable value into an evaluator being built.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:178 Precomputed.set
    pub fn set(&self, builder: EvaluatorBuilder, value: T) -> EvaluatorBuilder {
        builder.inject(self.key, PrecomputedValue::new(value))
    }

    /// Injects a new variable value between evaluations.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:178 Precomputed.set
    pub fn set_on(&self, evaluator: &blaze_skyframe::Evaluator, value: T) {
        evaluator.inject(self.key, PrecomputedValue::new(value));
    }
}

impl<T> fmt::Debug for Precomputed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Precomputed{{key={:?}, shareable=true}}", self.key)
    }
}

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
