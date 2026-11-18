//! Port of `BazelModuleResolutionFunctionTest` (src/test/java/com/google/devtools/build/lib/bazel/
//! bzlmod), for the parts that do not need Skyframe or MODULE.bazel evaluation.
//!
//! Java writes MODULE.bazel text and evaluates `BazelModuleResolutionValue` in Skyframe. Here the
//! modules that `ModuleFileFunction` would produce from that text are written out by hand and fed
//! to `BazelModuleResolutionFunction::discover_and_select` through a map-backed
//! `ModuleFileLoader`; `embedBazelVersion(v)` becomes the `current_bazel_version` argument.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use blaze_bzlmod_model::{
    BazelCompatibilityMode, BazelModuleResolutionFunction, CheckDirectDepsMode, Checksum,
    DiscoverAndSelectResult, ExternalDepsCode, ExternalDepsException, InterimModule, ModuleFile,
    ModuleFileLoader, ModuleKey, ModuleOverride, NonRegistryOverride, NonRootModuleFileValue,
    Registry, RegistryException, RegistryFetcher, RepoSpec, RootModuleFileValue, Version,
    YankedVersionsLookup, YankedVersionsUtil, YankedVersionsValue, local_path_repo_spec,
};
use blaze_events::{EventCollector, EventKind, ExtendedEventHandler};
use common::*;
use indexmap::{IndexMap, IndexSet};

/// A registry that only has a URL (module files come from the loader).
#[derive(Debug)]
struct FakeRegistry {
    url: String,
}

impl Registry for FakeRegistry {
    fn get_url(&self) -> String {
        self.url.clone()
    }

