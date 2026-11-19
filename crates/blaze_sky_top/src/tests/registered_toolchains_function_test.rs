//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java`
//! (a `ToolchainTestCase`): tests for `RegisteredToolchainsFunction` and
//! `RegisteredToolchainsValue`.
//!
//! `testRegisteredToolchainsValue_equalsAndHashCode` needs no harness and stays in
//! `blaze_sky_analysis/src/toolchains/tests.rs`.

use blaze_cmdline::Label;
use blaze_cmdline::package_identifier::PackageIdentifier;
use blaze_sky_analysis::toolchains::{RegisteredToolchainsKey, RegisteredToolchainsValue};
use blaze_skyframe::EvaluationResult;

use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_contains_at_least, assert_contains_at_least_in_order,
    assert_has_no_error, collect_toolchain_labels, error_message, get_value, label,
    main_repo_package,
};

fn key(t: &mut ToolchainTestCase, debug: bool) -> RegisteredToolchainsKey {
    RegisteredToolchainsKey::key(t.target_config_key(), debug)
}

// Java: ToolchainTestCase.requestToolchainsFromSkyframe
fn request_toolchains_from_skyframe(
    t: &mut ToolchainTestCase,
    toolchains_key: &RegisteredToolchainsKey,
) -> EvaluationResult {
    t.evaluate_key(toolchains_key)
}

