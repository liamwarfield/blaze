//! Skyframe keys and values of `lib/bazel/bzlmod`.

use std::fmt;
use std::sync::Arc;

use blaze_bzlmod_model::{
    BazelDepGraphValue, BazelLockFileValue, BazelModuleResolutionValue, Checksum, Facts,
    InterimModule, LockFileModuleExtensionWithFactors, ModuleExtensionId, ModuleKey,
    NonRootModuleFileValue, Registry, RepoSpec, RootModuleFileValue, YankedVersionsValue,
};
use blaze_cmdline::RepositoryName;
use blaze_skyframe::{FunctionHermeticity, Key, SkyFunctionName};
use indexmap::IndexMap;

use crate::inspector::BazelModuleInspectorValue;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:149 SkyFunctions.MODULE_FILE
pub const MODULE_FILE: SkyFunctionName = SkyFunctionName("MODULE_FILE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:157 SkyFunctions.BAZEL_MOD_TIDY
pub const BAZEL_MOD_TIDY: SkyFunctionName = SkyFunctionName("BAZEL_MOD_TIDY");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:159 SkyFunctions.BAZEL_MODULE_RESOLUTION
pub const BAZEL_MODULE_RESOLUTION: SkyFunctionName = SkyFunctionName("BAZEL_MODULE_RESOLUTION");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:161 SkyFunctions.BAZEL_MODULE_INSPECTION
pub const BAZEL_MODULE_INSPECTION: SkyFunctionName = SkyFunctionName("BAZEL_MODULE_INSPECTION");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:165 SkyFunctions.SINGLE_EXTENSION
pub const SINGLE_EXTENSION: SkyFunctionName = SkyFunctionName("SINGLE_EXTENSION");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:167 SkyFunctions.SINGLE_EXTENSION_EVAL
pub const SINGLE_EXTENSION_EVAL: SkyFunctionName = SkyFunctionName("SINGLE_EXTENSION_EVAL");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:169 SkyFunctions.BAZEL_DEP_GRAPH
pub const BAZEL_DEP_GRAPH: SkyFunctionName = SkyFunctionName("BAZEL_DEP_GRAPH");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:171 SkyFunctions.BAZEL_LOCK_FILE
pub const BAZEL_LOCK_FILE: SkyFunctionName = SkyFunctionName("BAZEL_LOCK_FILE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:175 SkyFunctions.REGISTRY
pub const REGISTRY: SkyFunctionName = SkyFunctionName("REGISTRY");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:176 SkyFunctions.REPO_SPEC
pub const REPO_SPEC: SkyFunctionName = SkyFunctionName("REPO_SPEC");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:177 SkyFunctions.YANKED_VERSIONS
pub const YANKED_VERSIONS: SkyFunctionName = SkyFunctionName("YANKED_VERSIONS");

/// `SkyKey` for `ModuleFileValue` computation. `Debug` is the Java record's `toString`
/// (`Key[moduleKey=<root>]`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:88 ModuleFileValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ModuleFileKey {
    pub module_key: ModuleKey,
}

impl ModuleFileKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:82 ModuleFileValue.key
    pub fn key(module_key: ModuleKey) -> ModuleFileKey {
        ModuleFileKey { module_key }
    }

    /// `ModuleFileValue.KEY_FOR_ROOT_MODULE`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:33 ModuleFileValue.KEY_FOR_ROOT_MODULE
    pub fn key_for_root_module() -> ModuleFileKey {
        ModuleFileKey::key(ModuleKey::root())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:88 ModuleFileValue.Key.moduleKey
    pub fn module_key(&self) -> &ModuleKey {
        &self.module_key
    }
}

impl fmt::Debug for ModuleFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key[moduleKey={}]", self.module_key)
    }
}

/// Java's identity comparison of `InterimModule.getRegistry()` (a `NotComparableSkyValue`):
/// both absent or the same object.
// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:109 InterimModule.getRegistry
fn same_registry(old: &InterimModule, new: &InterimModule) -> bool {
    match (&old.registry, &new.registry) {
        (None, None) => true,
        (Some(old), Some(new)) => Arc::ptr_eq(old, new),
        _ => false,
    }
}

