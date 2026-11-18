//! Port of the parts of `BzlmodTestUtil` the inspector and mod command tests use
//! (`createModuleKey`, `InterimModuleBuilder`, `AugmentedModuleBuilder`,
//! `createRepositoryMapping`). `InterimModuleBuilder` is copied from
//! `crates/blaze_bzlmod_model/tests/common/mod.rs`.
#![allow(dead_code)]

use blaze_bzlmod::inspector::{
    AugmentedModule, AugmentedModuleBuilder as Builder, ResolutionReason,
};
use blaze_bzlmod_model::{
    DepSpec, InterimModule, InterimModuleBuilder as ModuleBuilder, ModuleKey, Version,
};
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use indexmap::IndexMap;

/// Simple wrapper around the `ModuleKey` constructor that takes a string version.
// Java: BzlmodTestUtil.createModuleKey
pub fn create_module_key(name: &str, version: &str) -> ModuleKey {
    ModuleKey::new(name, Version::parse(version).unwrap())
}

/// `ImmutableMap.Builder.buildOrThrow()` over entries.
pub fn map_of<V>(entries: Vec<(ModuleKey, V)>) -> IndexMap<ModuleKey, V> {
    let mut g = IndexMap::new();
    for (k, v) in entries {
        if g.insert(k.clone(), v).is_some() {
            panic!("Multiple entries with same key: {k}");
        }
    }
    g
}

/// `BzlmodTestUtil.InterimModuleBuilder`.
pub struct InterimModuleBuilder {
    builder: ModuleBuilder,
    key: ModuleKey,
    deps: IndexMap<String, DepSpec>,
    original_deps: IndexMap<String, DepSpec>,
}

fn put_or_throw(map: &mut IndexMap<String, DepSpec>, k: &str, v: DepSpec) {
    if map.insert(k.to_string(), v).is_some() {
        panic!("Multiple entries with same key: {k}");
    }
}

impl InterimModuleBuilder {
    pub fn create_v(name: &str, version: Version, compatibility_level: i32) -> Self {
        let key = ModuleKey::new(name, version.clone());
        let mut builder = InterimModule::builder();
        builder
            .set_name(name)
            .set_version(version)
            .set_key(key.clone())
            .set_compatibility_level(compatibility_level);
        InterimModuleBuilder {
            builder,
            key,
            deps: IndexMap::new(),
            original_deps: IndexMap::new(),
        }
    }

    pub fn create_cl(name: &str, version: &str, compatibility_level: i32) -> Self {
        Self::create_v(name, Version::parse(version).unwrap(), compatibility_level)
    }

    pub fn create(name: &str, version: &str) -> Self {
        Self::create_cl(name, version, 0)
    }

    pub fn add_dep(mut self, dep_repo_name: &str, key: ModuleKey) -> Self {
        put_or_throw(
            &mut self.deps,
            dep_repo_name,
            DepSpec::from_module_key(&key),
        );
        self
    }

    pub fn add_original_dep(mut self, dep_repo_name: &str, key: ModuleKey) -> Self {
        put_or_throw(
            &mut self.original_deps,
            dep_repo_name,
            DepSpec::from_module_key(&key),
        );
        self
    }

    pub fn set_key(mut self, value: ModuleKey) -> Self {
        self.key = value.clone();
        self.builder.set_key(value);
        self
    }

    pub fn build_entry(self) -> (ModuleKey, InterimModule) {
        let key = self.key.clone();
        (key, self.build())
    }

    pub fn build(mut self) -> InterimModule {
        // Copy dep entries that have not been changed to original deps.
        let mut original_deps = self.original_deps.clone();
        for (k, v) in &self.deps {
            if !self.original_deps.contains_key(k) {
                put_or_throw(&mut original_deps, k, v.clone());
            }
        }
        self.builder
            .set_deps(self.deps)
            .set_original_deps(original_deps)
            .build()
    }
}

/// `BzlmodTestUtil.AugmentedModuleBuilder`.
pub struct AugmentedModuleBuilder {
    builder: Builder,
    key: ModuleKey,
}

// Java: BzlmodTestUtil.AugmentedModuleBuilder.buildAugmentedModule(ModuleKey, String, Version, boolean)
pub fn build_augmented_module_full(
    key: ModuleKey,
    name: &str,
    version: Version,
    loaded: bool,
) -> AugmentedModuleBuilder {
    let mut builder = AugmentedModule::builder(&key);
    builder
        .set_name(name)
        .set_version(version)
        .set_repo_name(name)
        .set_loaded(loaded);
    AugmentedModuleBuilder { builder, key }
}