/// `assertToolchainLabels(value, packageRoot)`: the labels the Truth subject is about.
// Java: com/google/devtools/build/lib/rules/platform/ToolchainTestCase.java:65 ToolchainTestCase.assertToolchainLabels
fn toolchain_labels(
    registered_toolchains_value: &RegisteredToolchainsValue,
    package_root: Option<&PackageIdentifier>,
) -> Vec<Label> {
    collect_toolchain_labels(
        &registered_toolchains_value.registered_toolchains,
        package_root,
    )
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains
#[test]
fn test_registered_toolchains() {
    let mut t = ToolchainTestCase::new();
    // Request the toolchains.
    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    let value = get_value(&result, &toolchains_key);

    // Check that the number of toolchains created for this test is correct.
    assert_eq!(
        value
            .registered_toolchains
            .iter()
            .filter(|toolchain| toolchain.toolchain_type() == &t.test_toolchain_type_info)
            .count(),
        2
    );

    assert!(value.registered_toolchains.iter().any(|toolchain| {
        toolchain.toolchain_type() == &t.test_toolchain_type_info
            && toolchain.exec_constraints().get(&t.setting).as_ref() == Some(&t.linux_constraint)
            && toolchain.target_constraints().get(&t.setting).as_ref() == Some(&t.mac_constraint)
            && toolchain.resolved_toolchain_label() == &label("//toolchain:toolchain_1_impl")
    }));

    assert!(value.registered_toolchains.iter().any(|toolchain| {
        toolchain.toolchain_type() == &t.test_toolchain_type_info
            && toolchain.exec_constraints().get(&t.setting).as_ref() == Some(&t.mac_constraint)
            && toolchain.target_constraints().get(&t.setting).as_ref() == Some(&t.linux_constraint)
            && toolchain.resolved_toolchain_label() == &label("//toolchain:toolchain_2_impl")
    }));
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_flagOverride
#[test]
fn test_registered_toolchains_flag_override() {
    let mut t = ToolchainTestCase::new();

    // Add an extra toolchain.
    t.scratch.file(
        "extra/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain(
    name = "extra_toolchain",
    exec_compatible_with = ["//constraints:linux"],
    target_compatible_with = ["//constraints:linux"],
    toolchain = ":extra_toolchain_impl",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "extra_toolchain_impl",
    data = "extra",
)
"#],
    );

    t.rewrite_module_dot_bazel(&["register_toolchains('//toolchain:toolchain_1')\n"]);
    t.use_configuration(&["--extra_toolchains=//extra:extra_toolchain"]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_toolchains flag is first in the list.
    assert_contains_at_least_in_order(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[
            label("//extra:extra_toolchain_impl"),
            label("//toolchain:toolchain_1_impl"),
        ],
    );
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_flagOverride_multiple
#[test]
fn test_registered_toolchains_flag_override_multiple() {
    let mut t = ToolchainTestCase::new();

    // Add an extra toolchain.
    t.scratch.file(
        "extra/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain(
    name = "extra_toolchain_1",
    exec_compatible_with = ["//constraints:linux"],
    target_compatible_with = ["//constraints:linux"],
    toolchain = ":extra_toolchain_impl_1",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "extra_toolchain_impl_1",
    data = "extra",
)

toolchain(
    name = "extra_toolchain_2",
    exec_compatible_with = ["//constraints:mac"],
    target_compatible_with = ["//constraints:linux"],
    toolchain = ":extra_toolchain_impl_2",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "extra_toolchain_impl_2",
    data = "extra2",
)
"#],
    );

    t.use_configuration(&[
        "--extra_toolchains=//extra:extra_toolchain_1",
        "--extra_toolchains=//extra:extra_toolchain_2",
    ]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_toolchains flag is first in the list.
    assert_contains_at_least_in_order(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[
            label("//extra:extra_toolchain_impl_2"),
            label("//extra:extra_toolchain_impl_1"),
            label("//toolchain:toolchain_1_impl"),
        ],
    );
}

const NOT_A_TOOLCHAIN_MESSAGE: &str = "invalid registered toolchain '//error:not_a_toolchain': \
                                       target does not provide the DeclaredToolchainInfo provider";

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_notToolchain
#[test]
fn test_registered_toolchains_not_toolchain() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//error:not_a_toolchain\")\n"]);
    t.scratch
        .file("error/BUILD", &["filegroup(name = 'not_a_toolchain')"]);

    // Request the toolchains.
    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    let message = error_message(&result, &toolchains_key);
    assert!(message.contains(NOT_A_TOOLCHAIN_MESSAGE), "{message}");
}

const FAKE_TOOLCHAIN_BZL: &str = r#"def _fake_impl(ctx):
  pass

toolchain = rule(implementation = _fake_impl)
"#;

const FAKE_TOOLCHAIN_BUILD: &str = r#"load(':fake_toolchain.bzl', fake_toolchain='toolchain')
fake_toolchain(name = 'not_a_toolchain')
"#;

// Test confirming that targets with the kind `toolchain rule` will be properly rejected if they
// don't provide the DeclaredToolchainInfo provider.
// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_fakeToolchain
#[test]
fn test_registered_toolchains_fake_toolchain() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//error:not_a_toolchain\")\n"]);
    t.scratch
        .file("error/fake_toolchain.bzl", &[FAKE_TOOLCHAIN_BZL]);
    t.scratch.file("error/BUILD", &[FAKE_TOOLCHAIN_BUILD]);

    // Request the toolchains.
    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    let message = error_message(&result, &toolchains_key);
    assert!(message.contains(NOT_A_TOOLCHAIN_MESSAGE), "{message}");
}

// Test exercising an edge case in the current RegisteredToolchainsFunction logic: if a target
// has the kind `toolchain rule`, it must provide the DeclaredToolchainInfo provider, or the
// RegisteredToolchainsFunction will fail.
// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_wildcard_fakeToolchain
#[test]
fn test_registered_toolchains_wildcard_fake_toolchain() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//error:all\")\n"]);
    t.scratch
        .file("error/fake_toolchain.bzl", &[FAKE_TOOLCHAIN_BZL]);
    t.scratch.file("error/BUILD", &[FAKE_TOOLCHAIN_BUILD]);

    // Request the toolchains.
    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    let message = error_message(&result, &toolchains_key);
    assert!(message.contains(NOT_A_TOOLCHAIN_MESSAGE), "{message}");
}

