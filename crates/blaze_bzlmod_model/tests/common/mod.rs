//! Port of `BzlmodTestUtil` (the parts that do not need Starlark).
#![allow(dead_code)]

use std::sync::Arc;

use blaze_bzlmod_model::{
    DepSpec, InterimModule, InterimModuleBuilder as Builder, Module, ModuleExtensionUsage,
    ModuleKey, Registry, Version,
};
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use indexmap::IndexMap;

/// Simple wrapper around the `ModuleKey` constructor that takes a string version.
// Java: BzlmodTestUtil.createModuleKey
pub fn create_module_key(name: &str, version: &str) -> ModuleKey {
    ModuleKey::new(name, Version::parse(version).unwrap())
}

// Java: BzlmodTestUtil.createDepSpec
pub fn create_dep_spec(name: &str, version: &str, max_compatibility_level: i32) -> DepSpec {
    DepSpec::new(
        name,
        Version::parse(version).unwrap(),
        max_compatibility_level,
    )
}

// Java: BzlmodTestUtil.buildModule
pub fn build_module(name: &str, version: &str) -> Module {
    Module::new(
        name,
        Version::parse(version).unwrap(),
        create_module_key(name, version),
    )
}

/// `BzlmodTestUtil.InterimModuleBuilder`.
pub struct InterimModuleBuilder {
    builder: Builder,
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

    pub fn add_dep_spec(mut self, dep_repo_name: &str, dep_spec: DepSpec) -> Self {
        put_or_throw(&mut self.deps, dep_repo_name, dep_spec);
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

    pub fn add_original_dep_spec(mut self, dep_repo_name: &str, dep_spec: DepSpec) -> Self {
        put_or_throw(&mut self.original_deps, dep_repo_name, dep_spec);
        self
    }

    pub fn add_nodep_dep(mut self, key: ModuleKey) -> Self {
        self.builder.add_nodep_dep(DepSpec::from_module_key(&key));
        self
    }

    pub fn set_key(mut self, value: ModuleKey) -> Self {
        self.key = value.clone();
        self.builder.set_key(value);
        self
    }

    pub fn set_repo_name(mut self, value: &str) -> Self {
        self.builder.set_repo_name(value);
        self
    }

    pub fn set_registry(mut self, value: Arc<dyn Registry>) -> Self {
        self.builder.set_registry(Some(value));
        self
    }

    pub fn add_execution_platforms_to_register(mut self, value: &[&str]) -> Self {
        self.builder
            .add_execution_platforms_to_register(value.iter().copied());
        self
    }

    pub fn add_toolchains_to_register(mut self, value: &[&str]) -> Self {
        self.builder
            .add_toolchains_to_register(value.iter().copied());
        self
    }

    pub fn add_flag_alias(mut self, native_flag: &str, starlark_flag: &str) -> Self {
        self.builder.add_flag_alias(native_flag, starlark_flag);
        self
    }

    pub fn add_extension_usage(mut self, value: ModuleExtensionUsage) -> Self {
        self.builder.add_extension_usage(value);
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

// Java: BzlmodTestUtil.createRepositoryMapping
pub fn create_repository_mapping(key: &ModuleKey, names: &[&str]) -> RepositoryMapping {
    let mut entries = Vec::new();
    for pair in names.chunks(2) {
        entries.push((pair[0], RepositoryName::create_unvalidated(pair[1])));
    }
    RepositoryMapping::create(entries, key.get_canonical_repo_name_without_version())
}

/// An ordered dep graph from entries.
pub fn graph(entries: Vec<(ModuleKey, InterimModule)>) -> IndexMap<ModuleKey, InterimModule> {
    let mut g = IndexMap::new();
    for (k, v) in entries {
        if g.insert(k.clone(), v).is_some() {
            panic!("duplicate key {k}");
        }
    }
    g
}
