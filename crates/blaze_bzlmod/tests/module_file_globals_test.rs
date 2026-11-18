//! Port of the module-file-evaluation tests of
//! src/test/java/com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunctionTest.java
//! (`ModuleFileGlobals`, `ModuleThreadContext`, `execModuleFile`).
//!
//! The harness (`module_file_common`) runs parse/compile, `exec_module_file`, `build_module` and
//! `build_overrides` directly. Tests of `ModuleFileFunction`'s own logic (registries, include
//! label validation, patches, root-module checks) are ignored until the MODULE_FILE SkyFunction
//! has a test harness.

mod module_file_common;

use blaze_bzlmod::module_file::module_file_globals::validate_module_name;
use blaze_bzlmod_model::{
    ArchiveRepoSpecBuilder, AttributeValue, AttributeValues, ModuleExtensionUsage,
    ModuleExtensionUsageProxy, ModuleKey, ModuleOverride, MultipleVersionOverride,
    NonRegistryOverride, SingleVersionOverride, Tag, Version, local_path_repo_spec,
};
use blaze_cmdline::label_constants::MODULE_DOT_BAZEL_FILE_NAME;
use blaze_events::Location;
use blaze_starlark::StarlarkSemantics;
use indexmap::IndexMap;
use module_file_common::{
    InterimModuleBuilder, ModuleFileHarness, create_dep_spec, create_module_key,
};

fn lines(lines: &[&str]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

fn attrs(entries: &[(&str, &str)]) -> AttributeValues {
    AttributeValues::create(
        entries
            .iter()
            .map(|(k, v)| (*k, AttributeValue::string(*v))),
    )
}

fn imports(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn proxy(
    file: &str,
    line: i32,
    column: i32,
    dev_dependency: bool,
    proxy_name: &str,
    proxy_imports: &[(&str, &str)],
) -> ModuleExtensionUsageProxy {
    ModuleExtensionUsageProxy {
        location: Location::from_file_line_column(file, line, column),
        proxy_name: proxy_name.to_owned(),
        containing_module_file_path: MODULE_DOT_BAZEL_FILE_NAME.clone(),
        dev_dependency,
        imports: imports(proxy_imports),
    }
}

fn tag(
    tag_name: &str,
    attribute_values: AttributeValues,
    dev_dependency: bool,
    file: &str,
    line: i32,
    column: i32,
) -> Tag {
    Tag {
        tag_name: tag_name.to_owned(),
        attribute_values,
        dev_dependency,
        location: Location::from_file_line_column(file, line, column),
    }
}

fn usage(
    extension_bzl_file: &str,
    extension_name: &str,
    proxies: Vec<ModuleExtensionUsageProxy>,
    tags: Vec<Tag>,
) -> ModuleExtensionUsage {
    ModuleExtensionUsage {
        extension_bzl_file: extension_bzl_file.to_owned(),
        extension_name: extension_name.to_owned(),
        isolation_key: None,
        proxies,
        tags,
        repo_overrides: IndexMap::new(),
    }
}

fn single_version_override(version: &str) -> ModuleOverride {
    ModuleOverride::SingleVersion(SingleVersionOverride {
        version: Version::parse(version).unwrap(),
        registry: String::new(),
        patches: Vec::new(),
        patch_cmds: Vec::new(),
        patch_strip: 0,
    })
}

const ROOT: &str = "/workspace/MODULE.bazel";
const MYMOD: &str = "fake:0/modules/mymod/1.0/MODULE.bazel";

#[test]
fn test_root_module() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
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
            ]),
            &[],
        )
        .unwrap();
    assert_eq!(
        result.module,
        InterimModuleBuilder::create_cl("aaa", "0.1", 0)
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
    let mut expected_overrides = IndexMap::new();
    expected_overrides.insert("ddd".to_owned(), single_version_override("18"));
    expected_overrides.insert(
        "eee".to_owned(),
        ModuleOverride::NonRegistry(NonRegistryOverride::new(local_path_repo_spec(
            "somewhere/else",
        ))),
    );
    expected_overrides.insert(
        "fff".to_owned(),
        ModuleOverride::MultipleVersion(MultipleVersionOverride {
            versions: vec![
                Version::parse("1.0").unwrap(),
                Version::parse("2.0").unwrap(),
            ],
            registry: String::new(),
        }),
    );
    expected_overrides.insert(
        "ggg".to_owned(),
        ModuleOverride::NonRegistry(NonRegistryOverride::new(
            ArchiveRepoSpecBuilder::new()
                .set_urls(vec!["https://hello.com/world.zip".to_owned()])
                .build(),
        )),
    );
    assert_eq!(result.overrides, expected_overrides);
    // nonRegistryOverrideCanonicalRepoToModuleName / nonRegistryOverrideModuleToRepoName are
    // computed by ModuleFileFunction from these overrides, not by the evaluation.
}

#[test]
fn test_root_module_no_module_function_is_okay() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(&lines(&["bazel_dep(name='bbb',version='1.0')"]), &[])
        .unwrap();
    assert_eq!(
        result.module,
        InterimModuleBuilder::create("", "")
            .set_key(ModuleKey::root())
            .add_dep("bbb", create_module_key("bbb", "1.0"))
            .build()
    );
    assert!(result.overrides.is_empty());
}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (root override check is in ModuleFileFunction)"]
fn test_root_module_bad_self_override() {}