impl Key for ModuleFileKey {
    type Value = ModuleFileValue;
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:96 ModuleFileValue.Key.functionName
    const FUNCTION: SkyFunctionName = MODULE_FILE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:150 SkyFunctions.MODULE_FILE (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: both `ModuleFileValue`s are records; `InterimModule` is an AutoValue whose
    /// `getRegistry()` is a `Registry`, a `NotComparableSkyValue` without `equals` (identity).
    /// `InterimModule::eq` compares registries by URL, so the registry is compared here by
    /// identity as well: a re-computed REGISTRY makes the module file value unequal, as in Java.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:50 ModuleFileValue.NonRootModuleFileValue
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:109 InterimModule.getRegistry
    fn values_equal(old: &ModuleFileValue, new: &ModuleFileValue) -> bool {
        old == new && same_registry(old.module(), new.module())
    }
}

/// The result of `ModuleFileFunction`: the root module's value for
/// [`ModuleFileKey::key_for_root_module`], otherwise a non-root module's.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:31 ModuleFileValue
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum ModuleFileValue {
    Root(RootModuleFileValue),
    NonRoot(NonRootModuleFileValue),
}

impl ModuleFileValue {
    /// The module from the module file evaluation.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:40 ModuleFileValue.module
    pub fn module(&self) -> &InterimModule {
        match self {
            ModuleFileValue::Root(v) => &v.module,
            ModuleFileValue::NonRoot(v) => &v.module,
        }
    }

    /// Hashes of files obtained (or known to be missing) from registries while obtaining this
    /// module file. Empty for the root module, which is not obtained from a registry.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileValue.java:46 ModuleFileValue.registryFileHashes
    pub fn registry_file_hashes(&self) -> IndexMap<String, Option<Checksum>> {
        match self {
            ModuleFileValue::Root(_) => IndexMap::new(),
            ModuleFileValue::NonRoot(v) => v.registry_file_hashes.clone(),
        }
    }

    /// Java's `(RootModuleFileValue) env.getValue(KEY_FOR_ROOT_MODULE)` cast.
    pub fn as_root(&self) -> &RootModuleFileValue {
        match self {
            ModuleFileValue::Root(v) => v,
            ModuleFileValue::NonRoot(_) => {
                panic!("ClassCastException: NonRootModuleFileValue is not a RootModuleFileValue")
            }
        }
    }
}

/// `BazelModuleResolutionValue.KEY`.
///
/// PORT: Java's key is a lambda whose `toString` is a JVM class name; `Debug` prints the
/// function name instead.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionValue.java:35 BazelModuleResolutionValue.KEY
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BazelModuleResolutionKey;

impl fmt::Debug for BazelModuleResolutionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{BAZEL_MODULE_RESOLUTION}")
    }
}

impl Key for BazelModuleResolutionKey {
    type Value = BazelModuleResolutionValue;
    const FUNCTION: SkyFunctionName = BAZEL_MODULE_RESOLUTION;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:160 SkyFunctions.BAZEL_MODULE_RESOLUTION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: an AutoValue (resolved and unpruned dep graphs, registry file hashes, selected yanked
    /// versions); the unpruned graph's `InterimModule`s compare their registries by identity (see
    /// `ModuleFileKey`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionValue.java:32 BazelModuleResolutionValue
    fn values_equal(old: &BazelModuleResolutionValue, new: &BazelModuleResolutionValue) -> bool {
        old == new
            && old.unpruned_dep_graph.iter().all(|(key, module)| {
                new.unpruned_dep_graph
                    .get(key)
                    .is_some_and(|other| same_registry(module, other))
            })
    }
}

/// `BazelDepGraphValue.KEY`. PORT: `Debug` prints the function name (Java: a lambda).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:37 BazelDepGraphValue.KEY
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BazelDepGraphKey;

impl fmt::Debug for BazelDepGraphKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{BAZEL_DEP_GRAPH}")
    }
}

impl Key for BazelDepGraphKey {
    type Value = BazelDepGraphValue;
    const FUNCTION: SkyFunctionName = BAZEL_DEP_GRAPH;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:170 SkyFunctions.BAZEL_DEP_GRAPH (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: an AutoValue of the dep graph, the canonical repo name lookup, the abridged modules, the extension usages table, the extension unique names and the root module name.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphValue.java:36 BazelDepGraphValue
    fn values_equal(old: &BazelDepGraphValue, new: &BazelDepGraphValue) -> bool {
        old == new
    }
}

/// `BazelModuleInspectorValue.KEY`. PORT: `Debug` prints the function name (Java: a lambda).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:69 BazelModuleInspectorValue.KEY
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BazelModuleInspectorKey;

impl fmt::Debug for BazelModuleInspectorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{BAZEL_MODULE_INSPECTION}")
    }
}

