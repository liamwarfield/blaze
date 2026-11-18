//! Ports of `ModuleOverride` (with `SingleVersionOverride`, `MultipleVersionOverride`,
//! `NonRegistryOverride`), `InterimModule` (with `DepSpec`), `Module` and `AbridgedModule`.

use std::fmt;
use std::sync::{Arc, LazyLock};

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, RepositoryMapping, RepositoryName};
use indexmap::{IndexMap, IndexSet};

use crate::module_extension::ModuleExtensionUsage;
use crate::module_key::ModuleKey;
use crate::registry::Registry;
use crate::repo_spec::{
    AttributeValue, AttributeValues, GIT_REPOSITORY, HTTP_ARCHIVE, LOCAL_REPOSITORY, RepoRuleId,
    RepoSpec,
};
use crate::version::Version;

/// Specifies that the module should be pinned to a single version, and/or come from a specific
/// registry, and/or use local patches. Port of `SingleVersionOverride`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleVersionOverride.java:37 SingleVersionOverride
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleVersionOverride {
    /// The version to pin the module to; empty if it shouldn't be pinned.
    pub version: Version,
    /// The registry to use instead of the default list; empty for no override.
    pub registry: String,
    /// The labels of patches to apply after retrieving per the registry.
    pub patches: Vec<Label>,
    /// The patch commands to execute after retrieving per the registry.
    pub patch_cmds: Vec<String>,
    /// The number of path segments to strip from the paths in the supplied patches.
    pub patch_strip: i32,
}

/// Specifies that the module should still come from a registry, but multiple versions of it
/// should be allowed to coexist. Port of `MultipleVersionOverride`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/MultipleVersionOverride.java:27 MultipleVersionOverride
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipleVersionOverride {
    pub versions: Vec<Version>,
    pub registry: String,
}

/// An override specifying that the module should not be retrieved from a registry or participate
/// in version resolution, and should come from a custom repo rule instantiation instead
/// (`archive_override`, `git_override`, `local_path_override`). Port of `NonRegistryOverride`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/NonRegistryOverride.java:28 NonRegistryOverride
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonRegistryOverride {
    /// `None` only for [`NonRegistryOverride::bazel_tools_override`].
    repo_spec: Option<RepoSpec>,
}

/// Starlark rules loaded from bazel_tools that may define Bazel module repositories with
/// non-registry overrides.
// Java: com/google/devtools/build/lib/bazel/bzlmod/NonRegistryOverride.java:32 NonRegistryOverride.BOOTSTRAP_REPO_RULES
pub static BOOTSTRAP_REPO_RULES: LazyLock<Vec<RepoRuleId>> = LazyLock::new(|| {
    vec![
        HTTP_ARCHIVE.clone(),
        GIT_REPOSITORY.clone(),
        LOCAL_REPOSITORY.clone(),
    ]
});

impl NonRegistryOverride {
    pub fn new(repo_spec: RepoSpec) -> NonRegistryOverride {
        NonRegistryOverride {
            repo_spec: Some(repo_spec),
        }
    }

