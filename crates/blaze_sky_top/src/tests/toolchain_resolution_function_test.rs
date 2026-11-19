//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java`
//! (a `ToolchainTestCase`): tests for `UnloadedToolchainContext` and
//! `ToolchainResolutionFunction`.

use std::sync::Arc;

use blaze_analysis::toolchains::{
    ToolchainContextKey, ToolchainExceptionKind, ToolchainTypeRequirement, UnloadedToolchainContext,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_optdefs::fragment::platform_options::PlatformOptions;
use blaze_options::Foreign;
use blaze_skyframe::EvaluationResult;

use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_doesnt_have_toolchain_type, assert_error_is_toolchain_exception,
    assert_has_error, assert_has_execution_platform, assert_has_no_error,
    assert_has_resolved_toolchain, assert_has_target_platform, assert_has_toolchain_type,
    error_message, get_value, label, optional_requirement,
};

// Java: ToolchainResolutionFunctionTest.invokeToolchainResolution
fn invoke_toolchain_resolution(
    t: &mut ToolchainTestCase,
    key: &ToolchainContextKey,
) -> EvaluationResult {
    t.evaluate_key(key)
}

/// `ToolchainContextKey.key().configurationKey(configKey).toolchainTypes(types...).build()`.
fn key_with_types(
    config_key: BuildConfigurationKey,
    toolchain_types: &[ToolchainTypeRequirement],
) -> ToolchainContextKey {
    ToolchainContextKey::key()
        .configuration_key(config_key)
        .toolchain_types(toolchain_types.iter().cloned())
        .build()
}

/// `assertThatEvaluationResult(result).hasNoError()`, then `result.get(key)` is not null.
fn get_context(
    result: &EvaluationResult,
    key: &ToolchainContextKey,
) -> Arc<UnloadedToolchainContext> {
    assert_has_no_error(result);
    get_value(result, key)
}

/// A copy of `targetConfigKey`'s options with `PlatformOptions` changed, as a new key (Java:
/// `targetConfigKey.getOptions().clone()`, a mutation, `BuildConfigurationKey.create`).
fn modified_config_key(
    t: &mut ToolchainTestCase,
    modify: impl FnOnce(&mut PlatformOptions),
) -> BuildConfigurationKey {
    let target_config_key = t.target_config_key();
    let mut new_options = (**target_config_key.get_options()).clone();
    modify(
        new_options
            .get_mut::<PlatformOptions>()
            .expect("PlatformOptions"),
    );
    BuildConfigurationKey::create(Arc::new(new_options))
}

/// The two toolchains most tests add to package `extra`.
fn add_extra_linux_and_mac_toolchains(t: &mut ToolchainTestCase) {
    t.add_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );
    t.add_toolchain(
        "extra",
        "extra_toolchain_mac",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
}

const PLATFORM_ALIASES: &str = r#"alias(name = 'mac', actual = '//platforms:mac')
alias(name = 'linux', actual = '//platforms:linux')
"#;

// Java: ToolchainResolutionFunctionTest.resolve
#[test]
fn resolve() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, because platform
    // mac is listed first.
    add_extra_linux_and_mac_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_useTargetPlatformConstraints_inheritedFromParentPlatform
