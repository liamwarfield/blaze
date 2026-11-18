//! Port of `BazelDepGraphFunctionTest`: the resolution function is mocked by passing the dep
//! graph to `BazelDepGraphFunction::compute` directly.

mod common;

use blaze_bzlmod_model::{
    AbridgedModule, BazelDepGraphFunction, ModuleExtensionId, ModuleExtensionUsage,
    ModuleExtensionUsageProxy, ModuleKey, RepoOverride,
};
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_cmdline::{Label, RepositoryName};
use blaze_events::Location;
use common::*;
use indexmap::IndexMap;

fn create_module_extension_usage(
    bzl_file: &str,
    name: &str,
    imports: &[&str],
) -> ModuleExtensionUsage {
    let mut imports_map = IndexMap::new();
    for pair in imports.chunks(2) {
        imports_map.insert(pair[0].to_string(), pair[1].to_string());
    }
    ModuleExtensionUsage {
        extension_bzl_file: bzl_file.into(),
        extension_name: name.into(),
        isolation_key: None,
        repo_overrides: IndexMap::new(),
        tags: Vec::new(),
        proxies: vec![ModuleExtensionUsageProxy {
            dev_dependency: false,
            location: Location::builtin().clone(),
            imports: imports_map,
            containing_module_file_path: MODULE_DOT_BAZEL_FILE_NAME.clone(),
            proxy_name: String::new(),
        }],
    }
}

