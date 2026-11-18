//! The predeclared environments of every Starlark dialect, with and without builtins injection.
//!
//! Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java.

use std::collections::{HashMap, HashSet};

use blaze_starlark::eval::frozen_into;
use indexmap::IndexSet;
use starlark::values::OwnedFrozenValue;

use crate::builtins::builtins_internal_module::BuiltinsInternalModule;
use crate::builtins::env::{EnvEntry, StarlarkEnv, create_struct, freeze_value};
use crate::builtins::registered::RuleClassProviderRegistrations;
use crate::builtins::starlark_globals::StarlarkGlobals;

/// The names Starlark predeclares in every file (Java `Starlark.UNIVERSE`).
// Java: net/starlark/java/eval/Starlark.java:69 Starlark.UNIVERSE
pub const UNIVERSE: &[&str] = &[
    "False",
    "True",
    "None",
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "dir",
    "enumerate",
    "fail",
    "float",
    "getattr",
    "hasattr",
    "hash",
    "int",
    "len",
    "list",
    "max",
    "min",
    "print",
    "range",
    "repr",
    "reversed",
    "sorted",
    "str",
    "tuple",
    "type",
    "zip",
];

/// Indicates a problem performing builtins injection.
// Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:538 BazelStarlarkEnvironment.InjectionException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InjectionException(pub String);

/// The predeclared environments of every Starlark dialect. All of the environments exclude the
/// symbols in [`UNIVERSE`], which the interpreter adds automatically.
// Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:56 BazelStarlarkEnvironment
pub struct BazelStarlarkEnvironment {
    starlark_globals: StarlarkGlobals,
    rule_function_names: IndexSet<String>,
    registered_bzl_toplevel_names: IndexSet<String>,
    /// The top-level predeclared symbols, excluding `native`, for a .bzl file (regardless of who
    /// loads it), before injection.
    bzl_toplevels_without_native: StarlarkEnv,
    /// The `native` module fields for a BUILD-loaded bzl module, before builtins injection.
    uninjected_build_bzl_native_bindings: StarlarkEnv,
    /// The top-level predeclared symbols (including `native`) for a BUILD-loaded bzl module,
    /// before builtins injection.
    uninjected_build_bzl_env: StarlarkEnv,
    /// The top-level predeclared symbols for BUILD files, before builtins injection and prelude.
    uninjected_build_env: StarlarkEnv,
    /// The `native` module fields for a MODULE-loaded bzl module, before builtins injection.
    uninjected_module_bzl_native_bindings: StarlarkEnv,
    /// The top-level predeclared symbols for a MODULE-loaded bzl module, before injection.
    uninjected_module_bzl_env: StarlarkEnv,
    /// The top-level predeclared symbols for a bzl module in the `@_builtins` pseudo-repo.
    builtins_bzl_env: StarlarkEnv,
    /// Java `starlarkGlobals.getSclToplevels()`.
    scl_toplevels: StarlarkEnv,
}

