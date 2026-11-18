//! Port of the `BazelModuleResolutionFunctionTest` cases that need the Skyframe harness
//! (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).
//!
//! The logic-level cases (hand-written modules fed to `discover_and_select`) are ported in
//! `blaze_bzlmod_model/tests/bazel_module_resolution_function_test.rs`. Here the cases that need
//! MODULE.bazel evaluation, and those that exercise the YANKED_VERSIONS and REGISTRY functions,
//! evaluate `BazelModuleResolutionValue.KEY` with a [`FakeRegistry`].
//!
//! PORT: Java runs these in `BuildViewTestCase`, whose analysis mock adds builtin modules; this
//! harness has none (like `ModuleFileFunctionTest`).

mod skyframe_common;

use std::sync::Arc;

use blaze_bzlmod::skyframe::{BazelModuleResolutionKey, RegistryKey, YankedVersionsKey};
use blaze_bzlmod_model::{
    BazelModuleResolutionValue, RegistryFactoryImpl, Version, YankedVersionsValue,
};
use blaze_skyframe::{EvaluationResult, SkyKey};
use indexmap::IndexMap;
use skyframe_common::*;

/// The `BuildViewTestCase` setup: one fake registry (`registry`) as the only registry.
fn set_up() -> (Harness, Arc<FakeRegistry>) {
    let mut h = Harness::new();
    let registry = h.registry_factory.new_fake_registry("/foo");
    h.set_registries(&[registry.url()]);
    (h, registry)
}

fn evaluate_resolution(h: &Harness) -> EvaluationResult {
    h.evaluate(&[SkyKey::new(BazelModuleResolutionKey)])
}

fn resolution_value(result: &EvaluationResult) -> Arc<BazelModuleResolutionValue> {
    result
        .get(&BazelModuleResolutionKey)
        .expect("resolution value")
}

#[test]
fn test_bazel_invalid_compatibility() {
    let (h, _registry) = set_up();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='mod', version='1.0', bazel_compatibility=['>5.1.0dd'])"],
    );

    let result = evaluate_resolution(&h);

    assert!(result.has_error());
    h.assert_contains_event("invalid version argument '>5.1.0dd'");
}

#[test]
fn test_simple_bazel_compatibility_failure() {
    let (mut h, _registry) = set_up();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='mod', version='1.0', bazel_compatibility=['>5.1.0', '<5.1.4'])"],
    );

    // embedBazelVersion("5.1.4")
    h.bazel_version = "5.1.4".to_string();
    let result = evaluate_resolution(&h);

    assert!(result.has_error());
    h.assert_contains_event(
        "Bazel version 5.1.4 is not compatible with module \"<root>\" (bazel_compatibility: \
         [>5.1.0, <5.1.4])",
    );
}

fn setup_modules_for_yanked_version(h: &Harness, registry: &Arc<FakeRegistry>) {
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='mod', version='1.0')",
            "bazel_dep(name = 'a', version = '1.0')",
        ],
    );

    registry
        .add_module(
            create_module_key("a", "1.0"),
            &[
                "module(name='a', version='1.0');",
                "bazel_dep(name='b', version='1.0')",
            ],
        )
        .add_module(
            create_module_key("b", "1.0"),
            &["module(name='b', version='1.0');"],
        )
        .add_yanked_version(
            "b",
            IndexMap::from([(
                Version::parse("1.0").unwrap(),
                "1.0 is a bad version!".to_string(),
            )]),
        );
}

#[test]
fn test_yanked_version_check_success() {
    let (h, registry) = set_up();
    setup_modules_for_yanked_version(&h, &registry);
    let result = evaluate_resolution(&h);

    assert!(result.has_error());
    assert!(error_string(&result).contains(
        "Yanked version detected in your resolved dependency graph: b@1.0, for the reason: 1.0 \
         is a bad version!"
    ));
}

#[test]
fn test_yanked_version_check_ignored_by_all() {
    let (mut h, registry) = set_up();
    h.allowed_yanked_versions = vec!["all".to_string()];
    setup_modules_for_yanked_version(&h, &registry);
    let result = evaluate_resolution(&h);
    assert!(!result.has_error(), "{}", error_string(&result));
}

