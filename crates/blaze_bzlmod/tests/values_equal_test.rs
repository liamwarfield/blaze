//! Change pruning of the bzlmod keys (package I2): `Key::values_equal` against the Java value
//! classes' `equals`, observed through the evaluator (a recomputed equal value does not rebuild
//! its parent).

mod skyframe_common;

use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::{ModuleFileKey, ModuleFileValue, YankedVersionsKey};
use blaze_bzlmod::skyframe::precomputed::{ClientEnvironmentKey, EnvironmentVariableValue};
use blaze_bzlmod_model::{
    InterimModule, ModuleKey, NonRootModuleFileValue, Registry, Version, YankedVersionsValue,
};
use blaze_skyframe::testing::parent_rebuilt;
use indexmap::IndexMap;
use skyframe_common::FakeRegistry;

/// Java: `EnvironmentVariableValue` is a record of the value.
// Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
#[test]
fn recomputed_equal_client_environment_variable_prunes_parent() {
    let key = ClientEnvironmentKey::key("PATH");
    let value = |v: Option<&str>| EnvironmentVariableValue(v.map(str::to_string));
    assert!(!parent_rebuilt(
        key.clone(),
        value(Some("/bin")),
        value(Some("/bin"))
    ));
    assert!(!parent_rebuilt(key.clone(), value(None), value(None)));
    assert!(parent_rebuilt(
        key,
        value(Some("/bin")),
        value(Some("/usr/bin"))
    ));
}

/// Java: `YankedVersionsValue` is a record of the optional map.
// Java: com/google/devtools/build/lib/bazel/bzlmod/YankedVersionsValue.java:29 YankedVersionsValue
#[test]
fn recomputed_equal_yanked_versions_prunes_parent() {
    let key = YankedVersionsKey::create("foo", "https://bcr");
    let yanked = |reason: &str| YankedVersionsValue {
        yanked_versions: Some(IndexMap::from([(
            Version::parse("1.0").unwrap(),
            reason.to_string(),
        )])),
    };
    assert!(!parent_rebuilt(key.clone(), yanked("bad"), yanked("bad")));
    assert!(parent_rebuilt(key, yanked("bad"), yanked("worse")));
}

fn non_root(registry: &Arc<dyn Registry>) -> ModuleFileValue {
    let mut module = InterimModule::builder();
    module
        .set_name("foo")
        .set_version(Version::parse("1.0").unwrap())
        .set_key(ModuleKey::new("foo", Version::parse("1.0").unwrap()))
        .set_registry(Some(registry.clone()));
    ModuleFileValue::NonRoot(NonRootModuleFileValue {
        module: module.build(),
        registry_file_hashes: IndexMap::new(),
    })
}

/// Java: `InterimModule.getRegistry()` is a `Registry`, a `NotComparableSkyValue` with
/// identity `equals`: a module file value over the same registry object prunes; one over a
/// re-created registry with the same URL does not.
// Java: com/google/devtools/build/lib/bazel/bzlmod/InterimModule.java:109 InterimModule.getRegistry
#[test]
fn module_file_value_compares_registry_by_identity() {
    let key = ModuleFileKey::key(ModuleKey::new("foo", Version::parse("1.0").unwrap()));
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new("https://bcr", "/bcr"));
    let same_url: Arc<dyn Registry> = Arc::new(FakeRegistry::new("https://bcr", "/bcr"));
    assert!(!parent_rebuilt(
        key.clone(),
        non_root(&registry),
        non_root(&registry)
    ));
    assert!(parent_rebuilt(
        key,
        non_root(&registry),
        non_root(&same_url)
    ));
}