#[test]
fn test_root_module_override_builtin_module() {
    let mut h = ModuleFileHarness::new();
    h.builtin_modules.insert(
        "bazel_tools".to_owned(),
        NonRegistryOverride::new(local_path_repo_spec("/workspace/bazel_tools_original")),
    );
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "local_path_override(module_name='bazel_tools',path='./bazel_tools_new')",
            ]),
            &[],
        )
        .unwrap();
    assert_eq!(
        result.overrides.get("bazel_tools"),
        Some(&ModuleOverride::NonRegistry(NonRegistryOverride::new(
            local_path_repo_spec("./bazel_tools_new")
        )))
    );
}

#[test]
fn test_root_module_overrides_ignored_with_ignore_dev_dependency() {
    let mut h = ModuleFileHarness::new();
    h.ignore_dev_deps = true;
    let result = h
        .eval_root(
            &lines(&[
                "bazel_dep(name='aaa')",
                "single_version_override(module_name='ddd',version='18')",
                "local_path_override(module_name='eee',path='somewhere/else')",
                "multiple_version_override(module_name='fff',versions=['1.0','2.0'])",
                "archive_override(module_name='ggg',urls=['https://hello.com/world.zip'])",
            ]),
            &[],
        )
        .unwrap();
    assert!(result.overrides.is_empty());
}

#[test]
fn test_root_module_include_good() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "include('//java:java.MODULE.bazel')",
                "bazel_dep(name='foo', version='1.0')",
                "register_toolchains('//:whatever')",
                "include('//python:python.MODULE.bazel', dev_dependency=True)",
            ]),
            &[
                (
                    "//java:java.MODULE.bazel",
                    &lines(&["bazel_dep(name='java-foo', version='1.0')"]),
                ),
                (
                    "//python:python.MODULE.bazel",
                    &lines(&[
                        "bazel_dep(name='py-foo', version='1.0', repo_name='python-foo')",
                        "single_version_override(module_name='java-foo', version='2.0')",
                        "include('//python:toolchains/toolchains.MODULE.bazel')",
                    ]),
                ),
                (
                    "//python:toolchains/toolchains.MODULE.bazel",
                    &lines(&["register_toolchains('//:python-whatever')"]),
                ),
            ],
        )
        .unwrap();
    let expected_module = InterimModuleBuilder::create("aaa", "")
        .set_key(ModuleKey::root())
        .add_dep("java-foo", create_module_key("java-foo", "1.0"))
        .add_dep("foo", create_module_key("foo", "1.0"))
        .add_dep("python-foo", create_module_key("py-foo", "1.0"))
        .add_toolchains_to_register(&["//:whatever", "//:python-whatever"])
        .build();
    assert_eq!(result.module, expected_module);
    // specifically assert the order of deps, which is significant; Map.equals semantics don't test
    // this.
    assert_eq!(
        result.module.deps.iter().collect::<Vec<_>>(),
        expected_module.deps.iter().collect::<Vec<_>>()
    );
    let mut expected_overrides = IndexMap::new();
    expected_overrides.insert("java-foo".to_owned(), single_version_override("2.0"));
    assert_eq!(result.overrides, expected_overrides);
}

#[test]
fn test_root_module_include_nested_include_survives_restart() {
    // Skyframe restarts do not exist in the harness; the evaluation result is what is checked.
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "include('//:first.MODULE.bazel')",
                "include('//:second.MODULE.bazel')",
            ]),
            &[
                (
                    "//:first.MODULE.bazel",
                    &lines(&["include('//:nested.MODULE.bazel')"]),
                ),
                (
                    "//:nested.MODULE.bazel",
                    &lines(&["bazel_dep(name='nested', version='1.0')"]),
                ),
                (
                    "//:second.MODULE.bazel",
                    &lines(&["bazel_dep(name='second', version='1.0')"]),
                ),
            ],
        )
        .unwrap();
    assert_eq!(
        result.module.deps.into_iter().collect::<Vec<_>>(),
        vec![
            ("nested".to_owned(), create_dep_spec("nested", "1.0", -1)),
            ("second".to_owned(), create_dep_spec("second", "1.0", -1)),
        ]
    );
}

#[test]
fn test_root_module_dev_include_ignored_with_ignore_dev_dependency() {
    let mut h = ModuleFileHarness::new();
    h.ignore_dev_deps = true;
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "bazel_dep(name='foo', version='1.0')",
                "include('//missing:dev.MODULE.bazel', dev_dependency=True)",
                "include('//prod:prod.MODULE.bazel')",
            ]),
            &[(
                "//prod:prod.MODULE.bazel",
                &lines(&[
                    "bazel_dep(name='bar', version='2.0')",
                    "include('//missing:nested-dev.MODULE.bazel', dev_dependency=True)",
                ]),
            )],
        )
        .unwrap();
    assert_eq!(
        result.module.deps.into_iter().collect::<Vec<_>>(),
        vec![
            ("foo".to_owned(), create_dep_spec("foo", "1.0", -1)),
            ("bar".to_owned(), create_dep_spec("bar", "2.0", -1)),
        ]
    );
}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (include label validation is in ModuleFileFunction)"]
fn test_root_module_include_bad_other_repo_label() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (include label validation is in ModuleFileFunction)"]
fn test_root_module_include_bad_relative_label() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (include label validation is in ModuleFileFunction)"]
fn test_root_module_include_bad_not_ending_in_module_bazel() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (include label validation is in ModuleFileFunction)"]
fn test_root_module_include_bad_starts_with_period() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (include label validation is in ModuleFileFunction)"]
fn test_root_module_include_bad_bad_label_syntax() {}