impl Key for BazelModuleInspectorKey {
    type Value = BazelModuleInspectorValue;
    const FUNCTION: SkyFunctionName = BAZEL_MODULE_INSPECTION;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:162 SkyFunctions.BAZEL_MODULE_INSPECTION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record; its `errors` are `ExternalDepsException`s, which have no `equals`
    /// (identity), so two values are equal only when neither has errors.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:53 BazelModuleInspectorValue
    fn values_equal(old: &BazelModuleInspectorValue, new: &BazelModuleInspectorValue) -> bool {
        old.errors.is_empty()
            && new.errors.is_empty()
            && old.dep_graph == new.dep_graph
            && old.modules_index == new.modules_index
            && old.extension_to_repo_internal_names == new.extension_to_repo_internal_names
            && old.module_key_to_canonical_names == new.module_key_to_canonical_names
    }
}

/// The two lockfile keys: `BazelLockFileValue.KEY` (the workspace `MODULE.bazel.lock`) and
/// `BazelLockFileValue.HIDDEN_KEY` (`MODULE.bazel.lock` under the output base).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:66 BazelLockFileValue.KEY
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BazelLockFileKey {
    hidden: bool,
}

impl BazelLockFileKey {
    /// The (regular) lockfile, stored as MODULE.bazel.lock under the workspace directory.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:66 BazelLockFileValue.KEY
    pub const KEY: BazelLockFileKey = BazelLockFileKey { hidden: false };
    /// The hidden lockfile, stored as MODULE.bazel.lock under the output base.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:96 BazelLockFileValue.HIDDEN_KEY
    pub const HIDDEN_KEY: BazelLockFileKey = BazelLockFileKey { hidden: true };

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
}

impl fmt::Debug for BazelLockFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.hidden {
            "BazelLockFileValue.HIDDEN_KEY"
        } else {
            "BazelLockFileValue.KEY"
        })
    }
}

impl Key for BazelLockFileKey {
    type Value = BazelLockFileValue;
    const FUNCTION: SkyFunctionName = BAZEL_LOCK_FILE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:172 SkyFunctions.BAZEL_LOCK_FILE (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: an AutoValue of the version, registry file hashes, selected yanked versions, module extensions and facts. PORT: facts are compared in order (Java's `Dict` is a map); both sides sort them, so only a float NaN can differ, and then Rust is the side that does not prune.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileValue.java:44 BazelLockFileValue
    fn values_equal(old: &BazelLockFileValue, new: &BazelLockFileValue) -> bool {
        old == new
    }
}

/// The key for `RegistryFunction`; the value is the registry.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryKey.java:27 RegistryKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RegistryKey {
    pub url: String,
}

impl RegistryKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryKey.java:35 RegistryKey.create
    pub fn create(url: impl Into<String>) -> RegistryKey {
        RegistryKey { url: url.into() }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for RegistryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RegistryKey[url={}]", self.url)
    }
}

impl Key for RegistryKey {
    type Value = Arc<dyn Registry>;
    const FUNCTION: SkyFunctionName = REGISTRY;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:175 SkyFunctions.REGISTRY (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    // Default `values_equal` (never equal): Java's `Registry` is a `NotComparableSkyValue`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Registry.java:27 Registry
}

/// The key for `RepoSpecFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecKey.java:27 RepoSpecKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RepoSpecKey {
    pub module_key: ModuleKey,
    pub registry_url: String,
}

impl RepoSpecKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecKey.java:35 RepoSpecKey.of
    pub fn of(module: &InterimModule) -> RepoSpecKey {
        let registry = module
            .registry
            .as_ref()
            .expect("module must not have a non-registry override");
        RepoSpecKey::create(module.key.clone(), registry.get_url())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecKey.java:43 RepoSpecKey.create
    pub fn create(module_key: ModuleKey, registry_url: impl Into<String>) -> RepoSpecKey {
        RepoSpecKey {
            module_key,
            registry_url: registry_url.into(),
        }
    }
}

impl fmt::Debug for RepoSpecKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RepoSpecKey[moduleKey={}, registryUrl={}]",
            self.module_key, self.registry_url
        )
    }
}

impl Key for RepoSpecKey {
    type Value = RepoSpecValue;
    const FUNCTION: SkyFunctionName = REPO_SPEC;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:176 SkyFunctions.REPO_SPEC (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: a record of the `RepoSpec` record and the registry file hashes.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecValue.java:28 RepoSpecValue
    fn values_equal(old: &RepoSpecValue, new: &RepoSpecValue) -> bool {
        old == new
    }
}

/// The value for `RepoSpecFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecValue.java:28 RepoSpecValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSpecValue {
    pub repo_spec: RepoSpec,
    pub registry_file_hashes: IndexMap<String, Option<Checksum>>,
}

