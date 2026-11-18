//! Port of `ModuleFileFunctionTest` (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).
//!
//! These tests evaluate MODULE_FILE through the Skyframe functions of `blaze_bzlmod::skyframe`
//! and need MODULE.bazel evaluation (`crate::module_file`).

mod skyframe_common;

use std::sync::Arc;

use blaze_bzlmod::skyframe::{ModuleFileKey, ModuleFileValue};
use blaze_bzlmod_model::{
    ArchiveRepoSpecBuilder, ModuleKey, ModuleOverride, MultipleVersionOverride,
    NonRegistryOverride, Registry, SingleVersionOverride, Version, local_path_repo_spec,
};
use blaze_cmdline::RepositoryName;
use blaze_skyframe::{EvaluationResult, SkyKey};
use indexmap::IndexMap;
use skyframe_common::*;

fn root_key() -> SkyKey {
    SkyKey::new(ModuleFileKey::key_for_root_module())
}

fn root_value(result: &EvaluationResult) -> Arc<ModuleFileValue> {
    result
        .get(&ModuleFileKey::key_for_root_module())
        .expect("root module file value")
}

fn svo(version: &str) -> ModuleOverride {
    ModuleOverride::SingleVersion(SingleVersionOverride {
        version: Version::parse(version).unwrap(),
        registry: String::new(),
        patches: vec![],
        patch_cmds: vec![],
        patch_strip: 0,
    })
}

/// A harness with one fake registry at "/foo" set as the only registry.
fn with_registry() -> Harness {
    let mut h = Harness::new();
    let registry = h.registry_factory.new_fake_registry("/foo");
    h.set_registries(&[registry.url()]);
    h
}

#[test]
fn test_root_module() {
    let mut h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(",
            "    name='aaa',",
            "    version='0.1',",
            "    compatibility_level=4,",
            ")",
            "bazel_dep(name='bbb',version='1.0')",
            "bazel_dep(name='ccc',version='2.0',repo_name='see')",
            "bazel_dep(name='ddd',version='3.0',repo_name=None)",
            "bazel_dep(name='ggg',repo_name='gee')",
            "register_toolchains('//my:toolchain', '//my:toolchain2')",
            "register_execution_platforms('//my:platform', '//my:platform2')",
            "flag_alias(name = 'native_flag1', starlark_flag = '//my:starlark_label1')",
            "flag_alias(name = 'native_flag2', starlark_flag = '//my:starlark_label2')",
            "single_version_override(module_name='ddd',version='18')",
            "local_path_override(module_name='eee',path='somewhere/else')",
            "multiple_version_override(module_name='fff',versions=['1.0','2.0'])",
            "archive_override(module_name='ggg',urls=['https://hello.com/world.zip'])",
        ],
    );
    let registry = h.registry_factory.new_fake_registry("/foo");
    h.set_registries(&[registry.url()]);

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    let value = root_value(&result);
    let root = value.as_root();
    assert_eq!(
        root.module,
        InterimModuleBuilder::create("aaa", "0.1")
            .set_key(ModuleKey::root())
            .add_execution_platforms_to_register(&["//my:platform", "//my:platform2"])
            .add_toolchains_to_register(&["//my:toolchain", "//my:toolchain2"])
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .add_dep("see", create_module_key("ccc", "2.0"))
            .add_dep("gee", create_module_key("ggg", ""))
            .add_flag_alias("native_flag1", "@aaa//my:starlark_label1")
            .add_flag_alias("native_flag2", "@aaa//my:starlark_label2")
            .add_nodep_dep(create_module_key("ddd", "3.0"))
            .build()
    );
    let mut archive = ArchiveRepoSpecBuilder::new();
    archive.set_urls(vec!["https://hello.com/world.zip".to_string()]);
    let expected_overrides: IndexMap<String, ModuleOverride> = IndexMap::from([
        ("ddd".to_string(), svo("18")),
        (
            "eee".to_string(),
            ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec(
                "somewhere/else",
            ))),
        ),
        (
            "fff".to_string(),
            ModuleOverride::MultipleVersion(MultipleVersionOverride {
                versions: vec![
                    Version::parse("1.0").unwrap(),
                    Version::parse("2.0").unwrap(),
                ],
                registry: String::new(),
            }),
        ),
        (
            "ggg".to_string(),
            ModuleOverride::NonRegistry(NonRegistryOverride::new(archive.build())),
        ),
    ]);
    assert_eq!(root.overrides, expected_overrides);
    assert_eq!(
        root.non_registry_override_canonical_repo_to_module_name,
        IndexMap::from([
            (RepositoryName::create("eee+").unwrap(), "eee".to_string()),
            (RepositoryName::create("ggg+").unwrap(), "ggg".to_string()),
        ])
    );
    assert_eq!(
        root.non_registry_override_module_to_repo_name,
        IndexMap::from([("ggg".to_string(), "gee".to_string())])
    );
}