#[test]
fn test_root_module_include_bad_self_cycle() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&["module(name='aaa')", "include('//java:java.MODULE.bazel')"]),
        &[(
            "//java:java.MODULE.bazel",
            &lines(&["include('//java:java.MODULE.bazel')"]),
        )],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "include() cycle detected: //java:java.MODULE.bazel -> //java:java.MODULE.bazel",
    );
}

#[test]
fn test_root_module_include_bad_indirect_cycle() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&["module(name='aaa')", "include('//java:java.MODULE.bazel')"]),
        &[
            (
                "//java:java.MODULE.bazel",
                &lines(&["include('//python:python.MODULE.bazel')"]),
            ),
            (
                "//python:python.MODULE.bazel",
                &lines(&["include('//java:java.MODULE.bazel')"]),
            ),
        ],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "include() cycle detected: //java:java.MODULE.bazel -> //python:python.MODULE.bazel -> //java:java.MODULE.bazel",
    );
}

#[test]
fn test_root_module_include_same_file_multiple_times() {
    // The same file can be reached via multiple include() paths; it is then executed once per path,
    // just as if its contents had been inlined at each call site.
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "include('//java:java.MODULE.bazel')",
                "include('//python:python.MODULE.bazel')",
            ]),
            &[
                (
                    "//java:java.MODULE.bazel",
                    &lines(&["include('//common:common.MODULE.bazel')"]),
                ),
                (
                    "//python:python.MODULE.bazel",
                    &lines(&["include('//common:common.MODULE.bazel')"]),
                ),
                (
                    "//common:common.MODULE.bazel",
                    &lines(&["register_toolchains('//:common-toolchain')"]),
                ),
            ],
        )
        .unwrap();
    assert_eq!(
        result.module.toolchains_to_register,
        vec!["//:common-toolchain", "//:common-toolchain"]
    );
}

#[test]
fn test_root_module_include_bad_module_after_include() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&["include('//java:java.MODULE.bazel')"]),
        &[(
            "//java:java.MODULE.bazel",
            &lines(&[
                "module(name='bet-you-didnt-expect-this-didya')",
                "bazel_dep(name='java-foo', version='1.0', repo_name='foo')",
            ]),
        )],
    );
    assert!(result.is_err());
    h.assert_contains_event("if module() is called, it must be called before any other functions");
}

#[test]
fn test_root_module_include_bad_repo_name_collision() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "include('//java:java.MODULE.bazel')",
            "include('//python:python.MODULE.bazel')",
        ]),
        &[
            (
                "//java:java.MODULE.bazel",
                &lines(&["bazel_dep(name='java-foo', version='1.0', repo_name='foo')"]),
            ),
            (
                "//python:python.MODULE.bazel",
                &lines(&["bazel_dep(name='python-foo', version='1.0', repo_name='foo')"]),
            ),
        ],
    );
    assert!(result.is_err());
    h.assert_contains_event("The repo name 'foo' cannot be defined");
    h.assert_contains_event("as it is already defined");
}

#[test]
fn test_root_module_include_bad_trying_to_leak_bindings() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "FOO_NAME = 'foo'",
            "include('//java:java.MODULE.bazel')",
        ]),
        &[(
            "//java:java.MODULE.bazel",
            &lines(&["bazel_dep(name=FOO_NAME, version='1.0')"]),
        )],
    );
    assert!(result.is_err());
    h.assert_contains_event("name 'FOO_NAME' is not defined");
}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registry lookup in ModuleFileFunction)"]
fn forgot_version() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registries)"]
fn test_registries_cascade() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (non-root include check is in ModuleFileFunction)"]
fn test_non_root_module_cannot_use_include() {}

#[test]
fn test_registry_module_dev_include_is_ignored() {
    let h = ModuleFileHarness::new();
    let key = create_module_key("foo", "1.0");
    let module = h
        .eval_registry_module(
            &key,
            &lines(&[
                "module(name='foo',version='1.0')",
                "include('//missing:dev.MODULE.bazel', dev_dependency=True)",
            ]),
        )
        .unwrap();
    assert_eq!(module.name, "foo");
}

#[test]
#[ignore = "b/389163906 - figure out how to convert this class to BuildViewTestCase; the presence of the many builtin modules in the default AnalysisMock makes it very hard"]
fn test_local_path_override() {}

#[test]
#[ignore = "b/389163906 - figure out how to convert this class to BuildViewTestCase; the presence of the many builtin modules in the default AnalysisMock makes it very hard"]
fn test_command_line_module_overrides() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registries)"]
fn test_registry_override() {}

