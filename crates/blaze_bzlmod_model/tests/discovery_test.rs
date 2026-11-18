//! Port of `DiscoveryTest` (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).
//!
//! Java evaluates real MODULE.bazel text through `ModuleFileFunction` and a `FakeRegistry`. Here
//! the registry is a port of `FakeRegistry` (it serves the same module file text, so the recorded
//! hashes are the same), and [`FakeModuleFileLoader`] stands in for `ModuleFileFunction`: it picks
//! the registry the same way (override registry, else the first registry that has the module) and
//! returns the `InterimModule` that evaluating that text produces, written out by hand.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use blaze_bzlmod_model::{
    Checksum, Discovery, DiscoveryResult, ExternalDepsCode, ExternalDepsException, InterimModule,
    ModuleFile, ModuleFileLoader, ModuleKey, ModuleOverride, NonRootModuleFileValue, Registry,
    RegistryException, RegistryFetcher, RegistryFileDownloadEvent, RepoSpec, RootModuleFileValue,
    SingleVersionOverride, Version, YankedVersionsValue, local_path_repo_spec,
};
use blaze_events::{ExtendedEventHandler, StoredEventHandler};
use common::*;
use indexmap::IndexMap;

/// Port of `FakeRegistry`: serves module files by key.
#[derive(Debug)]
struct FakeRegistry {
    url: String,
    root_path: String,
    modules: HashMap<ModuleKey, String>,
}