#[test]
fn test_root_module_no_module_function_is_okay() {
    let h = with_registry();
    h.scratch_file("MODULE.bazel", &["bazel_dep(name='bbb',version='1.0')"]);

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    let value = root_value(&result);
    let root = value.as_root();
    assert_eq!(
        root.module,
        InterimModuleBuilder::create("", "")
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .build()
    );
    assert!(root.overrides.is_empty());
    assert!(
        root.non_registry_override_canonical_repo_to_module_name
            .is_empty()
    );
    assert!(root.non_registry_override_module_to_repo_name.is_empty());
}

#[test]
fn test_root_module_bad_self_override() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "single_version_override(module_name='aaa',version='7')",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    assert!(error_string(&result).contains("invalid override for the root module"));
}

#[test]
fn test_root_module_override_builtin_module() {
    let root_dir_placeholder = Harness::new();
    let builtin_modules = IndexMap::from([(
        "bazel_tools".to_string(),
        NonRegistryOverride::new(local_path_repo_spec(
            &root_dir_placeholder
                .root_directory
                .join("bazel_tools_original")
                .to_string_lossy(),
        )),
    )]);
    let mut h = Harness::set_up_with_builtin_modules(builtin_modules);
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "local_path_override(module_name='bazel_tools',path='./bazel_tools_new')",
        ],
    );
    let registry = h.registry_factory.new_fake_registry("/foo");
    h.set_registries(&[registry.url()]);

    let result = h.evaluate(&[root_key()]);
    let value = root_value(&result);
    let bazel_tools_override = value.as_root().overrides.get("bazel_tools").cloned();
    assert_eq!(
        bazel_tools_override,
        Some(ModuleOverride::NonRegistry(NonRegistryOverride::new(
            local_path_repo_spec("./bazel_tools_new")
        )))
    );
}

#[test]
fn test_root_module_overrides_ignored_with_ignore_dev_dependency() {
    let mut h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='aaa')",
            "single_version_override(module_name='ddd',version='18')",
            "local_path_override(module_name='eee',path='somewhere/else')",
            "multiple_version_override(module_name='fff',versions=['1.0','2.0'])",
            "archive_override(module_name='ggg',urls=['https://hello.com/world.zip'])",
        ],
    );
    h.ignore_dev_deps = true;

    let result = h.evaluate(&[root_key()]);
    assert!(root_value(&result).as_root().overrides.is_empty());
}

#[test]
fn test_root_module_include_good() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "include('//java:java.MODULE.bazel')",
            "bazel_dep(name='foo', version='1.0')",
            "register_toolchains('//:whatever')",
            "include('//python:python.MODULE.bazel', dev_dependency=True)",
        ],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["bazel_dep(name='java-foo', version='1.0')"],
    );
    h.scratch_file("python/BUILD", &[]);
    h.scratch_file(
        "python/python.MODULE.bazel",
        &[
            "bazel_dep(name='py-foo', version='1.0', repo_name='python-foo')",
            "single_version_override(module_name='java-foo', version='2.0')",
            "include('//python:toolchains/toolchains.MODULE.bazel')",
        ],
    );
    h.scratch_file(
        "python/toolchains/toolchains.MODULE.bazel",
        &["register_toolchains('//:python-whatever')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    let value = root_value(&result);
    let root = value.as_root();
    let expected_module = InterimModuleBuilder::create("aaa", "")
        .set_key(ModuleKey::root())
        .add_dep("java-foo", create_module_key("java-foo", "1.0"))
        .add_dep("foo", create_module_key("foo", "1.0"))
        .add_dep("python-foo", create_module_key("py-foo", "1.0"))
        .add_toolchains_to_register(&["//:whatever", "//:python-whatever"])
        .build();
    assert_eq!(root.module, expected_module);
    // specifically assert the order of deps, which is significant; Map.equals semantics don't
    // test this.
    assert_eq!(
        root.module.deps.iter().collect::<Vec<_>>(),
        expected_module.deps.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        root.overrides,
        IndexMap::from([("java-foo".to_string(), svo("2.0"))])
    );
}

#[test]
#[ignore = "needs FILE (FileValue) to warm a node before a Skyframe restart; the blocking evaluator never restarts"]
fn test_root_module_include_nested_include_survives_restart() {}

#[test]
fn test_root_module_dev_include_ignored_with_ignore_dev_dependency() {
    let mut h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "bazel_dep(name='foo', version='1.0')",
            "include('//missing:dev.MODULE.bazel', dev_dependency=True)",
            "include('//prod:prod.MODULE.bazel')",
        ],
    );
    h.scratch_file("prod/BUILD", &[]);
    h.scratch_file(
        "prod/prod.MODULE.bazel",
        &[
            "bazel_dep(name='bar', version='2.0')",
            "include('//missing:nested-dev.MODULE.bazel', dev_dependency=True)",
        ],
    );
    h.ignore_dev_deps = true;

    let result = h.evaluate(&[root_key()]);
    assert!(!result.has_error());
    let deps: Vec<_> = root_value(&result)
        .as_root()
        .module
        .deps
        .clone()
        .into_iter()
        .collect();
    assert_eq!(
        deps,
        vec![
            ("foo".to_string(), create_dep_spec("foo", "1.0", -1)),
            ("bar".to_string(), create_dep_spec("bar", "2.0", -1)),
        ]
    );
}

