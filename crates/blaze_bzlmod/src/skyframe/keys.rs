//! Skyframe keys and values of `lib/bazel/bzlmod`.

use std::fmt;
use std::sync::Arc;

use blaze_bzlmod_model::{
    BazelDepGraphValue, BazelLockFileValue, BazelModuleResolutionValue, Checksum, Facts,
    InterimModule, LockFileModuleExtensionWithFactors, ModuleExtensionId, ModuleKey,
    NonRootModuleFileValue, Registry, RepoSpec, RootModuleFileValue, YankedVersionsValue,
};
use blaze_cmdline::RepositoryName;
use blaze_skyframe::{Key, SkyFunctionName};
use indexmap::IndexMap;

use crate::inspector::BazelModuleInspectorValue;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:149 SkyFunctions.MODULE_FILE
pub const MODULE_FILE: SkyFunctionName = SkyFunctionName("MODULE_FILE");
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

impl Key for ModuleFileKey {
    type Value = ModuleFileValue;
    const FUNCTION: SkyFunctionName = MODULE_FILE;
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
}

/// Port of `RootModuleFileFixup` (the buildozer fixup for incorrect `use_repo` declarations).
///
/// PORT: not ported yet (it is produced by `ModuleExtensionMetadata.generateFixup`, which runs
/// with the extension); `SingleExtensionValue::fixup` is always `None` until then.
pub type RootModuleFileFixup = ();

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
    /// PORT: see [`RootModuleFileFixup`].
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
    const FUNCTION: SkyFunctionName = SINGLE_EXTENSION;
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
    const FUNCTION: SkyFunctionName = SINGLE_EXTENSION_EVAL;
}