impl BazelStarlarkEnvironment {
    /// `rule_functions` maps a native rule class name to the (uninjected) Starlark callable that
    /// instantiates it, in registration order.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:128 BazelStarlarkEnvironment.<init>
    pub fn new(
        starlark_globals: StarlarkGlobals,
        bazel_version: &str,
        rule_functions: StarlarkEnv,
        registrations: &RuleClassProviderRegistrations,
    ) -> BazelStarlarkEnvironment {
        let registered_build_file_toplevels = &registrations.registered_build_file_toplevels;
        let registered_bzl_toplevels = &registrations.registered_bzl_toplevels;
        let builtins_internals = &registrations.builtins_internals;

        let bzl_toplevels_without_native =
            Self::create_bzl_toplevels_without_native(&starlark_globals, registered_bzl_toplevels);
        // TODO(#11954): Use the same "native" object for both BUILD- and MODULE-loaded .bzls, and
        // just have it be a dynamic error to call the wrong thing at the wrong time. This is a
        // breaking change.
        let uninjected_build_bzl_native_bindings =
            Self::create_uninjected_build_bzl_native_bindings(
                &starlark_globals,
                &rule_functions,
                registered_build_file_toplevels,
            );
        let uninjected_module_bzl_native_bindings =
            Self::create_uninjected_module_bzl_native_bindings(
                &starlark_globals,
                &rule_functions,
                bazel_version,
            );
        let uninjected_build_bzl_env = Self::create_uninjected_bzl_env(
            &bzl_toplevels_without_native,
            &uninjected_build_bzl_native_bindings,
        );
        let uninjected_module_bzl_env = Self::create_uninjected_bzl_env(
            &bzl_toplevels_without_native,
            &uninjected_module_bzl_native_bindings,
        );
        let builtins_bzl_env = Self::create_builtins_bzl_env(
            &starlark_globals,
            builtins_internals,
            &uninjected_build_bzl_native_bindings,
            &uninjected_build_bzl_env,
        );
        let uninjected_build_env = Self::create_uninjected_build_env(
            &starlark_globals,
            &rule_functions,
            registered_build_file_toplevels,
        );
        let scl_toplevels = starlark_globals.get_scl_toplevels();
        BazelStarlarkEnvironment {
            rule_function_names: rule_functions.keys().cloned().collect(),
            registered_bzl_toplevel_names: registered_bzl_toplevels.keys().cloned().collect(),
            starlark_globals,
            bzl_toplevels_without_native,
            uninjected_build_bzl_native_bindings,
            uninjected_build_bzl_env,
            uninjected_build_env,
            uninjected_module_bzl_native_bindings,
            uninjected_module_bzl_env,
            builtins_bzl_env,
            scl_toplevels,
        }
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:172 BazelStarlarkEnvironment.getStarlarkGlobals
    pub fn get_starlark_globals(&self) -> &StarlarkGlobals {
        &self.starlark_globals
    }

    /// Java `getStarlarkGlobals().getSclToplevels()`.
    pub fn get_scl_toplevels(&self) -> &StarlarkEnv {
        &self.scl_toplevels
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:180 BazelStarlarkEnvironment.getUninjectedBuildBzlNativeBindings
    pub fn get_uninjected_build_bzl_native_bindings(&self) -> &StarlarkEnv {
        &self.uninjected_build_bzl_native_bindings
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:188 BazelStarlarkEnvironment.getUninjectedModuleBzlNativeBindings
    pub fn get_uninjected_module_bzl_native_bindings(&self) -> &StarlarkEnv {
        &self.uninjected_module_bzl_native_bindings
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:199 BazelStarlarkEnvironment.getUninjectedBuildBzlEnv
    pub fn get_uninjected_build_bzl_env(&self) -> &StarlarkEnv {
        &self.uninjected_build_bzl_env
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:211 BazelStarlarkEnvironment.getUninjectedBuildEnv
    pub fn get_uninjected_build_env(&self) -> &StarlarkEnv {
        &self.uninjected_build_env
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:222 BazelStarlarkEnvironment.getUninjectedModuleBzlEnv
    pub fn get_uninjected_module_bzl_env(&self) -> &StarlarkEnv {
        &self.uninjected_module_bzl_env
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:230 BazelStarlarkEnvironment.getBuiltinsBzlEnv
    pub fn get_builtins_bzl_env(&self) -> &StarlarkEnv {
        &self.builtins_bzl_env
    }

    /// The names of the native rule functions (Java `ruleFunctions.keySet()`).
    pub fn rule_function_names(&self) -> &IndexSet<String> {
        &self.rule_function_names
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:244 BazelStarlarkEnvironment.createBzlToplevelsWithoutNative
    fn create_bzl_toplevels_without_native(
        starlark_globals: &StarlarkGlobals,
        registered_bzl_toplevels: &StarlarkEnv,
    ) -> StarlarkEnv {
        let mut env = starlark_globals.get_fixed_bzl_toplevels();
        for (k, v) in registered_bzl_toplevels {
            env.insert(k.clone(), v.clone());
        }
        env
    }

    /// Produces everything that would be in the `native` object for BUILD-loaded bzl files if
    /// builtins injection didn't happen.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:256 BazelStarlarkEnvironment.createUninjectedBuildBzlNativeBindings
    fn create_uninjected_build_bzl_native_bindings(
        starlark_globals: &StarlarkGlobals,
        rule_functions: &StarlarkEnv,
        registered_build_file_toplevels: &StarlarkEnv,
    ) -> StarlarkEnv {
        let mut env = starlark_globals.get_fixed_build_file_toplevels_shared_with_native();
        for (k, v) in rule_functions.iter().chain(registered_build_file_toplevels) {
            env.insert(k.clone(), v.clone());
        }
        env
    }

    /// Produce everything that would be in the `native` object for MODULE-loaded bzl files if
    /// builtins injection didn't happen.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:271 BazelStarlarkEnvironment.createUninjectedModuleBzlNativeBindings
    fn create_uninjected_module_bzl_native_bindings(
        starlark_globals: &StarlarkGlobals,
        rule_functions: &StarlarkEnv,
        bazel_version: &str,
    ) -> StarlarkEnv {
        let mut env = starlark_globals.get_fixed_build_file_toplevels_shared_with_native();
        for (k, v) in rule_functions {
            env.insert(k.clone(), v.clone());
        }
        let version = bazel_version.to_owned();
        env.insert(
            "bazel_version".to_owned(),
            EnvEntry::plain(freeze_value(move |m| m.heap().alloc(version.as_str()))),
        );
        env
    }

    /// Constructs a "native" module object with the given contents.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:281 BazelStarlarkEnvironment.createNativeModule
    fn create_native_module(bindings: &StarlarkEnv) -> OwnedFrozenValue {
        create_struct(bindings)
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:285 BazelStarlarkEnvironment.createUninjectedBzlEnv
    fn create_uninjected_bzl_env(
        bzl_toplevels_without_native: &StarlarkEnv,
        uninjected_bzl_native_bindings: &StarlarkEnv,
    ) -> StarlarkEnv {
        let mut env = bzl_toplevels_without_native.clone();
        // Determine the "native" module.
        env.insert(
            "native".to_owned(),
            EnvEntry::plain(Self::create_native_module(uninjected_bzl_native_bindings)),
        );
        env
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:297 BazelStarlarkEnvironment.createUninjectedBuildEnv
    fn create_uninjected_build_env(
        starlark_globals: &StarlarkGlobals,
        rule_functions: &StarlarkEnv,
        registered_build_file_toplevels: &StarlarkEnv,
    ) -> StarlarkEnv {
        let mut env = starlark_globals.get_fixed_build_file_toplevels_shared_with_native();
        for (k, v) in starlark_globals
            .get_fixed_build_file_toplevels_not_in_native()
            .iter()
            .chain(rule_functions)
            .chain(registered_build_file_toplevels)
        {
            env.insert(k.clone(), v.clone());
        }
        env
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:309 BazelStarlarkEnvironment.createBuiltinsBzlEnv
    fn create_builtins_bzl_env(
        starlark_globals: &StarlarkGlobals,
        builtins_internals: &StarlarkEnv,
        uninjected_build_bzl_native_bindings: &StarlarkEnv,
        uninjected_build_bzl_env: &StarlarkEnv,
    ) -> StarlarkEnv {
        let mut env = starlark_globals.get_fixed_bzl_toplevels();

        // For _builtins.toplevel, replace all GuardedValues with the underlying value;
        // StarlarkSemantics flags do not affect @_builtins.
        //
        // We do this because otherwise we'd need to differentiate the _builtins.toplevel object
        // (and therefore the @_builtins environment) based on StarlarkSemantics. That seems
        // unnecessary. Instead we trust @_builtins to not misuse flag-guarded features, same as
        // native code.
        let unwrapped_build_bzl_symbols: StarlarkEnv = uninjected_build_bzl_env
            .iter()
            .map(|(k, e)| (k.clone(), EnvEntry::plain(e.value.clone())))
            .collect();

        // createNativeModule() is good enough for the "toplevel" and "internal" objects too.
        let native = Self::create_native_module(uninjected_build_bzl_native_bindings);
        let toplevel = Self::create_native_module(&unwrapped_build_bzl_symbols);
        let internal = Self::create_native_module(builtins_internals);
        let builtins_module = freeze_value(|m| {
            let n = frozen_into(m, &native);
            let t = frozen_into(m, &toplevel);
            let i = frozen_into(m, &internal);
            m.heap().alloc(BuiltinsInternalModule::new(
                n.unpack_frozen().expect("frozen"),
                t.unpack_frozen().expect("frozen"),
                i.unpack_frozen().expect("frozen"),
            ))
        });
        let conflicting_value =
            env.insert("_builtins".to_owned(), EnvEntry::plain(builtins_module));
        blaze_util::check_state!(
            conflicting_value.is_none(),
            "'_builtins' name is reserved for builtins injection"
        );
        env
    }

    /// Throws [`InjectionException`] with an appropriate error message if the given `symbol` is
    /// not in both `existing_symbols` and `injectable_symbols`. `kind` is a string describing the
    /// domain of `symbol`.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:346 BazelStarlarkEnvironment.validateSymbolIsInjectable
    fn validate_symbol_is_injectable(
        symbol: &str,
        existing_symbols: &dyn Fn(&str) -> bool,
        injectable_symbols: &IndexSet<String>,
        kind: &str,
    ) -> Result<(), InjectionException> {
        if !existing_symbols(symbol) {
            return Err(InjectionException(format!(
                "Injected {} '{}' must override an existing one by that name",
                kind, symbol
            )));
        } else if !injectable_symbols.contains(symbol) {
            return Err(InjectionException(format!(
                "Cannot override '{}' with an injected {}",
                symbol, kind
            )));
        }
        Ok(())
    }

    /// Given a string prefixed with + or -, returns that prefix character, or `None` otherwise.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:362 BazelStarlarkEnvironment.getKeyPrefix
    fn get_key_prefix(key: &str) -> Option<char> {
        let prefix = key.chars().next()?;
        if !(prefix == '+' || prefix == '-') {
            return None;
        }
        Some(prefix)
    }

    /// Given a string prefixed with + or -, returns the remainder of the string, or the whole
    /// string otherwise.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:378 BazelStarlarkEnvironment.getKeySuffix
    fn get_key_suffix(key: &str) -> &str {
        if Self::get_key_prefix(key).is_none() {
            key
        } else {
            &key[1..]
        }
    }

    /// Given the +/- prefixed items of `--experimental_builtins_injection_override`, returns a
    /// map from each item to whether it last appeared with the + prefix.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:389 BazelStarlarkEnvironment.parseInjectionOverridesList
    fn parse_injection_overrides_list(
        overrides: &[String],
    ) -> Result<HashMap<String, bool>, InjectionException> {
        let mut result = HashMap::new();
        for prefixed_item in overrides {
            let Some(prefix) = Self::get_key_prefix(prefixed_item) else {
                return Err(InjectionException(format!(
                    "Invalid injection override item: '{}'",
                    prefixed_item
                )));
            };
            result.insert(prefixed_item[1..].to_owned(), prefix == '+');
        }
        Ok(result)
    }

    /// Given an exports dict key, and an override map, return whether injection should be applied
    /// for that key.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:409 BazelStarlarkEnvironment.injectionApplies
    fn injection_applies(key: &str, overrides: &HashMap<String, bool>) -> bool {
        let Some(prefix) = Self::get_key_prefix(key) else {
            // Unprefixed; overrides don't get a say in the matter.
            return true;
        };
        match overrides.get(&key[1..]) {
            None => prefix == '+',
            Some(o) => *o,
        }
    }

    /// Constructs an environment for a BUILD-loaded bzl file based on the default environment, the
    /// maps corresponding to the `exported_toplevels` and `exported_rules` dicts, and the value of
    /// `--experimental_builtins_injection_override`.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:442 BazelStarlarkEnvironment.createBuildBzlEnvUsingInjection
    pub fn create_build_bzl_env_using_injection(
        &self,
        exported_toplevels: &StarlarkEnv,
        exported_rules: &StarlarkEnv,
        overrides_list: &[String],
    ) -> Result<StarlarkEnv, InjectionException> {
        self.create_bzl_env_using_injection(
            exported_toplevels,
            exported_rules,
            overrides_list,
            &self.uninjected_build_bzl_native_bindings,
        )
    }

    /// Constructs an environment for a MODULE-loaded bzl file (see
    /// [`BazelStarlarkEnvironment::create_build_bzl_env_using_injection`]).
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:459 BazelStarlarkEnvironment.createModuleBzlEnvUsingInjection
    pub fn create_module_bzl_env_using_injection(
        &self,
        exported_toplevels: &StarlarkEnv,
        exported_rules: &StarlarkEnv,
        overrides_list: &[String],
    ) -> Result<StarlarkEnv, InjectionException> {
        self.create_bzl_env_using_injection(
            exported_toplevels,
            exported_rules,
            overrides_list,
            &self.uninjected_module_bzl_native_bindings,
        )
    }

    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:470 BazelStarlarkEnvironment.createBzlEnvUsingInjection
    fn create_bzl_env_using_injection(
        &self,
        exported_toplevels: &StarlarkEnv,
        exported_rules: &StarlarkEnv,
        overrides_list: &[String],
        native_base: &StarlarkEnv,
    ) -> Result<StarlarkEnv, InjectionException> {
        let overrides_map = Self::parse_injection_overrides_list(overrides_list)?;

        let mut env = self.bzl_toplevels_without_native.clone();

        // Determine "native" bindings.
        let mut native_bindings = native_base.clone();
        for (key, value) in exported_rules {
            let name = Self::get_key_suffix(key);
            let existing: HashSet<String> = native_bindings.keys().cloned().collect();
            Self::validate_symbol_is_injectable(
                name,
                &|s| existing.contains(s),
                &self.rule_function_names,
                "rule",
            )?;
            if Self::injection_applies(key, &overrides_map) {
                native_bindings.insert(name.to_owned(), EnvEntry::plain(value.value.clone()));
            }
        }
        env.insert(
            "native".to_owned(),
            EnvEntry::plain(Self::create_native_module(&native_bindings)),
        );

        // Determine top-level symbols.
        for (key, value) in exported_toplevels {
            let name = Self::get_key_suffix(key);
            let existing: HashSet<String> = env.keys().cloned().collect();
            Self::validate_symbol_is_injectable(
                name,
                &|s| existing.contains(s) || UNIVERSE.contains(&s),
                &self.registered_bzl_toplevel_names,
                "top-level symbol",
            )?;
            if Self::injection_applies(key, &overrides_map) {
                env.insert(name.to_owned(), EnvEntry::plain(value.value.clone()));
            }
        }

        Ok(env)
    }

    /// Constructs an environment for a BUILD file based on the default environment, the map
    /// corresponding to the `exported_rules` dict, and the value of
    /// `--experimental_builtins_injection_override`.
    // Java: com/google/devtools/build/lib/packages/BazelStarlarkEnvironment.java:520 BazelStarlarkEnvironment.createBuildEnvUsingInjection
    pub fn create_build_env_using_injection(
        &self,
        exported_rules: &StarlarkEnv,
        overrides_list: &[String],
    ) -> Result<StarlarkEnv, InjectionException> {
        let overrides_map = Self::parse_injection_overrides_list(overrides_list)?;

        let mut env = self.uninjected_build_env.clone();
        for (key, value) in exported_rules {
            let name = Self::get_key_suffix(key);
            let existing: HashSet<String> = env.keys().cloned().collect();
            Self::validate_symbol_is_injectable(
                name,
                &|s| existing.contains(s) || UNIVERSE.contains(&s),
                &self.rule_function_names,
                "rule",
            )?;
            if Self::injection_applies(key, &overrides_map) {
                env.insert(name.to_owned(), EnvEntry::plain(value.value.clone()));
            }
        }
        Ok(env)
    }
}