impl FakeRegistry {
    fn add_module(mut self, key: ModuleKey, module_file_lines: &[&str]) -> Self {
        self.modules.insert(key, module_file_lines.join("\n"));
        self
    }
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
        let maybe_content = self.modules.get(key).map(|s| s.as_bytes().to_vec());
        event_handler.post(Arc::new(RegistryFileDownloadEvent::create(
            uri.clone(),
            maybe_content.as_deref(),
        )));
        match maybe_content {
            None => Err(RegistryException::NotFound(format!(
                "module not found: {key}"
            ))),
            Some(c) => Ok(ModuleFile::create(c, uri)),
        }
    }

    fn get_repo_spec(
        &self,
        key: &ModuleKey,
        _module_file_hashes: &IndexMap<String, Option<Checksum>>,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<RepoSpec, RegistryException> {
        Ok(local_path_repo_spec(&format!(
            "{}/{}",
            self.root_path,
            key.get_canonical_repo_name_with_version().get_name()
        )))
    }

    fn get_yanked_versions(
        &self,
        _module_name: &str,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<IndexMap<Version, String>>, RegistryException> {
        Ok(None)
    }

    fn try_get_yanked_versions_from_lockfile(
        &self,
        _selected_module_key: &ModuleKey,
    ) -> Option<YankedVersionsValue> {
        None
    }
}

/// Port of `FakeRegistry.Factory`: names registries `fake:0`, `fake:1`, ...
#[derive(Default)]
struct FakeRegistryFactory {
    num_fakes: usize,
}

impl FakeRegistryFactory {
    fn new_fake_registry(&mut self, root_path: &str) -> FakeRegistry {
        let url = format!("fake:{}", self.num_fakes);
        self.num_fakes += 1;
        FakeRegistry {
            url,
            root_path: root_path.to_string(),
            modules: HashMap::new(),
        }
    }
}

struct NoFetcher;

impl RegistryFetcher for NoFetcher {
    fn download_and_read_one_url_for_bzlmod(
        &self,
        url: &blaze_bzlmod_model::JavaUri,
        _client_env: &IndexMap<String, String>,
        _checksum: Option<&Checksum>,
    ) -> Result<Vec<u8>, RegistryException> {
        panic!("unexpected download of {url}")
    }
}

/// Stands in for `ModuleFileFunction` on non-root modules.
struct FakeModuleFileLoader {
    /// `ModuleFileFunction.REGISTRIES`.
    registries: Vec<Arc<FakeRegistry>>,
    /// Every registry the root module may name in an override.
    all_registries: Vec<Arc<FakeRegistry>>,
    root_overrides: IndexMap<String, ModuleOverride>,
    /// (registry url, module key) -> what evaluating that registry's module file produces
    /// (without the registry field, which the loader fills in).
    evaluated: HashMap<(String, ModuleKey), InterimModule>,
}

impl FakeModuleFileLoader {
    fn new(
        registries: Vec<Arc<FakeRegistry>>,
        all_registries: Vec<Arc<FakeRegistry>>,
        root_overrides: IndexMap<String, ModuleOverride>,
    ) -> Self {
        FakeModuleFileLoader {
            registries,
            all_registries,
            root_overrides,
            evaluated: HashMap::new(),
        }
    }

    fn evaluates(mut self, registry: &Arc<FakeRegistry>, module: InterimModule) -> Self {
        self.evaluated
            .insert((registry.get_url(), module.key.clone()), module);
        self
    }
}

impl ModuleFileLoader for FakeModuleFileLoader {
    // Mirrors ModuleFileFunction.getModuleFileFromRegistry.
    fn get_module_file(
        &self,
        key: &ModuleKey,
    ) -> Result<NonRootModuleFileValue, ExternalDepsException> {
        let mut registries = self.registries.clone();
        if let Some(r) = self
            .root_overrides
            .get(key.name())
            .and_then(|o| o.get_registry())
            .filter(|r| !r.is_empty())
        {
            registries = self
                .all_registries
                .iter()
                .filter(|reg| reg.get_url() == r)
                .cloned()
                .collect();
        }
        let download_event_handler = StoredEventHandler::new();
        let mut not_found_trace = Vec::new();
        for registry in &registries {
            match registry.get_module_file(key, &download_event_handler, &NoFetcher) {
                Err(RegistryException::NotFound(m)) => {
                    not_found_trace.push(m);
                    continue;
                }
                Err(e) => panic!("{e}"),
                Ok(_) => {}
            }
            let mut module = self.evaluated[&(registry.get_url(), key.clone())].clone();
            let registry: Arc<dyn Registry> = registry.clone();
            module.registry = Some(registry);
            return Ok(NonRootModuleFileValue {
                module,
                registry_file_hashes: RegistryFileDownloadEvent::collect_to_map(
                    &download_event_handler.get_posts(),
                ),
            });
        }
        Err(ExternalDepsException::with_message(
            ExternalDepsCode::ModuleNotFound,
            format!(
                "module {key} not found in registries:\n* {}",
                not_found_trace.join("\n* ")
            ),
        ))
    }
}

fn root(module: InterimModule) -> RootModuleFileValue {
    RootModuleFileValue::new(module, IndexMap::new())
}

/// `DiscoveryValue.registryFileHashes` uses checksum strings.
fn hashes(result: &DiscoveryResult) -> Vec<(String, Option<String>)> {
    result
        .registry_file_hashes
        .iter()
        .map(|(k, v)| (k.clone(), v.as_ref().map(|c| c.to_string())))
        .collect()
}

fn hash_keys(result: &DiscoveryResult) -> Vec<String> {
    let mut keys: Vec<String> = result.registry_file_hashes.keys().cloned().collect();
    keys.sort();
    keys
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// `assertThat(depGraph.entrySet()).containsExactly(...)` (any order).
fn assert_dep_graph(result: &DiscoveryResult, expected: Vec<(ModuleKey, InterimModule)>) {
    let actual = &result.dep_graph;
    assert_eq!(actual.len(), expected.len(), "{actual:#?}");
    for (k, v) in &expected {
        assert_eq!(actual.get(k), Some(v), "entry {k}");
    }
}

fn reg(r: &Arc<FakeRegistry>) -> Arc<dyn Registry> {
    r.clone()
}

#[test]
fn test_simple_diamond() {
    // module(name='aaa',version='0.1')
    // bazel_dep(name='bbb',version='1.0')
    // bazel_dep(name='ccc',version='2.0')
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_dep("ccc", create_module_key("ccc", "2.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0');bazel_dep(name='ddd',version='3.0')"],
            )
            .add_module(
                create_module_key("ccc", "2.0"),
                &["module(name='ccc', version='2.0');bazel_dep(name='ddd',version='3.0')"],
            )
            .add_module(
                create_module_key("ddd", "3.0"),
                // Add a random override here; it should be ignored
                &["module(name='ddd', version='3.0');local_path_override(module_name='ff',path='f')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ddd", create_module_key("ddd", "3.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd", create_module_key("ddd", "3.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ddd", "3.0").build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ddd", create_module_key("ddd", "3.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd", create_module_key("ddd", "3.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ddd", "3.0")
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
    let url = registry.get_url();
    assert_eq!(
        hashes(&discovery_value),
        vec![
            (
                format!("{url}/modules/bbb/1.0/MODULE.bazel"),
                Some("3f48e6d8694e0aa0d16617fd97b7d84da0e17ee9932c18cbc71888c12563372d".into())
            ),
            (
                format!("{url}/modules/ccc/2.0/MODULE.bazel"),
                Some("e613d4192495192c3d46ee444dc9882a176a9e7a243d1b5a840ab0f01553e8d6".into())
            ),
            (
                format!("{url}/modules/ddd/3.0/MODULE.bazel"),
                Some("f80d91453520d193b0b79f1501eb902b5b01a991762cc7fb659fc580b95648fd".into())
            ),
        ]
    );
}

#[test]
#[ignore = "needs ModuleFileFunction (Starlark): dev_dependency handling happens while evaluating MODULE.bazel"]
fn test_dev_dependency() {}

#[test]
#[ignore = "needs ModuleFileFunction (Starlark): --ignore_dev_dependency is applied while evaluating MODULE.bazel"]
fn test_ignore_dev_dependency() {}

#[test]
fn test_nodep_unfulfilled() {
    // bazel_dep(name='ccc',version='1.0',repo_name=None) is a nodep dep.
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_nodep_dep(create_module_key("ccc", "1.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &["module(name='ccc', version='1.0')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "1.0").build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
    let url = registry.get_url();
    assert_eq!(
        hash_keys(&discovery_value),
        vec![format!("{url}/modules/bbb/1.0/MODULE.bazel")]
    );
}

#[test]
fn test_nodep_fulfilled() {
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_nodep_dep(create_module_key("ccc", "2.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0');bazel_dep(name='ccc',version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &["module(name='ccc', version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "2.0"),
                &["module(name='ccc', version='2.0')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "2.0").build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .add_nodep_dep(create_module_key("ccc", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "1.0")
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
    let url = registry.get_url();
    assert_eq!(
        hash_keys(&discovery_value),
        sorted(vec![
            format!("{url}/modules/bbb/1.0/MODULE.bazel"),
            format!("{url}/modules/ccc/2.0/MODULE.bazel"),
            format!("{url}/modules/ccc/1.0/MODULE.bazel"),
        ])
    );
}

#[test]
fn test_nodep_fulfilled_many_rounds() {
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_nodep_dep(create_module_key("ccc", "2.0"))
        .add_nodep_dep(create_module_key("ddd", "2.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0');bazel_dep(name='ccc',version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &["module(name='ccc', version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "2.0"),
                &["module(name='ccc', version='2.0');bazel_dep(name='ddd',version='1.0')"],
            )
            .add_module(
                create_module_key("ddd", "1.0"),
                &["module(name='ddd', version='1.0')"],
            )
            .add_module(
                create_module_key("ddd", "2.0"),
                &["module(name='ddd', version='2.0')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ddd", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ddd", "2.0").build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .add_nodep_dep(create_module_key("ccc", "2.0"))
                .add_nodep_dep(create_module_key("ddd", "2.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "1.0")
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .set_registry(reg(&registry))
                .add_dep("ddd", create_module_key("ddd", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("ddd", "1.0")
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ddd", "2.0")
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
    let url = registry.get_url();
    assert_eq!(
        hash_keys(&discovery_value),
        sorted(vec![
            format!("{url}/modules/bbb/1.0/MODULE.bazel"),
            format!("{url}/modules/ccc/2.0/MODULE.bazel"),
            format!("{url}/modules/ddd/2.0/MODULE.bazel"),
            format!("{url}/modules/ccc/1.0/MODULE.bazel"),
            format!("{url}/modules/ddd/1.0/MODULE.bazel"),
        ])
    );
}

#[test]
fn test_nodep_fulfilled_with_override() {
    // Regression test for https://github.com/bazelbuild/bazel/issues/26495
    // single_version_override(module_name='bbb',version='2.0')
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_dep("ccc", create_module_key("ccc", "1.0"))
        .build();
    let overrides = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::SingleVersion(SingleVersionOverride {
            version: Version::parse("2.0").unwrap(),
            registry: String::new(),
            patches: vec![],
            patch_cmds: vec![],
            patch_strip: 0,
        }),
    )]);
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0')"],
            )
            .add_module(
                create_module_key("bbb", "2.0"),
                &["module(name='bbb', version='2.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &[
                    "module(name='ccc', version='1.0')",
                    "bazel_dep(name='bbb', version='1.0', repo_name=None)",
                ],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], overrides.clone())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "2.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "1.0")
                .add_nodep_dep(create_module_key("bbb", "1.0"))
                .build(),
        );

    let discovery_value =
        Discovery::run(&loader, &RootModuleFileValue::new(root_module, overrides)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "2.0"))
                .add_original_dep("bbb", create_module_key("bbb", "1.0"))
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "2.0")
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "1.0")
                .add_nodep_dep(create_module_key("bbb", "2.0"))
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
    let url = registry.get_url();
    assert_eq!(
        hash_keys(&discovery_value),
        sorted(vec![
            format!("{url}/modules/bbb/2.0/MODULE.bazel"),
            format!("{url}/modules/ccc/1.0/MODULE.bazel"),
        ])
    );
}

#[test]
fn test_circular_dependency() {
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0');bazel_dep(name='ccc',version='2.0')"],
            )
            .add_module(
                create_module_key("ccc", "2.0"),
                &["module(name='ccc', version='2.0');bazel_dep(name='bbb',version='1.0')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
}

#[test]
fn test_circular_dependency_on_root_module() {
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "1.0"),
                &["module(name='bbb', version='1.0');bazel_dep(name='aaa',version='2.0')"],
            )
            .add_module(
                create_module_key("aaa", "2.0"),
                &["module(name='aaa', version='2.0')"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("aaa", create_module_key("aaa", "2.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("aaa", "2.0").build(),
        );

    let discovery_value = Discovery::run(&loader, &root(root_module)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "1.0"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("aaa", ModuleKey::root())
                .add_original_dep("aaa", create_module_key("aaa", "2.0"))
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
}

#[test]
fn test_single_version_override() {
    // single_version_override(module_name='ccc',version='2.0')
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "0.1"))
        .build();
    let overrides = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::SingleVersion(SingleVersionOverride {
            version: Version::parse("2.0").unwrap(),
            registry: String::new(),
            patches: vec![],
            patch_cmds: vec![],
            patch_strip: 0,
        }),
    )]);
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "0.1"),
                &["module(name='bbb', version='0.1');bazel_dep(name='ccc',version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &["module(name='ccc', version='1.0');"],
            )
            .add_module(
                create_module_key("ccc", "2.0"),
                &["module(name='ccc', version='2.0');"],
            ),
    );
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], overrides.clone())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "0.1")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "1.0").build(),
        )
        .evaluates(
            &registry,
            InterimModuleBuilder::create("ccc", "2.0").build(),
        );

    let discovery_value =
        Discovery::run(&loader, &RootModuleFileValue::new(root_module, overrides)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "0.1"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "0.1")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .add_original_dep("ccc", create_module_key("ccc", "1.0"))
                .set_registry(reg(&registry))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "2.0")
                .set_registry(reg(&registry))
                .build_entry(),
        ],
    );
}