#[test]
fn test_module_extensions_good() {
    let h = ModuleFileHarness::new();
    let my_mod = create_module_key("mymod", "1.0");
    let module = h
        .eval_registry_module(
            &my_mod,
            &lines(&[
                "module(name='mymod',version='1.0')",
                "myext1 = use_extension('//:defs.bzl','myext1')",
                "use_repo(myext1, 'repo1')",
                "myext1.tag(key='val')",
                "myext2 = use_extension('//:defs.bzl','myext2')",
                "use_repo(myext2, 'repo2', other_repo1='repo1')",
                "myext2.tag1(key1='val1')",
                "myext2.tag2(key2='val2')",
                "bazel_dep(name='rules_jvm_external',version='2.0')",
                "maven = use_extension('@rules_jvm_external//:defs.bzl','maven')",
                "use_repo(maven, mvn='maven')",
                "maven.dep(coord='junit')",
                "use_repo(maven, 'junit', 'guava')",
                "maven.dep(coord='guava')",
            ]),
        )
        .unwrap();
    assert_eq!(
        module,
        InterimModuleBuilder::create("mymod", "1.0")
            .add_dep(
                "rules_jvm_external",
                create_module_key("rules_jvm_external", "2.0")
            )
            .add_extension_usage(usage(
                "@mymod//:defs.bzl",
                "myext1",
                vec![proxy(MYMOD, 2, 23, false, "myext1", &[("repo1", "repo1")])],
                vec![tag("tag", attrs(&[("key", "val")]), false, MYMOD, 4, 11)],
            ))
            .add_extension_usage(usage(
                "@mymod//:defs.bzl",
                "myext2",
                vec![proxy(
                    MYMOD,
                    5,
                    23,
                    false,
                    "myext2",
                    &[("other_repo1", "repo1"), ("repo2", "repo2")],
                )],
                vec![
                    tag("tag1", attrs(&[("key1", "val1")]), false, MYMOD, 7, 12),
                    tag("tag2", attrs(&[("key2", "val2")]), false, MYMOD, 8, 12),
                ],
            ))
            .add_extension_usage(usage(
                "@rules_jvm_external//:defs.bzl",
                "maven",
                vec![proxy(
                    MYMOD,
                    10,
                    22,
                    false,
                    "maven",
                    &[("mvn", "maven"), ("junit", "junit"), ("guava", "guava")],
                )],
                vec![
                    tag("dep", attrs(&[("coord", "junit")]), false, MYMOD, 12, 10),
                    tag("dep", attrs(&[("coord", "guava")]), false, MYMOD, 14, 10),
                ],
            ))
            .build()
    );
}

#[test]
fn test_module_extensions_duplicate_proxy_as_root() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "myext1 = use_extension('//:defs.bzl','myext',dev_dependency=True)",
                "myext1.tag(name = 'tag1')",
                "use_repo(myext1, 'alpha')",
                "myext2 = use_extension('//:defs.bzl','myext')",
                "myext2.tag(name = 'tag2')",
                "use_repo(myext2, 'beta')",
                "myext3 = use_extension('//:defs.bzl','myext',dev_dependency=True)",
                "myext3.tag(name = 'tag3')",
                "use_repo(myext3, 'gamma')",
                "myext4 = use_extension('//:defs.bzl','myext')",
                "myext4.tag(name = 'tag4')",
                "use_repo(myext4, 'delta')",
            ]),
            &[],
        )
        .unwrap();
    assert_eq!(
        result.module,
        InterimModuleBuilder::create("", "")
            .set_key(ModuleKey::root())
            .add_extension_usage(usage(
                "@//:defs.bzl",
                "myext",
                vec![
                    proxy(ROOT, 1, 23, true, "myext1", &[("alpha", "alpha")]),
                    proxy(ROOT, 4, 23, false, "myext2", &[("beta", "beta")]),
                    proxy(ROOT, 7, 23, true, "myext3", &[("gamma", "gamma")]),
                    proxy(ROOT, 10, 23, false, "myext4", &[("delta", "delta")]),
                ],
                vec![
                    tag("tag", attrs(&[("name", "tag1")]), true, ROOT, 2, 11),
                    tag("tag", attrs(&[("name", "tag2")]), false, ROOT, 5, 11),
                    tag("tag", attrs(&[("name", "tag3")]), true, ROOT, 8, 11),
                    tag("tag", attrs(&[("name", "tag4")]), false, ROOT, 11, 11),
                ],
            ))
            .build()
    );
}

#[test]
fn test_module_extensions_duplicate_proxy_as_dep() {
    let h = ModuleFileHarness::new();
    let my_mod = create_module_key("mymod", "1.0");
    let module = h
        .eval_registry_module(
            &my_mod,
            &lines(&[
                "module(name='mymod',version='1.0')",
                "myext1 = use_extension('//:defs.bzl','myext',dev_dependency=True)",
                "myext1.tag(name = 'tag1')",
                "use_repo(myext1, 'alpha')",
                "myext2 = use_extension('//:defs.bzl','myext')",
                "myext2.tag(name = 'tag2')",
                "use_repo(myext2, 'beta')",
                "myext3 = use_extension('//:defs.bzl','myext',dev_dependency=True)",
                "myext3.tag(name = 'tag3')",
                "use_repo(myext3, 'gamma')",
                "myext4 = use_extension('//:defs.bzl','myext')",
                "myext4.tag(name = 'tag4')",
                "use_repo(myext4, 'delta')",
            ]),
        )
        .unwrap();
    assert_eq!(
        module,
        InterimModuleBuilder::create("mymod", "1.0")
            .add_extension_usage(usage(
                "@mymod//:defs.bzl",
                "myext",
                vec![
                    proxy(MYMOD, 5, 23, false, "myext2", &[("beta", "beta")]),
                    proxy(MYMOD, 11, 23, false, "myext4", &[("delta", "delta")]),
                ],
                vec![
                    tag("tag", attrs(&[("name", "tag2")]), false, MYMOD, 6, 11),
                    tag("tag", attrs(&[("name", "tag4")]), false, MYMOD, 12, 11),
                ],
            ))
            .build()
    );
}