fn assert_root_error_contains(lines: &[&str], text: &str) {
    let h = with_registry();
    h.scratch_file("MODULE.bazel", lines);
    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    let error = error_string(&result);
    assert!(error.contains(text), "{error:?} does not contain {text:?}");
}

#[test]
fn test_root_module_include_bad_other_repo_label() {
    assert_root_error_contains(
        &[
            "module(name='aaa')",
            "include('@haha//java:java.MODULE.bazel')",
        ],
        "starting with double slashes",
    );
}

#[test]
fn test_root_module_include_bad_relative_label() {
    assert_root_error_contains(
        &["module(name='aaa')", "include(':relative.MODULE.bazel')"],
        "starting with double slashes",
    );
}

#[test]
fn test_root_module_include_bad_not_ending_in_module_bazel() {
    assert_root_error_contains(
        &["module(name='aaa')", "include('//:MODULE.bazel.segment')"],
        "have a name ending in '.MODULE.bazel'",
    );
}

#[test]
fn test_root_module_include_bad_starts_with_period() {
    assert_root_error_contains(
        &["module(name='aaa')", "include('//pkg:dir/.MODULE.bazel')"],
        "the name of the file to be included must not start with '.'",
    );
}

#[test]
fn test_root_module_include_bad_bad_label_syntax() {
    assert_root_error_contains(
        &["module(name='aaa')", "include('//haha/:::.MODULE.bazel')"],
        "bad include label",
    );
}

#[test]
fn test_root_module_include_bad_self_cycle() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa')", "include('//java:java.MODULE.bazel')"],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["include('//java:java.MODULE.bazel')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "include() cycle detected: //java:java.MODULE.bazel -> //java:java.MODULE.bazel",
    );
}

#[test]
fn test_root_module_include_bad_indirect_cycle() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa')", "include('//java:java.MODULE.bazel')"],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["include('//python:python.MODULE.bazel')"],
    );
    h.scratch_file("python/BUILD", &[]);
    h.scratch_file(
        "python/python.MODULE.bazel",
        &["include('//java:java.MODULE.bazel')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "include() cycle detected: //java:java.MODULE.bazel -> //python:python.MODULE.bazel -> \
         //java:java.MODULE.bazel",
    );
}

#[test]
fn test_root_module_include_same_file_multiple_times() {
    // The same file can be reached via multiple include() paths; it is then executed once per
    // path, just as if its contents had been inlined at each call site.
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "include('//java:java.MODULE.bazel')",
            "include('//python:python.MODULE.bazel')",
        ],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["include('//common:common.MODULE.bazel')"],
    );
    h.scratch_file("python/BUILD", &[]);
    h.scratch_file(
        "python/python.MODULE.bazel",
        &["include('//common:common.MODULE.bazel')"],
    );
    h.scratch_file("common/BUILD", &[]);
    h.scratch_file(
        "common/common.MODULE.bazel",
        &["register_toolchains('//:common-toolchain')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    assert_eq!(
        root_value(&result).as_root().module.toolchains_to_register,
        vec!["//:common-toolchain", "//:common-toolchain"]
    );
}