#[test]
fn test_registry_override() {
    let mut factory = FakeRegistryFactory::default();
    let registry1 = Arc::new(
        factory
            .new_fake_registry("/foo")
            .add_module(
                create_module_key("bbb", "0.1"),
                &["module(name='bbb', version='0.1');bazel_dep(name='ccc',version='1.0')"],
            )
            .add_module(
                create_module_key("ccc", "1.0"),
                &["module(name='ccc', version='1.0');"],
            ),
    );
    let registry2 = Arc::new(factory.new_fake_registry("/bar").add_module(
        create_module_key("ccc", "1.0"),
        &["module(name='ccc', version='1.0');bazel_dep(name='bbb',version='0.1')"],
    ));
    // single_version_override(module_name='ccc',registry='<registry2>')
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "0.1"))
        .build();
    let overrides = IndexMap::from([(
        "ccc".to_string(),
        ModuleOverride::SingleVersion(SingleVersionOverride {
            version: Version::empty(),
            registry: registry2.get_url(),
            patches: vec![],
            patch_cmds: vec![],
            patch_strip: 0,
        }),
    )]);
    let loader = FakeModuleFileLoader::new(
        vec![registry1.clone()],
        vec![registry1.clone(), registry2.clone()],
        overrides.clone(),
    )
    .evaluates(
        &registry1,
        InterimModuleBuilder::create("bbb", "0.1")
            .add_dep("ccc", create_module_key("ccc", "1.0"))
            .build(),
    )
    .evaluates(
        &registry1,
        InterimModuleBuilder::create("ccc", "1.0").build(),
    )
    .evaluates(
        &registry2,
        InterimModuleBuilder::create("ccc", "1.0")
            .add_dep("bbb", create_module_key("bbb", "0.1"))
            .build(),
    );

    let discovery_value =
        Discovery::run(&loader, &RootModuleFileValue::new(root_module, overrides)).unwrap();
    assert_dep_graph(
        &discovery_value,
        vec![
            InterimModuleBuilder::create("aaa", "0.1")
                .set_key(ModuleKey::root())
                .add_dep("bbb", create_module_key("bbb", "0.1"))
                .build_entry(),
            InterimModuleBuilder::create("bbb", "0.1")
                .add_dep("ccc", create_module_key("ccc", "1.0"))
                .set_registry(reg(&registry1))
                .build_entry(),
            InterimModuleBuilder::create("ccc", "1.0")
                .add_dep("bbb", create_module_key("bbb", "0.1"))
                .set_registry(reg(&registry2))
                .build_entry(),
        ],
    );
}