#[test]
fn test_yanked_version_check_ignored_by_specific() {
    let (mut h, registry) = set_up();
    h.allowed_yanked_versions = vec!["b@1.0".to_string()];
    setup_modules_for_yanked_version(&h, &registry);
    let result = evaluate_resolution(&h);
    assert!(!result.has_error(), "{}", error_string(&result));
}

#[test]
fn test_bad_yanked_version_format() {
    let (mut h, registry) = set_up();
    h.allowed_yanked_versions = vec!["b+1.0".to_string()];
    setup_modules_for_yanked_version(&h, &registry);
    let result = evaluate_resolution(&h);
    assert!(result.has_error());
    assert!(error_string(&result).contains(
        "Parsing command line flag --allow_yanked_versions=b+1.0 failed, module versions must be \
         of the form '<module name>@<version>'"
    ));
}

#[test]
fn test_print_behavior() {
    let (h, registry) = set_up();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='mod', version='1.0')",
            "print('hello from root module')",
            "bazel_dep(name = 'a', version = '1.0')",
            "bazel_dep(name = 'b', version = '1.1')",
            "single_version_override(module_name = 'b', version = '1.1')",
            "local_path_override(module_name='a', path='a')",
        ],
    );
    h.scratch_file(
        "a/MODULE.bazel",
        &[
            "module(name='a', version='1.0')",
            "print('hello from overridden a')",
            "bazel_dep(name='b', version='1.0')",
        ],
    );

    registry
        .add_module(
            create_module_key("a", "1.0"),
            &[
                "module(name='a', version='1.0')",
                "print('hello from a@1.0')",
                "bazel_dep(name='b', version='1.0')",
            ],
        )
        .add_module(
            create_module_key("c", "1.0"),
            &["module(name='c', version='1.0')"],
        )
        .add_module(
            create_module_key("c", "1.1"),
            &["module(name='c', version='1.1')"],
        )
        .add_module(
            create_module_key("b", "1.0"),
            &[
                "module(name='b', version='1.0', compatibility_level = 2)",
                "bazel_dep(name='c', version='1.1')",
                "print('hello from b@1.0')",
            ],
        )
        .add_module(
            create_module_key("b", "1.1"),
            &[
                "module(name='b', version='1.1', compatibility_level = 3)",
                "bazel_dep(name='c', version='1.0')",
                "print('hello from b@1.1')",
            ],
        );

    evaluate_resolution(&h);

    h.assert_contains_event("hello from root module");
    h.assert_contains_event("hello from overridden a");
    h.assert_does_not_contain_event("hello from a@1.0");
    h.assert_does_not_contain_event("hello from b@1.0");
    h.assert_does_not_contain_event("hello from b@1.1");
}

#[test]
fn nodep_would_be_fulfilled_if_non_dev_dep() {
    let (h, registry) = set_up();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='b',version='1.0')",
            "bazel_dep(name='c',version='1.0')",
        ],
    );

    registry
        .add_module(
            create_module_key("b", "1.0"),
            &["module(name='b', version='1.0');bazel_dep(name='d', version='1.0')"],
        )
        .add_module(
            create_module_key("c", "1.0"),
            &[
                "module(name='c', version='1.0')",
                "bazel_dep(name='d',version='1.1',repo_name=None,dev_dependency=True)",
            ],
        )
        .add_module(
            create_module_key("d", "1.0"),
            &["module(name='d', version='1.0')"],
        )
        .add_module(
            create_module_key("d", "1.1"),
            &["module(name='d', version='1.1')"],
        );

    let result = evaluate_resolution(&h);
    assert_no_error(&result);
    let value = resolution_value(&result);
    let dep_graph = &value.resolved_dep_graph;
    assert!(!dep_graph.contains_key(&create_module_key("d", "1.1")));
    assert_eq!(
        dep_graph[&create_module_key("b", "1.0")].deps["d"].version(),
        &Version::parse("1.0").unwrap()
    );
    assert!(
        !dep_graph[&create_module_key("c", "1.0")]
            .deps
            .contains_key("d")
    );
}

