//! STARLARK_BUILTINS: evaluating the `@_builtins` pseudo-repository and applying its exports
//! (builtins injection).
//!
//! Java: com/google/devtools/build/lib/skyframe/{StarlarkBuiltinsFunction,
//! StarlarkBuiltinsValue}.java.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::builtins::BazelStarlarkEnvironment;
use blaze_packages::builtins::env::{EnvEntry, GlobalsCache, StarlarkEnv};
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, SkyKey, Transience,
};
use blaze_starlark::StarlarkSemantics;
use starlark::environment::{FrozenModule, Globals};
use starlark::values::OwnedFrozenValue;
use starlark::values::dict::DictRef;

use crate::bzl_load::{BzlLoadFailedException, BzlLoadKey, BzlLoadValue};
use crate::precomputed::{AUTOLOAD_SYMBOLS, STARLARK_SEMANTICS};
use crate::repos::ExternalRepos;
use crate::sky_functions;
use blaze_packages::builtins::autoload_symbols::AutoloadException;
use blaze_util::Fingerprint;
use indexmap::IndexMap;

/// Reports whether the given repository is the special builtins pseudo-repository.
pub use crate::repos::is_builtins_repo;

/// The values exported by `@_builtins//:exports.bzl` and the environments they determine.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:34 StarlarkBuiltinsValue
pub struct StarlarkBuiltinsValue {
    /// Top-level predeclared symbols for a .bzl file loaded on behalf of a BUILD file, after
    /// builtins injection has been applied.
    pub predeclared_for_build_bzl: StarlarkEnv,
    /// Top-level predeclared symbols for a .bzl file loaded on behalf of a MODULE file after
    /// builtins injection has been applied.
    pub predeclared_for_module_bzl: StarlarkEnv,
    /// Top-level predeclared symbols for a BUILD file, after builtins injection but before any
    /// prelude file has been applied.
    pub predeclared_for_build: StarlarkEnv,
    /// Contents of the `exported_to_java` dict.
    pub exported_to_java: StarlarkEnv,
    /// Transitive digest of all .bzl files in `@_builtins`.
    pub transitive_digest: Vec<u8>,
    /// The StarlarkSemantics used for `@_builtins` evaluation.
    pub starlark_semantics: Arc<StarlarkSemantics>,
    /// `Globals` built from the environments above (Blaze only).
    pub globals_cache: GlobalsCache,
}

impl fmt::Debug for StarlarkBuiltinsValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StarlarkBuiltinsValue{{transitiveDigest={}}}",
            hex::encode(&self.transitive_digest)
        )
    }
}

impl StarlarkBuiltinsValue {
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:89 StarlarkBuiltinsValue.create
    pub fn create(
        predeclared_for_build_bzl: StarlarkEnv,
        predeclared_for_module_bzl: StarlarkEnv,
        predeclared_for_build: StarlarkEnv,
        exported_to_java: StarlarkEnv,
        transitive_digest: Vec<u8>,
        starlark_semantics: Arc<StarlarkSemantics>,
    ) -> StarlarkBuiltinsValue {
        StarlarkBuiltinsValue {
            predeclared_for_build_bzl,
            predeclared_for_module_bzl,
            predeclared_for_build,
            exported_to_java,
            transitive_digest,
            starlark_semantics,
            globals_cache: GlobalsCache::default(),
        }
    }

    /// Constructs a placeholder builtins value to be used when builtins injection is disabled, or
    /// for use within builtins evaluation itself. The placeholder simply wraps the
    /// StarlarkSemantics object.
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:113 StarlarkBuiltinsValue.createEmpty
    pub fn create_empty(starlark_semantics: Arc<StarlarkSemantics>) -> StarlarkBuiltinsValue {
        Self::create(
            StarlarkEnv::new(),
            StarlarkEnv::new(),
            StarlarkEnv::new(),
            StarlarkEnv::new(),
            Vec::new(),
            starlark_semantics,
        )
    }

    /// The `Globals` of `env` (one of this value's environments) for a file with `label`,
    /// filtered by the guards of `guards`.
    pub fn globals(
        &self,
        env: &StarlarkEnv,
        guards: Option<&StarlarkEnv>,
        label: Option<&Label>,
    ) -> Arc<Globals> {
        self.globals_cache
            .get(env, guards, &self.starlark_semantics, label)
    }
}

/// Skyframe key for retrieving the `@_builtins` definitions. There is only one value at a time
/// (two with autoloads). `Debug` prints `Starlark @_builtins`.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:137 StarlarkBuiltinsValue.Key
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StarlarkBuiltinsKey {
    pub with_autoloads: bool,
}