#[test]
fn test_root_module_include_bad_module_after_include() {
    let h = with_registry();
    h.scratch_file("MODULE.bazel", &["include('//java:java.MODULE.bazel')"]);
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &[
            "module(name='bet-you-didnt-expect-this-didya')",
            "bazel_dep(name='java-foo', version='1.0', repo_name='foo')",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event("if module() is called, it must be called before any other functions");
}

#[test]
fn test_root_module_include_bad_repo_name_collision() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "include('//java:java.MODULE.bazel')",
            "include('//python:python.MODULE.bazel')",
        ],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["bazel_dep(name='java-foo', version='1.0', repo_name='foo')"],
    );
    h.scratch_file("python/BUILD", &[]);
    h.scratch_file(
        "python/python.MODULE.bazel",
        &["bazel_dep(name='python-foo', version='1.0', repo_name='foo')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event("The repo name 'foo' cannot be defined");
    h.assert_contains_event("as it is already defined");
}

#[test]
fn test_root_module_include_bad_trying_to_leak_bindings() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "FOO_NAME = 'foo'",
            "include('//java:java.MODULE.bazel')",
        ],
    );
    h.scratch_file("java/BUILD", &[]);
    h.scratch_file(
        "java/java.MODULE.bazel",
        &["bazel_dep(name=FOO_NAME, version='1.0')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    // PORT: starlark-rust's core-language error text (decisions.md); Java: "name 'FOO_NAME' is not
    // defined".
    h.assert_contains_event("FOO_NAME");
}

#[test]
fn forgot_version() {
    let h = with_registry();

    let sky_key = SkyKey::new(ModuleFileKey::key(create_module_key("bbb", "")));
    let result = h.evaluate(&[sky_key]);
    assert!(result.has_error());
    assert!(error_string(&result).contains("bad bazel_dep on module 'bbb' with no version"));
}

#[test]
fn test_registries_cascade() {
    // Registry1 has no module B@1.0; registry2 and registry3 both have it. We should be using the
    // B@1.0 from registry2.
    let mut h = Harness::new();
    let registry1 = h.registry_factory.new_fake_registry("/foo");
    let registry2 = h.registry_factory.new_fake_registry("/bar").add_module(
        create_module_key("bbb", "1.0"),
        &["module(name='bbb',version='1.0');bazel_dep(name='ccc',version='2.0')"],
    );
    let registry3 = h.registry_factory.new_fake_registry("/baz").add_module(
        create_module_key("bbb", "1.0"),
        &["module(name='bbb',version='1.0');bazel_dep(name='ddd',version='3.0')"],
    );
    h.set_registries(&[registry1.url(), registry2.url(), registry3.url()]);

    let key = ModuleFileKey::key(create_module_key("bbb", "1.0"));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert_no_error(&result);
    let module_file_value = result.get(&key).unwrap();
    assert_eq!(
        *module_file_value.module(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .set_registry(registry2.clone() as Arc<dyn Registry>)
            .build()
    );
}

#[test]
fn test_non_root_module_cannot_use_include() {
    let mut h = Harness::new();
    let registry = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "include('//java:MODULE.bazel.segment')",
        ],
    );
    h.set_registries(&[registry.url()]);

    let result = h.evaluate(&[SkyKey::new(ModuleFileKey::key(create_module_key(
        "foo", "1.0",
    )))]);
    assert!(result.has_error());
    assert!(error_string(&result).contains("but it can only be used in the root module"));
}

#[test]
fn test_registry_module_dev_include_is_ignored() {
    let mut h = Harness::new();
    let registry = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "include('//missing:dev.MODULE.bazel', dev_dependency=True)",
        ],
    );
    h.set_registries(&[registry.url()]);

    let key = ModuleFileKey::key(create_module_key("foo", "1.0"));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert!(!result.has_error());
    assert_eq!(result.get(&key).unwrap().module().name, "foo");
}

/// Java ignores this test (b/389163906: its BuildViewTestCase harness has many builtin
/// modules); this harness has none, so it runs.
#[test]
fn test_local_path_override() {
    // There is an override for B to use the local path "code_for_b", so we shouldn't even be
    // looking at the registry.
    let mut h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa',version='0.1')",
            "local_path_override(module_name='bbb',path='code_for_b')",
        ],
    );
    h.scratch_file(
        "code_for_b/MODULE.bazel",
        &[
            "module(name='bbb',version='1.0')",
            "bazel_dep(name='ccc',version='2.0')",
        ],
    );
    let registry = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("bbb", "1.0"),
        &["module(name='bbb',version='1.0');bazel_dep(name='ccc',version='3.0')"],
    );
    h.set_registries(&[registry.url()]);

    // The version is empty here due to the override.
    let key = ModuleFileKey::key(create_module_key("bbb", ""));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&key).unwrap().module(),
        InterimModuleBuilder::create("bbb", "1.0")
            .set_key(create_module_key("bbb", ""))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build()
    );
}