    fn get_module_file(
        &self,
        _key: &ModuleKey,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<ModuleFile, RegistryException> {
        unreachable!()
    }

    fn get_repo_spec(
        &self,
        _key: &ModuleKey,
        _module_file_hashes: &IndexMap<String, Option<Checksum>>,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<RepoSpec, RegistryException> {
        unreachable!()
    }

    fn get_yanked_versions(
        &self,
        _module_name: &str,
        _event_handler: &dyn ExtendedEventHandler,
        _download_manager: &dyn RegistryFetcher,
    ) -> Result<Option<IndexMap<Version, String>>, RegistryException> {
        unreachable!()
    }

    fn try_get_yanked_versions_from_lockfile(
        &self,
        _selected_module_key: &ModuleKey,
    ) -> Option<YankedVersionsValue> {
        // Like FakeRegistry: always ask YANKED_VERSIONS.
        None
    }
}

/// The modules of the fake registry, plus its yanked versions (`FakeRegistry.addYankedVersion`).
struct Setup {
    registry: Arc<dyn Registry>,
    modules: HashMap<ModuleKey, InterimModule>,
    yanked: HashMap<String, IndexMap<Version, String>>,
}

impl Setup {
    fn new() -> Self {
        Setup {
            registry: Arc::new(FakeRegistry {
                url: "fake:0".to_string(),
            }),
            modules: HashMap::new(),
            yanked: HashMap::new(),
        }
    }

    fn add_module(mut self, module: InterimModule) -> Self {
        self.modules.insert(module.key.clone(), module);
        self
    }

    fn add_yanked_version(mut self, module_name: &str, yanked: &[(&str, &str)]) -> Self {
        self.yanked.insert(
            module_name.to_string(),
            yanked
                .iter()
                .map(|(v, r)| (Version::parse(v).unwrap(), r.to_string()))
                .collect(),
        );
        self
    }
}

impl ModuleFileLoader for Setup {
    fn get_module_file(
        &self,
        key: &ModuleKey,
    ) -> Result<NonRootModuleFileValue, ExternalDepsException> {
        let Some(module) = self.modules.get(key) else {
            return Err(ExternalDepsException::with_message(
                ExternalDepsCode::ModuleNotFound,
                format!("module {key} not found in registries:\n* module not found: {key}"),
            ));
        };
        let mut module = module.clone();
        module.registry = Some(self.registry.clone());
        Ok(NonRootModuleFileValue {
            module,
            registry_file_hashes: IndexMap::new(),
        })
    }
}

impl YankedVersionsLookup for Setup {
    fn get_yanked_versions(&self, module_name: &str, _registry_url: &str) -> YankedVersionsValue {
        YankedVersionsValue::create(self.yanked.get(module_name).cloned())
    }
}

/// `SkyframeExecutorTestUtils.evaluate(BazelModuleResolutionValue.KEY)` up to selection and the
/// checks, with the defaults of the Java test setup (check_direct_dependencies off,
/// bazel_compatibility_mode error).
fn resolve(
    setup: &Setup,
    root: &RootModuleFileValue,
    allowed_yanked_versions: &[&str],
    bazel_compatibility_mode: BazelCompatibilityMode,
    bazel_version: &str,
    events: &EventCollector,
) -> Result<DiscoverAndSelectResult, ExternalDepsException> {
    let allowed: Vec<String> = allowed_yanked_versions
        .iter()
        .map(|s| s.to_string())
        .collect();
    let allowed: Option<IndexSet<ModuleKey>> =
        YankedVersionsUtil::parse_allowed_yanked_versions(None, &allowed)?;
    BazelModuleResolutionFunction::discover_and_select(
        setup,
        setup,
        root,
        allowed.as_ref(),
        CheckDirectDepsMode::Off,
        bazel_compatibility_mode,
        bazel_version,
        events,
    )
}

fn messages(events: &EventCollector, kind: EventKind) -> Vec<String> {
    events
        .filtered(kind)
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect()
}

fn assert_contains_event(events: &EventCollector, text: &str) {
    let all: Vec<String> = events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect();
    assert!(
        all.iter().any(|m| m.contains(text)),
        "{text:?} not in {all:#?}"
    );
}

fn root_module_with_compat(compat: &[&str]) -> InterimModule {
    // module(name='mod', version='1.0', bazel_compatibility=[...])
    let mut b = InterimModule::builder();
    b.set_name("mod")
        .set_version(Version::parse("1.0").unwrap())
        .set_key(ModuleKey::root())
        .add_bazel_compatibility_values(compat.iter().copied());
    b.build()
}

fn with_compat(module: InterimModule, compat: &[&str]) -> InterimModule {
    module
        .to_builder()
        .add_bazel_compatibility_values(compat.iter().copied())
        .build()
}

#[test]
#[ignore = "needs ModuleFileFunction (Starlark): bazel_compatibility values are validated by module()"]
fn test_bazel_invalid_compatibility() {}

#[test]
fn test_simple_bazel_compatibility_failure() {
    let setup = Setup::new();
    let root = RootModuleFileValue::new(
        root_module_with_compat(&[">5.1.0", "<5.1.4"]),
        IndexMap::new(),
    );
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "5.1.4",
        &events,
    );
    assert!(result.is_err());
    assert_contains_event(
        &events,
        "Bazel version 5.1.4 is not compatible with module \"<root>\" (bazel_compatibility: \
         [>5.1.0, <5.1.4])",
    );
}

#[test]
fn test_bazel_compatibility_warning() {
    let setup = Setup::new();
    let root = RootModuleFileValue::new(
        root_module_with_compat(&[">5.1.0", "<5.1.4"]),
        IndexMap::new(),
    );
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Warning,
        "5.1.4",
        &events,
    );
    assert!(result.is_ok());
    assert_eq!(
        messages(&events, EventKind::Warning),
        vec![
            "Bazel version 5.1.4 is not compatible with module \"<root>\" (bazel_compatibility: \
             [>5.1.0, <5.1.4])"
        ]
    );
}

#[test]
fn test_disabling_bazel_compatibility() {
    let setup = Setup::new();
    let root = RootModuleFileValue::new(
        root_module_with_compat(&[">5.1.0", "<5.1.4"]),
        IndexMap::new(),
    );
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Off,
        "5.1.4",
        &events,
    );
    assert!(result.is_ok());
    assert_eq!(events.count(), 0);
}