#[test]
fn resolve_use_target_platform_constraints_inherited_from_parent_platform() {
    let mut t = ToolchainTestCase::new();
    // The target platform only sets its constraints via a parent platform. A toolchain with
    // use_target_platform_constraints must still reject execution platforms whose constraints
    // (also only set via a parent) differ from those of the target platform.
    t.scratch.append_file(
        "platforms/BUILD",
        &[r#"platform(
    name = "linux_base",
    constraint_values = ["//constraints:linux"],
)

platform(
    name = "linux_child",
    parents = [":linux_base"],
)

platform(
    name = "mac_base",
    constraint_values = ["//constraints:mac"],
)

platform(
    name = "mac_child",
    parents = [":mac_base"],
)
"#],
    );
    t.scratch.file(
        "extra/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain(
    name = "extra_toolchain",
    toolchain = ":extra_toolchain_impl",
    toolchain_type = "//toolchain:test_toolchain",
    use_target_platform_constraints = True,
)

test_toolchain(
    name = "extra_toolchain_impl",
    data = "baz",
)
"#],
    );
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//extra:extra_toolchain")
register_execution_platforms("//platforms:mac_child", "//platforms:linux_child")
"#]);

    t.use_configuration(&["--platforms=//platforms:linux_child"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(&*unloaded_toolchain_context, "//extra:extra_toolchain_impl");
    // //platforms:mac_child is registered first, but is not compatible with the target platform.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux_child");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux_child");
}

// Java: ToolchainResolutionFunctionTest.resolve_hostPlatform
#[test]
fn resolve_host_platform() {
    let mut t = ToolchainTestCase::new();
    add_extra_linux_and_mac_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
"#,
    ]);

    t.use_configuration(&[
        "--platforms=//platforms:linux",
        "--host_platform=//platforms:linux",
    ]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_hostPlatform_alias
#[test]
fn resolve_host_platform_alias() {
    let mut t = ToolchainTestCase::new();
    add_extra_linux_and_mac_toolchains(&mut t);
    // Set up aliases for the platforms.
    t.scratch.file("alias/BUILD", &[PLATFORM_ALIASES]);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
"#,
    ]);

    t.use_configuration(&[
        "--platforms=//platforms:linux",
        "--host_platform=//alias:linux",
    ]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_optional
#[test]
fn resolve_optional() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, because platform
    // mac is listed first.
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_mac",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.optional_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(
        &*unloaded_toolchain_context,
        &t.optional_toolchain_type_label,
    );
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_optional_on_first_platform
#[test]
fn resolve_optional_on_first_platform() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, independent of platform order
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_mac",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.optional_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(
        &*unloaded_toolchain_context,
        &t.optional_toolchain_type_label,
    );
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_optional_on_second_platform
#[test]
fn resolve_optional_on_second_platform() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, independent of platform order
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_mac",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:linux", "//platforms:mac")
"#]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.optional_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(
        &*unloaded_toolchain_context,
        &t.optional_toolchain_type_label,
    );
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_max_optional_on_second_platform
#[test]
fn resolve_max_optional_on_second_platform() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, independent of platform order
    // and independent of non-existence of the second optional toolchain
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_mac",
        &["//constraints:mac"],
        &["//constraints:linux"],
        "baz",
    );
    t.scratch.append_file(
        "toolchain/BUILD",
        &["toolchain_type(name = 'extra_optional_toolchain')"],
    );
    let extra_optional_toolchain_type_label = label("//toolchain:extra_optional_toolchain");
    let extra_optional_toolchain_type = optional_requirement(extra_optional_toolchain_type_label);
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//extra:extra_toolchain_mac")
register_execution_platforms('//platforms:linux', '//platforms:mac')
"#]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.optional_toolchain_type.clone(),
            extra_optional_toolchain_type,
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(
        &*unloaded_toolchain_context,
        &t.optional_toolchain_type_label,
    );
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