/// Java ignores this test (b/389163906); this harness has no builtin modules, so it runs.
#[test]
fn test_command_line_module_overrides() {
    let mut h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa',version='0.1')",
            "bazel_dep(name = \"bbb\", version = \"1.0\")",
            "local_path_override(module_name='bbb', path='ignored_override')",
        ],
    );

    // Command line override has the priority. Thus, "used_override" with dependency on 'ccc'
    // should be selected.
    h.scratch_file(
        "ignored_override/MODULE.bazel",
        &["module(name='bbb',version='1.0')"],
    );
    h.scratch_file(
        "used_override/MODULE.bazel",
        &[
            "module(name='bbb',version='1.0')",
            "bazel_dep(name='ccc',version='2.0')",
        ],
    );

    // ModuleFileFuncion.MODULE_OVERRIDES should be filled from command line options.
    h.module_overrides = IndexMap::from([(
        "bbb".to_string(),
        ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec(
            "used_override",
        ))),
    )]);

    let registry = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("bbb", "1.0"),
        &["module(name='bbb',version='1.0');bazel_dep(name='ccc',version='3.0')"],
    );
    h.set_registries(&[registry.url()]);

    // The version is empty here due to the override.
    let key = ModuleFileKey::key(create_module_key("bbb", ""));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&key).unwrap().module(),
        InterimModuleBuilder::create("bbb", "1.0")
            .set_key(create_module_key("bbb", ""))
            .add_dep("ccc", create_module_key("ccc", "2.0"))
            .build()
    );
}

#[test]
fn test_registry_override() {
    let mut h = Harness::new();
    let registry1 = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("bbb", "1.0"),
        &[
            "module(name='bbb',version='1.0',compatibility_level=4)",
            "bazel_dep(name='ccc',version='2.0')",
        ],
    );
    let registry2 = h.registry_factory.new_fake_registry("/foo").add_module(
        create_module_key("bbb", "1.0"),
        &[
            "module(name='bbb',version='1.0',compatibility_level=6)",
            "bazel_dep(name='ccc',version='3.0')",
        ],
    );
    let module_bazel = format!(
        "module(name='aaa',version='0.1')\n\
         bazel_dep(name = \"bbb\", version = \"1.0\")\n\
         single_version_override(module_name='bbb', registry='{}')",
        registry2.url()
    );
    h.scratch_file("MODULE.bazel", &[&module_bazel]);
    h.set_registries(&[registry1.url()]);

    // Override the registry for B to be registry2 (instead of the default registry1).
    let key = ModuleFileKey::key(create_module_key("bbb", "1.0"));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&key).unwrap().module(),
        InterimModuleBuilder::create("bbb", "1.0")
            .add_dep("ccc", create_module_key("ccc", "3.0"))
            .set_registry(registry2.clone() as Arc<dyn Registry>)
            .build()
    );
}

#[test]
#[ignore = "not ported yet: asserts ModuleExtensionUsage values built by ModuleFileGlobals (agent A's use_extension tests)"]
fn test_module_extensions_good() {}

#[test]
#[ignore = "not ported yet: asserts ModuleExtensionUsage values built by ModuleFileGlobals"]
fn test_module_extensions_duplicate_proxy_as_root() {}

#[test]
#[ignore = "not ported yet: asserts ModuleExtensionUsage values built by ModuleFileGlobals"]
fn test_module_extensions_duplicate_proxy_as_dep() {}

#[test]
#[ignore = "not ported yet: asserts ModuleFileGlobals use_repo errors"]
fn test_module_extensions_repo_name_collision_local_repo_name() {}

#[test]
#[ignore = "not ported yet: asserts ModuleFileGlobals use_repo errors"]
fn test_module_extensions_repo_name_collision_exported_repo_name() {}

#[test]
#[ignore = "not ported yet: asserts ModuleExtensionUsage values of use_repo_rule"]
fn test_module_extensions_innate() {}

#[test]
fn test_module_file_execute_syntax_error() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa',version='0.1')", "foo()"],
    );

    h.evaluate(&[root_key()]);
    // PORT: starlark-rust's core-language error text (decisions.md); Java: "name 'foo' is not
    // defined".
    h.assert_contains_event("foo");
}