/// The toolchains of the `targetPattern` tests.
fn add_extra_toolchains(t: &mut ToolchainTestCase) {
    t.scratch
        .append_file("extra/BUILD", &["filegroup(name = 'not_a_platform')"]);
    t.add_toolchain(
        "extra",
        "extra_toolchain1",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "foo",
    );
    t.add_toolchain(
        "extra",
        "extra_toolchain2",
        &["//constraints:linux"],
        &["//constraints:mac"],
        "bar",
    );
    t.add_toolchain(
        "extra/more",
        "more_toolchain",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetPattern_workspace
#[test]
fn test_registered_toolchains_target_pattern_workspace() {
    let mut t = ToolchainTestCase::new();
    add_extra_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//extra/...\")\n"]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    let mut labels = toolchain_labels(
        &get_value(&result, &toolchains_key),
        Some(&main_repo_package("extra")),
    );
    // containsExactly (any order).
    let mut expected = vec![
        label("//extra:extra_toolchain1_impl"),
        label("//extra:extra_toolchain2_impl"),
        label("//extra/more:more_toolchain_impl"),
    ];
    labels.sort_by_key(|l| l.to_string());
    expected.sort_by_key(|l| l.to_string());
    assert_eq!(labels, expected);
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetPattern_flagOverride
#[test]
fn test_registered_toolchains_target_pattern_flag_override() {
    let mut t = ToolchainTestCase::new();
    add_extra_toolchains(&mut t);
    t.use_configuration(&["--extra_toolchains=//extra/..."]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    assert_contains_at_least(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[
            label("//extra:extra_toolchain1_impl"),
            label("//extra:extra_toolchain2_impl"),
            label("//extra/more:more_toolchain_impl"),
        ],
    );
}

// Java: RegisteredToolchainsFunctionTest.addSimpleToolchain
fn add_simple_toolchain(t: &mut ToolchainTestCase, package_name: &str, toolchain_name: &str) {
    t.add_toolchain(package_name, toolchain_name, &[], &[], "foo");
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetPattern_order
#[test]
fn test_registered_toolchains_target_pattern_order() {
    let mut t = ToolchainTestCase::new();
    for package in [
        "extra",
        "extra/yyy",
        "extra/xxx",
        "extra/zzz",
        "extra/yyy/yyy",
        "extra/yyy/xxx",
        "extra/yyy/zzz",
        "extra/xxx/yyy",
        "extra/xxx/xxx",
        "extra/xxx/zzz",
    ] {
        add_simple_toolchain(&mut t, package, "bbb");
        add_simple_toolchain(&mut t, package, "ccc");
        add_simple_toolchain(&mut t, package, "aaa");
    }
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//extra/...\")\n"]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    let expected: Vec<Label> = [
        "//extra/xxx/xxx:aaa_impl",
        "//extra/xxx/xxx:bbb_impl",
        "//extra/xxx/xxx:ccc_impl",
        "//extra/xxx/yyy:aaa_impl",
        "//extra/xxx/yyy:bbb_impl",
        "//extra/xxx/yyy:ccc_impl",
        "//extra/xxx/zzz:aaa_impl",
        "//extra/xxx/zzz:bbb_impl",
        "//extra/xxx/zzz:ccc_impl",
        "//extra/xxx:aaa_impl",
        "//extra/xxx:bbb_impl",
        "//extra/xxx:ccc_impl",
        "//extra/yyy/xxx:aaa_impl",
        "//extra/yyy/xxx:bbb_impl",
        "//extra/yyy/xxx:ccc_impl",
        "//extra/yyy/yyy:aaa_impl",
        "//extra/yyy/yyy:bbb_impl",
        "//extra/yyy/yyy:ccc_impl",
        "//extra/yyy/zzz:aaa_impl",
        "//extra/yyy/zzz:bbb_impl",
        "//extra/yyy/zzz:ccc_impl",
        "//extra/yyy:aaa_impl",
        "//extra/yyy:bbb_impl",
        "//extra/yyy:ccc_impl",
        "//extra/zzz:aaa_impl",
        "//extra/zzz:bbb_impl",
        "//extra/zzz:ccc_impl",
        "//extra:aaa_impl",
        "//extra:bbb_impl",
        "//extra:ccc_impl",
    ]
    .iter()
    .map(|l| label(l))
    .collect();
    assert_eq!(
        toolchain_labels(
            &get_value(&result, &toolchains_key),
            Some(&main_repo_package("extra")),
        ),
        expected
    );
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_reload
#[test]
fn test_registered_toolchains_reload() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//toolchain:toolchain_1\")\n"]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    assert_contains_at_least(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[label("//toolchain:toolchain_1_impl")],
    );

    // Re-write the MODULE.bazel.
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//toolchain:toolchain_2\")\n"]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);
    assert_contains_at_least(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[label("//toolchain:toolchain_2_impl")],
    );
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_bzlmod
#[test]
fn test_registered_toolchains_bzlmod() {
    let mut t = ToolchainTestCase::new();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "register_toolchains('//:tool')",
            "register_toolchains('//:dev_tool',dev_dependency=True)",
            "bazel_dep(name='bbb',version='1.0')",
            "bazel_dep(name='ccc',version='1.1')",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    t.add_registry_module(
        "bbb",
        "1.0",
        &[
            "module(name='bbb',version='1.0')",
            "register_toolchains('//:tool')",
            "register_toolchains('//:dev_tool',dev_dependency=True)",
            "bazel_dep(name='ddd',version='1.0')",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    t.add_registry_module(
        "ccc",
        "1.1",
        &[
            "module(name='ccc',version='1.1')",
            "register_toolchains('//:tool')",
            "register_toolchains('//:dev_tool',dev_dependency=True)",
            "bazel_dep(name='ddd',version='1.1')",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    // ddd@1.0 is not selected
    t.add_registry_module(
        "ddd",
        "1.0",
        &[
            "module(name='ddd',version='1.0')",
            "register_toolchains('//:tool')",
            "register_toolchains('//:dev_tool',dev_dependency=True)",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    t.add_registry_module(
        "ddd",
        "1.1",
        &[
            "module(name='ddd',version='1.1')",
            "register_toolchains('@eee//:tool', '//:tool')",
            "register_toolchains('@eee//:dev_tool',dev_dependency=True)",
            "bazel_dep(name='eee',version='1.0')",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    t.add_registry_module(
        "eee",
        "1.0",
        &[
            "module(name='eee',version='1.0')",
            "bazel_dep(name='toolchain_def',version='1.0')",
        ],
    );
    t.add_registry_module(
        "toolchain_def",
        "1.0",
        &["module(name='toolchain_def',version='1.0')"],
    );

    // Everyone depends on toolchain_def@1.0 for the declare_toolchain macro.
    let module_root = t.module_root();
    let toolchain_def_dir = module_root.get_child("toolchain_def+1.0");
    t.scratch.file(
        toolchain_def_dir.get_child("REPO.bazel").get_path_string(),
        &[],
    );
    t.scratch.file(
        toolchain_def_dir.get_child("BUILD").get_path_string(),
        &["toolchain_type(name = 'test_toolchain')"],
    );
    t.scratch.file(
        toolchain_def_dir
            .get_child("toolchain_def.bzl")
            .get_path_string(),
        &[
            "def _impl(ctx):",
            "    toolchain = platform_common.ToolchainInfo(data = ctx.attr.data)",
            "    return [toolchain]",
            "test_toolchain = rule(implementation = _impl, attrs = {'data': attr.string()})",
            "def declare_toolchain(name):",
            "    native.toolchain(",
            "        name = name,",
            "        toolchain_type = Label('//:test_toolchain'),",
            "        toolchain = ':' + name + '_impl')",
            "    test_toolchain(",
            "        name = name + '_impl',",
            "        data = 'stuff')",
        ],
    );

    // Now create the toolchains for each module.
    for repo in ["bbb+1.0", "ccc+1.1", "ddd+1.0", "ddd+1.1", "eee+1.0"] {
        t.scratch.file(
            module_root
                .get_child(repo)
                .get_child("REPO.bazel")
                .get_path_string(),
            &[],
        );
        t.scratch.file(
            module_root
                .get_child(repo)
                .get_child("BUILD")
                .get_path_string(),
            &[
                "load('@toolchain_def//:toolchain_def.bzl', 'declare_toolchain')",
                "declare_toolchain(name='tool')",
                "declare_toolchain(name='dev_tool')",
            ],
        );
    }
    t.scratch.overwrite_file(
        "BUILD",
        &[
            "load('@toolchain_def//:toolchain_def.bzl', 'declare_toolchain')",
            "declare_toolchain(name='dev_tool')",
            "declare_toolchain(name='tool')",
        ],
    );
    t.invalidate_packages();

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    if result.has_error() {
        panic!(
            "{:?}",
            result
                .get_first_error()
                .and_then(|e| e.get_exception().cloned())
        );
    }
    assert_has_no_error(&result);

    // Verify that the toolchains registered with bzlmod come in the BFS order
    assert_contains_at_least_in_order(
        &toolchain_labels(&get_value(&result, &toolchains_key), None),
        &[
            // Root module toolchains
            label("//:tool_impl"),
            label("//:dev_tool_impl"),
            // Other modules' toolchains
            label("@@bbb+//:tool_impl"),
            label("@@ccc+//:tool_impl"),
            label("@@eee+//:tool_impl"),
            label("@@ddd+//:tool_impl"),
        ],
    );
}

const TARGET_SETTING_OPTIMIZED: &str = r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

config_setting(
    name = "optimized",
    values = {
       "compilation_mode": "opt",
    },
)

toolchain(
    name = "extra_toolchain",
    exec_compatible_with = ["//constraints:linux"],
    target_compatible_with = ["//constraints:linux"],
    target_settings = [
        ":optimized",
    ],
    toolchain = ":extra_toolchain_impl",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "extra_toolchain_impl",
    data = "extra",
)
"#;

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetSetting
#[test]
fn test_registered_toolchains_target_setting() {
    let mut t = ToolchainTestCase::new();
    // Add an extra toolchain with a target_setting
    t.scratch.file("extra/BUILD", &[TARGET_SETTING_OPTIMIZED]);

    t.rewrite_module_dot_bazel(&[
        "register_toolchains(\"//toolchain:toolchain_1\", \"//extra:extra_toolchain\")\n",
    ]);

    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_toolchains flag is not present, because of
    // the configuration.
    let registered_toolchains_value = get_value(&result, &toolchains_key);
    let labels = toolchain_labels(&registered_toolchains_value, None);
    assert_contains_at_least(&labels, &[label("//toolchain:toolchain_1_impl")]);
    assert!(
        !labels.contains(&label("//extra:extra_toolchain_impl")),
        "{labels:?}"
    );
    assert!(registered_toolchains_value.rejected_toolchains.is_none());
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetSetting_debug
#[test]
fn test_registered_toolchains_target_setting_debug() {
    let mut t = ToolchainTestCase::new();
    // Add an extra toolchain with a target_setting
    t.scratch.file("extra/BUILD", &[TARGET_SETTING_OPTIMIZED]);

    t.rewrite_module_dot_bazel(&[
        "register_toolchains(\"//toolchain:toolchain_1\", \"//extra:extra_toolchain\")\n",
    ]);

    let toolchains_key = key(&mut t, /* debug= */ true);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    assert_has_no_error(&result);

    // Verify that the message about the unmatched config_setting is present.
    let registered_toolchains_value = get_value(&result, &toolchains_key);
    let rejected_toolchains = registered_toolchains_value
        .rejected_toolchains
        .as_ref()
        .expect("rejectedToolchains() is not null");
    assert_eq!(
        rejected_toolchains
            .get(&t.test_toolchain_type_label)
            .and_then(|row| row.get(&label("//extra:extra_toolchain")))
            .map(String::as_str),
        Some("mismatching target_settings: optimized"),
        "{rejected_toolchains:?}"
    );
}

// Java: RegisteredToolchainsFunctionTest.testRegisteredToolchains_targetSetting_error
#[test]
#[ignore = "BLAZE GAP: unported FeatureFlagValue.trimFlagValues (blaze_analysis \
            config/transitions.rs ConfigFeatureFlagTaggedTrimming panics under \
            --enforce_transitive_configs_for_config_feature_flag)"]
fn test_registered_toolchains_target_setting_error() {
    let mut t = ToolchainTestCase::new();
    // Add an extra toolchain with a target_setting
    t.scratch.file(
        "extra/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

config_setting(
    name = "flagged",
    flag_values = {":flag": "default"},
    transitive_configs = [":flag"],
)

config_feature_flag(
    name = "flag",
    allowed_values = [
        "default",
        "left",
        "right",
    ],
    default_value = "default",
)

toolchain(
    name = "extra_toolchain",
    exec_compatible_with = ["//constraints:linux"],
    target_compatible_with = ["//constraints:linux"],
    target_settings = [
        ":flagged",
    ],
    toolchain = ":extra_toolchain_impl",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "extra_toolchain_impl",
    data = "extra",
)
"#],
    );

    t.rewrite_module_dot_bazel(&[
        "register_toolchains(\"//toolchain:toolchain_1\", \"//extra:extra_toolchain\")\n",
    ]);

    // Need this so the feature flag is actually gone from the configuration.
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    let toolchains_key = key(&mut t, /* debug= */ false);
    let result = request_toolchains_from_skyframe(&mut t, &toolchains_key);
    let message = error_message(&result, &toolchains_key);
    assert!(
        message.contains(
            "Unrecoverable errors resolving config_setting associated with \
             //extra:extra_toolchain: For config_setting flagged: Feature flag \
             //extra:flag was accessed in a configuration it is not present in."
        ),
        "{message}"
    );
}