impl RepoSpecValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RepoSpecValue.java:36 RepoSpecValue.create
    pub fn create(
        repo_spec: RepoSpec,
        registry_file_hashes: IndexMap<String, Option<Checksum>>,
    ) -> RepoSpecValue {
        RepoSpecValue {
            repo_spec,
            registry_file_hashes,
        }
    }
}

/// The key for `YankedVersionsFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:44 YankedVersionsValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct YankedVersionsKey {
    pub module_name: String,
    pub registry_url: String,
}

impl YankedVersionsKey {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:53 YankedVersionsValue.Key.create
    pub fn create(module_name: impl Into<String>, registry_url: impl Into<String>) -> Self {
        YankedVersionsKey {
            module_name: module_name.into(),
            registry_url: registry_url.into(),
        }
    }
}

impl fmt::Debug for YankedVersionsKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Key[moduleName={}, registryUrl={}]",
            self.module_name, self.registry_url
        )
    }
}

impl Key for YankedVersionsKey {
    type Value = YankedVersionsValue;
    const FUNCTION: SkyFunctionName = YANKED_VERSIONS;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:178 SkyFunctions.YANKED_VERSIONS (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: a record of the optional map from version to yank reason.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:29 YankedVersionsValue
    fn values_equal(old: &YankedVersionsValue, new: &YankedVersionsValue) -> bool {
        old == new
    }
}

/// Generated when incorrect use_repo calls are detected in the root module file and its
/// includes according to `ModuleExtensionMetadata`, and contains the buildozer commands required
/// to bring them into the expected state (produced by blaze_repo_rules'
/// `ModuleExtensionMetadata.generateFixup`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/RootModuleFileFixup.java:33 RootModuleFileFixup
#[derive(Debug, Clone, PartialEq)]
pub struct RootModuleFileFixup {
    /// The keys are the paths to the root MODULE.bazel file (or its includes); the values are
    /// the buildozer commands required to bring the keyed file into the expected state (Java
    /// `ImmutableListMultimap`: keys in first-insertion order).
    pub module_file_path_to_buildozer_commands: IndexMap<blaze_vfs::PathFragment, Vec<String>>,
    pub usage: blaze_bzlmod_model::ModuleExtensionUsage,
    pub warning: blaze_events::Event,
}

impl RootModuleFileFixup {
    /// A human-readable message describing the fixup after it has been applied.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RootModuleFileFixup.java:39 RootModuleFileFixup.getSuccessMessage
    pub fn get_success_message(&self) -> String {
        let extension_id = format!(
            "{}%{}",
            self.usage.extension_bzl_file, self.usage.extension_name
        );
        match &self.usage.isolation_key {
            Some(key) => format!(
                "Updated use_repo calls for isolated usage '{}' of {}",
                key.usage_exported_name(),
                extension_id
            ),
            None => format!("Updated use_repo calls for {extension_id}"),
        }
    }
}

/// The result of evaluating a single module extension.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:48 SingleExtensionValue
#[derive(Debug, Clone, PartialEq)]
pub struct SingleExtensionValue {
    /// The repos generated by the extension: "internal name" to repo spec.
    pub generated_repo_specs: IndexMap<String, RepoSpec>,
    /// Canonical repo names of the generated repos to their internal names (a bimap in Java).
    pub canonical_repo_name_to_internal_names: IndexMap<RepositoryName, String>,
    /// The information stored about the extension in the lockfile. Present if and only if the
    /// lockfile mode is UPDATE (or REFRESH).
    pub lock_file_info: Option<LockFileModuleExtensionWithFactors>,
    /// The buildozer fixup for incorrect `use_repo` declarations by the root module (if any).
    pub fixup: Option<RootModuleFileFixup>,
    pub facts: Facts,
    pub facts_version: i32,
}

impl SingleExtensionValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:64 SingleExtensionValue.key
    pub fn key(id: ModuleExtensionId) -> SingleExtensionKey {
        SingleExtensionKey(id)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:81 SingleExtensionValue.evalKey
    pub fn eval_key(id: ModuleExtensionId) -> SingleExtensionEvalKey {
        SingleExtensionEvalKey(id)
    }

    /// Returns the canonical repo name for a repo generated by an extension.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:72 SingleExtensionValue.repositoryName
    pub fn repository_name(extension_unique_name: &str, internal_name: &str) -> RepositoryName {
        RepositoryName::create_unvalidated(&format!("{extension_unique_name}+{internal_name}"))
    }
}

/// The `SkyKey` of a [`SingleExtensionValue`] whose imports have been validated. `Debug` is
/// `SINGLE_EXTENSION:<id>` (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:90 SingleExtensionValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SingleExtensionKey(pub ModuleExtensionId);

impl SingleExtensionKey {
    pub fn argument(&self) -> &ModuleExtensionId {
        &self.0
    }
}

impl fmt::Debug for SingleExtensionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", SINGLE_EXTENSION, self.0)
    }
}