/// The setup of `resolve_multiple`, `resolve_multiple_optional` and
/// `resolve_multiple_optional_missing`.
fn add_main_toolchains(t: &mut ToolchainTestCase, with_second: bool) {
    t.add_toolchain(
        "main",
        "main_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );
    if with_second {
        t.add_toolchain_with_type(
            "main",
            "second_toolchain_linux",
            &label("//second:toolchain_type"),
            &["//constraints:linux"],
            &["//constraints:linux"],
            "baz",
        );
    }
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//main:all")
register_execution_platforms("//platforms:linux")
"#]);
}

// Java: ToolchainResolutionFunctionTest.resolve_multiple
#[test]
fn resolve_multiple() {
    let mut t = ToolchainTestCase::new();
    let second_toolchain_type_label = label("//second:toolchain_type");
    let second_toolchain_type_requirement =
        ToolchainTypeRequirement::create(second_toolchain_type_label.clone());
    t.scratch
        .file("second/BUILD", &["toolchain_type(name = 'toolchain_type')"]);

    add_main_toolchains(&mut t, true);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.test_toolchain_type.clone(),
            second_toolchain_type_requirement,
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//main:main_toolchain_linux_impl",
    );
    assert_has_toolchain_type(&*unloaded_toolchain_context, &second_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//main:second_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_mandatory_missing
#[test]
fn resolve_mandatory_missing() {
    let mut t = ToolchainTestCase::new();
    // There is no toolchain for the requested type.
    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_eq!(
        error_message(&result, &key),
        "No matching toolchains found for types:
  //toolchain:test_toolchain
To debug, rerun with --toolchain_resolution_debug='//toolchain:test_toolchain'
For more information on platforms or toolchains see https://bazel.build/concepts/platforms-intro."
    );
}

// Java: ToolchainResolutionFunctionTest.unresolved_toolchain_message_regex_quotes
#[test]
fn unresolved_toolchain_message_regex_quotes() {
    let platform_info = blaze_analysis::platform::PlatformInfo::builder()
        .set_label(label("//platforms:test_platform"))
        .build()
        .expect("PlatformInfo");
    let toolchain_type_info = blaze_analysis::platform::ToolchainTypeInfo::create(label(
        "@@repo+//toolchain:test_toolchain",
    ));
    let mut missing_toolchain_types = indexmap::IndexSet::new();
    missing_toolchain_types.insert(toolchain_type_info);

    let expected = "No matching toolchains found for types:
  @@repo+//toolchain:test_toolchain
To debug, rerun with --toolchain_resolution_debug='\\Q@@repo+//toolchain:test_toolchain\\E'
";
    // Java: assertThat(new ToolchainResolutionFunction.UnresolvedToolchainsException(
    // platformInfo, missingToolchainTypes)).hasMessageThat().isEqualTo(expected).
    let exception =
        blaze_sky_analysis::toolchains::toolchain_resolution_function::unresolved_toolchains_exception(
            &platform_info,
            &missing_toolchain_types,
        );
    assert_eq!(exception.get_message(), expected);
}

// Java: ToolchainResolutionFunctionTest.resolve_mandatory_missing_customPlatformMessage
#[test]
fn resolve_mandatory_missing_custom_platform_message() {
    let mut t = ToolchainTestCase::new();
    t.scratch.append_file(
        "platforms/BUILD",
        &[r#"platform(
    name = "linux_custom_message",
    parents = [":linux"],
    missing_toolchain_error = "Check custom docs for setup instructions",
)
"#],
    );

    // There is no toolchain for the requested type.
    t.use_configuration(&["--platforms=//platforms:linux_custom_message"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let message = error_message(&result, &key);
    assert!(
        message.contains("Check custom docs for setup instructions"),
        "{message}"
    );
    assert!(
        !message.contains("see https://bazel.build/concepts/platforms-intro"),
        "{message}"
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_multiple_optional
#[test]
fn resolve_multiple_optional() {
    let mut t = ToolchainTestCase::new();
    let second_toolchain_type_label = label("//second:toolchain_type");
    let second_toolchain_type_requirement =
        optional_requirement(second_toolchain_type_label.clone());
    t.scratch
        .file("second/BUILD", &["toolchain_type(name = 'toolchain_type')"]);

    add_main_toolchains(&mut t, true);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.test_toolchain_type.clone(),
            second_toolchain_type_requirement,
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//main:main_toolchain_linux_impl",
    );
    assert_has_toolchain_type(&*unloaded_toolchain_context, &second_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//main:second_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_multiple_optional_missing
#[test]
fn resolve_multiple_optional_missing() {
    use blaze_analysis::toolchains::ToolchainContext;

    let mut t = ToolchainTestCase::new();
    let second_toolchain_type_label = label("//second:toolchain_type");
    let second_toolchain_type_requirement =
        optional_requirement(second_toolchain_type_label.clone());
    t.scratch
        .file("second/BUILD", &["toolchain_type(name = 'toolchain_type')"]);

    add_main_toolchains(&mut t, false);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.test_toolchain_type.clone(),
            second_toolchain_type_requirement,
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//main:main_toolchain_linux_impl",
    );
    assert_has_toolchain_type(&*unloaded_toolchain_context, &second_toolchain_type_label);
    assert!(
        !unloaded_toolchain_context
            .resolved_toolchain_labels()
            .contains(&label("//main:second_toolchain_linux_impl"))
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_toolchainTypeAlias
#[test]
fn resolve_toolchain_type_alias() {
    let mut t = ToolchainTestCase::new();
    t.add_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//extra:extra_toolchain_linux")
register_execution_platforms("//platforms:linux")
"#]);

    // Set up an alias for the toolchain type.
    let aliased_toolchain_type_label = label("//alias:toolchain_type");
    t.scratch.file(
        "alias/BUILD",
        &["alias(name = 'toolchain_type', actual = '//toolchain:test_toolchain')"],
    );

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[ToolchainTypeRequirement::create(
            aliased_toolchain_type_label,
        )],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_noToolchainType
#[test]
fn resolve_no_toolchain_type() {
    use blaze_analysis::toolchains::ToolchainContext;

    let mut t = ToolchainTestCase::new();
    t.scratch.file("host/BUILD", &["platform(name = 'host')"]);
    t.rewrite_module_dot_bazel(&[
        r#"register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&[
        "--host_platform=//host:host",
        "--platforms=//platforms:linux",
    ]);
    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert!(unloaded_toolchain_context.toolchain_types().is_empty());
    // Even with no toolchains requested, should still select the first execution platform.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_noToolchainType_hostNotAvailable
#[test]
fn resolve_no_toolchain_type_host_not_available() {
    use blaze_analysis::toolchains::ToolchainContext;

    let mut t = ToolchainTestCase::new();
    t.scratch.file("host/BUILD", &["platform(name = 'host')"]);
    t.scratch.file(
        "sample/BUILD",
        &[r#"constraint_setting(name = "demo")

constraint_value(
    name = "demo_a",
    constraint_setting = ":demo",
)

constraint_value(
    name = "demo_b",
    constraint_setting = ":demo",
)

platform(
    name = "sample_a",
    constraint_values = [":demo_a"],
)

platform(
    name = "sample_b",
    constraint_values = [":demo_b"],
)
"#],
    );
    t.rewrite_module_dot_bazel(&[r#"register_execution_platforms(
    "//platforms:mac",
    "//platforms:linux",
    "//sample:sample_a",
    "//sample:sample_b",
)
"#]);

    t.use_configuration(&[
        "--host_platform=//host:host",
        "--platforms=//platforms:linux",
    ]);
    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .exec_constraint_labels([label("//sample:demo_b")])
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert!(unloaded_toolchain_context.toolchain_types().is_empty());
    assert_has_execution_platform(&*unloaded_toolchain_context, "//sample:sample_b");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_unavailableToolchainType_single
#[test]
fn resolve_unavailable_toolchain_type_single() {
    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    t.scratch.file("fake/toolchain/BUILD", &[""]);
    t.use_configuration(&[
        "--host_platform=//platforms:linux",
        "--platforms=//platforms:mac",
    ]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.test_toolchain_type.clone(),
            ToolchainTypeRequirement::create(label("//fake/toolchain:type_1")),
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidToolchainType,
    );
    t.assert_contains_event("no such target '//fake/toolchain:type_1'");
}

// Java: ToolchainResolutionFunctionTest.resolve_optional_unavailableToolchainType_single
#[test]
fn resolve_optional_unavailable_toolchain_type_single() {
    use blaze_analysis::toolchains::ToolchainContext;

    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    t.scratch.file("fake/toolchain/BUILD", &[""]);
    t.use_configuration(&[
        "--host_platform=//platforms:linux",
        "--platforms=//platforms:linux",
    ]);
    let key = key_with_types(t.target_config_key(), &[t.optional_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(
        &*unloaded_toolchain_context,
        &t.optional_toolchain_type_label,
    );
    assert!(
        unloaded_toolchain_context
            .resolved_toolchain_labels()
            .is_empty()
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_unavailableToolchainType_multiple
#[test]
fn resolve_unavailable_toolchain_type_multiple() {
    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    t.scratch.file("fake/toolchain/BUILD", &[""]);
    t.use_configuration(&[
        "--host_platform=//platforms:linux",
        "--platforms=//platforms:mac",
    ]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            t.test_toolchain_type.clone(),
            ToolchainTypeRequirement::create(label("//fake/toolchain:type_1")),
            ToolchainTypeRequirement::create(label("//fake/toolchain:type_2")),
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidToolchainType,
    );
    // Only one of the missing types will be reported, so do not check the specific error message.
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidToolchainType
#[test]
fn resolve_invalid_toolchain_type() {
    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    t.scratch.file(
        "fake/toolchain/BUILD",
        &["filegroup(name = 'not_a_toolchain')"],
    );
    t.use_configuration(&[
        "--host_platform=//platforms:linux",
        "--platforms=//platforms:mac",
    ]);
    let key = key_with_types(
        t.target_config_key(),
        &[ToolchainTypeRequirement::create(label(
            "//fake/toolchain:not_a_toolchain",
        ))],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidToolchainType,
    );
    let message = error_message(&result, &key);
    assert!(
        message.contains("but does not provide ToolchainTypeInfo"),
        "{message}"
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidToolchainType_ignored
#[test]
fn resolve_invalid_toolchain_type_ignored() {
    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    t.scratch.file(
        "fake/toolchain/BUILD",
        &["filegroup(name = 'not_a_toolchain')"],
    );
    t.use_configuration(&[
        "--host_platform=//platforms:linux",
        "--platforms=//platforms:mac",
    ]);
    let key = key_with_types(
        t.target_config_key(),
        &[ToolchainTypeRequirement {
            toolchain_type: label("//fake/toolchain:not_a_toolchain"),
            mandatory: true,
            ignore_if_invalid: true,
        }],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);
    assert_doesnt_have_toolchain_type(
        &*unloaded_toolchain_context,
        "//fake/toolchain:not_a_toolchain",
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidTargetPlatform_badTarget
#[test]
fn resolve_invalid_target_platform_bad_target() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_platform')"]);

    // Manually create a configuration key: trying to call `useConfiguration` will immediately throw
    // the exception this is checking for.
    let config_key = modified_config_key(&mut t, |p| {
        p.platforms = vec![Foreign(label("//invalid:not_a_platform"))];
    });

    // Create the toolchain context key and evaluate it.
    let key = key_with_types(config_key, &[t.test_toolchain_type.clone()]);

    t.remove_fail_fast_handler(); // expect errors
    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    let message = error_message(&result, &key);
    assert!(
        message.contains(
            "//invalid:not_a_platform was referenced as a platform, but does not provide PlatformInfo"
        ),
        "{message}"
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidTargetPlatform_badPackage
#[test]
fn resolve_invalid_target_platform_bad_package() {
    let mut t = ToolchainTestCase::new();
    t.scratch.delete_file("invalid");

    // Manually create a configuration key: trying to call `useConfiguration` will immediately throw
    // the exception this is checking for.
    let config_key = modified_config_key(&mut t, |p| {
        p.platforms = vec![Foreign(label("//invalid:not_a_platform"))];
    });

    // Create the toolchain context key and evaluate it.
    let key = key_with_types(config_key, &[t.test_toolchain_type.clone()]);

    t.remove_fail_fast_handler(); // expect errors
    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    let message = error_message(&result, &key);
    assert!(message.contains("BUILD file not found"), "{message}");
}

// Java: ToolchainResolutionFunctionTest.resolve_executionPlatform_alias
#[test]
fn resolve_execution_platform_alias() {
    let mut t = ToolchainTestCase::new();
    // This should select platform mac, toolchain extra_toolchain_mac, because platform
    // mac is listed first.
    add_extra_linux_and_mac_toolchains(&mut t);
    // Set up aliases for the platforms.
    t.scratch.file("alias/BUILD", &[PLATFORM_ALIASES]);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms('//alias:mac', '//alias:linux')
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_mac_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:mac");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidHostPlatform
#[test]
fn resolve_invalid_host_platform() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_platform')"]);

    // Manually create a configuration key: trying to call `useConfiguration` will immediately throw
    // the exception this is checking for.
    let config_key = modified_config_key(&mut t, |p| {
        p.host_platform = Foreign(label("//invalid:not_a_platform"));
    });

    let key = key_with_types(config_key, &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    let message = error_message(&result, &key);
    assert!(message.contains("//invalid:not_a_platform"), "{message}");
}

// Java: ToolchainResolutionFunctionTest.resolve_invalidExecutionPlatform
#[test]
fn resolve_invalid_execution_platform() {
    let mut t = ToolchainTestCase::new();
    // Have to use a rule that doesn't require a target platform, or else there will be a cycle.
    t.scratch.file(
        "invalid/BUILD",
        &["toolchain_type(name = 'not_a_platform')"],
    );
    t.use_configuration(&["--extra_execution_platforms=//invalid:not_a_platform"]);
    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    let message = error_message(&result, &key);
    assert!(message.contains("//invalid:not_a_platform"), "{message}");
}

// Java: ToolchainResolutionFunctionTest.resolve_execConstraints
#[test]
fn resolve_exec_constraints() {
    let mut t = ToolchainTestCase::new();
    // This should select platform linux, toolchain extra_toolchain_linux, due to extra constraints,
    // even though platform mac is registered first.
    add_extra_linux_and_mac_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .toolchain_types([t.test_toolchain_type.clone()])
        .exec_constraint_labels([label("//constraints:linux")])
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_execConstraints_invalid
#[test]
fn resolve_exec_constraints_invalid() {
    let mut t = ToolchainTestCase::new();
    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .toolchain_types([t.test_toolchain_type.clone()])
        .exec_constraint_labels([label("//platforms:linux")])
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidConstraintValue,
    );
    let message = error_message(&result, &key);
    assert!(message.contains("//platforms:linux"), "{message}");
}

// Java: ToolchainResolutionFunctionTest.resolve_noMatchingPlatform
#[test]
fn resolve_no_matching_platform() {
    let mut t = ToolchainTestCase::new();
    // Write toolchain A, and a toolchain implementing it.
    t.scratch.append_file(
        "a/BUILD",
        &[r#"toolchain_type(name = "toolchain_type_A")

toolchain(
    name = "toolchain",
    exec_compatible_with = ["//constraints:mac"],
    target_compatible_with = [],
    toolchain = ":toolchain_impl",
    toolchain_type = ":toolchain_type_A",
)

filegroup(name = "toolchain_impl")
"#],
    );
    // Write toolchain B, and a toolchain implementing it.
    t.scratch.append_file(
        "b/BUILD",
        &[r#"load("//toolchain:toolchain_def.bzl", "test_toolchain")

toolchain_type(name = "toolchain_type_B")

toolchain(
    name = "toolchain",
    exec_compatible_with = ["//constraints:linux"],
    target_compatible_with = [],
    toolchain = ":toolchain_impl",
    toolchain_type = ":toolchain_type_B",
)

filegroup(name = "toolchain_impl")
"#],
    );

    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//a:toolchain", "//b:toolchain")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = key_with_types(
        t.target_config_key(),
        &[
            ToolchainTypeRequirement::create(label("//a:toolchain_type_A")),
            ToolchainTypeRequirement::create(label("//b:toolchain_type_B")),
        ],
    );

    let result = invoke_toolchain_resolution(&mut t, &key);
    let unloaded_toolchain_context = get_context(&result, &key);
    assert!(unloaded_toolchain_context.error_data().is_some());
}

/// The key of the `resolve_forceExecutionPlatform*` tests.
fn force_linux_key(t: &mut ToolchainTestCase) -> ToolchainContextKey {
    ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .toolchain_types([t.test_toolchain_type.clone()])
        .force_execution_platform(label("//platforms:linux"))
        .build()
}

// Java: ToolchainResolutionFunctionTest.resolve_forceExecutionPlatform
#[test]
fn resolve_force_execution_platform() {
    let mut t = ToolchainTestCase::new();
    // This should select execution platform linux, toolchain extra_toolchain_linux, due to the
    // forced execution platform, even though execution platform mac is registered first.
    add_extra_linux_and_mac_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = force_linux_key(&mut t);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_forceExecutionPlatform_alias
#[test]
fn resolve_force_execution_platform_alias() {
    let mut t = ToolchainTestCase::new();
    // This should select execution platform linux, toolchain extra_toolchain_linux, due to the
    // forced execution platform, even though execution platform mac is registered first.
    add_extra_linux_and_mac_toolchains(&mut t);
    // Set up aliases for the platforms.
    t.scratch.file("alias/BUILD", &[PLATFORM_ALIASES]);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
register_execution_platforms("//alias:mac", "//alias:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = force_linux_key(&mut t);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_forceExecutionPlatform_host
#[test]
fn resolve_force_execution_platform_host() {
    let mut t = ToolchainTestCase::new();
    // This should select execution platform linux, toolchain extra_toolchain_linux, due to the
    // forced execution platform, even though execution platform mac is registered first.
    add_extra_linux_and_mac_toolchains(&mut t);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")
"#,
    ]);

    t.use_configuration(&[
        "--platforms=//platforms:linux",
        "--host_platform=//platforms:linux",
    ]);
    let key = force_linux_key(&mut t);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Regression test for https://github.com/bazelbuild/bazel/issues/22607, where the aliased host
// platform didn't match with the dereferenced forced execution platform and so no toolchain
// was selected.
// Java: ToolchainResolutionFunctionTest.resolve_forceExecutionPlatform_host_alias
#[test]
fn resolve_force_execution_platform_host_alias() {
    let mut t = ToolchainTestCase::new();
    // This should select execution platform linux, toolchain extra_toolchain_linux, due to the
    // forced execution platform, even though execution platform mac is registered first.
    add_extra_linux_and_mac_toolchains(&mut t);
    // Set up aliases for the platforms.
    t.scratch.file("alias/BUILD", &[PLATFORM_ALIASES]);
    t.rewrite_module_dot_bazel(&[
        r#"register_toolchains("//extra:extra_toolchain_linux", "//extra:extra_toolchain_mac")

# This test requires an execution platform that isn't the forced platform in order to
# trigger.
register_execution_platforms("//alias:mac")
"#,
    ]);

    t.use_configuration(&[
        "--platforms=//platforms:linux",
        "--host_platform=//alias:linux",
    ]);
    // Use the actual label for the forced exec platform, since this was redeferenced
    // earlier in analysis.
    let key = force_linux_key(&mut t);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_toolchain_type(&*unloaded_toolchain_context, &t.test_toolchain_type_label);
    assert_has_resolved_toolchain(
        &*unloaded_toolchain_context,
        "//extra:extra_toolchain_linux_impl",
    );
    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.resolve_forceExecutionPlatform_noRequiredToolchains
#[test]
fn resolve_force_execution_platform_no_required_toolchains() {
    let mut t = ToolchainTestCase::new();
    // This should select execution platform linux, due to the forced execution platform, even
    // though execution platform mac is registered first.
    t.rewrite_module_dot_bazel(&[
        r#"register_execution_platforms("//platforms:mac", "//platforms:linux")
"#,
    ]);

    t.use_configuration(&["--platforms=//platforms:linux"]);
    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .force_execution_platform(label("//platforms:linux"))
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    assert_has_execution_platform(&*unloaded_toolchain_context, "//platforms:linux");
    assert_has_target_platform(&*unloaded_toolchain_context, "//platforms:linux");
}

// Java: ToolchainResolutionFunctionTest.errorProperlyReportedWhenInvalidConfigurationConfiguration
#[test]
fn error_properly_reported_when_invalid_configuration_configuration() {
    let mut t = ToolchainTestCase::new();
    // It would be absolutely insane for a user to have a toolchain w/ a config_setting that reads a
    // config_feature_flag; however, should still test the InvalidConfigurationException codepath.
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//strange:strange_toolchain")
register_execution_platforms("//platforms:mac", "//platforms:linux")
"#]);
    t.scratch.file(
        "strange/BUILD",
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
    name = "strange_toolchain",
    target_settings = [":flagged"],
    toolchain = ":strange_test_toolchain",
    toolchain_type = "//toolchain:test_toolchain",
)

test_toolchain(
    name = "strange_test_toolchain",
    data = "foo",
)
"#],
    );
    t.scratch.file(
        "rule/rule_def.bzl",
        &[r#"def _impl(ctx):
    pass

my_rule = rule(
    implementation = _impl,
    toolchains = ["//toolchain:test_toolchain"],
)
"#],
    );
    t.scratch.file(
        "rule/BUILD",
        &[r#"load("//rule:rule_def.bzl", "my_rule")

my_rule(
    name = "me",
)
"#],
    );
    // Need this so the feature flag actually gone from the configuration.
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    t.remove_fail_fast_handler(); // expect errors
    assert!(t.get_configured_target("//rule:me").is_none());
    t.assert_contains_event(
        "Unrecoverable errors resolving config_setting associated with \
         //strange:strange_toolchain: For config_setting flagged: Feature flag \
         //strange:flag was accessed in a configuration it is not present in.",
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_checkPlatformAllowedToolchains_match
#[test]
fn resolve_check_platform_allowed_toolchains_match() {
    let mut t = ToolchainTestCase::new();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"platform(
    name = "allows_single_toolchain",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        "//toolchain:test_toolchain",
    ],
)

platform(
    name = "allows_all",
)
"#],
    );
    t.add_toolchain("toolchain", "toolchain_impl", &[], &[], "baz");
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//toolchain:toolchain_impl")
register_execution_platforms("//allowed:allows_single_toolchain", "//allowed:allows_all")
"#]);

    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    // The platform allows the required toolchain type, so it is selected.
    assert_has_execution_platform(
        &*unloaded_toolchain_context,
        "//allowed:allows_single_toolchain",
    );
}

// Java: ToolchainResolutionFunctionTest.resolve_checkPlatformAllowedToolchains_failsMatch
#[test]
fn resolve_check_platform_allowed_toolchains_fails_match() {
    let mut t = ToolchainTestCase::new();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"toolchain_type(name = "other_toolchain")

platform(
    name = "allows_single_toolchain",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        ":other_toolchain",
    ],
)

platform(
    name = "allows_all",
)
"#],
    );
    t.add_toolchain("toolchain", "toolchain_impl", &[], &[], "baz");
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//toolchain:toolchain_impl")
register_execution_platforms("//allowed:allows_single_toolchain", "//allowed:allows_all")
"#]);

    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    // The platform does not allow the required toolchain type, so it is not selected.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//allowed:allows_all");
}

// Java: ToolchainResolutionFunctionTest.resolve_checkPlatformAllowedToolchains_noneRequested_failsMatch
#[test]
fn resolve_check_platform_allowed_toolchains_none_requested_fails_match() {
    let mut t = ToolchainTestCase::new();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"platform(
    name = "allows_single_toolchain",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        "//toolchain:test_toolchain",
    ],
)

platform(
    name = "allows_all",
)
"#],
    );
    t.add_toolchain("toolchain", "toolchain_impl", &[], &[], "baz");
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//toolchain:toolchain_impl")
register_execution_platforms("//allowed:allows_single_toolchain", "//allowed:allows_all")
"#]);

    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    // The platform requires a toolchain type, so it is not selected.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//allowed:allows_all");
}

// Java: ToolchainResolutionFunctionTest.resolve_checkPlatformAllowedToolchains_noToolchainType_noneRequested
#[test]
fn resolve_check_platform_allowed_toolchains_no_toolchain_type_none_requested() {
    let mut t = ToolchainTestCase::new();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"platform(
    name = "allows_none",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        # Empty, so doesn't match anything.
    ],
)

platform(
    name = "allows_all",
)
"#],
    );
    t.rewrite_module_dot_bazel(&[
        r#"register_execution_platforms("//allowed:allows_none", "//allowed:allows_all")
"#,
    ]);

    let key = ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .build();

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    // The platform doesn't have any toolchains specified.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//allowed:allows_all");
}

// Java: ToolchainResolutionFunctionTest.resolve_checkPlatformAllowedToolchains_noToolchainType
#[test]
fn resolve_check_platform_allowed_toolchains_no_toolchain_type() {
    let mut t = ToolchainTestCase::new();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"platform(
    name = "allows_none",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        # Empty, so doesn't match anything.
    ],
)

platform(
    name = "allows_all",
)
"#],
    );
    t.add_toolchain("toolchain", "toolchain_impl", &[], &[], "baz");
    t.rewrite_module_dot_bazel(&[r#"register_toolchains("//toolchain:toolchain_impl")
register_execution_platforms("//allowed:allows_none", "//allowed:allows_all")
"#]);

    let key = key_with_types(t.target_config_key(), &[t.test_toolchain_type.clone()]);

    let result = invoke_toolchain_resolution(&mut t, &key);

    let unloaded_toolchain_context = get_context(&result, &key);

    // The platform doesn't have any toolchains specified, but the request does.
    assert_has_execution_platform(&*unloaded_toolchain_context, "//allowed:allows_all");
}