#[test]
fn test_module_extensions_repo_name_collision_local_repo_name() {
    let h = ModuleFileHarness::new();
    let result = h.eval_registry_module(
        &create_module_key("mymod", "1.0"),
        &lines(&[
            "module(name='mymod',version='1.0')",
            "myext = use_extension('//:defs.bzl','myext')",
            "use_repo(myext, mymod='some_repo')",
        ]),
    );
    assert!(result.is_err());
    h.assert_contains_event("The repo name 'mymod' cannot be defined");
    h.assert_contains_event("as it is already defined as the current module name");
}

#[test]
fn test_module_extensions_repo_name_collision_exported_repo_name() {
    let h = ModuleFileHarness::new();
    let result = h.eval_registry_module(
        &create_module_key("mymod", "1.0"),
        &lines(&[
            "module(name='mymod',version='1.0')",
            "myext = use_extension('//:defs.bzl','myext')",
            "use_repo(myext, 'some_repo', again='some_repo')",
        ]),
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "The repo exported as 'some_repo' by module extension 'myext' is already imported at",
    );
}

#[test]
fn test_module_extensions_innate() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "repo = use_repo_rule('//:repo.bzl','repo')",
                "repo(name='repo_name', value='something')",
                "http_archive = use_repo_rule('@bazel_tools//:http.bzl','http_archive')",
                "http_archive(name='guava',url='guava.com')",
                "http_archive(name='vuaga',url='vuaga.com',dev_dependency=True)",
            ]),
            &[],
        )
        .unwrap();
    assert_eq!(
        result.module,
        InterimModuleBuilder::create("", "")
            .set_key(ModuleKey::root())
            .add_extension_usage(usage(
                "//:MODULE.bazel",
                "//:repo.bzl repo",
                vec![proxy(ROOT, 2, 5, false, "", &[("repo_name", "repo_name")])],
                vec![tag(
                    "repo",
                    attrs(&[("name", "repo_name"), ("value", "something")]),
                    false,
                    ROOT,
                    2,
                    5
                )],
            ))
            .add_extension_usage(usage(
                "//:MODULE.bazel",
                "@bazel_tools//:http.bzl http_archive",
                vec![
                    proxy(ROOT, 4, 13, false, "", &[("guava", "guava")]),
                    proxy(ROOT, 5, 13, true, "", &[("vuaga", "vuaga")]),
                ],
                vec![
                    tag(
                        "repo",
                        attrs(&[("name", "guava"), ("url", "guava.com")]),
                        false,
                        ROOT,
                        4,
                        13
                    ),
                    tag(
                        "repo",
                        attrs(&[("name", "vuaga"), ("url", "vuaga.com")]),
                        true,
                        ROOT,
                        5,
                        13
                    ),
                ],
            ))
            .build()
    );
}

#[test]
fn test_module_file_execute_syntax_error() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(&lines(&["module(name='aaa',version='0.1')", "foo()"]), &[]);
    h.assert_contains_event("name 'foo' is not defined");
}

#[test]
fn test_module_file_execute_eval_error() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&["module(name='aaa',version='0.1',compatibility_level=\"4\")"]),
        &[],
    );
    h.assert_contains_event(
        "parameter 'compatibility_level' got value of type 'string', want 'int'",
    );
}

#[test]
fn test_module_file_execute_compatibility_level_warning() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "module(name='aaa',version='0.1',compatibility_level=4)",
            "bazel_dep(name='bbb',version='1.0',max_compatibility_level=5)",
        ]),
        &[],
    );
    h.assert_contains_event("The attribute 'compatibility_level' in module() is a no-op");
    h.assert_contains_event("The attribute 'max_compatibility_level' in bazel_dep() is a no-op");
}

#[test]
fn validate_module_name_test() {
    validate_module_name("abc").unwrap();
    validate_module_name("a3").unwrap();
    validate_module_name("a.e").unwrap();
    validate_module_name("a.-_e").unwrap();
    validate_module_name("a").unwrap();

    assert!(validate_module_name("").is_err());
    assert!(validate_module_name("fooBar").is_err());
    assert!(validate_module_name("_foo").is_err());
    assert!(validate_module_name("foo#bar").is_err());
    assert!(validate_module_name("foo~bar").is_err());
    assert!(validate_module_name("foo+bar").is_err());
}

#[test]
fn bad_module_name_module() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(&lines(&["module(name='f.',version='0.1')"]), &[]);
    h.assert_contains_event("invalid module name 'f.'");
}

#[test]
fn bad_module_name_bazel_dep() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(&lines(&["bazel_dep(name='f.',version='0.1')"]), &[]);
    h.assert_contains_event("invalid module name 'f.'");
}

#[test]
fn bad_repo_name_module() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&["module(name='foo',version='0.1',repo_name='_foo')"]),
        &[],
    );
    h.assert_contains_event("invalid user-provided repo name '_foo'");
}

#[test]
fn bad_repo_name_bazel_dep() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&["bazel_dep(name='foo',version='0.1',repo_name='_foo')"]),
        &[],
    );
    h.assert_contains_event("invalid user-provided repo name '_foo'");
}

#[test]
fn bad_repo_name_use_repo() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "ext=use_extension('//:hello.bzl', 'ext')",
            "use_repo(ext, foo='_foo')",
        ]),
        &[],
    );
    h.assert_contains_event("invalid user-provided repo name '_foo'");
}

#[test]
fn bad_repo_name_use_repo_assigned_name() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "ext=use_extension('//:hello.bzl', 'ext')",
            "use_repo(ext, _foo='foo')",
        ]),
        &[],
    );
    h.assert_contains_event("invalid user-provided repo name '_foo'");
}