impl Key for SingleExtensionKey {
    type Value = SingleExtensionValue;
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:107 SingleExtensionValue.Key.functionName
    const FUNCTION: SkyFunctionName = SINGLE_EXTENSION;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:166 SkyFunctions.SINGLE_EXTENSION (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record (generated repo specs, canonical names, lockfile info, fixup, facts, facts version).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:48 SingleExtensionValue
    fn values_equal(old: &SingleExtensionValue, new: &SingleExtensionValue) -> bool {
        old == new
    }
}

/// The `SkyKey` of a [`SingleExtensionValue`] with the <b>unvalidated</b> result of extension
/// evaluation. `Debug` is `SINGLE_EXTENSION_EVAL:<id>`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:123 SingleExtensionValue.EvalKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SingleExtensionEvalKey(pub ModuleExtensionId);

impl SingleExtensionEvalKey {
    pub fn argument(&self) -> &ModuleExtensionId {
        &self.0
    }
}

impl fmt::Debug for SingleExtensionEvalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", SINGLE_EXTENSION_EVAL, self.0)
    }
}

impl Key for SingleExtensionEvalKey {
    type Value = SingleExtensionValue;
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:140 SingleExtensionValue.EvalKey.functionName
    const FUNCTION: SkyFunctionName = SINGLE_EXTENSION_EVAL;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:168 SkyFunctions.SINGLE_EXTENSION_EVAL (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: the same record as SINGLE_EXTENSION's.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:48 SingleExtensionValue
    fn values_equal(old: &SingleExtensionValue, new: &SingleExtensionValue) -> bool {
        old == new
    }
}

/// All Skyframe information required for the `bazel mod tidy` command.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyValue.java:38 BazelModTidyValue
#[derive(Debug, Clone)]
pub struct BazelModTidyValue {
    /// Buildozer fixups for incorrect use_repo declarations by the root module.
    pub fixups: Vec<RootModuleFileFixup>,
    /// The path of the buildozer binary provided by the "buildozer" module.
    pub buildozer: blaze_vfs::Path,
    /// The set of paths to the root MODULE.bazel file and all its includes.
    pub module_file_paths: indexmap::IndexSet<blaze_vfs::PathFragment>,
    /// Errors encountered while evaluating prerequisites for `bazel mod tidy`.
    pub errors: Vec<blaze_bzlmod_model::ExternalDepsException>,
}

impl BazelModTidyValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyValue.java:53 BazelModTidyValue.create
    pub fn create(
        fixups: Vec<RootModuleFileFixup>,
        buildozer: blaze_vfs::Path,
        module_file_paths: indexmap::IndexSet<blaze_vfs::PathFragment>,
        errors: Vec<blaze_bzlmod_model::ExternalDepsException>,
    ) -> BazelModTidyValue {
        BazelModTidyValue {
            fixups,
            buildozer,
            module_file_paths,
            errors,
        }
    }
}

/// `BazelModTidyValue.KEY`.
///
/// PORT: Java's key is a lambda whose `toString` is a JVM class name; `Debug` prints the
/// function name instead.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyValue.java:51 BazelModTidyValue.KEY
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BazelModTidyKey;

impl fmt::Debug for BazelModTidyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{BAZEL_MOD_TIDY}")
    }
}

impl Key for BazelModTidyKey {
    type Value = BazelModTidyValue;
    const FUNCTION: SkyFunctionName = BAZEL_MOD_TIDY;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:158 SkyFunctions.BAZEL_MOD_TIDY (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: a record; its `errors` are `ExternalDepsException`s (identity), so two values are
    /// equal only when neither has errors.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyValue.java:38 BazelModTidyValue
    fn values_equal(old: &BazelModTidyValue, new: &BazelModTidyValue) -> bool {
        old.errors.is_empty()
            && new.errors.is_empty()
            && old.fixups == new.fixups
            && old.buildozer == new.buildozer
            && old.module_file_paths == new.module_file_paths
    }
}