/// The resolved graph keeps the RepoSpecs from REPO_SPEC and their registry file hashes
/// (`BazelModuleResolutionFunction.compute`); the MODULE.bazel hashes come from MODULE_FILE.
#[test]
fn repo_specs_and_registry_file_hashes() {
    let (h, registry) = set_up();
    setup_modules_for_yanked_version(&h, &registry);
    let mut h = h;
    h.allowed_yanked_versions = vec!["all".to_string()];
    let result = evaluate_resolution(&h);
    assert_no_error(&result);
    let value = resolution_value(&result);
    let url = registry.url();
    let keys: Vec<&String> = value.registry_file_hashes.keys().collect();
    assert_eq!(
        keys,
        vec![
            &format!("{url}/modules/a/1.0/MODULE.bazel"),
            &format!("{url}/modules/b/1.0/MODULE.bazel"),
            &format!("{url}/modules/a/1.0/source.json"),
            &format!("{url}/modules/b/1.0/source.json"),
        ]
    );
    let a = &value.resolved_dep_graph[&create_module_key("a", "1.0")];
    assert_eq!(
        a.repo_spec,
        Some(blaze_bzlmod_model::local_path_repo_spec("/foo/a+1.0"))
    );
    assert_eq!(
        value.selected_yanked_versions,
        IndexMap::from([(
            create_module_key("b", "1.0"),
            "1.0 is a bad version!".to_string()
        )])
    );
}

/// Blaze-only: the registry functions without MODULE.bazel evaluation. Expected values follow
/// the Java of `YankedVersionsFunction` and `RegistryFunction`.
mod registry_functions {
    use super::*;

    #[test]
    fn yanked_versions_function_reads_registry() {
        let (h, registry) = set_up();
        registry.add_yanked_version(
            "b",
            IndexMap::from([(Version::parse("1.0").unwrap(), "bad".to_string())]),
        );
        let key = YankedVersionsKey::create("b", registry.url());
        let result = h.evaluate(&[SkyKey::new(key.clone())]);
        assert_no_error(&result);
        assert_eq!(
            *result.get(&key).unwrap(),
            YankedVersionsValue::create(Some(IndexMap::from([(
                Version::parse("1.0").unwrap(),
                "bad".to_string()
            )])))
        );
        let unknown = YankedVersionsKey::create("zzz", registry.url());
        let result = h.evaluate(&[SkyKey::new(unknown.clone())]);
        assert_eq!(
            *result.get(&unknown).unwrap(),
            YankedVersionsValue::create(None)
        );
    }

    #[test]
    fn registry_function_invalid_url() {
        let h = Harness::new();
        let mut config = h.config();
        config.registry_factory = Arc::new(RegistryFactoryImpl::new(Box::new(
            IndexMap::<String, String>::new,
        )));
        let evaluator = blaze_bzlmod::skyframe::register_bzlmod_functions(
            blaze_skyframe::EvaluatorBuilder::new(),
            Arc::new(config),
        )
        .build();
        let key = RegistryKey::create("no-scheme");
        let result = evaluator
            .evaluate(&[SkyKey::new(key)], &h.evaluation_context())
            .unwrap();
        assert!(result.has_error());
        let error = error_string(&result);
        assert!(
            error.contains("Invalid registry URL: no-scheme: "),
            "{error}"
        );
        assert!(
            error.contains(
                "Registry URL has no scheme -- supported schemes are: http://, https:// and \
                 file://"
            ),
            "{error}"
        );
    }

    #[test]
    fn registry_function_replaces_workspace() {
        let h = Harness::new();
        let mut config = h.config();
        config.registry_factory = Arc::new(RegistryFactoryImpl::new(Box::new(
            IndexMap::<String, String>::new,
        )));
        let evaluator = blaze_bzlmod::skyframe::register_bzlmod_functions(
            blaze_skyframe::EvaluatorBuilder::new(),
            Arc::new(config),
        )
        .build();
        let key = RegistryKey::create("file://%workspace%/registry");
        let result = evaluator
            .evaluate(&[SkyKey::new(key.clone())], &h.evaluation_context())
            .unwrap();
        assert_no_error(&result);
        assert_eq!(
            result.get(&key).unwrap().get_url(),
            format!("file://{}/registry", h.root_directory.display())
        );
    }
}