impl StarlarkBuiltinsKey {
    /// Returns the SkyKey for BuiltinsValue optionally amended with externally loaded symbols
    /// and rules.
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:129 StarlarkBuiltinsValue.key
    pub fn key(with_autoloads: bool) -> Self {
        StarlarkBuiltinsKey { with_autoloads }
    }
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsValue.java:159 StarlarkBuiltinsValue.Key.toString
impl fmt::Debug for StarlarkBuiltinsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Starlark @_builtins")
    }
}

impl Key for StarlarkBuiltinsKey {
    type Value = StarlarkBuiltinsValue;
    const FUNCTION: SkyFunctionName = sky_functions::STARLARK_BUILTINS;
}

/// An exception that occurs while trying to determine the injected builtins. This exception type
/// typically wraps a [`BzlLoadFailedException`] and is wrapped by one in turn.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:311 StarlarkBuiltinsFunction.BuiltinsFailedException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct BuiltinsFailedException {
    message: String,
    transience: Transience,
}

impl BuiltinsFailedException {
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:320 StarlarkBuiltinsFunction.BuiltinsFailedException.getTransience
    pub fn get_transience(&self) -> Transience {
        self.transience
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:324 StarlarkBuiltinsFunction.BuiltinsFailedException.errorEvaluatingBuiltinsBzls
    fn error_evaluating_builtins_bzls(cause: &BzlLoadFailedException) -> Self {
        BuiltinsFailedException {
            message: format!("Failed to load builtins sources: {}", cause.get_message()),
            transience: cause.get_transience(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:336 StarlarkBuiltinsFunction.BuiltinsFailedException.errorEvaluatingAutoloadedBzls
    fn error_evaluating_autoloaded_bzls(
        cause: &BzlLoadFailedException,
        bzlmod_enabled: bool,
    ) -> Self {
        let additional_message = if bzlmod_enabled {
            ""
        } else {
            " Most likely you need to upgrade the version of rules repository in the WORKSPACE \
             file."
        };
        BuiltinsFailedException {
            message: format!(
                "Failed to autoload external symbols: {}{}",
                cause.get_message(),
                additional_message
            ),
            transience: cause.get_transience(),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:350 StarlarkBuiltinsFunction.BuiltinsFailedException.errorApplyingExports
    fn error_applying_exports(cause: &dyn fmt::Display) -> Self {
        BuiltinsFailedException {
            message: format!("Failed to apply declared builtins: {}", cause),
            transience: Transience::Persistent,
        }
    }
}

/// The two failures Java catches around applying the exports (`EvalException |
/// InjectionException`, and `AutoloadException`).
enum ApplyError {
    Exports(String),
    Autoload(AutoloadException),
}

/// The label where `@_builtins` symbols are exported from.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:89 StarlarkBuiltinsFunction.EXPORTS_ENTRYPOINT
pub fn exports_entrypoint() -> Label {
    Label::parse_canonical_unchecked("@_builtins//:exports.bzl")
}

/// Key for loading exports.bzl. `key_for_builtins` (as opposed to `key_for_build`) ensures we
/// can resolve `@_builtins`, which is otherwise inaccessible. It also prevents us from
/// cyclically requesting StarlarkBuiltinsFunction again to evaluate exports.bzl.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:98 StarlarkBuiltinsFunction.EXPORTS_ENTRYPOINT_KEY
pub fn exports_entrypoint_key() -> BzlLoadKey {
    BzlLoadKey::key_for_builtins(exports_entrypoint())
}

/// Attempts to retrieve the string-keyed dict named `dict_name` from the given module.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:292 StarlarkBuiltinsFunction.getDict
fn get_dict(module: &FrozenModule, dict_name: &str) -> Result<StarlarkEnv, String> {
    let value: OwnedFrozenValue = match module.get_option(dict_name) {
        Ok(Some(v)) => v,
        _ => {
            return Err(format!(
                "expected a '{}' dictionary to be defined",
                dict_name
            ));
        }
    };
    let what = format!("{} dict", dict_name);
    let Some(dict) = DictRef::from_value(value.value()) else {
        // Java: Dict.cast -> "got %s for '%s', want dict"
        return Err(format!(
            "got {} for '{}', want dict",
            value.value().get_type(),
            what
        ));
    };
    let mut out = StarlarkEnv::new();
    for (k, v) in dict.iter() {
        let Some(name) = k.unpack_str() else {
            return Err(format!(
                "got dict<{}, {}> for '{}', want dict<string, any>",
                k.get_type(),
                v.get_type(),
                what
            ));
        };
        let fv = v
            .unpack_frozen()
            .expect("values of a frozen module are frozen");
        // SAFETY: `fv` lives in the module's heap, which `value` (from the same heap) owns.
        let owned = unsafe { OwnedFrozenValue::new(value.owner().clone(), fv) };
        out.insert(name.to_owned(), EnvEntry::plain(owned));
    }
    Ok(out)
}

/// A Skyframe function that evaluates the `@_builtins` pseudo-repository and reports the values
/// exported by `exports.bzl`. The `@_builtins` pseudo-repository shares a repo mapping with the
/// `@bazel_tools` repository.
///
/// PORT: Java's inlining mode (used only with bzl inlining of PACKAGE) is not ported; Bazel's
/// default Skyframe evaluation path is.
// Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:82 StarlarkBuiltinsFunction
pub struct StarlarkBuiltinsFunction {
    bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
    /// PORT: the Bzlmod values autoloads read (`BazelDepGraphValue`, the root module file)
    /// are requested through the loading seam.
    external_repos: Arc<dyn ExternalRepos>,
}

impl StarlarkBuiltinsFunction {
    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:103 StarlarkBuiltinsFunction.<init>
    pub fn new(
        bazel_starlark_environment: Arc<BazelStarlarkEnvironment>,
        external_repos: Arc<dyn ExternalRepos>,
    ) -> Self {
        StarlarkBuiltinsFunction {
            bazel_starlark_environment,
            external_repos,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:107 StarlarkBuiltinsFunction.compute
    pub fn compute(
        &self,
        key: &StarlarkBuiltinsKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<StarlarkBuiltinsValue>, ComputeError> {
        match self.compute_internal(env, key.with_autoloads)? {
            Ok(v) => Ok(v),
            // Java: StarlarkBuiltinsFunctionException(BuiltinsFailedException)
            Err(e) => {
                let t = e.get_transience();
                Err(SkyFunctionException::new(e, t).into())
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/StarlarkBuiltinsFunction.java:163 StarlarkBuiltinsFunction.computeInternal
    fn compute_internal(
        &self,
        env: &mut Environment<'_>,
        mut is_with_autoloads: bool,
    ) -> Result<Result<Option<StarlarkBuiltinsValue>, BuiltinsFailedException>, ComputeError> {
        let Some(starlark_semantics) = STARLARK_SEMANTICS.get(env)? else {
            return Ok(Ok(None));
        };
        // Return the empty value if builtins injection is disabled.
        if starlark_semantics.experimental_builtins_bzl_path.is_empty() {
            return Ok(Ok(Some(StarlarkBuiltinsValue::create_empty(
                starlark_semantics,
            ))));
        }
        let Some(autoload_symbols) = AUTOLOAD_SYMBOLS.get(env)? else {
            return Ok(Ok(None));
        };
        if autoload_symbols.is_enabled() {
            // We can't do autoloads where the rules are implemented (disabling them when running
            // in main repository named rules_python)
            let Some(root_module_name) = self.external_repos.get_root_module_name(env)? else {
                return Ok(Ok(None));
            };
            if autoload_symbols.autoloads_disabled_for_repo(&root_module_name) {
                is_with_autoloads = false;
            }
        }

        // Load exports.bzl.
        let exports_value = match env
            .get_value_or_throw::<BzlLoadKey, BzlLoadFailedException>(exports_entrypoint_key())?
        {
            Ok(v) => v,
            Err(ex) => {
                return Ok(Err(
                    BuiltinsFailedException::error_evaluating_builtins_bzls(&ex),
                ));
            }
        };

        // If requested also loads "autoloads": rules and providers from external repositories.
        let auto_bzl_load_keys: IndexMap<String, BzlLoadKey> = if is_with_autoloads {
            // Java: AutoloadSymbols.getLoadKeys(env)
            let Some(bazel_dep_graph_value) = self.external_repos.get_bazel_dep_graph_value(env)?
            else {
                return Ok(Ok(None));
            };
            autoload_symbols
                .get_load_keys(&bazel_dep_graph_value)
                .into_iter()
                .map(|(symbol, label)| (symbol, BzlLoadKey::key_for_build(label)))
                .collect()
        } else {
            IndexMap::new()
        };
        let sky_keys: Vec<SkyKey> = auto_bzl_load_keys
            .values()
            .cloned()
            .map(SkyKey::new)
            .collect();
        let values = env.get_values_and_exceptions(&sky_keys)?;
        let mut auto_bzl_load_values: IndexMap<String, Arc<BzlLoadValue>> = IndexMap::new();
        for (symbol, key) in &auto_bzl_load_keys {
            match values.get_or_throw_typed::<BzlLoadKey, BzlLoadFailedException>(key) {
                Ok(Some(value)) => {
                    auto_bzl_load_values.insert(symbol.clone(), value);
                }
                Ok(None) => {}
                Err(ex) => {
                    return Ok(Err(
                        BuiltinsFailedException::error_evaluating_autoloaded_bzls(
                            &ex, /* bzlmod_enabled= */ true,
                        ),
                    ));
                }
            }
        }
        if env.values_missing() {
            return Ok(Ok(None));
        }
        let exports_value = exports_value.expect("not missing");

        // Compute digest of exports.bzl and possibly externally loaded symbols (Bazel)
        let mut transitive_digest = exports_value.get_transitive_digest().to_vec();
        if !auto_bzl_load_values.is_empty() {
            let mut fp = Fingerprint::new();
            fp.add_bytes(&transitive_digest);
            for value in auto_bzl_load_values.values() {
                fp.add_bytes(value.get_transitive_digest());
            }
            transitive_digest = fp.digest_and_reset();
        }

        // Apply declarations of exports.bzl to the native predeclared symbols.
        let module = exports_value.get_module();
        let result = (|| -> Result<StarlarkBuiltinsValue, ApplyError> {
            let exported_toplevels =
                get_dict(module, "exported_toplevels").map_err(ApplyError::Exports)?;
            let exported_rules = get_dict(module, "exported_rules").map_err(ApplyError::Exports)?;
            let exported_to_java =
                get_dict(module, "exported_to_java").map_err(ApplyError::Exports)?;
            let overrides = starlark_semantics
                .experimental_builtins_injection_override
                .clone()
                .unwrap_or_default();
            let env_ = &self.bazel_starlark_environment;
            let predeclared_for_build_bzl = env_
                .create_build_bzl_env_using_injection(
                    &exported_toplevels,
                    &exported_rules,
                    &overrides,
                )
                .map_err(|e| ApplyError::Exports(e.to_string()))?;
            let predeclared_for_module_bzl = env_
                .create_module_bzl_env_using_injection(
                    &exported_toplevels,
                    &exported_rules,
                    &overrides,
                )
                .map_err(|e| ApplyError::Exports(e.to_string()))?;
            let predeclared_for_build = env_
                .create_build_env_using_injection(&exported_rules, &overrides)
                .map_err(|e| ApplyError::Exports(e.to_string()))?;

            // Apply declarations of externally loaded symbols to the native predeclared symbols.
            // Java: v.getModule().getGlobal(name), per autoloaded symbol.
            let lookups: Vec<(String, Box<dyn Fn(&str) -> Option<OwnedFrozenValue> + '_>)> =
                auto_bzl_load_values
                    .iter()
                    .map(|(symbol, value)| {
                        let module = value.get_module();
                        let get_global: Box<dyn Fn(&str) -> Option<OwnedFrozenValue> + '_> =
                            Box::new(move |name| module.get_option(name).ok().flatten());
                        (symbol.clone(), get_global)
                    })
                    .collect();
            let autoload_values: IndexMap<String, &dyn Fn(&str) -> Option<OwnedFrozenValue>> =
                lookups
                    .iter()
                    .map(|(symbol, f)| (symbol.clone(), f.as_ref()))
                    .collect();
            let new_symbols = autoload_symbols
                .process_loads(&autoload_values)
                .map_err(ApplyError::Autoload)?;
            let predeclared_for_build = autoload_symbols.modify_build_env(
                is_with_autoloads,
                &predeclared_for_build,
                &new_symbols,
            );
            let predeclared_for_build_bzl = autoload_symbols.modify_build_bzl_env(
                is_with_autoloads,
                &predeclared_for_build_bzl,
                &new_symbols,
            );

            Ok(StarlarkBuiltinsValue::create(
                predeclared_for_build_bzl,
                predeclared_for_module_bzl,
                predeclared_for_build,
                exported_to_java,
                transitive_digest,
                starlark_semantics.clone(),
            ))
        })();
        match result {
            Ok(v) => Ok(Ok(Some(v))),
            Err(ApplyError::Exports(e)) => {
                Ok(Err(BuiltinsFailedException::error_applying_exports(&e)))
            }
            Err(ApplyError::Autoload(ex)) => Ok(Err(BuiltinsFailedException {
                message: format!("Failed to apply symbols loaded externally: {}", ex.0),
                transience: Transience::Persistent,
            })),
        }
    }
}