// Java: BzlmodTestUtil.AugmentedModuleBuilder.buildAugmentedModule(String, String, boolean)
pub fn build_augmented_module_loaded(
    name: &str,
    version: &str,
    loaded: bool,
) -> AugmentedModuleBuilder {
    let key = create_module_key(name, version);
    build_augmented_module_full(key, name, Version::parse(version).unwrap(), loaded)
}

// Java: BzlmodTestUtil.AugmentedModuleBuilder.buildAugmentedModule(String, String)
pub fn build_augmented_module(name: &str, version: &str) -> AugmentedModuleBuilder {
    build_augmented_module_loaded(name, version, true)
}

// Java: BzlmodTestUtil.AugmentedModuleBuilder.buildAugmentedModule(ModuleKey, String)
pub fn build_augmented_module_key(key: ModuleKey, name: &str) -> AugmentedModuleBuilder {
    let version = key.version().clone();
    build_augmented_module_full(key, name, version, true)
}

impl AugmentedModuleBuilder {
    pub fn add_changed_dep(
        mut self,
        name: &str,
        version: &str,
        old_version: &str,
        reason: ResolutionReason,
    ) -> Self {
        self.builder
            .add_dep(name, create_module_key(name, version))
            .add_unused_dep(name, create_module_key(name, old_version))
            .add_dep_reason(name, reason);
        self
    }

    pub fn add_changed_dep_repo(
        mut self,
        repo_name: &str,
        module_name: &str,
        version: &str,
        old_version: &str,
        reason: ResolutionReason,
    ) -> Self {
        self.builder
            .add_dep(repo_name, create_module_key(module_name, version))
            .add_unused_dep(repo_name, create_module_key(module_name, old_version))
            .add_dep_reason(repo_name, reason);
        self
    }

    pub fn add_dep(mut self, name: &str, version: &str) -> Self {
        self.builder
            .add_dep(name, create_module_key(name, version))
            .add_dep_reason(name, ResolutionReason::ORIGINAL);
        self
    }

    pub fn add_dep_repo(mut self, repo_name: &str, module_name: &str, version: &str) -> Self {
        self.builder
            .add_dep(repo_name, create_module_key(module_name, version))
            .add_dep_reason(repo_name, ResolutionReason::ORIGINAL);
        self
    }

    pub fn add_dependant(mut self, name: &str, version: &str) -> Self {
        self.builder.add_dependant(create_module_key(name, version));
        self
    }

    pub fn add_dependant_key(mut self, key: ModuleKey) -> Self {
        self.builder.add_dependant(key);
        self
    }

    pub fn add_original_dependant(mut self, name: &str, version: &str) -> Self {
        self.builder
            .add_original_dependant(create_module_key(name, version));
        self
    }

    pub fn add_original_dependant_key(mut self, key: ModuleKey) -> Self {
        self.builder.add_original_dependant(key);
        self
    }

    pub fn add_still_dependant(mut self, name: &str, version: &str) -> Self {
        self.builder
            .add_original_dependant(create_module_key(name, version));
        self.builder.add_dependant(create_module_key(name, version));
        self
    }

    pub fn add_still_dependant_key(mut self, key: ModuleKey) -> Self {
        self.builder.add_original_dependant(key.clone());
        self.builder.add_dependant(key);
        self
    }

    pub fn build_entry(self) -> (ModuleKey, AugmentedModule) {
        (self.key, self.builder.build())
    }
}

// Java: BzlmodTestUtil.createRepositoryMapping
pub fn create_repository_mapping(key: &ModuleKey, names: &[&str]) -> RepositoryMapping {
    let mut entries = Vec::new();
    for pair in names.chunks(2) {
        entries.push((pair[0], RepositoryName::create_unvalidated(pair[1])));
    }
    RepositoryMapping::create(entries, key.get_canonical_repo_name_without_version())
}

/// `Files.readAllLines` of what was written: lines split on `\n`, without a trailing empty
/// element for a final line terminator.
pub fn read_all_lines(bytes: &[u8]) -> Vec<String> {
    let s = String::from_utf8(bytes.to_vec()).unwrap();
    let mut lines: Vec<String> = s.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}