#[test]
fn test_builtin_modules_for_root() {
    let mut h = ModuleFileHarness::new();
    let mut builtin_modules = IndexMap::new();
    builtin_modules.insert(
        "bazel_tools".to_owned(),
        NonRegistryOverride::new(local_path_repo_spec("/tools")),
    );
    builtin_modules.insert(
        "other_tools".to_owned(),
        NonRegistryOverride::new(local_path_repo_spec("/other_tools")),
    );
    h.builtin_modules = builtin_modules.clone();
    let result = h
        .eval_root(&lines(&["bazel_dep(name='foo',version='1.0')"]), &[])
        .unwrap();
    assert_eq!(
        result.module,
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
    assert_eq!(result.overrides, expected);
}

#[test]
#[ignore = "b/389163906 - figure out how to convert this class to BuildViewTestCase; the presence of the many builtin modules in the default AnalysisMock makes it very hard"]
fn test_builtin_modules_for_builtin_modules() {}

#[test]
fn module_repo_name() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&["module(name='aaa',version='0.1',repo_name='bbb')"]),
            &[],
        )
        .unwrap();
    assert_eq!(
        result.module,
        InterimModuleBuilder::create("aaa", "0.1")
            .set_key(ModuleKey::root())
            .set_repo_name("bbb")
            .build()
    );
}

#[test]
fn module_repo_name_conflict() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "module(name='aaa',version='0.1',repo_name='bbb')",
            "bazel_dep(name='bbb',version='1.0')",
        ]),
        &[],
    );
    h.assert_contains_event("The repo name 'bbb' cannot be defined");
    h.assert_contains_event("as it is already defined as the module's own repo name");
}

#[test]
fn module_called_twice() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "module(name='aaa',version='0.1',repo_name='bbb')",
            "module(name='aaa',version='0.1',repo_name='bbb')",
        ]),
        &[],
    );
    h.assert_contains_event("the module() directive can only be called once");
}

#[test]
fn module_called_late() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&[
            "use_extension('//:extensions.bzl', 'my_ext')",
            "module(name='aaa',version='0.1',repo_name='bbb')",
        ]),
        &[],
    );
    h.assert_contains_event("if module() is called, it must be called before any other functions");
}

#[test]
fn restricted_syntax() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&["if 3+5>7: module(name='aaa',version='0.1',repo_name='bbb')"]),
        &[],
    );
    h.assert_contains_event(
        "`if` statements are not allowed in MODULE.bazel files. You may use an `if` expression for simple cases",
    );
}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (the isolated-usage import check is in ModuleFileFunction)"]
fn isolated_usage_without_imports() {}

#[test]
fn isolated_usage_not_exported() {
    let mut h = ModuleFileHarness::new();
    h.semantics = StarlarkSemantics {
        experimental_isolated_extension_usages: true,
        ..StarlarkSemantics::default()
    };
    let _ = h.eval_root(
        &lines(&["use_extension('//:extensions.bzl', 'my_ext', isolate = True)"]),
        &[],
    );
    h.assert_contains_event(
        "Isolated extension usage at /workspace/MODULE.bazel:1:14 must be assigned to a top-level variable",
    );
}

#[test]
fn isolated_usage_not_enabled() {
    let h = ModuleFileHarness::new();
    let _ = h.eval_root(
        &lines(&["use_extension('//:extensions.bzl', 'my_ext', isolate = True)"]),
        &[],
    );
    h.assert_contains_event(
        "Error in use_extension: in call to use_extension(), parameter 'isolate' is experimental and thus unavailable with the current flags. It may be enabled by setting --experimental_isolated_extension_usages",
    );
}

#[test]
fn test_register_toolchains_single_package_restriction_under_dir() {
    let mut h = ModuleFileHarness::new();
    h.semantics = StarlarkSemantics {
        experimental_single_package_toolchain_binding: true,
        ..StarlarkSemantics::default()
    };
    let result = h.eval_root(
        &lines(&["module(name='aaa')", "register_toolchains('//bar/...')"]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "invalid target pattern \"//bar/...\": register_toolchain target patterns may only refer to targets within a single package",
    );
}

#[test]
fn test_register_toolchains_path_syntax() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&["module(name='aaa')", "register_toolchains('bar/baz')"]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "Expected absolute target patterns (must begin with '//' or '@') for 'register_toolchains' argument, but got 'bar/baz' as an argument",
    );
}

#[test]
fn test_register_toolchains_single_package_restriction() {
    let mut h = ModuleFileHarness::new();
    h.semantics = StarlarkSemantics {
        experimental_single_package_toolchain_binding: true,
        ..StarlarkSemantics::default()
    };
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "register_toolchains('//:whatever')",
            "register_toolchains('//bar:all')",
            "register_toolchains('//qux:baz')",
        ]),
        &[],
    );
    assert!(result.is_ok());
}

#[test]
fn test_no_usages() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            &lines(&[
                "module(name='aaa')",
                "http_archive = use_repo_rule('@bazel_tools//tools/build_defs/repo:http.bzl', 'http_archive')",
            ]),
            &[],
        )
        .unwrap();
    assert!(result.module.extension_usages.is_empty());
}

#[test]
fn test_invalid_repo_in_single_version_override_patches() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "single_version_override(",
            "    module_name = 'bbb',",
            "    version = '1.0',",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "Error in single_version_override: invalid label in 'patches': only patches in the main repository can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_repo_in_archive_override_patches() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "archive_override(",
            "    module_name = 'bbb',",
            "    urls = ['https://example.com'],",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "Error in archive_override: invalid label in 'patches': only patches in the main repository can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_repo_in_git_override_patches() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "git_override(",
            "    module_name = 'bbb',",
            "    remote = 'https://example.com',",
            "    commit = 'abcd1234',",
            "    patches = ['@unknown_repo//:patch.bzl'],",
            ")",
        ]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "Error in git_override: invalid label in 'patches': only patches in the main repository can be applied, not from '@unknown_repo'",
    );
}

