//! Shared helpers for the MODULE.bazel evaluation tests: `BzlmodTestUtil` builders (copied from
//! blaze_bzlmod_model/tests/common) and a harness that runs the evaluation steps of
//! `ModuleFileFunction` (parse/compile, `execModuleFile`, `buildModule`, `buildOverrides`).
#![allow(dead_code)]

use std::sync::Arc;

use blaze_bzlmod::module_file::{CompiledModuleFile, exec_module_file};
use blaze_bzlmod_model::{
    DepSpec, ExternalDepsCode, ExternalDepsException, InterimModule,
    InterimModuleBuilder as Builder, ModuleExtensionUsage, ModuleFile, ModuleKey, ModuleOverride,
    NonRegistryOverride, Version,
};
use blaze_cmdline::Label;
use blaze_events::{Event, EventHandler, StoredEventHandler};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
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
    pub fn create_cl(name: &str, version: &str, compatibility_level: i32) -> Self {
        let version = Version::parse(version).unwrap();
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

/// The location string of the root MODULE.bazel in `ModuleFileFunctionTest`.
pub const ROOT_MODULE_FILE: &str = "/workspace/MODULE.bazel";

/// The location string `FakeRegistry` gives a module file.
pub fn registry_module_file_location(key: &ModuleKey) -> String {
    format!(
        "fake:0/modules/{}/{}/MODULE.bazel",
        key.name(),
        key.version()
    )
}

/// The parts of `RootModuleFileValue` that come from evaluating the file.
pub struct RootModuleResult {
    pub module: InterimModule,
    pub overrides: IndexMap<String, ModuleOverride>,
}

/// Runs the MODULE.bazel evaluation steps of `ModuleFileFunction` without Skyframe.
pub struct ModuleFileHarness {
    pub semantics: StarlarkSemantics,
    pub builtin_modules: IndexMap<String, NonRegistryOverride>,
    pub ignore_dev_deps: bool,
    pub injected_repositories: IndexMap<String, PathFragment>,
    pub events: StoredEventHandler,
}

impl ModuleFileHarness {
    /// `ModuleFileFunctionTest.setup`: no builtin modules, default semantics, dev deps kept.
    pub fn new() -> Self {
        ModuleFileHarness {
            semantics: StarlarkSemantics::default(),
            builtin_modules: IndexMap::new(),
            ignore_dev_deps: false,
            injected_repositories: IndexMap::new(),
            events: StoredEventHandler::new(),
        }
    }

    fn compile(
        &self,
        content: &str,
        location: &str,
        key: &ModuleKey,
    ) -> Result<CompiledModuleFile, ExternalDepsException> {
        CompiledModuleFile::parse_and_compile(
            ModuleFile::create(content.as_bytes().to_vec(), location),
            key,
            &self.semantics,
            &self.events,
        )
    }

    /// The root module: `content` is /workspace/MODULE.bazel, `includes` are
    /// `(label, content)` pairs of the files it may include (all compiled up front, as
    /// `ModuleFileFunction` does for the files reachable through `include()`).
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:565 ModuleFileFunction.computeForRootModule (evaluation part)
    pub fn eval_root(
        &self,
        content: &str,
        includes: &[(&str, &str)],
    ) -> Result<RootModuleResult, ExternalDepsException> {
        let root = ModuleKey::root();
        let compiled = self.compile(content, ROOT_MODULE_FILE, &root)?;
        let mut include_label_to_compiled_module_file = IndexMap::new();
        for (label, include_content) in includes {
            let path = Label::parse_canonical_unchecked(label).to_path_fragment();
            let location = format!("/workspace/{}", path.get_path_string());
            include_label_to_compiled_module_file.insert(
                (*label).to_owned(),
                self.compile(include_content, &location, &root)?,
            );
        }
        let mut context = exec_module_file(
            &compiled,
            Some(Arc::new(include_label_to_compiled_module_file)),
            &root,
            self.ignore_dev_deps,
            &self.builtin_modules,
            &self.injected_repositories,
            /* print_is_noop= */ false,
            &Arc::new(self.semantics.clone()),
            &self.events,
        )?;
        let module = match context.build_module(None) {
            Ok(module) => module,
            Err(e) => {
                self.events.handle(&Event::error_at(
                    e.innermost_location.clone(),
                    e.message_with_stack.clone(),
                ));
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    "error executing MODULE.bazel file for the root module",
                ));
            }
        };
        let overrides = context.build_overrides();
        Ok(RootModuleResult { module, overrides })
    }

    /// A module from a registry: dev dependencies ignored, `print` a no-op, no includes.
    // PORT: the registry is not passed to `buildModule` (it is `null` here); expected modules
    // are built without `setRegistry`.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:235 ModuleFileFunction.compute (evaluation part)
    pub fn eval_registry_module(
        &self,
        key: &ModuleKey,
        content: &str,
    ) -> Result<InterimModule, ExternalDepsException> {
        let compiled = self.compile(content, &registry_module_file_location(key), key)?;
        let mut context = exec_module_file(
            &compiled,
            None,
            key,
            /* ignore_dev_deps= */ true,
            &self.builtin_modules,
            &IndexMap::new(),
            /* print_is_noop= */ true,
            &Arc::new(self.semantics.clone()),
            &self.events,
        )?;
        match context.build_module(None) {
            Ok(module) => Ok(module),
            Err(e) => {
                self.events.handle(&Event::error_at(
                    e.innermost_location.clone(),
                    e.message_with_stack.clone(),
                ));
                Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!("error executing MODULE.bazel file for {key}"),
                ))
            }
        }
    }

    /// `assertContainsEvent`: some event's `toString()` contains `expected`.
    pub fn assert_contains_event(&self, expected: &str) {
        let events = self.events.get_events();
        assert!(
            events.iter().any(|e| e.to_string().contains(expected)),
            "no event contains {expected:?}; events:\n{}",
            events
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
