//! Shared harness for the bzlmod Skyframe tests: ports of `FakeRegistry` (with its `Factory`),
//! the parts of `BzlmodTestUtil` the tests use, and the evaluator setup of
//! `ModuleFileFunctionTest.setUpWithBuiltinModules`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use blaze_bzlmod::skyframe::{
    BuiltinRepositoryDirectoryLookup, BzlmodConfig, register_bzlmod_functions,
    register_lockfile_only_extension_functions,
};
use blaze_bzlmod_model::gson::JsonWriter;
use blaze_bzlmod_model::{
    AttributeValuesAdapter, BazelCompatibilityMode, CheckDirectDepsMode, Checksum, DepSpec,
    FileRegistryFetcher, InterimModule, InterimModuleBuilder as Builder, LockfileMode, ModuleFile,
    ModuleKey, ModuleOverride, NonRegistryOverride, Registry, RegistryException, RegistryFactory,
    RegistryFetcher, RegistryFileDownloadEvent, RepoSpec, UriSyntaxException, Version,
    YankedVersionsValue, local_path_repo_spec,
};
use blaze_events::{EventKind, ExtendedEventHandler, StoredEventHandler};
use blaze_skyframe::{EvaluationContext, EvaluationResult, Evaluator, EvaluatorBuilder, SkyKey};
use blaze_starlark::StarlarkSemantics;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

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
            .set_key(key)
            .set_compatibility_level(compatibility_level);
        InterimModuleBuilder {
            builder,
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

/// Fake implementation of [`Registry`], where modules can be freely added and stored in memory.
/// The contents of the modules are expected to be located under a given file path as
/// subdirectories.
// Java: com/google/devtools/build/lib/bazel/bzlmod/FakeRegistry.java (src/test) FakeRegistry
#[derive(Debug)]
pub struct FakeRegistry {
    url: String,
    root_path: String,
    modules: Mutex<HashMap<ModuleKey, String>>,
    yanked_version_map: Mutex<HashMap<String, IndexMap<Version, String>>>,
}

impl FakeRegistry {
    pub fn new(url: &str, root_path: &str) -> FakeRegistry {
        FakeRegistry {
            url: url.to_string(),
            root_path: root_path.to_string(),
            modules: Mutex::new(HashMap::new()),
            yanked_version_map: Mutex::new(HashMap::new()),
        }
    }

    /// The lines are joined with '\n'.
    pub fn add_module(self: &Arc<Self>, key: ModuleKey, module_file_lines: &[&str]) -> Arc<Self> {
        self.modules
            .lock()
            .unwrap()
            .insert(key, module_file_lines.join("\n"));
        self.clone()
    }

    pub fn add_yanked_version(
        self: &Arc<Self>,
        module_name: &str,
        yanked_versions: IndexMap<Version, String>,
    ) -> Arc<Self> {
        self.yanked_version_map
            .lock()
            .unwrap()
            .insert(module_name.to_string(), yanked_versions);
        self.clone()
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }
}

/// `GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON.toJson(repoSpec)`.
fn repo_spec_json(spec: &RepoSpec) -> String {
    let mut out = JsonWriter::new(false);
    out.begin_object();
    out.name("repoRuleId");
    match spec.repo_rule_id() {
        None => {
            out.null_value();
        }
        Some(id) => {
            out.value_str(&id.to_string());
        }
    }
    out.name("attributes");
    AttributeValuesAdapter::write(&mut out, spec.attributes());
    out.end_object();
    out.into_string()
}

impl Registry for FakeRegistry {
    fn get_url(&self) -> String {
        self.url.clone()
    }

    fn get_module_file(
        &self,
        key: &ModuleKey,
        event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<ModuleFile, RegistryException> {
        let uri = format!(
            "{}/modules/{}/{}/MODULE.bazel",
            self.url,
            key.name(),
            key.version()
        );
        let maybe_content = self
            .modules
            .lock()
            .unwrap()
            .get(key)
            .map(|v| v.as_bytes().to_vec());
        event_handler.post(Arc::new(RegistryFileDownloadEvent::create(
            uri.clone(),
            maybe_content.as_deref(),
        )));
        match maybe_content {
            None => Err(RegistryException::NotFound(format!(
                "module not found: {key}"
            ))),
            Some(content) => Ok(ModuleFile::create(content, uri)),
        }
    }

    fn get_repo_spec(
        &self,
        key: &ModuleKey,
        _module_file_hashes: &IndexMap<String, Option<Checksum>>,
        event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<RepoSpec, RegistryException> {
        let repo_spec = local_path_repo_spec(&format!(
            "{}/{}",
            self.root_path,
            key.get_canonical_repo_name_with_version().get_name()
        ));
        event_handler.post(Arc::new(RegistryFileDownloadEvent::create(
            format!(
                "{}/modules/{}/{}/source.json",
                self.url,
                key.name(),
                key.version()
            ),
            Some(repo_spec_json(&repo_spec).as_bytes()),
        )));
        Ok(repo_spec)
    }

    fn get_yanked_versions(
        &self,
        module_name: &str,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<IndexMap<Version, String>>, RegistryException> {
        Ok(self
            .yanked_version_map
            .lock()
            .unwrap()
            .get(module_name)
            .cloned())
    }

    fn try_get_yanked_versions_from_lockfile(
        &self,
        _selected_module_key: &ModuleKey,
    ) -> Option<YankedVersionsValue> {
        None
    }
}

/// Fake [`RegistryFactory`] that only supports [`FakeRegistry`].
// Java: FakeRegistry.Factory
#[derive(Default)]
pub struct FakeRegistryFactory {
    num_fakes: AtomicUsize,
    registries: Mutex<HashMap<String, Arc<FakeRegistry>>>,
}

impl FakeRegistryFactory {
    pub fn new_fake_registry(&self, root_path: &str) -> Arc<FakeRegistry> {
        let n = self.num_fakes.fetch_add(1, Ordering::SeqCst);
        let registry = Arc::new(FakeRegistry::new(&format!("fake:{n}"), root_path));
        self.registries
            .lock()
            .unwrap()
            .insert(registry.url(), registry.clone());
        registry
    }
}

impl RegistryFactory for FakeRegistryFactory {
    fn create_registry(
        &self,
        url: &str,
        _lockfile_mode: LockfileMode,
        _file_hashes: &IndexMap<String, Option<Checksum>>,
        _previously_selected_yanked_versions: &IndexMap<ModuleKey, String>,
        _vendor_dir: Option<PathFragment>,
        _module_mirrors: &IndexSet<String>,
    ) -> Result<Arc<dyn Registry>, UriSyntaxException> {
        let registry = self
            .registries
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or_else(|| panic!("unknown registry url: {url}"));
        Ok(registry)
    }
}

/// The evaluator setup of `ModuleFileFunctionTest` (`FoundationTestCase` + the injected
/// precomputed values), over a temporary workspace and output base.
pub struct Harness {
    _workspace: tempfile::TempDir,
    _output_base: tempfile::TempDir,
    pub root_directory: PathBuf,
    pub output_base: PathBuf,
    pub registry_factory: Arc<FakeRegistryFactory>,
    pub builtin_modules: IndexMap<String, NonRegistryOverride>,
    pub registries: IndexSet<String>,
    pub ignore_dev_deps: bool,
    pub module_overrides: IndexMap<String, ModuleOverride>,
    pub allowed_yanked_versions: Vec<String>,
    pub lockfile_mode: LockfileMode,
    pub check_direct_dependencies: CheckDirectDepsMode,
    pub bazel_compatibility_mode: BazelCompatibilityMode,
    /// `BlazeVersionInfo.instance().getVersion()`: empty in tests unless embedded.
    pub bazel_version: String,
    pub client_env: IndexMap<String, String>,
    /// `PrecomputedValue.STARLARK_SEMANTICS`.
    pub starlark_semantics: StarlarkSemantics,
    pub registry_fetcher: Arc<dyn RegistryFetcher>,
    /// The reporter.
    pub events: Arc<StoredEventHandler>,
}

impl Harness {
    // Java: ModuleFileFunctionTest.setup
    pub fn new() -> Harness {
        Self::set_up_with_builtin_modules(IndexMap::new())
    }

    // Java: ModuleFileFunctionTest.setUpWithBuiltinModules
    pub fn set_up_with_builtin_modules(
        builtin_modules: IndexMap<String, NonRegistryOverride>,
    ) -> Harness {
        let workspace = tempfile::tempdir().unwrap();
        let output_base = tempfile::tempdir().unwrap();
        Harness {
            root_directory: workspace.path().canonicalize().unwrap(),
            output_base: output_base.path().canonicalize().unwrap(),
            _workspace: workspace,
            _output_base: output_base,
            registry_factory: Arc::new(FakeRegistryFactory::default()),
            builtin_modules,
            registries: IndexSet::new(),
            ignore_dev_deps: false,
            module_overrides: IndexMap::new(),
            allowed_yanked_versions: Vec::new(),
            lockfile_mode: LockfileMode::Update,
            check_direct_dependencies: CheckDirectDepsMode::Warning,
            bazel_compatibility_mode: BazelCompatibilityMode::Error,
            bazel_version: String::new(),
            client_env: IndexMap::new(),
            starlark_semantics: StarlarkSemantics::default(),
            registry_fetcher: Arc::new(FileRegistryFetcher),
            events: Arc::new(StoredEventHandler::new()),
        }
    }

    /// `ModuleFileFunction.REGISTRIES.set(differencer, ImmutableSet.of(urls...))`.
    pub fn set_registries(&mut self, urls: &[String]) {
        self.registries = urls.iter().cloned().collect();
    }

    /// `scratch.overwriteFile(rootDirectory.getRelative(path), lines...)`: each line followed by
    /// a newline.
    pub fn scratch_file(&self, path: &str, lines: &[&str]) -> PathBuf {
        let p = self.root_directory.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(&p, content).unwrap();
        p
    }

    pub fn config(&self) -> BzlmodConfig {
        BzlmodConfig {
            workspace_root: self.root_directory.clone(),
            output_base: self.output_base.clone(),
            install_base: self.output_base.join("install"),
            starlark_semantics: self.starlark_semantics.clone(),
            registries: self.registries.clone(),
            module_mirrors: IndexMap::new(),
            ignore_dev_deps: self.ignore_dev_deps,
            module_overrides: self.module_overrides.clone(),
            injected_repositories: IndexMap::new(),
            check_direct_dependencies: self.check_direct_dependencies,
            bazel_compatibility_mode: self.bazel_compatibility_mode,
            lockfile_mode: self.lockfile_mode,
            allowed_yanked_versions: self.allowed_yanked_versions.clone(),
            vendor_directory: None,
            last_registry_invalidation: SystemTime::UNIX_EPOCH,
            client_env: Arc::new(self.client_env.clone()),
            registry_fetcher: self.registry_fetcher.clone(),
            registry_factory: self.registry_factory.clone(),
            bazel_version: self.bazel_version.clone(),
            builtin_modules: self.builtin_modules.clone(),
            repository_directory_lookup: Arc::new(BuiltinRepositoryDirectoryLookup {
                embedded_tools: self.output_base.join("install/embedded_tools"),
                workspace_root: self.root_directory.clone(),
            }),
        }
    }

    pub fn evaluator(&self) -> Evaluator {
        self.evaluator_with(|b| b)
    }

    /// An evaluator with extra registrations or injections.
    pub fn evaluator_with(
        &self,
        f: impl FnOnce(EvaluatorBuilder) -> EvaluatorBuilder,
    ) -> Evaluator {
        let builder = register_bzlmod_functions(EvaluatorBuilder::new(), Arc::new(self.config()));
        let builder = register_lockfile_only_extension_functions(builder);
        f(builder).build()
    }

    pub fn evaluation_context(&self) -> EvaluationContext {
        EvaluationContext {
            keep_going: false,
            parallelism: 8,
            event_handler: self.events.clone(),
            ..EvaluationContext::default()
        }
    }

    /// `evaluator.evaluate(keys, evaluationContext)` on a fresh evaluator.
    pub fn evaluate(&self, keys: &[SkyKey]) -> EvaluationResult {
        self.evaluator()
            .evaluate(keys, &self.evaluation_context())
            .expect("not interrupted")
    }

    pub fn event_messages(&self) -> Vec<String> {
        self.events
            .get_events()
            .iter()
            .map(|e| e.get_message().into_owned())
            .collect()
    }

    pub fn event_messages_of(&self, kind: EventKind) -> Vec<String> {
        self.events
            .get_events()
            .iter()
            .filter(|e| e.get_kind() == kind)
            .map(|e| e.get_message().into_owned())
            .collect()
    }

    // Java: FoundationTestCase.assertContainsEvent
    pub fn assert_contains_event(&self, text: &str) {
        let messages = self.event_messages();
        assert!(
            messages.iter().any(|m| m.contains(text)),
            "no event contains {text:?}; events: {messages:#?}"
        );
    }

    // Java: FoundationTestCase.assertDoesNotContainEvent
    pub fn assert_does_not_contain_event(&self, text: &str) {
        let messages = self.event_messages();
        assert!(
            !messages.iter().any(|m| m.contains(text)),
            "an event contains {text:?}; events: {messages:#?}"
        );
    }
}

/// `result.getError().toString()`.
pub fn error_string(result: &EvaluationResult) -> String {
    match result.get_first_error() {
        Some(e) => format!("{e:?}"),
        None => panic!("expected an error, got {result:?}"),
    }
}

/// `if (result.hasError()) fail(result.getError().toString())`.
pub fn assert_no_error(result: &EvaluationResult) {
    if result.has_error() {
        panic!("{}", error_string(result));
    }
}

pub fn write_file(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}