/// Root depends on "a" which depends on "b". The only versions that work with the root, a and b
/// constraints are between (not including) 5.1.2 and 5.1.4.
fn setup_modules_for_compatibility() -> (Setup, RootModuleFileValue) {
    let root_module = root_module_with_compat(&[">5.1.0", "<5.1.6"])
        .to_builder()
        .set_deps(IndexMap::from([(
            "a".to_string(),
            blaze_bzlmod_model::DepSpec::from_module_key(&create_module_key("a", "1.0")),
        )]))
        .set_original_deps(IndexMap::from([(
            "a".to_string(),
            blaze_bzlmod_model::DepSpec::from_module_key(&create_module_key("a", "1.0")),
        )]))
        .build();
    let setup = Setup::new()
        .add_module(with_compat(
            InterimModuleBuilder::create("a", "1.0")
                .add_dep("b", create_module_key("b", "1.0"))
                .build(),
            &[">=5.1.2", "-5.1.4"],
        ))
        .add_module(with_compat(
            InterimModuleBuilder::create("b", "1.0").build(),
            &["<=5.1.4", "-5.1.2"],
        ));
    (
        setup,
        RootModuleFileValue::new(root_module, IndexMap::new()),
    )
}

#[test]
fn test_bazel_compatibility_success() {
    let (setup, root) = setup_modules_for_compatibility();
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "5.1.4-pre.20220421.3",
        &events,
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn test_bazel_compatibility_failure() {
    let (setup, root) = setup_modules_for_compatibility();
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "5.1.5rc444",
        &events,
    );
    assert!(result.is_err());
    assert_contains_event(
        &events,
        "Bazel version 5.1.5rc444 is not compatible with module \"b@1.0\" (bazel_compatibility: \
         [<=5.1.4, -5.1.2])",
    );
}

#[test]
fn test_rc_is_compatible_with_release_requirement() {
    let setup = Setup::new();
    let root = RootModuleFileValue::new(root_module_with_compat(&[">=6.4.0"]), IndexMap::new());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "6.4.0rc1",
        &events,
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn test_prerelease_is_not_compatible_with_release_requirement() {
    let setup = Setup::new();
    let root = RootModuleFileValue::new(root_module_with_compat(&[">=6.4.0"]), IndexMap::new());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "6.4.0-pre-1",
        &events,
    );
    assert!(result.is_err());
    assert_contains_event(
        &events,
        "Bazel version 6.4.0-pre-1 is not compatible with module \"<root>\" (bazel_compatibility: \
         [>=6.4.0])",
    );
}

