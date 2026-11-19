//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java`
//! (a `ToolchainTestCase`): tests for `RegisteredExecutionPlatformsFunction` and
//! `RegisteredExecutionPlatformsValue`.
//!
//! `testRegisteredExecutionPlatformsValue_equalsAndHashCode` needs no harness and stays in
//! `blaze_sky_analysis/src/toolchains/tests.rs`.

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::toolchains::ToolchainExceptionKind;
use blaze_cmdline::Label;
use blaze_cmdline::package_identifier::PackageIdentifier;
use blaze_sky_analysis::toolchains::{
    RegisteredExecutionPlatformsKey, RegisteredExecutionPlatformsValue,
};
use blaze_skyframe::EvaluationResult;

use crate::build_view::BuildViewError;
use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_contains_at_least, assert_contains_at_least_in_order,
    assert_error_is_toolchain_exception, assert_has_error, assert_has_error_entry,
    assert_has_no_error, error_message, filter_label, get_value, label, main_repo_package,
};

// Java: RegisteredExecutionPlatformsFunctionTest.requestExecutionPlatformsFromSkyframe
fn request_execution_platforms_from_skyframe(
    t: &mut ToolchainTestCase,
    execution_platforms_key: &RegisteredExecutionPlatformsKey,
) -> EvaluationResult {
    t.evaluate_key(execution_platforms_key)
}

/// `assertExecutionPlatformLabels(value, packageRoot)`: the labels the Truth subject is about.
// Java: RegisteredExecutionPlatformsFunctionTest.assertExecutionPlatformLabels
fn execution_platform_labels(
    registered_execution_platforms_value: &RegisteredExecutionPlatformsValue,
    package_root: Option<&PackageIdentifier>,
) -> Vec<Label> {
    collect_execution_platform_labels(
        &registered_execution_platforms_value.registered_execution_platform_keys,
        package_root,
    )
}

// Java: RegisteredExecutionPlatformsFunctionTest.collectExecutionPlatformLabels
fn collect_execution_platform_labels(
    execution_platform_keys: &[ConfiguredTargetKey],
    package_root: Option<&PackageIdentifier>,
) -> Vec<Label> {
    execution_platform_keys
        .iter()
        .map(|k| k.get_label().clone())
        .filter(|l| filter_label(package_root, l))
        .collect()
}

fn key(t: &mut ToolchainTestCase, debug: bool) -> RegisteredExecutionPlatformsKey {
    RegisteredExecutionPlatformsKey::key(t.target_config_key(), debug)
}

const TWO_EXECUTION_PLATFORMS: &str = r#"platform(name = "execution_platform_1")

platform(name = "execution_platform_2")
"#;

