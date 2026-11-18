//! Predeclared environments: Java's `ImmutableMap<String, Object>` of top-level symbols, as
//! frozen Starlark values, with Java's `GuardedValue`s.
//!
//! Java: net/starlark/java/eval/GuardedValue.java, FlagGuardedValue.java,
//! com/google/devtools/build/lib/starlarkbuildapi/core/{ContextGuardedValue,
//! ContextAndFlagGuardedValue}.java, analysis/BaseRuleClasses.java `createEmptySymbol`, and the
//! `StructProvider.STRUCT.create(...)` calls that build the `native` object.
//!
//! PORT: Java resolves a .bzl file against the predeclared map and reports an inaccessible
//! guarded symbol with `getErrorFromAttemptingAccess`. starlark-rust resolves against a `Globals`,
//! so [`build_globals`] leaves inaccessible symbols out (the error then is starlark-rust's
//! "not found" text).

use std::collections::HashMap;
use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::structs::{StarlarkInfo, StructProvider};
use indexmap::IndexMap;
use parking_lot::Mutex;
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::values::{OwnedFrozenValue, Value, ValueLike};

/// A predicate on a boolean `--flag` of the semantics.
pub type FlagGetter = fn(&StarlarkSemantics) -> bool;

/// Java `GuardedValue`: a predeclared symbol accessible only under some semantics or from some
/// repositories.
#[derive(Clone)]
pub enum Guard {
    /// `BaseRuleClasses.createEmptySymbol(name, bzlLoadLabel)`: never accessible.
    // Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:502 BaseRuleClasses.createEmptySymbol
    Removed { bzl_load_label: &'static str },
    /// `FlagGuardedValue.onlyWhenExperimentalFlagIsTrue(flag, obj)`.
    // Java: net/starlark/java/eval/FlagGuardedValue.java:26 FlagGuardedValue.onlyWhenExperimentalFlagIsTrue
    WhenExperimentalFlagIsTrue { flag: &'static str, get: FlagGetter },
    /// `FlagGuardedValue.onlyWhenIncompatibleFlagIsFalse(flag, obj)`.
    // Java: net/starlark/java/eval/FlagGuardedValue.java:60 FlagGuardedValue.onlyWhenIncompatibleFlagIsFalse
    WhenIncompatibleFlagIsFalse { flag: &'static str, get: FlagGetter },
    /// `ContextGuardedValue.onlyInAllowedRepos(obj, allowedEntries)`.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/core/ContextGuardedValue.java:35 ContextGuardedValue.onlyInAllowedRepos
    InAllowedRepos {
        allowed_entries: Vec<PackageIdentifier>,
    },
    /// `ContextAndFlagGuardedValue.onlyInAllowedReposOrWhenIncompatibleFlagIsFalse`.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/core/ContextAndFlagGuardedValue.java:35 ContextAndFlagGuardedValue.onlyInAllowedReposOrWhenIncompatibleFlagIsFalse
    InAllowedReposOrWhenIncompatibleFlagIsFalse {
        flag: &'static str,
        get: FlagGetter,
        allowed_entries: Vec<PackageIdentifier>,
    },
}

/// Java `String.matches(".*\\b" + Pattern.quote(name) + "\\b.*")`.
fn contains_word(haystack: &str, name: &str) -> bool {
    fn is_word(c: Option<char>) -> bool {
        c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    }
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next();
    let last = name.chars().last();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(name) {
        let i = start + pos;
        let before = haystack[..i].chars().last();
        let after = haystack[i + name.len()..].chars().next();
        if is_word(before) != is_word(first) && is_word(after) != is_word(last) {
            return true;
        }
        start = i + haystack[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
    false
}

impl Guard {
    /// `isObjectAccessibleUsingSemantics(semantics, clientData)`, where the client data is the
    /// `BazelCompileContext` of the file being compiled (`compile_label`; `None` when there is no
    /// such context).
    pub fn is_object_accessible_using_semantics(
        &self,
        semantics: &StarlarkSemantics,
        compile_label: Option<&Label>,
    ) -> bool {
        match self {
            Guard::Removed { .. } => false,
            Guard::WhenExperimentalFlagIsTrue { get, .. } => get(semantics),
            Guard::WhenIncompatibleFlagIsFalse { get, .. } => !get(semantics),
            Guard::InAllowedRepos { allowed_entries } => {
                Self::in_allowed_repos(allowed_entries, compile_label)
            }
            Guard::InAllowedReposOrWhenIncompatibleFlagIsFalse {
                get,
                allowed_entries,
                ..
            } => !get(semantics) || Self::in_allowed_repos(allowed_entries, compile_label),
        }
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/core/ContextGuardedValue.java:41 ContextGuardedValue.isObjectAccessibleUsingSemantics
    fn in_allowed_repos(allowed_entries: &[PackageIdentifier], label: Option<&Label>) -> bool {
        // Filtering of predeclareds is only done at compile time, when the client data is
        // BazelCompileContext and not BazelModuleContext.
        let Some(label) = label else {
            return false;
        };
        for entry in allowed_entries {
            let repo_name = label.get_repository().get_name();
            let repo_matches = if entry.get_repository().get_name().is_empty() {
                // String.matches has ^$ implicitly, so an empty pattern matches exactly the empty
                // string.
                repo_name.is_empty()
            } else {
                contains_word(repo_name, entry.get_repository().get_name())
            };
            if repo_matches
                && label
                    .get_package_fragment()
                    .starts_with(entry.get_package_fragment())
            {
                return true;
            }
        }
        false
    }

    /// Java `getErrorFromAttemptingAccess(name)`.
    pub fn get_error_from_attempting_access(&self, name: &str) -> String {
        let entries = |e: &[PackageIdentifier]| {
            e.iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Guard::Removed { bzl_load_label } => format!(
                "The {} symbol has been removed, add the following to your BUILD/bzl file:\n\n\
                 load(\"{}\", \"{}\")\n",
                name, bzl_load_label, name
            ),
            Guard::WhenExperimentalFlagIsTrue { flag, .. } => format!(
                "{} is experimental and thus unavailable with the current flags. It may be \
                 enabled by setting --{}",
                name, flag
            ),
            Guard::WhenIncompatibleFlagIsFalse { flag, .. } => format!(
                "{} is deprecated and will be removed soon. It may be temporarily re-enabled by \
                 setting --{}=false",
                name, flag
            ),
            Guard::InAllowedRepos { allowed_entries } => format!(
                "{} may only be used from one of the following repositories or prefixes: {}",
                name,
                entries(allowed_entries)
            ),
            Guard::InAllowedReposOrWhenIncompatibleFlagIsFalse {
                flag,
                allowed_entries,
                ..
            } => format!(
                "{} may only be used from one of the following repositories or prefixes: {}. It \
                 may be temporarily re-enabled for general use by setting --{}=false",
                name,
                entries(allowed_entries),
                flag
            ),
        }
    }
}

/// One predeclared symbol: its value (Java `GuardedValue.getObject()` for guarded ones) and its
/// guard, if any.
#[derive(Clone)]
pub struct EnvEntry {
    pub value: OwnedFrozenValue,
    pub guard: Option<Arc<Guard>>,
}

impl EnvEntry {
    pub fn plain(value: OwnedFrozenValue) -> EnvEntry {
        EnvEntry { value, guard: None }
    }

    pub fn guarded(value: OwnedFrozenValue, guard: Guard) -> EnvEntry {
        EnvEntry {
            value,
            guard: Some(Arc::new(guard)),
        }
    }
}

/// Java `ImmutableMap<String, Object>` of predeclared symbols (insertion order kept).
pub type StarlarkEnv = IndexMap<String, EnvEntry>;

/// The symbols a `#[starlark_module]` registration function defines, in definition order.
pub fn env_from_registration(register: &dyn Fn(&mut GlobalsBuilder)) -> StarlarkEnv {
    let mut builder = GlobalsBuilder::new();
    register(&mut builder);
    env_from_globals(&builder.build())
}

/// The symbols of `globals`.
pub fn env_from_globals(globals: &Globals) -> StarlarkEnv {
    let heap = globals.heap().clone();
    globals
        .iter()
        .map(|(name, value)| {
            // SAFETY: `value` lives in `heap`, which the OwnedFrozenValue keeps alive.
            let owned = unsafe { OwnedFrozenValue::new(heap.clone(), value) };
            (name.to_owned(), EnvEntry::plain(owned))
        })
        .collect()
}

/// A frozen value made by `make` on a fresh heap.
pub fn freeze_value(make: impl for<'v> FnOnce(&Module<'v>) -> Value<'v>) -> OwnedFrozenValue {
    Module::with_temp_heap(|module| {
        let v = make(&module);
        module.set("value", v);
        module
            .freeze()
            .expect("freezing a module of deeply immutable values")
            .get("value")
            .expect("the value just set")
    })
}

/// Java `StructProvider.STRUCT.create(bindings, errorMessageFormat)`: a struct of `bindings`
/// (guarded values unwrapped, as Java stores the objects).
///
/// PORT: Java's custom unknown-field message (e.g. "no native function or rule '%s'") is not
/// carried; the struct reports blaze_starlark's struct message.
// Java: com/google/devtools/build/lib/packages/StructProvider.java:73 StructProvider.create
pub fn create_struct(bindings: &StarlarkEnv) -> OwnedFrozenValue {
    freeze_value(|module| {
        let provider = module.heap().alloc(StructProvider);
        let fields = bindings
            .iter()
            .map(|(k, e)| {
                (
                    k.clone(),
                    blaze_starlark::eval::frozen_into(module, &e.value),
                )
            })
            .collect();
        module.heap().alloc(StarlarkInfo::new(provider, fields))
    })
}

/// The fields of a struct created by [`create_struct`] (Java `convertNativeStructToMap`), in
/// field order.
pub fn struct_fields(value: &OwnedFrozenValue) -> StarlarkEnv {
    let info = StarlarkInfo::from_value(value.value()).expect("a struct");
    info.fields()
        .map(|(name, v)| {
            let fv = v
                .unpack_frozen()
                .expect("fields of a frozen struct are frozen");
            // SAFETY: the field lives in `value`'s heap, which the new handle keeps alive.
            let owned = unsafe { OwnedFrozenValue::new(value.owner().clone(), fv) };
            (name.to_owned(), EnvEntry::plain(owned))
        })
        .collect()
}

/// The `Globals` of an environment for one file: the universe plus every entry accessible for
/// `semantics` and `compile_label`. Accessibility of a name is decided by `guards` (the
/// uninjected environment the file is compiled against) when given, else by the entry itself.
pub fn build_globals(
    env: &StarlarkEnv,
    guards: Option<&StarlarkEnv>,
    semantics: &StarlarkSemantics,
    compile_label: Option<&Label>,
) -> Globals {
    let mut builder = blaze_starlark::env::universe_builder(semantics);
    for (name, entry) in env {
        let guard = match guards {
            Some(g) => g.get(name).and_then(|e| e.guard.clone()),
            None => entry.guard.clone(),
        };
        if let Some(guard) = guard {
            if !guard.is_object_accessible_using_semantics(semantics, compile_label) {
                continue;
            }
        }
        // Java hides flag-disabled members of builtin objects per semantics
        // (`@StarlarkMethod(disableWithFlag = ...)`); `attr.license` is the one that matters for
        // environments (see attr_module::StarlarkAttrModuleWithLicense).
        let value = if name == "attr"
            && entry
                .value
                .value()
                .downcast_ref::<crate::starlark_defs::attr_module::StarlarkAttrModule>()
                .is_some()
        {
            crate::starlark_defs::attr_module::attr_for_semantics(semantics)
        } else {
            entry.value.clone()
        };
        // SAFETY: owned_frozen_value adds the value's heap as a reference of the globals heap.
        let fv = unsafe { value.owned_frozen_value(builder.frozen_heap()) };
        builder.set(name, fv);
    }
    builder.build()
}

/// A cache of [`build_globals`] results for one environment, keyed by which guarded names are
/// accessible.
#[derive(Default)]
pub struct GlobalsCache {
    cache: Mutex<HashMap<(usize, Vec<bool>), Arc<Globals>>>,
}

impl GlobalsCache {
    pub fn get(
        &self,
        env: &StarlarkEnv,
        guards: Option<&StarlarkEnv>,
        semantics: &StarlarkSemantics,
        compile_label: Option<&Label>,
    ) -> Arc<Globals> {
        let mask: Vec<bool> = env
            .iter()
            .filter_map(|(name, entry)| {
                let guard = match guards {
                    Some(g) => g.get(name).and_then(|e| e.guard.clone()),
                    None => entry.guard.clone(),
                };
                guard.map(|g| g.is_object_accessible_using_semantics(semantics, compile_label))
            })
            .collect();
        let key = (env as *const StarlarkEnv as usize, mask);
        if let Some(g) = self.cache.lock().get(&key) {
            return g.clone();
        }
        let globals = Arc::new(build_globals(env, guards, semantics, compile_label));
        self.cache.lock().insert(key, globals.clone());
        globals
    }
}