#[test]
fn test_module_file_execute_eval_error() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa',version='0.1',compatibility_level=\"4\")"],
    );

    h.evaluate(&[root_key()]);
    h.assert_contains_event(
        "parameter 'compatibility_level' got value of type 'string', want 'int'",
    );
}

#[test]
fn test_module_file_execute_compatibility_level_warning() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa',version='0.1',compatibility_level=4)",
            "bazel_dep(name='bbb',version='1.0',max_compatibility_level=5)",
        ],
    );

    h.evaluate(&[root_key()]);
    h.assert_contains_event("The attribute 'compatibility_level' in module() is a no-op");
    h.assert_contains_event("The attribute 'max_compatibility_level' in bazel_dep() is a no-op");
}

#[test]
#[ignore = "ModuleFileGlobals.validateModuleName is agent A's (module_file) unit test"]
fn validate_module_name() {}

fn assert_root_event(lines: &[&str], text: &str) {
    let h = with_registry();
    h.scratch_file("MODULE.bazel", lines);
    h.evaluate(&[root_key()]);
    h.assert_contains_event(text);
}

#[test]
fn bad_module_name_module() {
    assert_root_event(
        &["module(name='f.',version='0.1')"],
        "invalid module name 'f.'",
    );
}

#[test]
fn bad_module_name_bazel_dep() {
    assert_root_event(
        &["bazel_dep(name='f.',version='0.1')"],
        "invalid module name 'f.'",
    );
}

#[test]
fn bad_repo_name_module() {
    assert_root_event(
        &["module(name='foo',version='0.1',repo_name='_foo')"],
        "invalid user-provided repo name '_foo'",
    );
}

#[test]
fn bad_repo_name_bazel_dep() {
    assert_root_event(
        &["bazel_dep(name='foo',version='0.1',repo_name='_foo')"],
        "invalid user-provided repo name '_foo'",
    );
}

#[test]
fn bad_repo_name_use_repo() {
    assert_root_event(
        &[
            "ext=use_extension('//:hello.bzl', 'ext')",
            "use_repo(ext, foo='_foo')",
        ],
        "invalid user-provided repo name '_foo'",
    );
}

#[test]
fn bad_repo_name_use_repo_assigned_name() {
    assert_root_event(
        &[
            "ext=use_extension('//:hello.bzl', 'ext')",
            "use_repo(ext, _foo='foo')",
        ],
        "invalid user-provided repo name '_foo'",
    );
}

#[test]
fn test_builtin_modules_for_root() {
    let builtin_modules = IndexMap::from([
        (
            "bazel_tools".to_string(),
            NonRegistryOverride::new(local_path_repo_spec("/tools")),
        ),
        (
            "other_tools".to_string(),
            NonRegistryOverride::new(local_path_repo_spec("/other_tools")),
        ),
    ]);
    let mut h = Harness::set_up_with_builtin_modules(builtin_modules.clone());
    h.scratch_file("MODULE.bazel", &["bazel_dep(name='foo',version='1.0')"]);
    h.set_registries(&[]);

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    let value = root_value(&result);
    let root = value.as_root();
    assert_eq!(
        root.module,
        InterimModuleBuilder::create("", "")
            .add_dep("bazel_tools", create_module_key("bazel_tools", ""))
            .add_dep("other_tools", create_module_key("other_tools", ""))
            .add_dep("foo", create_module_key("foo", "1.0"))
            .build()
    );
    let expected: IndexMap<String, ModuleOverride> = builtin_modules
        .into_iter()
        .map(|(k, v)| (k, ModuleOverride::NonRegistry(v)))
        .collect();
    assert_eq!(root.overrides, expected);
}