#[test]
fn test_invalid_use_extension_label() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&[
            "module(name='aaa')",
            "use_extension('@foo/bar:extensions.bzl', 'my_ext')",
        ]),
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "Error in use_extension: invalid label \"@foo/bar:extensions.bzl\": invalid repository name 'foo/bar:extensions.bzl': repo names may contain only A-Z, a-z, 0-9, '-', '_', '.' and '+'",
    );
}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registries and patching of module files)"]
fn test_single_version_override_patches() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registries and patching of module files)"]
fn test_single_version_override_patches_default_patch_strip_for_git_patch() {}

#[test]
#[ignore = "needs MODULE_FILE SkyFunction harness (registries and patching of module files)"]
fn test_single_version_override_patches_fails_on_rename() {}

#[test]
fn test_override_repo_overriding_repo_doesnt_exist() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\next = use_extension('//:defs.bzl', 'ext')\noverride_repo(ext, 'foo')\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:3:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 3, column 14, in <toplevel>\n\t\toverride_repo(ext, 'foo')\nError in override_repo: The repo exported as 'foo' by module extension 'ext' is overridden with 'foo', but no repo is visible under this name",
    );
}

#[test]
fn test_override_repo_duplicate_override() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\nbazel_dep(name = \"override1\", version = \"1.0\")\nbazel_dep(name = \"override2\", version = \"1.0\")\next = use_extension('//:defs.bzl', 'ext')\noverride_repo(ext, foo = \"override1\")\noverride_repo(ext, foo = \"override2\")\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:6:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 6, column 14, in <toplevel>\n\t\toverride_repo(ext, foo = \"override2\")\nError in override_repo: The repo exported as 'foo' by module extension 'ext' is already overridden with 'override1' at /workspace/MODULE.bazel:5:14",
    );
}

#[test]
fn test_override_repo_chain_single_extension() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\nbazel_dep(name = \"override\", version = \"1.0\")\next = use_extension('//:defs.bzl', 'ext')\nuse_repo(ext, bar = \"foo\")\noverride_repo(ext, baz = \"bar\")\noverride_repo(ext, foo = \"override\")\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:5:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 5, column 14, in <toplevel>\n\t\toverride_repo(ext, baz = \"bar\")\nError in override_repo: The repo 'bar' used as an override for 'baz' in module extension 'ext' is itself overridden with 'override' at /workspace/MODULE.bazel:6:14, which is not supported.",
    );
}

#[test]
fn test_override_repo_chain_multiple_extensions() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\nbazel_dep(name = \"override\", version = \"1.0\")\next1 = use_extension('//:defs.bzl', 'ext1')\next2 = use_extension('//:defs.bzl', 'ext2')\noverride_repo(ext1, baz = \"bar\")\noverride_repo(ext2, foo = \"override\")\nuse_repo(ext2, bar = \"foo\")\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:5:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 5, column 14, in <toplevel>\n\t\toverride_repo(ext1, baz = \"bar\")\nError in override_repo: The repo 'bar' used as an override for 'baz' in module extension 'ext1' is itself overridden with 'override' at /workspace/MODULE.bazel:6:14, which is not supported.",
    );
}

#[test]
fn test_override_repo_cycle_single_extension() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\nbazel_dep(name = \"override\", version = \"1.0\")\next = use_extension('//:defs.bzl', 'ext')\nuse_repo(ext, my_foo = \"foo\", my_bar = \"bar\")\noverride_repo(ext, foo = \"my_bar\", bar = \"my_foo\")\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:5:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 5, column 14, in <toplevel>\n\t\toverride_repo(ext, foo = \"my_bar\", bar = \"my_foo\")\nError in override_repo: The repo 'my_foo' used as an override for 'bar' in module extension 'ext' is itself overridden with 'my_bar' at /workspace/MODULE.bazel:5:14, which is not supported.",
    );
}

#[test]
fn test_override_repo_cycle_multiple_extensions() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\next1 = use_extension('//:defs.bzl', 'ext1')\next2 = use_extension('//:defs.bzl', 'ext2')\noverride_repo(ext1, foo = \"my_bar\")\noverride_repo(ext2, bar = \"my_foo\")\nuse_repo(ext1, my_foo = \"foo\")\nuse_repo(ext2, my_bar = \"bar\")\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:5:14: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 5, column 14, in <toplevel>\n\t\toverride_repo(ext2, bar = \"my_foo\")\nError in override_repo: The repo 'my_foo' used as an override for 'bar' in module extension 'ext2' is itself overridden with 'my_bar' at /workspace/MODULE.bazel:4:14, which is not supported.",
    );
}

#[test]
fn test_override_repo_extra_keyword_arguments_literal() {
    let h = ModuleFileHarness::new();
    let result = h
        .eval_root(
            "module(name='aaa')\next1 = use_extension('//:defs.bzl', 'ext1')\nuse_repo(ext1, \"foo\")\next2 = use_extension('//:defs.bzl', 'ext2')\nuse_repo(ext2, \"bar.2\")\noverride_repo(ext2, **{\"foo.2\": \"foo\"})\n",
            &[],
        )
        .unwrap();
    let ext2_usage = &result.module.extension_usages[1];
    assert_eq!(
        ext2_usage.repo_overrides.keys().collect::<Vec<_>>(),
        vec!["foo.2"]
    );
}