fn platforms_1_and_2() -> [Label; 2] {
    [
        label("//extra:execution_platform_1"),
        label("//extra:execution_platform_2"),
    ]
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms
#[test]
fn test_registered_execution_platforms() {
    let mut t = ToolchainTestCase::new();
    // Request the executionPlatforms.
    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    let value = get_value(&result, &execution_platforms_key);

    assert!(value.registered_execution_platform_keys.is_empty());
    assert!(value.rejected_platforms.is_none());
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_flagOverride
#[test]
fn test_registered_execution_platforms_flag_override() {
    let mut t = ToolchainTestCase::new();

    // Add an extra execution platform.
    t.scratch.file("extra/BUILD", &[TWO_EXECUTION_PLATFORMS]);

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//extra:execution_platform_2\")\n",
    ]);
    t.use_configuration(&["--extra_execution_platforms=//extra:execution_platform_1"]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_execution_platforms flag is first in the
    // list.
    assert_contains_at_least_in_order(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &platforms_1_and_2(),
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_flagOverride_multiple
#[test]
fn test_registered_execution_platforms_flag_override_multiple() {
    let mut t = ToolchainTestCase::new();

    // Add an extra execution platform.
    t.scratch.file("extra/BUILD", &[TWO_EXECUTION_PLATFORMS]);

    t.use_configuration(&[
        "--extra_execution_platforms=//extra:execution_platform_1,//extra:execution_platform_2",
    ]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_execution_platforms flag is first in the
    // list.
    assert_contains_at_least_in_order(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &platforms_1_and_2(),
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_targetPattern_workspace
#[test]
fn test_registered_execution_platforms_target_pattern_workspace() {
    let mut t = ToolchainTestCase::new();

    // Add an extra execution platform.
    t.scratch.file("extra/BUILD", &[TWO_EXECUTION_PLATFORMS]);

    t.rewrite_module_dot_bazel(&["register_execution_platforms(\"//extra/...\")\n"]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_execution_platforms flag is first in the
    // list.
    assert_contains_at_least_in_order(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &platforms_1_and_2(),
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_aliased
#[test]
fn test_registered_execution_platforms_aliased() {
    let mut t = ToolchainTestCase::new();
    // Add an extra execution platform.
    t.scratch.file("extra/BUILD", &[TWO_EXECUTION_PLATFORMS]);
    t.scratch.file(
        "alias/BUILD",
        &[
            r#"alias(name = "alias_platform_1", actual = "//extra:execution_platform_1");

alias(name = "alias_platform_2", actual = "//extra:execution_platform_2");
"#,
        ],
    );

    t.rewrite_module_dot_bazel(&["register_execution_platforms(\"//alias/...\")\n"]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // Verify that aliases were resolved to actual targets.
    assert_contains_at_least_in_order(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &platforms_1_and_2(),
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_targetPattern_mixed
#[test]
fn test_registered_execution_platforms_target_pattern_mixed() {
    let mut t = ToolchainTestCase::new();

    // Add several targets, some of which are not actually platforms.
    t.scratch.file(
        "extra/BUILD",
        &[r#"platform(name = "execution_platform_1")

platform(name = "execution_platform_2")

filegroup(name = "not_an_execution_platform")
"#],
    );

    t.rewrite_module_dot_bazel(&["register_execution_platforms(\"//extra:all\")\n"]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // There should only be two execution platforms registered from //extra.
    // Verify that the target registered with the extra_execution_platforms flag is first in the
    // list.
    assert_eq!(
        execution_platform_labels(
            &get_value(&result, &execution_platforms_key),
            Some(&main_repo_package("extra")),
        ),
        platforms_1_and_2().to_vec()
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_targetPattern_flagOverride
#[test]
fn test_registered_execution_platforms_target_pattern_flag_override() {
    let mut t = ToolchainTestCase::new();

    // Add an extra execution platform.
    t.scratch.file("extra/BUILD", &[TWO_EXECUTION_PLATFORMS]);

    t.use_configuration(&["--extra_execution_platforms=//extra/..."]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);

    // Verify that the target registered with the extra_execution_platforms flag is first in the
    // list.
    assert_contains_at_least_in_order(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &platforms_1_and_2(),
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_notExecutionPlatform
#[test]
fn test_registered_execution_platforms_not_execution_platform() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//error:not_an_execution_platform\")\n",
    ]);
    // Have to use a rule that doesn't require a target platform, or else there will be a cycle.
    t.scratch.file(
        "error/BUILD",
        &["toolchain_type(name = \"not_an_execution_platform\")\n"],
    );

    // Request the executionPlatforms.
    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &execution_platforms_key,
        ToolchainExceptionKind::InvalidPlatform,
    );
    let message = error_message(&result, &execution_platforms_key);
    assert!(
        message.contains("//error:not_an_execution_platform"),
        "{message}"
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_reload
#[test]
fn test_registered_execution_platforms_reload() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .overwrite_file("platform/BUILD", &[TWO_EXECUTION_PLATFORMS]);

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//platform:execution_platform_1\")\n",
    ]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    assert_contains_at_least(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &[label("//platform:execution_platform_1")],
    );

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//platform:execution_platform_2\")\n",
    ]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    assert_contains_at_least(
        &execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        &[label("//platform:execution_platform_2")],
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_bzlmod
#[test]
fn test_registered_execution_platforms_bzlmod() {
    let mut t = ToolchainTestCase::new();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[r#"register_execution_platforms("//:plat")
register_execution_platforms("//:dev_plat", dev_dependency = True)
bazel_dep(name = "bbb", version = "1.0")
bazel_dep(name = "ccc", version = "1.1")
"#],
    );
    t.add_registry_module(
        "bbb",
        "1.0",
        &[r#"module(name = "bbb", version = "1.0")
register_execution_platforms("//:plat")
register_execution_platforms("//:dev_plat", dev_dependency = True)
bazel_dep(name = "ddd", version = "1.0")
"#],
    );
    t.add_registry_module(
        "ccc",
        "1.1",
        &[r#"module(name = "ccc", version = "1.1")
register_execution_platforms("//:plat")
register_execution_platforms("//:dev_plat", dev_dependency = True)
bazel_dep(name = "ddd", version = "1.1")
"#],
    );
    // ddd@1.0 is not selected
    t.add_registry_module(
        "ddd",
        "1.0",
        &[r#"module(name = "ddd", version = "1.0")
register_execution_platforms('//:plat')
"#],
    );
    t.add_registry_module(
        "ddd",
        "1.1",
        &[r#"module(name = "ddd", version = "1.1")
register_execution_platforms("@eee//:plat", "//:plat")
bazel_dep(name = "eee", version = "1.0")
"#],
    );
    t.add_registry_module(
        "eee",
        "1.0",
        &["module(name = \"eee\", version = \"1.0\")\n"],
    );
    let module_root = t.module_root();
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
            &["platform(name = \"plat\")\n"],
        );
    }
    t.scratch.overwrite_file(
        "BUILD",
        &["platform(name = \"plat\")\nplatform(name = \"dev_plat\")\n"],
    );
    t.invalidate_packages();

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    if result.has_error() {
        panic!(
            "{:?}",
            result
                .get_first_error()
                .and_then(|e| e.get_exception().cloned())
        );
    }
    assert_has_no_error(&result);

    // Verify that the execution platforms registered with bzlmod come in the BFS order
    assert_eq!(
        execution_platform_labels(&get_value(&result, &execution_platforms_key), None),
        vec![
            // Root module platforms
            label("//:plat"),
            label("//:dev_plat"),
            // Other modules' toolchains
            label("@@bbb+//:plat"),
            label("@@ccc+//:plat"),
            label("@@eee+//:plat"),
            label("@@ddd+//:plat"),
        ]
    );
}

// Regression test for https://github.com/bazelbuild/bazel/issues/10101.
// Java: RegisteredExecutionPlatformsFunctionTest.testInvalidExecutionPlatformLabelDoesntCrash
#[test]
fn test_invalid_execution_platform_label_doesnt_crash() {
    let mut t = ToolchainTestCase::new();
    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//test:bad_exec_platform_label\")\n",
    ]);
    t.scratch.file(
        "test/BUILD",
        &[r#"genrule(
    name = "g",
    srcs = [],
    outs = ["g.out"],
    cmd = "echo hi > $@",
)
"#],
    );
    t.remove_fail_fast_handler();
    // Java: assertThrows("invalid registered execution platform '//test:bad_exec_platform_label': "
    // + "no such target '//test:bad_exec_platform_label'", ViewCreationFailedException.class, ...)
    // (the string is the assertion's failure message).
    let result = t.update_targets(
        &["//test:g"],
        &[],
        /* keep_going= */ false,
        /* loading_phase_threads= */ 1,
    );
    assert!(
        matches!(result, Err(BuildViewError::ViewCreationFailed { .. })),
        "invalid registered execution platform '//test:bad_exec_platform_label': \
         no such target '//test:bad_exec_platform_label'"
    );
}

const REQUIRED_SETTINGS_OPTIMIZED: &str = r#"config_setting(
    name = "optimized",
    values = {
       "compilation_mode": "opt",
    },
)

platform(
    name = "required_platform",
    required_settings = [
        ":optimized",
    ],
)

platform(name = "always_platform")
"#;

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_requiredSettings_enabled
#[test]
fn test_registered_execution_platforms_required_settings_enabled() {
    let mut t = ToolchainTestCase::new();
    // Add an extra platform with a required_setting
    t.scratch
        .file("extra/BUILD", &[REQUIRED_SETTINGS_OPTIMIZED]);

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//extra:required_platform\", \"//extra:always_platform\")\n",
    ]);

    t.use_configuration(&["--compilation_mode=opt"]);
    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    let value = get_value(&result, &execution_platforms_key);

    // Both platforms should be present because the required settings match.
    assert_contains_at_least(
        &execution_platform_labels(&value, None),
        &[
            label("//extra:required_platform"),
            label("//extra:always_platform"),
        ],
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_requiredSettings_disabled
#[test]
fn test_registered_execution_platforms_required_settings_disabled() {
    let mut t = ToolchainTestCase::new();
    // Add an extra platform with a required_setting
    t.scratch
        .file("extra/BUILD", &[REQUIRED_SETTINGS_OPTIMIZED]);

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//extra:required_platform\", \"//extra:always_platform\")\n",
    ]);

    t.use_configuration(&["--compilation_mode=dbg"]);
    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    let value = get_value(&result, &execution_platforms_key);

    // The platform with required settings should not be present.
    let labels = execution_platform_labels(&value, None);
    assert_contains_at_least(&labels, &[label("//extra:always_platform")]);
    assert!(
        !labels.contains(&label("//extra:required_platform")),
        "{labels:?}"
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_requiredSettings_debug
#[test]
fn test_registered_execution_platforms_required_settings_debug() {
    let mut t = ToolchainTestCase::new();
    // Add an extra platform with a required_setting
    t.scratch
        .file("extra/BUILD", &[REQUIRED_SETTINGS_OPTIMIZED]);

    t.rewrite_module_dot_bazel(&[
        "register_execution_platforms(\"//extra:required_platform\", \"//extra:always_platform\")\n",
    ]);

    t.use_configuration(&["--compilation_mode=dbg"]);
    let execution_platforms_key = key(&mut t, /* debug= */ true);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);
    assert_has_no_error(&result);
    let value = get_value(&result, &execution_platforms_key);

    // Verify that the message about the unmatched config_setting is present.
    let rejected_platforms = value
        .rejected_platforms
        .as_ref()
        .expect("rejectedPlatforms() is not null");
    assert_eq!(
        rejected_platforms
            .get(&label("//extra:required_platform"))
            .map(String::as_str),
        Some("mismatching required_settings: optimized")
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_requiredSettings_config_error
#[test]
#[ignore = "BLAZE GAP: unported FeatureFlagValue.trimFlagValues (blaze_analysis \
            config/transitions.rs ConfigFeatureFlagTaggedTrimming panics under \
            --enforce_transitive_configs_for_config_feature_flag)"]
fn test_registered_execution_platforms_required_settings_config_error() {
    let mut t = ToolchainTestCase::new();
    // Add an extra platform with a required_setting
    t.scratch.file(
        "extra/BUILD",
        &[r#"config_setting(
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

platform(
    name = "required_platform",
    required_settings = [
        ":flagged",
    ],
)
"#],
    );

    t.rewrite_module_dot_bazel(&["register_execution_platforms(\"//extra:required_platform\")\n"]);

    // Need this so the feature flag is actually gone from the configuration.
    t.use_configuration(&["--enforce_transitive_configs_for_config_feature_flag"]);
    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);

    assert_has_error(&result);
    assert_has_error_entry(&result, &execution_platforms_key);
    let message = error_message(&result, &execution_platforms_key);
    assert!(
        message.contains(
            "Unrecoverable errors resolving config_setting associated with \
             //extra:required_platform: For config_setting flagged: Feature flag \
             //extra:flag was accessed in a configuration it is not present in."
        ),
        "{message}"
    );
}

// Java: RegisteredExecutionPlatformsFunctionTest.testRegisteredExecutionPlatforms_requiredSettings_cantDependOnConstraintValues_error
#[test]
fn test_registered_execution_platforms_required_settings_cant_depend_on_constraint_values_error() {
    let mut t = ToolchainTestCase::new();
    // Add an extra platform with a required_setting
    t.scratch.file(
        "extra/BUILD",
        &[r#"constraint_setting(name = "cs1")
constraint_value(name = "cv1", constraint_setting = ":cs1")
constraint_value(name = "cv2", constraint_setting = ":cs1")
config_setting(
    name = "setting",
    constraint_values = [":cv1"],
)

platform(
    name = "required_platform",
    required_settings = [
        ":setting",
        ":cv2",
    ],
)
"#],
    );

    t.rewrite_module_dot_bazel(&["register_execution_platforms(\"//extra:required_platform\")\n"]);

    let execution_platforms_key = key(&mut t, /* debug= */ false);
    let result = request_execution_platforms_from_skyframe(&mut t, &execution_platforms_key);

    assert_has_error(&result);
    assert_has_error_entry(&result, &execution_platforms_key);
}
