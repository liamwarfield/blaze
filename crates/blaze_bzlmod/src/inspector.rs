//! Ports of `BazelModuleInspectorValue` (with `AugmentedModule`, `ResolutionReason`) and the
//! Skyframe-independent part of `BazelModuleInspectorFunction` (`computeAugmentedGraph`).
//! OWNER: agent B. The SkyFunction wrapper is `crate::skyframe::inspector_function` (agent D).

use std::cmp::Ordering;
use std::collections::HashMap;

use blaze_bzlmod_model::{
    ExternalDepsException, InterimModule, ModuleExtensionId, ModuleKey, ModuleOverride, Version,
};
use blaze_cmdline::RepositoryName;
use indexmap::{IndexMap, IndexSet};

use crate::modcommand::java_hash::{
    JavaHashMapOrder, java_module_key_hash_code, java_string_hash_code,
};

/// The reason why a final dependency of a module was resolved the way it was.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:216 BazelModuleInspectorValue.AugmentedModule.ResolutionReason
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionReason {
    /// The dependency is the original dependency defined in the MODULE.bazel file.
    ORIGINAL,
    /// The dependency was replaced by the Minimal-Version Selection algorithm.
    MINIMAL_VERSION_SELECTION,
    /// The dependency was replaced by a `single_version_override` rule.
    SINGLE_VERSION_OVERRIDE,
    /// The dependency was replaced by a `multiple_version_override` rule.
    MULTIPLE_VERSION_OVERRIDE,
    /// The dependency was replaced by one of the `NonRegistryOverride` rules.
    NON_REGISTRY_OVERRIDE,
}

impl ResolutionReason {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:234 BazelModuleInspectorValue.AugmentedModule.ResolutionReason.getLabel
    pub fn get_label(self) -> &'static str {
        match self {
            ResolutionReason::ORIGINAL => "",
            ResolutionReason::MINIMAL_VERSION_SELECTION => "MVS",
            ResolutionReason::SINGLE_VERSION_OVERRIDE => "SVO",
            ResolutionReason::MULTIPLE_VERSION_OVERRIDE => "MVO",
            ResolutionReason::NON_REGISTRY_OVERRIDE => "NRO",
        }
    }

    /// Java `Enum.name()` (also `toString()`).
    pub fn name(self) -> &'static str {
        match self {
            ResolutionReason::ORIGINAL => "ORIGINAL",
            ResolutionReason::MINIMAL_VERSION_SELECTION => "MINIMAL_VERSION_SELECTION",
            ResolutionReason::SINGLE_VERSION_OVERRIDE => "SINGLE_VERSION_OVERRIDE",
            ResolutionReason::MULTIPLE_VERSION_OVERRIDE => "MULTIPLE_VERSION_OVERRIDE",
            ResolutionReason::NON_REGISTRY_OVERRIDE => "NON_REGISTRY_OVERRIDE",
        }
    }
}

impl std::fmt::Display for ResolutionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A wrapper for `Module`, augmented with references to dependants (and also those who are not
/// used in the final dep graph).
///
/// `deps` and `unused_deps` are Guava `ImmutableBiMap`s in Java: both keys and values are unique,
/// which [`AugmentedModuleBuilder::build`] checks. Equality ignores the order of the sets and
/// maps, like Java's collection `equals`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:106 BazelModuleInspectorValue.AugmentedModule
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AugmentedModule {
    /// Name of the module. Same as in `Module`.
    pub name: String,
    /// Version of the module. Same as in `Module`.
    pub version: Version,
    /// `ModuleKey` of this module. Same as in `Module`.
    pub key: ModuleKey,
    /// The apparent name used by the module to refer to its own repository.
    pub repo_name: String,
    /// The set of modules in the resolved dep graph that depend on this module *after* the
    /// module resolution.
    pub dependants: IndexSet<ModuleKey>,
    /// The set of modules in the complete dep graph that originally depended on this module
    /// *before* the module resolution (can contain unused nodes).
    pub original_dependants: IndexSet<ModuleKey>,
    /// A bi-map from the repo names of the resolved dependencies of this module to their actual
    /// module keys.
    pub deps: IndexMap<String, ModuleKey>,
    /// A bi-map from the repo names of the unused dependencies of this module to their actual
    /// module keys.
    pub unused_deps: IndexMap<String, ModuleKey>,
    /// A map from the repo name of the dependencies of this module to the rules that were used
    /// for their resolution.
    pub dep_reasons: IndexMap<String, ResolutionReason>,
    /// Whether the module was loaded and added to the dependency graph. Modules overridden by
    /// `single_version_override` and `NonRegistryOverride` are not loaded so their
    /// `originalDeps` are (yet) unknown.
    pub loaded: bool,
}

/// `ImmutableBiMap.inverse().get(value)`.
pub(crate) fn bimap_inverse_get<'a>(
    map: &'a IndexMap<String, ModuleKey>,
    value: &ModuleKey,
) -> Option<&'a String> {
    map.iter().find(|(_, v)| *v == value).map(|(k, _)| k)
}