fn setup_modules_for_yanked_version() -> (Setup, RootModuleFileValue) {
    // module(name='mod', version='1.0'); bazel_dep(name = 'a', version = '1.0')
    let root_module = InterimModuleBuilder::create("mod", "1.0")
        .set_key(ModuleKey::root())
        .add_dep("a", create_module_key("a", "1.0"))
        .build();
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("a", "1.0")
                .add_dep("b", create_module_key("b", "1.0"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("b", "1.0").build())
        .add_yanked_version("b", &[("1.0", "1.0 is a bad version!")]);
    (
        setup,
        RootModuleFileValue::new(root_module, IndexMap::new()),
    )
}

#[test]
fn test_yanked_version_check_success() {
    let (setup, root) = setup_modules_for_yanked_version();
    let events = EventCollector::new();
    let e = resolve(
        &setup,
        &root,
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap_err();
    assert!(
        e.get_message().contains(
            "Yanked version detected in your resolved dependency graph: b@1.0, for the reason: \
             1.0 is a bad version!"
        ),
        "{}",
        e.get_message()
    );
}

#[test]
fn test_yanked_version_check_ignored_by_all() {
    let (setup, root) = setup_modules_for_yanked_version();
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &["all"],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    assert_eq!(
        result.selected_yanked_versions,
        IndexMap::from([(
            create_module_key("b", "1.0"),
            "1.0 is a bad version!".to_string()
        )])
    );
}

/// Not a JUnit test: `BazelModuleResolutionFunction.collectYankedVersionsValues` collects the
/// modules whose yanked versions must be fetched in a `HashMap<ModuleKey, ...>`, so
/// `getSelectedYankedVersions()` (written to MODULE.bazel.lock in that order) follows Java's
/// hash order. The expected order was printed by OpenJDK 25 for a `HashMap` filled with these
/// seven keys in dependency order.
#[test]
fn selected_yanked_versions_are_in_java_hash_map_order() {
    const MODULES: [(&str, &str); 7] = [
        ("aaa", "1.0"),
        ("bbb", "1.1"),
        ("ccc", "2.0"),
        ("ddd", "0.0.1"),
        ("rules_cc", "0.0.9"),
        ("rules_java", "7.6.5"),
        ("platforms", "0.0.10"),
    ];
    let mut root_module = InterimModuleBuilder::create("mod", "1.0").set_key(ModuleKey::root());
    let mut setup = Setup::new();
    for (name, version) in MODULES {
        root_module = root_module.add_dep(name, create_module_key(name, version));
        setup = setup
            .add_module(InterimModuleBuilder::create(name, version).build())
            .add_yanked_version(name, &[(version, "yanked")]);
    }
    let root = RootModuleFileValue::new(root_module.build(), IndexMap::new());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &["all"],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    let order: Vec<String> = result
        .selected_yanked_versions
        .keys()
        .map(|k| k.to_string())
        .collect();
    assert_eq!(
        order,
        [
            "rules_java@7.6.5",
            "bbb@1.1",
            "aaa@1.0",
            "ccc@2.0",
            "rules_cc@0.0.9",
            "ddd@0.0.1",
            "platforms@0.0.10"
        ]
    );
}

#[test]
fn test_yanked_version_check_ignored_by_specific() {
    let (setup, root) = setup_modules_for_yanked_version();
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &root,
        &["b@1.0"],
        BazelCompatibilityMode::Error,
        "",
        &events,
    );
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn test_bad_yanked_version_format() {
    let (setup, root) = setup_modules_for_yanked_version();
    let events = EventCollector::new();
    let e = resolve(
        &setup,
        &root,
        &["b+1.0"],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap_err();
    assert!(
        e.get_message().contains(
            "Parsing command line flag --allow_yanked_versions=b+1.0 failed, module versions \
             must be of the form '<module name>@<version>'"
        ),
        "{}",
        e.get_message()
    );
}

#[test]
fn override_on_nonexistent_module() {
    // local_path_override(module_name='d', path='whatevs')
    let root_module = InterimModuleBuilder::create("mod", "1.0")
        .set_key(ModuleKey::root())
        .add_dep("a", create_module_key("a", "1.0"))
        .add_dep("b", create_module_key("b", "1.1"))
        .build();
    let overrides: IndexMap<String, ModuleOverride> = IndexMap::from([(
        "d".to_string(),
        ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec("whatevs"))),
    )]);
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("a", "1.0")
                .add_dep("b", create_module_key("b", "1.0"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("c", "1.0").build())
        .add_module(InterimModuleBuilder::create("c", "1.1").build())
        .add_module(
            InterimModuleBuilder::create("b", "1.0")
                .add_dep("c", create_module_key("c", "1.1"))
                .build(),
        )
        .add_module(
            InterimModuleBuilder::create("b", "1.1")
                .add_dep("c", create_module_key("c", "1.0"))
                .build(),
        );
    let events = EventCollector::new();
    let e = resolve(
        &setup,
        &RootModuleFileValue::new(root_module, overrides),
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap_err();
    assert!(
        e.get_message()
            .contains("the root module specifies overrides on nonexistent module(s): d"),
        "{}",
        e.get_message()
    );
}

#[test]
#[ignore = "needs ModuleFileFunction (Starlark): print() events come from MODULE.bazel evaluation"]
fn test_print_behavior() {}

/// The root module of the nodep tests: no `module()` call.
fn anonymous_root() -> InterimModuleBuilder {
    InterimModuleBuilder::create("", "").set_key(ModuleKey::root())
}

#[test]
fn nodep_unfulfilled() {
    // bazel_dep(name='b',version='1.0'); bazel_dep(name='c',version='1.0',repo_name=None)
    let root_module = anonymous_root()
        .add_dep("b", create_module_key("b", "1.0"))
        .add_nodep_dep(create_module_key("c", "1.0"))
        .build();
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("b", "1.0")
                .add_dep("d", create_module_key("d", "1.0"))
                .build(),
        )
        .add_module(
            InterimModuleBuilder::create("c", "1.0")
                .add_dep("d", create_module_key("d", "1.1"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("d", "1.0").build())
        .add_module(InterimModuleBuilder::create("d", "1.1").build());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &RootModuleFileValue::new(root_module, IndexMap::new()),
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    let dep_graph = &result.selection_result.resolved_dep_graph;
    assert!(!dep_graph.contains_key(&create_module_key("c", "1.0")));
    assert_eq!(
        dep_graph[&create_module_key("b", "1.0")].deps["d"].version,
        Version::parse("1.0").unwrap()
    );
}

#[test]
fn nodep_fulfilled() {
    // bazel_dep(name='b',version='1.0'); bazel_dep(name='c',version='1.0')
    let root_module = anonymous_root()
        .add_dep("b", create_module_key("b", "1.0"))
        .add_dep("c", create_module_key("c", "1.0"))
        .build();
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("b", "1.0")
                .add_dep("d", create_module_key("d", "1.0"))
                .build(),
        )
        .add_module(
            // bazel_dep(name='d',version='1.1',repo_name=None)
            InterimModuleBuilder::create("c", "1.0")
                .add_nodep_dep(create_module_key("d", "1.1"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("d", "1.0").build())
        .add_module(InterimModuleBuilder::create("d", "1.1").build());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &RootModuleFileValue::new(root_module, IndexMap::new()),
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    let dep_graph = &result.selection_result.resolved_dep_graph;
    assert!(dep_graph.contains_key(&create_module_key("d", "1.1")));
    assert_eq!(
        dep_graph[&create_module_key("b", "1.0")].deps["d"].version,
        Version::parse("1.1").unwrap()
    );
    assert!(
        !dep_graph[&create_module_key("c", "1.0")]
            .deps
            .contains_key("d")
    );
}

#[test]
fn nodep_fulfilled_dev_dep() {
    // bazel_dep(name='b',version='1.0'); bazel_dep(name='c',version='1.1',dev_dependency=True)
    // The root module's dev dependencies are kept by ModuleFileFunction (not ignored).
    let root_module = anonymous_root()
        .add_dep("b", create_module_key("b", "1.0"))
        .add_dep("c", create_module_key("c", "1.1"))
        .build();
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("b", "1.0")
                .add_dep("c", create_module_key("c", "1.0"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("c", "1.0").build())
        .add_module(InterimModuleBuilder::create("c", "1.1").build());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &RootModuleFileValue::new(root_module, IndexMap::new()),
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    let dep_graph = &result.selection_result.resolved_dep_graph;
    assert!(dep_graph.contains_key(&create_module_key("c", "1.1")));
    assert_eq!(
        dep_graph[&create_module_key("b", "1.0")].deps["c"].version,
        Version::parse("1.1").unwrap()
    );
}

#[test]
#[ignore = "needs ModuleFileFunction (Starlark): a non-root module's dev_dependency nodep is dropped while evaluating its MODULE.bazel"]
fn nodep_would_be_fulfilled_if_non_dev_dep() {
    // Hand-built: ModuleFileFunction drops c's dev dependency, so c has no nodep on d.
    let root_module = anonymous_root()
        .add_dep("b", create_module_key("b", "1.0"))
        .add_dep("c", create_module_key("c", "1.0"))
        .build();
    let setup = Setup::new()
        .add_module(
            InterimModuleBuilder::create("b", "1.0")
                .add_dep("d", create_module_key("d", "1.0"))
                .build(),
        )
        .add_module(InterimModuleBuilder::create("c", "1.0").build())
        .add_module(InterimModuleBuilder::create("d", "1.0").build())
        .add_module(InterimModuleBuilder::create("d", "1.1").build());
    let events = EventCollector::new();
    let result = resolve(
        &setup,
        &RootModuleFileValue::new(root_module, IndexMap::new()),
        &[],
        BazelCompatibilityMode::Error,
        "",
        &events,
    )
    .unwrap();
    let dep_graph = &result.selection_result.resolved_dep_graph;
    assert!(!dep_graph.contains_key(&create_module_key("d", "1.1")));
    assert_eq!(
        dep_graph[&create_module_key("b", "1.0")].deps["d"].version,
        Version::parse("1.0").unwrap()
    );
    assert!(
        !dep_graph[&create_module_key("c", "1.0")]
            .deps
            .contains_key("d")
    );
}

/// From `src/test/py/bazel/bzlmod/bazel_module_test.py` `testCheckDirectDependencies` (the
/// resolution part): the `--check_direct_dependencies` warnings and errors.
#[test]
fn test_check_direct_dependencies() {
    let root_module = anonymous_root()
        .add_dep("aaa", create_module_key("aaa", "1.0"))
        .add_dep("bbb", create_module_key("bbb", "1.0"))
        .add_dep("ccc", create_module_key("ccc", "1.1"))
        .build();
    let setup = Setup::new()
        .add_module(InterimModuleBuilder::create("aaa", "1.0").build())
        .add_module(InterimModuleBuilder::create("aaa", "1.1").build())
        .add_module(
            InterimModuleBuilder::create("bbb", "1.0")
                .add_dep("com_foo_aaa", create_module_key("aaa", "1.0"))
                .build(),
        )
        .add_module(
            InterimModuleBuilder::create("bbb", "1.1")
                .add_dep("aaa", create_module_key("aaa", "1.1"))
                .build(),
        )
        .add_module(
            InterimModuleBuilder::create("ccc", "1.1")
                .add_dep("aaa", create_module_key("aaa", "1.1"))
                .add_dep("bbb", create_module_key("bbb", "1.1"))
                .build(),
        );
    let root = RootModuleFileValue::new(root_module, IndexMap::new());
    let expected = vec![
        "For repository 'aaa', the root module requires module version aaa@1.0, but got aaa@1.1 \
         in the resolved dependency graph. Please update the version in your MODULE.bazel or set \
         --check_direct_dependencies=off",
        "For repository 'bbb', the root module requires module version bbb@1.0, but got bbb@1.1 \
         in the resolved dependency graph. Please update the version in your MODULE.bazel or set \
         --check_direct_dependencies=off",
    ];

    let warnings = EventCollector::new();
    BazelModuleResolutionFunction::discover_and_select(
        &setup,
        &setup,
        &root,
        None,
        CheckDirectDepsMode::Warning,
        BazelCompatibilityMode::Error,
        "",
        &warnings,
    )
    .unwrap();
    assert_eq!(messages(&warnings, EventKind::Warning), expected);

    let errors = EventCollector::new();
    let e = BazelModuleResolutionFunction::discover_and_select(
        &setup,
        &setup,
        &root,
        None,
        CheckDirectDepsMode::Error,
        BazelCompatibilityMode::Error,
        "",
        &errors,
    )
    .unwrap_err();
    assert_eq!(messages(&errors, EventKind::Error), expected);
    assert_eq!(e.get_message(), "Direct dependency check failed.");
    assert_eq!(e.get_code(), ExternalDepsCode::VersionResolutionError);
    // Exit code 48, as the Python test checks.
    assert_eq!(e.get_code().exit_code(), 48);
}
