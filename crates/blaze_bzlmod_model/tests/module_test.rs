//! Ports of `ModuleTest` and `InterimModuleTest`, plus `ModuleKey` canonical repo names.

mod common;

use blaze_bzlmod_model::{
    DepSpec, InterimModule, ModuleKey, ModuleOverride, RepoSpec, SingleVersionOverride, Version,
    local_path_repo_spec,
};
use blaze_cmdline::{Label, RepositoryName};
use common::*;
use indexmap::IndexMap;

#[test]
fn get_repo_mapping() {
    let key = create_module_key("test_module", "1.0");
    let foo_key = create_module_key("foo", "1.0");
    let bar_key = create_module_key("bar", "2.0");
    let mut module = build_module("test_module", "1.0");
    module.deps.insert("my_foo".into(), foo_key.clone());
    module.deps.insert("my_bar".into(), bar_key.clone());
    module.deps.insert("my_root".into(), ModuleKey::root());
    let lookup: IndexMap<ModuleKey, RepositoryName> =
        [key.clone(), foo_key, bar_key, ModuleKey::root()]
            .into_iter()
            .map(|k| {
                let n = k.get_canonical_repo_name_without_version();
                (k, n)
            })
            .collect();
    assert_eq!(
        module.get_repo_mapping_with_bazel_deps_only(&lookup),
        create_repository_mapping(
            &key,
            &[
                "test_module",
                "test_module+",
                "my_foo",
                "foo+",
                "my_bar",
                "bar+",
                "my_root",
                ""
            ]
        )
    );
}

#[test]
fn get_repo_mapping_as_main_module() {
    let foo_key = create_module_key("foo", "1.0");
    let bar_key = create_module_key("bar", "2.0");
    let mut module = build_module("test_module", "1.0");
    module.key = ModuleKey::root();
    module.deps.insert("my_foo".into(), foo_key.clone());
    module.deps.insert("my_bar".into(), bar_key.clone());
    let lookup: IndexMap<ModuleKey, RepositoryName> = [ModuleKey::root(), foo_key, bar_key]
        .into_iter()
        .map(|k| {
            let n = if k.is_root() {
                // getCanonicalRepoNameWithVersion of ROOT is the main repo.
                k.get_canonical_repo_name_without_version()
            } else {
                k.get_canonical_repo_name_with_version()
            };
            (k, n)
        })
        .collect();
    assert_eq!(
        module.get_repo_mapping_with_bazel_deps_only(&lookup),
        create_repository_mapping(
            &ModuleKey::root(),
            &[
                "",
                "",
                "test_module",
                "",
                "my_foo",
                "foo+1.0",
                "my_bar",
                "bar+2.0"
            ]
        )
    );
}

#[test]
#[ignore = "Windows only (WindowsPathOperations.isShortPath)"]
fn get_canonical_repo_name_is_not_a_windows_short_path() {}

// InterimModuleTest.withDepsTransformed
#[test]
fn with_deps_transformed() {
    let transformed = InterimModuleBuilder::create("", "")
        .add_dep("dep_foo", create_module_key("foo", "1.0"))
        .add_dep("dep_bar", create_module_key("bar", "2.0"))
        .add_nodep_dep(create_module_key("quux", "3.0"))
        .build()
        .with_deps_transformed(|dep_spec| {
            DepSpec::from_module_key(&create_module_key(
                &format!("{}_new", dep_spec.name),
                &format!("{}.1", dep_spec.version.get_normalized()),
            ))
        });
    assert_eq!(
        transformed,
        InterimModuleBuilder::create("", "")
            .add_dep("dep_foo", create_module_key("foo_new", "1.0.1"))
            .add_original_dep("dep_foo", create_module_key("foo", "1.0"))
            .add_dep("dep_bar", create_module_key("bar_new", "2.0.1"))
            .add_original_dep("dep_bar", create_module_key("bar", "2.0"))
            .add_nodep_dep(create_module_key("quux_new", "3.0.1"))
            .build()
    );
}

// Canonical repository names as Bazel 9.3.2 produces them (ModuleKey.getCanonicalRepoName).
#[test]
fn canonical_repo_names() {
    let k = create_module_key("rules_cc", "0.2.14");
    assert_eq!(
        k.get_canonical_repo_name_without_version().get_name(),
        "rules_cc+"
    );
    assert_eq!(
        k.get_canonical_repo_name_with_version().get_name(),
        "rules_cc+0.2.14"
    );
    assert_eq!(
        create_module_key("bazel_tools", "")
            .get_canonical_repo_name_without_version()
            .get_name(),
        "bazel_tools"
    );
    assert_eq!(
        create_module_key("platforms", "1.0.0")
            .get_canonical_repo_name_with_version()
            .get_name(),
        "platforms"
    );
    assert_eq!(
        ModuleKey::root()
            .get_canonical_repo_name_without_version()
            .get_name(),
        ""
    );
    // Non-registry override: empty version.
    assert_eq!(
        create_module_key("foo", "")
            .get_canonical_repo_name_without_version()
            .get_name(),
        "foo+"
    );
    assert_eq!(create_module_key("foo", "").to_string(), "foo@_");
    assert_eq!(ModuleKey::root().to_string(), "<root>");
    assert_eq!(ModuleKey::root().to_display_string(), "root module");
    assert_eq!(k.to_display_string(), "module 'rules_cc@0.2.14'");
    assert_eq!(
        ModuleKey::from_string("foo@_").unwrap(),
        create_module_key("foo", "")
    );
    assert_eq!(ModuleKey::from_string("<root>").unwrap(), ModuleKey::root());
    assert_eq!(
        ModuleKey::from_string("a@1.2+b").unwrap(),
        create_module_key("a", "1.2")
    );
}

#[test]
#[should_panic(expected = "IllegalStateException")]
fn canonical_repo_name_with_version_requires_version() {
    create_module_key("foo", "").get_canonical_repo_name_with_version();
}

#[test]
fn repo_name_validation_uses_cmdline_rules() {
    assert!(
        RepositoryName::create("rules_python++python+python_3_11_x86_64-unknown-linux-gnu").is_ok()
    );
    assert!(RepositoryName::create("foo bar").is_err());
}

// InterimModule.toModule appends single_version_override patches to the registry repo spec.
#[test]
fn to_module_appends_single_version_override_patches() {
    let interim = InterimModuleBuilder::create("foo", "1.0")
        .add_dep("bar", create_module_key("bar", "2.0"))
        .build();
    let spec = local_path_repo_spec("/x");
    let override_ = ModuleOverride::SingleVersion(SingleVersionOverride {
        version: Version::empty(),
        registry: String::new(),
        patches: vec![Label::parse_canonical_unchecked("//:a.patch")],
        patch_cmds: vec!["echo".into()],
        patch_strip: 1,
    });
    let module = InterimModule::to_module(&interim, Some(&override_), Some(&spec));
    assert_eq!(module.deps["bar"], create_module_key("bar", "2.0"));
    let attrs = module.repo_spec.unwrap();
    let names: Vec<&str> = attrs
        .attributes()
        .attributes()
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(names, ["path", "patches", "patch_cmds", "patch_args"]);
    assert_eq!(
        attrs.attributes().get("patch_args").unwrap(),
        &blaze_bzlmod_model::AttributeValue::string_list(["-p1"])
    );
    // No patches: the repo spec is kept as is.
    let module = InterimModule::to_module(&interim, None, Some(&spec));
    assert_eq!(module.repo_spec, Some::<RepoSpec>(spec));
}