impl AugmentedModule {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:107 BazelModuleInspectorValue.AugmentedModule.name
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn key(&self) -> &ModuleKey {
        &self.key
    }

    pub fn repo_name(&self) -> &str {
        &self.repo_name
    }

    pub fn dependants(&self) -> &IndexSet<ModuleKey> {
        &self.dependants
    }

    pub fn original_dependants(&self) -> &IndexSet<ModuleKey> {
        &self.original_dependants
    }

    pub fn deps(&self) -> &IndexMap<String, ModuleKey> {
        &self.deps
    }

    pub fn unused_deps(&self) -> &IndexMap<String, ModuleKey> {
        &self.unused_deps
    }

    pub fn dep_reasons(&self) -> &IndexMap<String, ResolutionReason> {
        &self.dep_reasons
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Shortcut for retrieving the sorted union of both used and unused deps based on the unused
    /// flag. The result is ordered by `ModuleKey.LEXICOGRAPHIC_COMPARATOR` (an
    /// `ImmutableSortedMap<ModuleKey, String>` in Java).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:133 BazelModuleInspectorValue.AugmentedModule.getAllDeps
    pub fn get_all_deps(&self, unused: bool) -> IndexMap<ModuleKey, String> {
        let mut map: HashMap<ModuleKey, String> = HashMap::new();
        for (repo, key) in &self.deps {
            map.insert(key.clone(), repo.clone());
        }
        if unused {
            for (repo, key) in &self.unused_deps {
                map.insert(key.clone(), repo.clone());
            }
        }
        let mut entries: Vec<(ModuleKey, String)> = map.into_iter().collect();
        entries.sort_by(|a, b| ModuleKey::lexicographic_cmp(&a.0, &b.0));
        for w in entries.windows(2) {
            if ModuleKey::lexicographic_cmp(&w[0].0, &w[1].0) == Ordering::Equal {
                panic!(
                    "IllegalArgumentException: Duplicate keys in mappings {}={} and {}={}",
                    w[0].0, w[0].1, w[1].0, w[1].1
                );
            }
        }
        entries.into_iter().collect()
    }

    /// Flag for checking whether the module is present in the resolved dep graph.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:145 BazelModuleInspectorValue.AugmentedModule.isUsed
    pub fn is_used(&self) -> bool {
        self.key == ModuleKey::root() || !self.dependants.is_empty()
    }

    /// Returns a new [`AugmentedModuleBuilder`] with `key` set.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:150 BazelModuleInspectorValue.AugmentedModule.builder
    pub fn builder(key: &ModuleKey) -> AugmentedModuleBuilder {
        AugmentedModuleBuilder {
            name: key.name().to_string(),
            version: key.version().clone(),
            key: key.clone(),
            repo_name: key.name().to_string(),
            loaded: false,
            original_dependants: IndexSet::new(),
            dependants: IndexSet::new(),
            deps: Vec::new(),
            unused_deps: Vec::new(),
            dep_reasons: Vec::new(),
        }
    }
}

/// Builder type for [`AugmentedModule`] (an `@AutoBuilder`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:160 BazelModuleInspectorValue.AugmentedModule.Builder
#[derive(Debug, Clone)]
pub struct AugmentedModuleBuilder {
    name: String,
    version: Version,
    key: ModuleKey,
    repo_name: String,
    loaded: bool,
    original_dependants: IndexSet<ModuleKey>,
    dependants: IndexSet<ModuleKey>,
    deps: Vec<(String, ModuleKey)>,
    unused_deps: Vec<(String, ModuleKey)>,
    dep_reasons: Vec<(String, ResolutionReason)>,
}

impl AugmentedModuleBuilder {
    pub fn set_name(&mut self, value: impl Into<String>) -> &mut Self {
        self.name = value.into();
        self
    }