/// Java ignores this test (b/389163906); this harness has no other builtin modules, so it runs.
#[test]
fn test_builtin_modules_for_builtin_modules() {
    let placeholder = Harness::new();
    let tools = placeholder.root_directory.join("tools");
    let builtin_modules = IndexMap::from([
        (
            "bazel_tools".to_string(),
            NonRegistryOverride::new(local_path_repo_spec(&tools.to_string_lossy())),
        ),
        (
            "other_tools".to_string(),
            NonRegistryOverride::new(local_path_repo_spec("/other_tools")),
        ),
    ]);
    let mut h = Harness::set_up_with_builtin_modules(builtin_modules);
    h.scratch_file("MODULE.bazel", &["bazel_dep(name='foo',version='1.0')"]);
    skyframe_common::write_file(
        &tools.join("MODULE.bazel"),
        "module(name='bazel_tools',version='1.0')\nbazel_dep(name='foo',version='2.0')\n",
    );
    // PORT: BuiltinRepositoryDirectoryLookup answers @bazel_tools from the install base's
    // embedded_tools; point it at the override's path instead.
    std::fs::create_dir_all(h.output_base.join("install")).unwrap();
    std::os::unix::fs::symlink(&tools, h.output_base.join("install/embedded_tools")).unwrap();
    h.set_registries(&[]);

    let key = ModuleFileKey::key(create_module_key("bazel_tools", ""));
    let result = h.evaluate(&[SkyKey::new(key.clone())]);
    assert_no_error(&result);
    assert_eq!(
        *result.get(&key).unwrap().module(),
        InterimModuleBuilder::create("bazel_tools", "1.0")
            .set_key(create_module_key("bazel_tools", ""))
            .add_dep("other_tools", create_module_key("other_tools", ""))
            .add_dep("foo", create_module_key("foo", "2.0"))
            .build()
    );
}

#[test]
fn module_repo_name() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa',version='0.1',repo_name='bbb')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert_no_error(&result);
    assert_eq!(
        root_value(&result).as_root().module,
        InterimModuleBuilder::create("aaa", "0.1")
            .set_key(ModuleKey::root())
            .set_repo_name("bbb")
            .build()
    );
}

#[test]
fn module_repo_name_conflict() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa',version='0.1',repo_name='bbb')",
            "bazel_dep(name='bbb',version='1.0')",
        ],
    );

    h.evaluate(&[root_key()]);
    h.assert_contains_event("The repo name 'bbb' cannot be defined");
    h.assert_contains_event("as it is already defined as the module's own repo name");
}

#[test]
fn module_called_twice() {
    assert_root_event(
        &[
            "module(name='aaa',version='0.1',repo_name='bbb')",
            "module(name='aaa',version='0.1',repo_name='bbb')",
        ],
        "the module() directive can only be called once",
    );
}

#[test]
fn module_called_late() {
    assert_root_event(
        &[
            "use_extension('//:extensions.bzl', 'my_ext')",
            "module(name='aaa',version='0.1',repo_name='bbb')",
        ],
        "if module() is called, it must be called before any other functions",
    );
}

#[test]
fn restricted_syntax() {
    assert_root_event(
        &["if 3+5>7: module(name='aaa',version='0.1',repo_name='bbb')"],
        "`if` statements are not allowed in MODULE.bazel files. You may use an `if` expression \
         for simple cases",
    );
}

#[test]
fn isolated_usage_without_imports() {
    let mut h = with_registry();
    h.starlark_semantics.experimental_isolated_extension_usages = true;
    h.scratch_file(
        "MODULE.bazel",
        &["isolated_ext = use_extension('//:extensions.bzl', 'my_ext', isolate = True)"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    let expected = format!(
        "the isolated usage at {}/MODULE.bazel:1:29 of extension my_ext defined in \
         @//:extensions.bzl has no effect as no repositories are imported from it. Either import \
         one or more repositories generated by the extension with use_repo or remove the usage.",
        h.root_directory.display()
    );
    assert!(error_string(&result).contains(&expected));
}

#[test]
fn isolated_usage_not_exported() {
    let mut h = with_registry();
    h.starlark_semantics.experimental_isolated_extension_usages = true;
    h.scratch_file(
        "MODULE.bazel",
        &["use_extension('//:extensions.bzl', 'my_ext', isolate = True)"],
    );

    h.evaluate(&[root_key()]);
    h.assert_contains_event(&format!(
        "Isolated extension usage at {}/MODULE.bazel:1:14 must be assigned to a top-level variable",
        h.root_directory.display()
    ));
}

#[test]
fn isolated_usage_not_enabled() {
    assert_root_event(
        &["use_extension('//:extensions.bzl', 'my_ext', isolate = True)"],
        "Error in use_extension: in call to use_extension(), parameter 'isolate' is experimental \
         and thus unavailable with the current flags. It may be enabled by setting \
         --experimental_isolated_extension_usages",
    );
}

#[test]
fn test_register_toolchains_single_package_restriction_under_dir() {
    let mut h = with_registry();
    h.starlark_semantics
        .experimental_single_package_toolchain_binding = true;
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa')", "register_toolchains('//bar/...')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "invalid target pattern \"//bar/...\": register_toolchain target patterns may only refer \
         to targets within a single package",
    );
}

#[test]
fn test_register_toolchains_path_syntax() {
    let h = with_registry();
    h.scratch_file(
        "MODULE.bazel",
        &["module(name='aaa')", "register_toolchains('bar/baz')"],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "Expected absolute target patterns (must begin with '//' or '@') for \
         'register_toolchains' argument, but got 'bar/baz' as an argument",
    );
}

#[test]
fn test_register_toolchains_single_package_restriction() {
    let mut h = with_registry();
    h.starlark_semantics
        .experimental_single_package_toolchain_binding = true;
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "register_toolchains('//:whatever')",
            "register_toolchains('//bar:all')",
            "register_toolchains('//qux:baz')",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(!result.has_error());
}

#[test]
fn test_no_usages() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "http_archive = use_repo_rule('@bazel_tools//tools/build_defs/repo:http.bzl', \
             'http_archive')",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(!result.has_error());
    assert!(
        root_value(&result)
            .as_root()
            .module
            .extension_usages
            .is_empty()
    );
}