#[test]
fn create_value_basic() {
    // Root depends on dep@1.0 and dep@2.0 at the same time with a multiple-version override.
    // Root also depends on rules_cc as a normal dep.
    // dep@1.0 depends on rules_java, which is overridden by a non-registry override.
    let mut root = build_module("my_root", "1.0");
    root.key = ModuleKey::root();
    root.deps
        .insert("my_dep_1".into(), create_module_key("dep", "1.0"));
    root.deps
        .insert("my_dep_2".into(), create_module_key("dep", "2.0"));
    root.deps
        .insert("rules_cc".into(), create_module_key("rules_cc", "1.0"));
    let mut dep1 = build_module("dep", "1.0");
    dep1.deps
        .insert("rules_java".into(), create_module_key("rules_java", ""));
    let mut rules_java = build_module("rules_java", "1.0");
    rules_java.key = create_module_key("rules_java", "");
    let dep_graph = IndexMap::from([
        (ModuleKey::root(), root),
        (create_module_key("dep", "1.0"), dep1),
        (create_module_key("dep", "2.0"), build_module("dep", "2.0")),
        (
            create_module_key("rules_cc", "1.0"),
            build_module("rules_cc", "1.0"),
        ),
        (create_module_key("rules_java", ""), rules_java),
    ]);
    let value = BazelDepGraphFunction::compute(&dep_graph).unwrap();
    let mut lookup: Vec<(RepositoryName, ModuleKey)> = value
        .canonical_repo_name_lookup
        .clone()
        .into_iter()
        .collect();
    lookup.sort_by(|a, b| a.0.get_name().cmp(b.0.get_name()));
    let mut want = vec![
        (RepositoryName::create("").unwrap(), ModuleKey::root()),
        (
            RepositoryName::create("dep+1.0").unwrap(),
            create_module_key("dep", "1.0"),
        ),
        (
            RepositoryName::create("dep+2.0").unwrap(),
            create_module_key("dep", "2.0"),
        ),
        (
            RepositoryName::create("rules_cc+").unwrap(),
            create_module_key("rules_cc", "1.0"),
        ),
        (
            RepositoryName::create("rules_java+").unwrap(),
            create_module_key("rules_java", ""),
        ),
    ];
    want.sort_by(|a, b| a.0.get_name().cmp(b.0.get_name()));
    assert_eq!(lookup, want);
    assert_eq!(
        value.abridged_modules,
        dep_graph
            .values()
            .map(AbridgedModule::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn create_value_module_extensions() {
    let rje_key = create_module_key("rules_jvm_external", "1.0");
    let rpy_key = create_module_key("rules_python", "2.0");
    let mut root = build_module("root", "1.0");
    root.key = ModuleKey::root();
    root.deps.insert("rje".into(), rje_key.clone());
    root.deps.insert("rpy".into(), rpy_key.clone());
    root.extension_usages.push(create_module_extension_usage(
        "@rje//:defs.bzl",
        "maven",
        &["av", "autovalue"],
    ));
    root.extension_usages.push(create_module_extension_usage(
        "@rpy//:defs.bzl",
        "pip",
        &["numpy", "numpy"],
    ));
    let dep_key = create_module_key("dep", "2.0");
    let mut dep = build_module("dep", "2.0");
    dep.deps.insert("rules_python".into(), rpy_key.clone());
    dep.extension_usages.push(create_module_extension_usage(
        "@rules_python//:defs.bzl",
        "pip",
        &["np", "numpy"],
    ));
    dep.extension_usages.push(create_module_extension_usage(
        "//:defs.bzl",
        "myext",
        &["oneext", "myext"],
    ));
    dep.extension_usages.push(create_module_extension_usage(
        "//incredible:conflict.bzl",
        "myext",
        &["twoext", "myext"],
    ));
    let dep_graph = IndexMap::from([
        (ModuleKey::root(), root.clone()),
        (dep_key.clone(), dep.clone()),
        (rje_key, build_module("rules_jvm_external", "1.0")),
        (rpy_key, build_module("rules_python", "2.0")),
    ]);

    let maven = ModuleExtensionId::create(
        Label::parse_canonical("@@rules_jvm_external+//:defs.bzl").unwrap(),
        "maven",
        None,
    );
    let pip = ModuleExtensionId::create(
        Label::parse_canonical("@@rules_python+//:defs.bzl").unwrap(),
        "pip",
        None,
    );
    let myext = ModuleExtensionId::create(
        Label::parse_canonical("@@dep+//:defs.bzl").unwrap(),
        "myext",
        None,
    );
    let myext2 = ModuleExtensionId::create(
        Label::parse_canonical("@@dep+//incredible:conflict.bzl").unwrap(),
        "myext",
        None,
    );

    let value = BazelDepGraphFunction::compute(&dep_graph).unwrap();
    let t = &value.extension_usages_table;
    assert_eq!(t.size(), 5);
    assert_eq!(
        t.get(&maven, &ModuleKey::root()),
        Some(&root.extension_usages[0])
    );
    assert_eq!(
        t.get(&pip, &ModuleKey::root()),
        Some(&root.extension_usages[1])
    );
    assert_eq!(t.get(&pip, &dep_key), Some(&dep.extension_usages[0]));
    assert_eq!(t.get(&myext, &dep_key), Some(&dep.extension_usages[1]));
    assert_eq!(t.get(&myext2, &dep_key), Some(&dep.extension_usages[2]));

    let mut names: Vec<(String, String)> = value
        .extension_unique_names
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    names.sort();
    let mut want = vec![
        (maven.to_string(), "rules_jvm_external++maven".to_string()),
        (pip.to_string(), "rules_python++pip".to_string()),
        (myext.to_string(), "dep++myext".to_string()),
        (myext2.to_string(), "dep++myext2".to_string()),
    ];
    want.sort();
    assert_eq!(names, want);

    assert_eq!(
        value.get_full_repo_mapping(&ModuleKey::root()),
        create_repository_mapping(
            &ModuleKey::root(),
            &[
                "",
                "",
                "root",
                "",
                "rje",
                "rules_jvm_external+",
                "rpy",
                "rules_python+",
                "av",
                "rules_jvm_external++maven+autovalue",
                "numpy",
                "rules_python++pip+numpy",
            ]
        )
    );
    assert_eq!(
        value.get_full_repo_mapping(&dep_key),
        create_repository_mapping(
            &dep_key,
            &[
                "dep",
                "dep+",
                "rules_python",
                "rules_python+",
                "np",
                "rules_python++pip+numpy",
                "oneext",
                "dep++myext+myext",
                "twoext",
                "dep++myext2+myext",
            ]
        )
    );
}

#[test]
fn use_extension_bad_label_fails() {
    let mut root = build_module("module", "1.0");
    root.key = ModuleKey::root();
    root.extension_usages
        .push(create_module_extension_usage("@foo//:defs.bzl", "bar", &[]));
    let dep_graph = IndexMap::from([(ModuleKey::root(), root)]);
    let e = BazelDepGraphFunction::compute(&dep_graph).unwrap_err();
    assert!(
        e.get_message().contains("no repo visible as '@foo' here"),
        "{}",
        e.get_message()
    );
}

// Not in the Java test: @bazel_tools, isolation keys, innate names and repo overrides.
#[test]
fn bazel_tools_isolated_and_overridden_repos() {
    let bt_key = create_module_key("bazel_tools", "");
    let mut bazel_tools = build_module("bazel_tools", "");
    bazel_tools.key = bt_key.clone();
    let rpy_key = create_module_key("rules_python", "1.4.0");
    let mut root = build_module("root", "1.0");
    root.key = ModuleKey::root();
    root.deps.insert("bazel_tools".into(), bt_key.clone());
    root.deps.insert("rules_python".into(), rpy_key.clone());
    let mut usage = create_module_extension_usage(
        "@rules_python//python/extensions:python.bzl",
        "python",
        &[
            "python_3_11_x86_64-unknown-linux-gnu",
            "python_3_11_x86_64-unknown-linux-gnu",
            "py",
            "pythons_hub",
        ],
    );
    usage.repo_overrides.insert(
        "pythons_hub".into(),
        RepoOverride {
            overriding_repo_name: "bazel_tools".into(),
            must_exist: true,
            location: Location::builtin().clone(),
        },
    );
    root.extension_usages.push(usage);
    let mut isolated =
        create_module_extension_usage("@rules_python//:defs.bzl", "pip", &["iso", "x"]);
    isolated.isolation_key = Some(blaze_bzlmod_model::IsolationKey::create(
        ModuleKey::root(),
        "pip_iso",
    ));
    root.extension_usages.push(isolated);
    root.extension_usages.push(create_module_extension_usage(
        "@bazel_tools//:defs.bzl",
        "@rules_python//:repo.bzl my_rule",
        &["innate", "innate"],
    ));
    let dep_graph = IndexMap::from([
        (ModuleKey::root(), root),
        (bt_key, bazel_tools),
        (rpy_key, build_module("rules_python", "1.4.0")),
    ]);
    let value = BazelDepGraphFunction::compute(&dep_graph).unwrap();
    let m = value.get_full_repo_mapping(&ModuleKey::root());
    assert_eq!(m.get("bazel_tools").get_name(), "bazel_tools");
    assert_eq!(
        m.get("python_3_11_x86_64-unknown-linux-gnu").get_name(),
        "rules_python++python+python_3_11_x86_64-unknown-linux-gnu"
    );
    assert_eq!(m.get("py").get_name(), "bazel_tools");
    assert_eq!(m.get("iso").get_name(), "rules_python++_pip+++pip_iso+x");
    assert_eq!(m.get("innate").get_name(), "bazel_tools+my_rule+innate");
}