    pub fn set_version(&mut self, value: Version) -> &mut Self {
        self.version = value;
        self
    }

    pub fn set_key(&mut self, value: ModuleKey) -> &mut Self {
        self.key = value;
        self
    }

    pub fn set_repo_name(&mut self, value: impl Into<String>) -> &mut Self {
        self.repo_name = value.into();
        self
    }

    pub fn set_loaded(&mut self, value: bool) -> &mut Self {
        self.loaded = value;
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:175 BazelModuleInspectorValue.AugmentedModule.Builder.addOriginalDependant
    pub fn add_original_dependant(&mut self, dep_key: ModuleKey) -> &mut Self {
        self.original_dependants.insert(dep_key);
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:183 BazelModuleInspectorValue.AugmentedModule.Builder.addDependant
    pub fn add_dependant(&mut self, dep_key: ModuleKey) -> &mut Self {
        self.dependants.insert(dep_key);
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:191 BazelModuleInspectorValue.AugmentedModule.Builder.addDep
    pub fn add_dep(&mut self, repo_name: impl Into<String>, key: ModuleKey) -> &mut Self {
        self.deps.push((repo_name.into(), key));
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:199 BazelModuleInspectorValue.AugmentedModule.Builder.addUnusedDep
    pub fn add_unused_dep(&mut self, repo_name: impl Into<String>, key: ModuleKey) -> &mut Self {
        self.unused_deps.push((repo_name.into(), key));
        self
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:207 BazelModuleInspectorValue.AugmentedModule.Builder.addDepReason
    pub fn add_dep_reason(
        &mut self,
        repo_name: impl Into<String>,
        reason: ResolutionReason,
    ) -> &mut Self {
        self.dep_reasons.push((repo_name.into(), reason));
        self
    }

    /// `ImmutableBiMap.Builder.build()`: duplicate keys or values are an error.
    fn build_bimap(entries: &[(String, ModuleKey)]) -> IndexMap<String, ModuleKey> {
        let mut map: IndexMap<String, ModuleKey> = IndexMap::new();
        for (k, v) in entries {
            if let Some(old) = map.get(k) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {k}={old} and {k}={v}"
                );
            }
            if let Some(old_key) = bimap_inverse_get(&map, v) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same value: {old_key}={v} and {k}={v}"
                );
            }
            map.insert(k.clone(), v.clone());
        }
        map
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:212 BazelModuleInspectorValue.AugmentedModule.Builder.build
    pub fn build(&self) -> AugmentedModule {
        let mut dep_reasons: IndexMap<String, ResolutionReason> = IndexMap::new();
        for (k, v) in &self.dep_reasons {
            if let Some(old) = dep_reasons.get(k) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {k}={old} and {k}={v}"
                );
            }
            dep_reasons.insert(k.clone(), *v);
        }
        AugmentedModule {
            name: self.name.clone(),
            version: self.version.clone(),
            key: self.key.clone(),
            repo_name: self.repo_name.clone(),
            dependants: self.dependants.clone(),
            original_dependants: self.original_dependants.clone(),
            deps: Self::build_bimap(&self.deps),
            unused_deps: Self::build_bimap(&self.unused_deps),
            dep_reasons,
            loaded: self.loaded,
        }
    }
}

/// The result of running Bazel module inspection pre-processing, containing the un-pruned and
/// augmented wrappers of the Bazel module dependency graph (post-version-resolution).
///
/// `dep_graph` and the per-name sets of `modules_index` iterate in Java's `HashMap` order
/// (see [`BazelModuleInspectorFunction::compute_augmented_graph`]).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:53 BazelModuleInspectorValue
#[derive(Debug, Clone)]
pub struct BazelModuleInspectorValue {
    pub dep_graph: IndexMap<ModuleKey, AugmentedModule>,
    pub modules_index: IndexMap<String, IndexSet<ModuleKey>>,
    /// `ImmutableSetMultimap<ModuleExtensionId, String>`.
    pub extension_to_repo_internal_names: IndexMap<ModuleExtensionId, IndexSet<String>>,
    pub module_key_to_canonical_names: IndexMap<ModuleKey, RepositoryName>,
    pub errors: Vec<ExternalDepsException>,
}

impl BazelModuleInspectorValue {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorValue.java:71 BazelModuleInspectorValue.create
    pub fn create(
        dep_graph: IndexMap<ModuleKey, AugmentedModule>,
        modules_index: IndexMap<String, IndexSet<ModuleKey>>,
        extension_to_repo_internal_names: IndexMap<ModuleExtensionId, IndexSet<String>>,
        module_key_to_canonical_names: IndexMap<ModuleKey, RepositoryName>,
        errors: Vec<ExternalDepsException>,
    ) -> BazelModuleInspectorValue {
        BazelModuleInspectorValue {
            dep_graph,
            modules_index,
            extension_to_repo_internal_names,
            module_key_to_canonical_names,
            errors,
        }
    }