    /// The sentinel override for `bazel_tools`, which comes from `embedded_tools`. Its repo spec
    /// is never inspected.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/NonRegistryOverride.java:49 NonRegistryOverride.BAZEL_TOOLS_OVERRIDE
    pub fn bazel_tools_override() -> NonRegistryOverride {
        NonRegistryOverride { repo_spec: None }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/NonRegistryOverride.java:51 NonRegistryOverride.repoSpec
    pub fn repo_spec(&self) -> &RepoSpec {
        self.repo_spec
            .as_ref()
            .expect("The bazel_tools override should never be inspected")
    }
}

/// An "override" applied to a particular module in the dependency graph. Port of the sealed
/// interface `ModuleOverride` (and `RegistryOverride`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleOverride.java:22 ModuleOverride
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleOverride {
    SingleVersion(SingleVersionOverride),
    MultipleVersion(MultipleVersionOverride),
    NonRegistry(NonRegistryOverride),
}

impl ModuleOverride {
    /// `RegistryOverride.getRegistry()`; `None` for non-registry overrides.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/RegistryOverride.java:29 RegistryOverride.getRegistry
    pub fn get_registry(&self) -> Option<&str> {
        match self {
            ModuleOverride::SingleVersion(o) => Some(&o.registry),
            ModuleOverride::MultipleVersion(o) => Some(&o.registry),
            ModuleOverride::NonRegistry(_) => None,
        }
    }
}

/// The specification of a dependency. `max_compatibility_level` is -1 when unset.
// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:59 InterimModule.DepSpec
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DepSpec {
    pub name: String,
    pub version: Version,
    pub max_compatibility_level: i32,
}

impl DepSpec {
    pub fn new(name: impl Into<String>, version: Version, max_compatibility_level: i32) -> Self {
        DepSpec {
            name: name.into(),
            version,
            max_compatibility_level,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:65 DepSpec.ROOT_MODULE
    pub fn root_module() -> DepSpec {
        DepSpec::from_module_key(&ModuleKey::root())
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:68 DepSpec.fromModuleKey
    pub fn from_module_key(key: &ModuleKey) -> DepSpec {
        DepSpec::new(key.name(), key.version().clone(), -1)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:72 DepSpec.withVersion
    pub fn with_version(&self, version: Version) -> DepSpec {
        DepSpec::new(self.name.clone(), version, self.max_compatibility_level)
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:76 DepSpec.toModuleKey
    pub fn to_module_key(&self) -> ModuleKey {
        ModuleKey::new(self.name.clone(), self.version.clone())
    }
}

impl fmt::Display for DepSpec {
    /// Java record `toString`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DepSpec[name={}, version={}, maxCompatibilityLevel={}]",
            self.name, self.version, self.max_compatibility_level
        )
    }
}

/// A node in the external dependency graph during module resolution (discovery and selection).
/// Port of `InterimModule` (an AutoValue; build with [`InterimModule::builder`]).
// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:47 InterimModule
#[derive(Debug, Clone)]
pub struct InterimModule {
    /// The name of the module, as specified in its MODULE.bazel; may be empty for the root.
    pub name: String,
    /// The version as specified in MODULE.bazel; may be empty for the root or a non-registry
    /// override.
    pub version: Version,
    /// The key of this module in the dependency graph (see Java `ModuleBase.getKey`).
    pub key: ModuleKey,
    /// The name of the repo representing this module, as seen by the module itself.
    pub repo_name: String,
    pub execution_platforms_to_register: Vec<String>,
    pub toolchains_to_register: Vec<String>,
    pub extension_usages: Vec<ModuleExtensionUsage>,
    pub flag_aliases: IndexMap<String, String>,
    /// The compatibility level (the "major version" in SemVer terms).
    pub compatibility_level: i32,
    pub bazel_compatibility: Vec<String>,
    /// The resolved direct dependencies, keyed by repo name.
    pub deps: IndexMap<String, DepSpec>,
    /// The original direct dependencies as declared in the MODULE file, keyed by repo name.
    pub original_deps: IndexMap<String, DepSpec>,
    /// "nodep" dependencies.
    pub nodep_deps: Vec<DepSpec>,
    /// The registry this module came from; `None` iff it has a `NonRegistryOverride`.
    pub registry: Option<Arc<dyn Registry>>,
}

impl PartialEq for InterimModule {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.version == other.version
            && self.key == other.key
            && self.repo_name == other.repo_name
            && self.execution_platforms_to_register == other.execution_platforms_to_register
            && self.toolchains_to_register == other.toolchains_to_register
            && self.extension_usages == other.extension_usages
            && self.flag_aliases == other.flag_aliases
            && self.compatibility_level == other.compatibility_level
            && self.bazel_compatibility == other.bazel_compatibility
            && self.deps == other.deps
            && self.original_deps == other.original_deps
            && self.nodep_deps == other.nodep_deps
            // PORT: Java compares registries with equals(); IndexRegistry has identity equality
            // and Skyframe holds one instance per URL, so comparing URLs is equivalent.
            && self.registry.as_ref().map(|r| r.get_url())
                == other.registry.as_ref().map(|r| r.get_url())
    }
}

impl Eq for InterimModule {}

/// Builder for [`InterimModule`] (`InterimModule.Builder`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:146 InterimModule.Builder
#[derive(Debug, Clone)]
pub struct InterimModuleBuilder {
    module: InterimModule,
    repo_name_set: bool,
}

impl InterimModule {
    /// Returns a new builder with name "", version EMPTY, key ROOT and compatibility level 0.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:122 InterimModule.builder
    pub fn builder() -> InterimModuleBuilder {
        InterimModuleBuilder {
            module: InterimModule {
                name: String::new(),
                version: Version::empty(),
                key: ModuleKey::root(),
                repo_name: String::new(),
                execution_platforms_to_register: Vec::new(),
                toolchains_to_register: Vec::new(),
                extension_usages: Vec::new(),
                flag_aliases: IndexMap::new(),
                compatibility_level: 0,
                bazel_compatibility: Vec::new(),
                deps: IndexMap::new(),
                original_deps: IndexMap::new(),
                nodep_deps: Vec::new(),
                registry: None,
            },
            repo_name_set: false,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:119 InterimModule.toBuilder
    pub fn to_builder(&self) -> InterimModuleBuilder {
        InterimModuleBuilder {
            module: self.clone(),
            repo_name_set: true,
        }
    }

    /// Returns a copy with all values in `deps` and `nodep_deps` transformed by `transform`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:134 InterimModule.withDepsTransformed
    pub fn with_deps_transformed(&self, mut transform: impl FnMut(&DepSpec) -> DepSpec) -> Self {
        let mut m = self.clone();
        m.deps = self
            .deps
            .iter()
            .map(|(k, v)| (k.clone(), transform(v)))
            .collect();
        m.nodep_deps = self.nodep_deps.iter().map(&mut transform).collect();
        m
    }

    pub fn get_key(&self) -> &ModuleKey {
        &self.key
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_registry(&self) -> Option<&Arc<dyn Registry>> {
        self.registry.as_ref()
    }

    /// Builds a [`Module`] from an [`InterimModule`], discarding resolution-only fields and adding
    /// the repo spec (`None` if the module has a non-registry override).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:236 InterimModule.toModule
    pub fn to_module(
        interim: &InterimModule,
        override_: Option<&ModuleOverride>,
        remote_repo_spec: Option<&RepoSpec>,
    ) -> Module {
        Module {
            name: interim.name.clone(),
            version: interim.version.clone(),
            key: interim.key.clone(),
            repo_name: interim.repo_name.clone(),
            execution_platforms_to_register: interim.execution_platforms_to_register.clone(),
            toolchains_to_register: interim.toolchains_to_register.clone(),
            deps: interim
                .deps
                .iter()
                .map(|(k, v)| (k.clone(), v.to_module_key()))
                .collect(),
            repo_spec: maybe_append_additional_patches(remote_repo_spec, override_),
            extension_usages: interim.extension_usages.clone(),
            flag_aliases: interim.flag_aliases.clone(),
        }
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:253 InterimModule.maybeAppendAdditionalPatches
fn maybe_append_additional_patches(
    repo_spec: Option<&RepoSpec>,
    override_: Option<&ModuleOverride>,
) -> Option<RepoSpec> {
    let Some(ModuleOverride::SingleVersion(single_version)) = override_ else {
        return repo_spec.cloned();
    };
    if single_version.patches.is_empty()
        && single_version.patch_cmds.is_empty()
        && single_version.patch_strip == 0
    {
        return repo_spec.cloned();
    }
    let repo_spec = repo_spec.expect("NullPointerException: repoSpec");
    let mut attrs: Vec<(String, AttributeValue)> = repo_spec.attributes().attributes().to_vec();
    attrs.push((
        "patches".to_string(),
        AttributeValue::List(
            single_version
                .patches
                .iter()
                .cloned()
                .map(AttributeValue::Label)
                .collect(),
        ),
    ));
    attrs.push((
        "patch_cmds".to_string(),
        AttributeValue::string_list(single_version.patch_cmds.iter().cloned()),
    ));
    attrs.push((
        "patch_args".to_string(),
        AttributeValue::string_list([format!("-p{}", single_version.patch_strip)]),
    ));
    Some(RepoSpec::new_nullable(
        repo_spec.repo_rule_id().cloned(),
        AttributeValues::create(attrs),
    ))
}

impl InterimModuleBuilder {
    pub fn set_name(&mut self, value: impl Into<String>) -> &mut Self {
        self.module.name = value.into();
        self
    }

    pub fn set_version(&mut self, value: Version) -> &mut Self {
        self.module.version = value;
        self
    }

    pub fn set_key(&mut self, value: ModuleKey) -> &mut Self {
        self.module.key = value;
        self
    }

    pub fn set_compatibility_level(&mut self, value: i32) -> &mut Self {
        self.module.compatibility_level = value;
        self
    }

    pub fn set_repo_name(&mut self, value: impl Into<String>) -> &mut Self {
        self.module.repo_name = value.into();
        self.repo_name_set = true;
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:167 Builder.addFlagAlias
    pub fn add_flag_alias(
        &mut self,
        native_name: impl Into<String>,
        starlark_label: impl Into<String>,
    ) -> &mut Self {
        let k = native_name.into();
        // PORT: ImmutableMap.Builder.build throws on duplicate keys.
        if self
            .module
            .flag_aliases
            .insert(k.clone(), starlark_label.into())
            .is_some()
        {
            panic!("IllegalArgumentException: Multiple entries with same key: {k}");
        }
        self
    }

    pub fn add_bazel_compatibility_values<S: Into<String>>(
        &mut self,
        values: impl IntoIterator<Item = S>,
    ) -> &mut Self {
        self.module
            .bazel_compatibility
            .extend(values.into_iter().map(Into::into));
        self
    }

    pub fn add_execution_platforms_to_register<S: Into<String>>(
        &mut self,
        values: impl IntoIterator<Item = S>,
    ) -> &mut Self {
        self.module
            .execution_platforms_to_register
            .extend(values.into_iter().map(Into::into));
        self
    }

    pub fn add_toolchains_to_register<S: Into<String>>(
        &mut self,
        values: impl IntoIterator<Item = S>,
    ) -> &mut Self {
        self.module
            .toolchains_to_register
            .extend(values.into_iter().map(Into::into));
        self
    }

    pub fn set_original_deps(&mut self, value: IndexMap<String, DepSpec>) -> &mut Self {
        self.module.original_deps = value;
        self
    }

    pub fn set_deps(&mut self, value: IndexMap<String, DepSpec>) -> &mut Self {
        self.module.deps = value;
        self
    }

    pub fn add_nodep_dep(&mut self, value: DepSpec) -> &mut Self {
        self.module.nodep_deps.push(value);
        self
    }

    pub fn set_nodep_deps(&mut self, value: Vec<DepSpec>) -> &mut Self {
        self.module.nodep_deps = value;
        self
    }

    pub fn set_registry(&mut self, value: Option<Arc<dyn Registry>>) -> &mut Self {
        self.module.registry = value;
        self
    }

    pub fn set_extension_usages(&mut self, value: Vec<ModuleExtensionUsage>) -> &mut Self {
        self.module.extension_usages = value;
        self
    }

    pub fn add_extension_usage(&mut self, value: ModuleExtensionUsage) -> &mut Self {
        self.module.extension_usages.push(value);
        self
    }

    /// Builds; the repo name defaults to the name.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:220 Builder.build
    pub fn build(&self) -> InterimModule {
        let mut m = self.module.clone();
        if !self.repo_name_set {
            m.repo_name = m.name.clone();
        }
        m
    }
}

/// A node in the external dependency graph after module resolution. Port of `Module`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/Module.java:37 Module
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub version: Version,
    pub key: ModuleKey,
    pub repo_name: String,
    pub execution_platforms_to_register: Vec<String>,
    pub toolchains_to_register: Vec<String>,
    /// The resolved direct dependencies: repo name to the dep's module key.
    pub deps: IndexMap<String, ModuleKey>,
    /// Only set for modules coming from registries.
    pub repo_spec: Option<RepoSpec>,
    pub extension_usages: Vec<ModuleExtensionUsage>,
    pub flag_aliases: IndexMap<String, String>,
}

impl Module {
    /// A module with the given name and version and nothing else (`Module.builder()` with the
    /// fields tests and the empty dep graph set).
    pub fn new(name: impl Into<String>, version: Version, key: ModuleKey) -> Module {
        let name = name.into();
        Module {
            repo_name: name.clone(),
            name,
            version,
            key,
            execution_platforms_to_register: Vec::new(),
            toolchains_to_register: Vec::new(),
            deps: IndexMap::new(),
            repo_spec: None,
            extension_usages: Vec::new(),
            flag_aliases: IndexMap::new(),
        }
    }

    pub fn get_key(&self) -> &ModuleKey {
        &self.key
    }

    /// Returns a [`RepositoryMapping`] with only Bazel module repos and no repos from module
    /// extensions.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/Module.java:49 Module.getRepoMappingWithBazelDepsOnly
    pub fn get_repo_mapping_with_bazel_deps_only(
        &self,
        module_key_to_repository_names: &IndexMap<ModuleKey, RepositoryName>,
    ) -> RepositoryMapping {
        let mut mapping: Vec<(String, RepositoryName)> = Vec::new();
        let mut seen: IndexSet<String> = IndexSet::new();
        let mut put = |k: &str, v: RepositoryName| {
            // PORT: ImmutableMap.Builder.buildOrThrow throws on duplicate keys.
            if !seen.insert(k.to_string()) {
                panic!("IllegalArgumentException: Multiple entries with same key: {k}");
            }
            mapping.push((k.to_string(), v));
        };
        // If this is the root module, then the main repository should be visible as `@`.
        if self.key.is_root() {
            put("", MAIN.clone());
        }
        // Every module should be able to reference itself as @<module repo name>.
        let owner = module_key_to_repository_names
            .get(&self.key)
            .expect("NullPointerException: owner")
            .clone();
        if !self.repo_name.is_empty() {
            put(&self.repo_name, owner.clone());
        }
        for (dep_name, dep_key) in &self.deps {
            // If `dep` is the root module, its canonical repo name is the empty string, which
            // maps to the main repo ("@").
            put(
                dep_name,
                module_key_to_repository_names
                    .get(dep_key)
                    .expect("NullPointerException: dep repository name")
                    .clone(),
            );
        }
        RepositoryMapping::create(mapping, owner)
    }
}

/// An abridged version of a [`Module`], used for module extension resolution. Port of
/// `AbridgedModule`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/AbridgedModule.java:27 AbridgedModule
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AbridgedModule {
    pub name: String,
    pub version: Version,
    pub key: ModuleKey,
}

impl AbridgedModule {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/AbridgedModule.java:34 AbridgedModule.from
    pub fn from(module: &Module) -> AbridgedModule {
        AbridgedModule {
            name: module.name.clone(),
            version: module.version.clone(),
            key: module.key.clone(),
        }
    }
}