#[test]
fn test_override_repo_extra_keyword_arguments_non_literal() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\next1 = use_extension('//:defs.bzl', 'ext1')\nuse_repo(ext1, \"foo\")\next2 = use_extension('//:defs.bzl', 'ext2')\nuse_repo(ext2, \"bar.2\")\nKWARGS = {\"foo.2\": \"foo\"}\noverride_repo(ext2, **KWARGS)\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:7:21: **kwargs arguments must be a literal dict in MODULE.bazel files.",
    );
}

#[test]
fn test_inject_repo_imported() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        "module(name='aaa')\nbazel_dep(name = 'my_repo', version = \"1.0\")\next = use_extension('//:defs.bzl', 'ext')\ninject_repo(ext, foo = 'my_repo')\nuse_repo(ext, bar = 'foo')\n",
        &[],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/MODULE.bazel:5:9: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 5, column 9, in <toplevel>\n\t\tuse_repo(ext, bar = 'foo')\nError in use_repo: Cannot import repo 'foo' that has been injected into module extension 'ext' at /workspace/MODULE.bazel:4:12. Please refer to @my_repo directly.",
    );
}

// The tests below are not in ModuleFileFunctionTest; they cover Java behaviour of the ported
// classes that the Java tests reach only through other suites.

/// `ModuleFileFunction.injectRepos`: one `local_repository` innate-extension tag and import per
/// injected repository, at `Location.BUILTIN`.
#[test]
fn inject_repos_adds_local_repository_usage() {
    let mut h = ModuleFileHarness::new();
    h.injected_repositories.insert(
        "foo".to_owned(),
        blaze_vfs::PathFragment::create("/abs/foo"),
    );
    let result = h.eval_root(&lines(&["module(name='aaa')"]), &[]).unwrap();
    let builtin = Location::builtin().clone();
    assert_eq!(
        result.module.extension_usages,
        vec![ModuleExtensionUsage {
            extension_bzl_file: "//:MODULE.bazel".to_owned(),
            extension_name: "@bazel_tools//tools/build_defs/repo:local.bzl local_repository"
                .to_owned(),
            isolation_key: None,
            proxies: vec![ModuleExtensionUsageProxy {
                location: builtin.clone(),
                proxy_name: String::new(),
                containing_module_file_path: MODULE_DOT_BAZEL_FILE_NAME.clone(),
                dev_dependency: true,
                imports: imports(&[("foo", "foo")]),
            }],
            tags: vec![Tag {
                tag_name: "repo".to_owned(),
                attribute_values: attrs(&[("name", "foo"), ("path", "/abs/foo")]),
                dev_dependency: true,
                location: builtin,
            }],
            repo_overrides: IndexMap::new(),
        }]
    );
}

/// `print()` reports DEBUG events at the call's `(` (Event.makeDebugPrintHandler) for the root
/// module, and is a no-op for registry modules.
#[test]
fn print_is_debug_event_unless_noop() {
    let h = ModuleFileHarness::new();
    h.eval_root(&lines(&["module(name='aaa')", "print('hi', 1)"]), &[])
        .unwrap();
    h.assert_contains_event("DEBUG /workspace/MODULE.bazel:2:6: hi 1");

    let h = ModuleFileHarness::new();
    h.eval_registry_module(
        &create_module_key("mymod", "1.0"),
        &lines(&["module(name='mymod',version='1.0')", "print('hi')"]),
    )
    .unwrap();
    assert!(h.events.get_events().is_empty());
}

/// Errors in an included file carry the including file's frames (Java's call stack spans the
/// `include` builtin).
#[test]
fn include_error_has_full_call_stack() {
    let h = ModuleFileHarness::new();
    let result = h.eval_root(
        &lines(&["module(name='aaa')", "include('//:a.MODULE.bazel')"]),
        &[("//:a.MODULE.bazel", &lines(&["bazel_dep(name='f.')"]))],
    );
    assert!(result.is_err());
    h.assert_contains_event(
        "ERROR /workspace/a.MODULE.bazel:1:10: Traceback (most recent call last):\n\tFile \"/workspace/MODULE.bazel\", line 2, column 8, in <toplevel>\n\t\tinclude('//:a.MODULE.bazel')\n\tFile \"<builtin>\", in include\n\tFile \"/workspace/a.MODULE.bazel\", line 1, column 10, in <toplevel>\n\t\tbazel_dep(name='f.')\nError in bazel_dep: invalid module name 'f.'",
    );
}

/// Calling a MODULE.bazel builtin with no `ModuleThreadContext` fails as Java's `fromOrFail`.
#[test]
fn natives_require_module_thread_context() {
    use blaze_bzlmod::module_file::module_file_globals;
    use starlark::environment::Module;
    use starlark::eval::Evaluator;
    use starlark::syntax::AstModule;

    let globals = module_file_globals(&StarlarkSemantics::default());
    let ast = AstModule::parse(
        "x.bzl",
        "bazel_dep(name='foo')\n".to_owned(),
        &blaze_starlark::FileKind::ModuleFile.dialect(),
    )
    .unwrap();
    let err = Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.eval_module(ast, &globals).unwrap_err().to_string()
    });
    assert!(
        err.contains("bazel_dep() can only be called from MODULE.bazel and files it includes"),
        "{err}"
    );
}