#[test]
#[ignore = "@Ignore in Java (b/389163906); also needs ModuleFileFunction (Starlark) and a local_path_override fetch"]
fn test_local_path_override() {}

#[test]
#[ignore = "@Ignore in Java (b/389163906); also needs ModuleFileFunction (Starlark) for builtin modules"]
fn test_builtin_modules_for_root() {}

/// Not a Java test: the dependency chain added by `maybeReportDependencyChain` when a module is
/// missing from all registries.
#[test]
fn missing_module_reports_dependency_chain() {
    let root_module = InterimModuleBuilder::create("aaa", "0.1")
        .set_key(ModuleKey::root())
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .build();
    let mut factory = FakeRegistryFactory::default();
    let registry = Arc::new(factory.new_fake_registry("/foo").add_module(
        create_module_key("bbb", "1.0"),
        &["module(name='bbb', version='1.0');bazel_dep(name='ccc',version='2.0')"],
    ));
    let loader = FakeModuleFileLoader::new(vec![registry.clone()], vec![], IndexMap::new())
        .evaluates(
            &registry,
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("ccc", create_module_key("ccc", "2.0"))
                .build(),
        );
    let e = Discovery::run(&loader, &root(root_module)).unwrap_err();
    assert_eq!(e.get_code(), ExternalDepsCode::BadModule);
    assert_eq!(
        e.get_message(),
        "in module dependency chain <root> -> bbb@1.0 -> ccc@2.0: module ccc@2.0 not found in \
         registries:\n* module not found: ccc@2.0"
    );
}
