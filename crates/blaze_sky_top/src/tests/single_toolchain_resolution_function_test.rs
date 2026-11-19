//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java`
//! (a `ToolchainTestCase`): tests for `SingleToolchainResolutionValue` and
//! `SingleToolchainResolutionFunction`.
//!
//! `testToolchainResolutionValue_equalsAndHashCode` needs no harness and stays in
//! `blaze_sky_analysis/src/toolchains/tests.rs`.

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_cmdline::Label;
use blaze_sky_analysis::toolchains::SingleToolchainResolutionKey;
use blaze_skyframe::EvaluationResult;

use crate::tests::toolchain_test_case::{ToolchainTestCase, assert_has_no_error, get_value, label};

/// The test class with its `@Before setUpKeys()` fields.
struct SingleToolchainResolutionFunctionTest {
    t: ToolchainTestCase,
    linux_ctkey: ConfiguredTargetKey,
    mac_ctkey: ConfiguredTargetKey,
}

/// `ConfiguredTargetKey.builder().setLabel(label).setConfiguration(getTargetConfiguration())`.
fn ct_key(t: &mut ToolchainTestCase, label_string: &str) -> ConfiguredTargetKey {
    let config = t.get_target_configuration();
    ConfiguredTargetKey::builder()
        .set_label(label(label_string))
        .set_configuration(Some(config.as_ref()))
        .build()
}

// Java: SingleToolchainResolutionFunctionTest.setUpKeys
fn set_up() -> SingleToolchainResolutionFunctionTest {
    let mut t = ToolchainTestCase::new();
    // This has to happen here so that targetConfiguration is populated.
    let linux_ctkey = ct_key(&mut t, "//platforms:linux");
    let mac_ctkey = ct_key(&mut t, "//platforms:mac");
    SingleToolchainResolutionFunctionTest {
        t,
        linux_ctkey,
        mac_ctkey,
    }
}

// Java: SingleToolchainResolutionFunctionTest.invokeToolchainResolution
fn invoke_toolchain_resolution(
    t: &mut ToolchainTestCase,
    key: &SingleToolchainResolutionKey,
) -> EvaluationResult {
    t.evaluate_key(key)
}

/// `SingleToolchainResolutionValue.key(configKey, toolchainType, toolchainTypeInfo,
/// targetPlatformKey, availableExecutionPlatformKeys)` (`debugTarget` false).
fn key(
    t: &mut ToolchainTestCase,
    target_platform_key: &ConfiguredTargetKey,
    available_execution_platform_keys: &[&ConfiguredTargetKey],
) -> SingleToolchainResolutionKey {
    SingleToolchainResolutionKey::key(
        t.target_config_key(),
        t.test_toolchain_type.clone(),
        t.test_toolchain_type_info.clone(),
        target_platform_key.clone(),
        available_execution_platform_keys
            .iter()
            .map(|k| (*k).clone())
            .collect(),
        /* debug_target= */ false,
    )
}

/// `assertThat(value.availableToolchainLabels()).containsExactly(k1, v1, k2, v2, ...)` (any
/// order).
fn assert_available_toolchain_labels(
    actual: &indexmap::IndexMap<ConfiguredTargetKey, Label>,
    expected: &[(&ConfiguredTargetKey, &str)],
) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (k, v) in expected {
        assert_eq!(actual.get(*k), Some(&label(v)), "{actual:?}");
    }
}

// Java: SingleToolchainResolutionFunctionTest.testResolution_singleExecutionPlatform
#[test]
fn test_resolution_single_execution_platform() {
    let SingleToolchainResolutionFunctionTest {
        mut t,
        linux_ctkey,
        mac_ctkey,
    } = set_up();
    let key = key(&mut t, &linux_ctkey, &[&mac_ctkey]);
    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_no_error(&result);

    let single_toolchain_resolution_value = get_value(&result, &key);
    assert_available_toolchain_labels(
        &single_toolchain_resolution_value.available_toolchain_labels,
        &[(&mac_ctkey, "//toolchain:toolchain_2_impl")],
    );
}

// Java: SingleToolchainResolutionFunctionTest.testResolution_multipleExecutionPlatforms
#[test]
fn test_resolution_multiple_execution_platforms() {
    let SingleToolchainResolutionFunctionTest {
        mut t,
        linux_ctkey,
        mac_ctkey,
    } = set_up();
    t.add_toolchain(
        "extra",
        "extra_toolchain",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );
    t.rewrite_module_dot_bazel(&[r#"register_toolchains(
    "//toolchain:toolchain_1",
    "//toolchain:toolchain_2",
    "//extra:extra_toolchain",
)
"#]);

    let key = key(&mut t, &linux_ctkey, &[&linux_ctkey, &mac_ctkey]);
    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_no_error(&result);

    let single_toolchain_resolution_value = get_value(&result, &key);
    assert_available_toolchain_labels(
        &single_toolchain_resolution_value.available_toolchain_labels,
        &[
            (&linux_ctkey, "//extra:extra_toolchain_impl"),
            (&mac_ctkey, "//toolchain:toolchain_2_impl"),
        ],
    );
}

// Java: SingleToolchainResolutionFunctionTest.testResolution_noneFound
#[test]
fn test_resolution_none_found() {
    let SingleToolchainResolutionFunctionTest {
        mut t,
        linux_ctkey,
        mac_ctkey,
    } = set_up();
    // Clear the toolchains.
    t.rewrite_module_dot_bazel(&[]);

    let key = key(&mut t, &linux_ctkey, &[&mac_ctkey]);
    let result = invoke_toolchain_resolution(&mut t, &key);

    let single_toolchain_resolution_value = get_value(&result, &key);
    assert!(
        single_toolchain_resolution_value
            .available_toolchain_labels
            .is_empty()
    );
}

// Java: SingleToolchainResolutionFunctionTest.testResolution_checkPlatformAllowedToolchains
#[test]
fn test_resolution_check_platform_allowed_toolchains() {
    let SingleToolchainResolutionFunctionTest {
        mut t, linux_ctkey, ..
    } = set_up();
    // Define two new execution platforms, only one of which is compatible with the test toolchain.
    t.scratch.file(
        "allowed/BUILD",
        &[r#"platform(
    name = "fails_match",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        # Empty, so doesn't match anything.
    ],
)

platform(
    name = "matches",
    check_toolchain_types = True,
    allowed_toolchain_types = [
        "//toolchain:test_toolchain",
    ],
)
"#],
    );
    let fails_match_platform_ct_key = ct_key(&mut t, "//allowed:fails_match");
    let allowed_platform_ct_key = ct_key(&mut t, "//allowed:matches");

    // Define the toolchains themselves.
    t.add_toolchain("extra", "extra_toolchain", &[], &[], "baz");
    t.rewrite_module_dot_bazel(&["register_toolchains(\"//extra:extra_toolchain\")\n"]);

    // Resolve toolchains.
    let key = key(
        &mut t,
        &linux_ctkey,
        &[&fails_match_platform_ct_key, &allowed_platform_ct_key],
    );
    let result = invoke_toolchain_resolution(&mut t, &key);

    assert_has_no_error(&result);

    let single_toolchain_resolution_value = get_value(&result, &key);
    assert_available_toolchain_labels(
        &single_toolchain_resolution_value.available_toolchain_labels,
        &[(&allowed_platform_ct_key, "//extra:extra_toolchain_impl")],
    );
}