    pub fn dep_graph(&self) -> &IndexMap<ModuleKey, AugmentedModule> {
        &self.dep_graph
    }

    pub fn modules_index(&self) -> &IndexMap<String, IndexSet<ModuleKey>> {
        &self.modules_index
    }

    pub fn extension_to_repo_internal_names(
        &self,
    ) -> &IndexMap<ModuleExtensionId, IndexSet<String>> {
        &self.extension_to_repo_internal_names
    }

    pub fn module_key_to_canonical_names(&self) -> &IndexMap<ModuleKey, RepositoryName> {
        &self.module_key_to_canonical_names
    }

    pub fn errors(&self) -> &[ExternalDepsException] {
        &self.errors
    }
}

/// Skyframe-independent parts of `BazelModuleInspectorFunction`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:44 BazelModuleInspectorFunction
pub struct BazelModuleInspectorFunction;

/// `HashMap<ModuleKey, AugmentedModule.Builder>` with Java's iteration order.
struct BuilderMap {
    order: JavaHashMapOrder<ModuleKey>,
    builders: HashMap<ModuleKey, AugmentedModuleBuilder>,
}

impl BuilderMap {
    /// `computeIfAbsent(key, k -> init(k))`, returning the builder.
    fn compute_if_absent(
        &mut self,
        key: &ModuleKey,
        init: impl FnOnce(&ModuleKey) -> AugmentedModuleBuilder,
    ) -> &mut AugmentedModuleBuilder {
        if self
            .order
            .compute_if_absent(key, java_module_key_hash_code(key))
        {
            self.builders.insert(key.clone(), init(key));
        }
        self.builders.get_mut(key).unwrap()
    }
}

impl BazelModuleInspectorFunction {
    /// Builds the augmented (bidirectional) dep graph.
    ///
    /// PORT: java-hash-order. Java collects the builders in a `HashMap` and copies its entries
    /// into the returned `ImmutableMap`, so the result iterates in that `HashMap`'s order (Java
    /// `ModuleKey.hashCode`, `computeIfAbsent` insertion); see `crate::modcommand::java_hash`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:92 BazelModuleInspectorFunction.computeAugmentedGraph
    pub fn compute_augmented_graph(
        unpruned_dep_graph: &IndexMap<ModuleKey, InterimModule>,
        used_modules: &IndexSet<ModuleKey>,
        overrides: &IndexMap<String, ModuleOverride>,
    ) -> IndexMap<ModuleKey, AugmentedModule> {
        let mut dep_graph_augment_builder = BuilderMap {
            order: JavaHashMapOrder::new(),
            builders: HashMap::new(),
        };

        // For all Modules in the un-pruned dep graph, inspect their dependencies and add
        // themselves to their children AugmentedModule as dependant. Also fill in their own
        // AugmentedModule with a map from their dependencies to the resolution reason that was
        // applied to each. The newly created graph will also contain ModuleAugments for
        // non-loaded modules.
        for (parent_key, parent_module) in unpruned_dep_graph {
            dep_graph_augment_builder
                .compute_if_absent(parent_key, |k| {
                    let mut b = AugmentedModule::builder(k);
                    b.set_name(parent_module.get_name())
                        .set_repo_name(parent_module.repo_name.clone());
                    b
                })
                .set_version(parent_module.version.clone())
                .set_loaded(true);

            for (child_dep, dep_spec) in &parent_module.deps {
                let original_key = match parent_module.original_deps.get(child_dep) {
                    Some(d) => d.to_module_key(),
                    None => panic!("java.lang.NullPointerException"),
                };
                let original_module = unpruned_dep_graph.get(&original_key);
                let key = dep_spec.to_module_key();
                let module = unpruned_dep_graph.get(&key);

                {
                    let original_child_builder = dep_graph_augment_builder
                        .compute_if_absent(&original_key, AugmentedModule::builder);
                    if let Some(original_module) = original_module {
                        original_child_builder
                            .set_name(original_module.get_name())
                            .set_version(original_module.version.clone())
                            .set_repo_name(original_module.repo_name.clone())
                            .set_loaded(true);
                    }
                }

                // originalDependants and dependants can differ because
                // parentModule could have had originalChild in the unresolved graph, but in the
                // resolved graph the originalChild could have become orphan due to an override or
                // selection
                {
                    // Java looks up newChildBuilder before this call; both are lookups in the
                    // same map, so the order of the two computeIfAbsent calls is kept here.
                    dep_graph_augment_builder.compute_if_absent(&key, |k| {
                        let Some(module) = module else {
                            panic!("java.lang.NullPointerException");
                        };
                        let mut b = AugmentedModule::builder(k);
                        b.set_name(module.get_name())
                            .set_version(module.version.clone())
                            .set_repo_name(module.repo_name.clone())
                            .set_loaded(true);
                        b
                    });
                }
                dep_graph_augment_builder
                    .builders
                    .get_mut(&original_key)
                    .unwrap()
                    .add_original_dependant(parent_key.clone());
                // also, even if the dep has not changed, the parentModule may not be referenced
                // anymore in the resolved graph, so parentModule will only be added above
                if used_modules.contains(parent_key) {
                    dep_graph_augment_builder
                        .builders
                        .get_mut(&key)
                        .unwrap()
                        .add_dependant(parent_key.clone());
                }

                let reason = if key.version() == original_key.version() {
                    ResolutionReason::ORIGINAL
                } else {
                    match overrides.get(key.name()) {
                        Some(ModuleOverride::SingleVersion(_)) => {
                            ResolutionReason::SINGLE_VERSION_OVERRIDE
                        }
                        Some(ModuleOverride::MultipleVersion(_)) => {
                            ResolutionReason::MULTIPLE_VERSION_OVERRIDE
                        }
                        Some(ModuleOverride::NonRegistry(_)) => {
                            ResolutionReason::NON_REGISTRY_OVERRIDE
                        }
                        None => ResolutionReason::MINIMAL_VERSION_SELECTION,
                    }
                };

                let parent_builder = dep_graph_augment_builder
                    .builders
                    .get_mut(parent_key)
                    .unwrap();
                if reason != ResolutionReason::ORIGINAL {
                    parent_builder.add_unused_dep(child_dep.clone(), original_key.clone());
                }
                parent_builder.add_dep(child_dep.clone(), key.clone());
                parent_builder.add_dep_reason(child_dep.clone(), reason);
            }
        }

        // PORT: java-hash-order (see above).
        dep_graph_augment_builder
            .order
            .iteration_order()
            .into_iter()
            .map(|k| {
                let built = dep_graph_augment_builder.builders[&k].build();
                (k, built)
            })
            .collect()
    }

    /// Group all ModuleKeys seen by their module name for easy lookup (the `modulesIndex` of
    /// `compute`).
    ///
    /// PORT: java-hash-order. Java uses `Collectors.groupingBy` (a `HashMap` filled with
    /// `computeIfAbsent`, keyed by `String`) over `depGraph.values()` and copies it into an
    /// `ImmutableMap`; the per-name sets keep `depGraph`'s order.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:76 BazelModuleInspectorFunction.compute
    pub fn compute_modules_index(
        dep_graph: &IndexMap<ModuleKey, AugmentedModule>,
    ) -> IndexMap<String, IndexSet<ModuleKey>> {
        let mut order: JavaHashMapOrder<String> = JavaHashMapOrder::new();
        let mut groups: HashMap<String, IndexSet<ModuleKey>> = HashMap::new();
        for module in dep_graph.values() {
            order.compute_if_absent(&module.name, java_string_hash_code(&module.name));
            groups
                .entry(module.name.clone())
                .or_default()
                .insert(module.key.clone());
        }
        order
            .iteration_order()
            .into_iter()
            .map(|name| {
                let set = groups.remove(&name).unwrap();
                (name, set)
            })
            .collect()
    }
}