#[test]
fn test_invalid_repo_in_single_version_override_patches() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "single_version_override(",
            "    module_name = 'bbb',",
            "    version = '1.0',",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "Error in single_version_override: invalid label in 'patches': only patches in the main \
         repository can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_repo_in_archive_override_patches() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "archive_override(",
            "    module_name = 'bbb',",
            "    urls = ['https://example.com'],",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "Error in archive_override: invalid label in 'patches': only patches in the main \
         repository can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_repo_in_git_override_patches() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "git_override(",
            "    module_name = 'bbb',",
            "    remote = 'https://example.com',",
            "    commit = 'abcd1234',",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "Error in git_override: invalid label in 'patches': only patches in the main repository \
         can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_use_extension_label() {
    let h = Harness::new();
    h.scratch_file(
        "MODULE.bazel",
        &[
            "module(name='aaa')",
            "use_extension('@foo/bar:extensions.bzl', 'my_ext')",
        ],
    );

    let result = h.evaluate(&[root_key()]);
    assert!(result.has_error());
    h.assert_contains_event(
        "Error in use_extension: invalid label \"@foo/bar:extensions.bzl\": invalid repository \
         name 'foo/bar:extensions.bzl': repo names may contain only A-Z, a-z, 0-9, '-', '_', '.' \
         and '+'",
    );
}

#[test]
#[ignore = "needs PatchUtil.applyToSingleFile (blaze_repo_fetch, which cannot build next to starlark yet)"]
fn test_single_version_override_patches() {}

#[test]
#[ignore = "needs PatchUtil.applyToSingleFile (blaze_repo_fetch, which cannot build next to starlark yet)"]
fn test_single_version_override_patches_default_patch_strip_for_git_patch() {}

#[test]
#[ignore = "needs PatchUtil.applyToSingleFile (blaze_repo_fetch, which cannot build next to starlark yet)"]
fn test_single_version_override_patches_fails_on_rename() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_overriding_repo_doesnt_exist() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_duplicate_override() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_chain_single_extension() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_chain_multiple_extensions() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_cycle_single_extension() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_cycle_multiple_extensions() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_extra_keyword_arguments_literal() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_override_repo_extra_keyword_arguments_non_literal() {}

#[test]
#[ignore = "not ported yet: asserts override_repo/inject_repo usages built by ModuleFileGlobals"]
fn test_inject_repo_imported() {}

/// Blaze-only: the ModuleFileFunction glue paths that need no MODULE.bazel evaluation.
mod glue {
    use super::*;

    /// `getModuleFile`: a registry miss in every registry is MODULE_NOT_FOUND with the trace of
    /// each registry's `NotFoundException` message (ModuleFileFunction.java:852).
    #[test]
    fn module_not_found_in_registries() {
        let mut h = Harness::new();
        h.scratch_file("MODULE.bazel", &[]);
        let r1 = h.registry_factory.new_fake_registry("/foo");
        let r2 = h.registry_factory.new_fake_registry("/bar");
        h.set_registries(&[r1.url(), r2.url()]);
        let result = h.evaluate(&[SkyKey::new(ModuleFileKey::key(create_module_key(
            "bbb", "1.0",
        )))]);
        assert!(result.has_error());
        let error = error_string(&result);
        assert!(
            error.contains(
                "module bbb@1.0 not found in registries:\n* module not found: bbb@1.0\n* module \
                 not found: bbb@1.0"
            ),
            "{error}"
        );
    }
}
