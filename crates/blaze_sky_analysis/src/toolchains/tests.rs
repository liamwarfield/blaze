//! Ports of the `skyframe/toolchains` JUnit tests.

use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::{DeclaredToolchainInfo, ToolchainTypeInfo};
use blaze_cmdline::Label;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_skyframe::SkyError;
use indexmap::IndexMap;

use crate::producers::errors::{
    ConfiguredTargetFunctionException, ConfiguredValueCreationException,
    InconsistentNullConfigException,
};
use crate::toolchains::lookup_error::{
    ConfiguredTargetLookupError, catch_configured_target_error,
    catch_configured_value_creation_exception,
};
use crate::toolchains::{RegisteredToolchainsValue, SingleToolchainResolutionValue};

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

// No JUnit test of its own. Java: ConfiguredTargetFunction throws ReportedException /
// UnreportedException / DependencyException, whose causes are what a parent's
// `getOrThrow(key, ConfiguredValueCreationException.class, NoSuchThingException.class,
// ActionConflictException.class)` catches (PlatformLookupUtil.findPlatformInfo and the other
// lookup helpers); an InconsistentNullConfigException is none of them, so the dep is missing.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetEvaluationExceptions.java:28 ConfiguredTargetEvaluationExceptions
#[test]
fn configured_target_function_exceptions_are_caught_by_their_cause() {
    let cvce =
        ConfiguredValueCreationException::new(None, "boom", &label("//a:b"), None, None, None);
    for wrapped in [
        ConfiguredTargetFunctionException::Reported(cvce.clone()),
        ConfiguredTargetFunctionException::Unreported(cvce.clone()),
    ] {
        let error: SkyError = Arc::new(wrapped);
        let caught = catch_configured_value_creation_exception(&error)
            .expect("ConfiguredValueCreationException is caught");
        // The cause handed on is the ConfiguredValueCreationException itself.
        assert_eq!(
            caught
                .downcast_ref::<ConfiguredValueCreationException>()
                .map(|c| c.get_message().to_owned()),
            Some("boom".to_owned())
        );
        match catch_configured_target_error(&error) {
            Some(ConfiguredTargetLookupError::ConfiguredValueCreation(cause)) => assert!(
                cause
                    .downcast_ref::<ConfiguredValueCreationException>()
                    .is_some()
            ),
            other => panic!("unexpected {other:?}"),
        }
    }

    let no_such_thing: SkyError = Arc::new(ConfiguredTargetFunctionException::NoSuchThing(
        NoSuchThingException::Other {
            message: "no such thing".to_owned(),
            detailed_exit_code: None,
        },
    ));
    assert!(catch_configured_value_creation_exception(&no_such_thing).is_none());
    assert!(matches!(
        catch_configured_target_error(&no_such_thing),
        Some(ConfiguredTargetLookupError::NoSuchThing(_))
    ));

    let action_conflict: SkyError = Arc::new(ConfiguredTargetFunctionException::ActionConflict(
        "conflict".to_owned(),
    ));
    assert!(catch_configured_value_creation_exception(&action_conflict).is_none());
    assert!(matches!(
        catch_configured_target_error(&action_conflict),
        Some(ConfiguredTargetLookupError::ActionConflict(_))
    ));

    let inconsistent_null_config: SkyError = Arc::new(
        ConfiguredTargetFunctionException::InconsistentNullConfig(InconsistentNullConfigException),
    );
    assert!(catch_configured_value_creation_exception(&inconsistent_null_config).is_none());
    assert!(catch_configured_target_error(&inconsistent_null_config).is_none());
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchainsValue_equalsAndHashCode
#[test]
fn test_registered_toolchains_value_equals_and_hash_code() {
    let toolchain = |impl_: &str, target: &str| {
        DeclaredToolchainInfo::builder()
            .toolchain_type(Some(ToolchainTypeInfo::create(label("//test:toolchain"))))
            .add_exec_constraints([])
            .add_target_constraints([])
            .resolved_toolchain_label(label(impl_))
            .target_label(label(target))
            .build()
            .unwrap()
    };
    let toolchain1 = toolchain("//test/toolchain_impl_1", "//test/toolchain_1");
    let toolchain2 = toolchain("//test/toolchain_impl_2", "//test/toolchain_2");
    let value = |v: Vec<DeclaredToolchainInfo>| RegisteredToolchainsValue {
        registered_toolchains: v,
        rejected_toolchains: None,
    };
    let a = value(vec![toolchain1.clone(), toolchain2.clone()]);
    let b = value(vec![toolchain1.clone(), toolchain2.clone()]);
    let groups = [
        value(vec![toolchain1.clone()]),
        value(vec![toolchain2.clone()]),
        value(vec![toolchain2, toolchain1]),
    ];
    assert_eq!(a, b);
    for (i, g) in groups.iter().enumerate() {
        assert_ne!(&a, g);
        for (j, h) in groups.iter().enumerate() {
            assert_eq!(i == j, g == h);
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java testToolchainResolutionValue_equalsAndHashCode
#[test]
fn test_toolchain_resolution_value_equals_and_hash_code() {
    let test_toolchain_type_info = ToolchainTypeInfo::create(label("//test:toolchain"));
    let linux_ctkey = ConfiguredTargetKey::builder()
        .set_label(label("//platforms:linux"))
        .build();
    let mac_ctkey = ConfiguredTargetKey::builder()
        .set_label(label("//platforms:mac"))
        .build();
    let value = |entries: &[(&ConfiguredTargetKey, &str)]| {
        let mut m = IndexMap::new();
        for (k, l) in entries {
            m.insert((*k).clone(), label(l));
        }
        SingleToolchainResolutionValue::create(test_toolchain_type_info.clone(), m)
    };
    let a = value(&[(&linux_ctkey, "//test:toolchain_impl_1")]);
    let b = value(&[(&linux_ctkey, "//test:toolchain_impl_1")]);
    let groups = [
        // Different execution platform, same label.
        value(&[(&mac_ctkey, "//test:toolchain_impl_1")]),
        // Same execution platform, different label.
        value(&[(&linux_ctkey, "//test:toolchain_impl_2")]),
        // Different execution platform, different label.
        value(&[(&mac_ctkey, "//test:toolchain_impl_2")]),
        // Multiple execution platforms.
        value(&[
            (&linux_ctkey, "//test:toolchain_impl_1"),
            (&mac_ctkey, "//test:toolchain_impl_1"),
        ]),
    ];
    assert_eq!(a, b);
    for (i, g) in groups.iter().enumerate() {
        assert_ne!(&a, g);
        for (j, h) in groups.iter().enumerate() {
            assert_eq!(i == j, g == h);
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatformsValue_equalsAndHashCode
#[test]
fn test_registered_execution_platforms_value_equals_and_hash_code() {
    use crate::toolchains::RegisteredExecutionPlatformsValue;
    let p1 = ConfiguredTargetKey::builder()
        .set_label(label("//test:executionPlatform1"))
        .build();
    let p2 = ConfiguredTargetKey::builder()
        .set_label(label("//test:executionPlatform2"))
        .build();
    let value = |v: Vec<ConfiguredTargetKey>| RegisteredExecutionPlatformsValue {
        registered_execution_platform_keys: v,
        rejected_platforms: None,
    };
    let a = value(vec![p1.clone(), p2.clone()]);
    let b = value(vec![p1.clone(), p2.clone()]);
    let groups = [
        value(vec![p1.clone()]),
        value(vec![p2.clone()]),
        value(vec![p2, p1]),
    ];
    assert_eq!(a, b);
    for (i, g) in groups.iter().enumerate() {
        assert_ne!(&a, g);
        for (j, h) in groups.iter().enumerate() {
            assert_eq!(i == j, g == h);
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtilTest.java testConstraintValueLookup
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_value_lookup() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtilTest.java testConstraintValueLookup_targetNotConstraintValue
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_value_lookup_target_not_constraint_value() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtilTest.java testConstraintValueLookup_targetDoesNotExist
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_constraint_value_lookup_target_does_not_exist() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtilTest.java testPlatformLookup
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_platform_lookup() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtilTest.java testPlatformLookup_targetNotPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_platform_lookup_target_not_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtilTest.java testPlatformLookup_targetDoesNotExist
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_platform_lookup_target_does_not_exist() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_flagOverride
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_flag_override() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_flagOverride_multiple
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_flag_override_multiple() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_targetPattern_workspace
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_target_pattern_workspace() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_aliased
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_aliased() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_targetPattern_mixed
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_target_pattern_mixed() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_targetPattern_flagOverride
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_target_pattern_flag_override() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_notExecutionPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_not_execution_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_reload
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_reload() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_bzlmod
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_bzlmod() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testInvalidExecutionPlatformLabelDoesntCrash
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_invalid_execution_platform_label_doesnt_crash() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_requiredSettings_enabled
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_required_settings_enabled() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_requiredSettings_disabled
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_required_settings_disabled() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_requiredSettings_debug
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_required_settings_debug() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_requiredSettings_config_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_required_settings_config_error() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunctionTest.java testRegisteredExecutionPlatforms_requiredSettings_cantDependOnConstraintValues_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_execution_platforms_required_settings_cant_depend_on_constraint_values_error() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_flagOverride
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_flag_override() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_flagOverride_multiple
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_flag_override_multiple() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_notToolchain
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_not_toolchain() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_fakeToolchain
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_fake_toolchain() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_wildcard_fakeToolchain
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_wildcard_fake_toolchain() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetPattern_workspace
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_pattern_workspace() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetPattern_flagOverride
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_pattern_flag_override() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetPattern_order
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_pattern_order() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_reload
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_reload() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_bzlmod
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_bzlmod() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetSetting
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_setting() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetSetting_debug
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_setting_debug() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunctionTest.java testRegisteredToolchains_targetSetting_error
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_registered_toolchains_target_setting_error() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java testResolution_singleExecutionPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_resolution_single_execution_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java testResolution_multipleExecutionPlatforms
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_resolution_multiple_execution_platforms() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java testResolution_noneFound
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_resolution_none_found() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunctionTest.java testResolution_checkPlatformAllowedToolchains
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_resolution_check_platform_allowed_toolchains() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_useTargetPlatformConstraints_inheritedFromParentPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_use_target_platform_constraints_inherited_from_parent_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_hostPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_host_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_hostPlatform_alias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_host_platform_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_optional
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_optional() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_optional_on_first_platform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_optional_on_first_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_optional_on_second_platform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_optional_on_second_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_max_optional_on_second_platform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_max_optional_on_second_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_multiple
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_multiple() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_mandatory_missing
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_mandatory_missing() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java unresolved_toolchain_message_regex_quotes
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn unresolved_toolchain_message_regex_quotes() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_mandatory_missing_customPlatformMessage
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_mandatory_missing_custom_platform_message() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_multiple_optional
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_multiple_optional() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_multiple_optional_missing
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_multiple_optional_missing() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_toolchainTypeAlias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_toolchain_type_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_noToolchainType
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_no_toolchain_type() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_noToolchainType_hostNotAvailable
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_no_toolchain_type_host_not_available() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_unavailableToolchainType_single
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_unavailable_toolchain_type_single() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_optional_unavailableToolchainType_single
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_optional_unavailable_toolchain_type_single() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_unavailableToolchainType_multiple
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_unavailable_toolchain_type_multiple() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidToolchainType
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_toolchain_type() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidToolchainType_ignored
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_toolchain_type_ignored() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidTargetPlatform_badTarget
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_target_platform_bad_target() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidTargetPlatform_badPackage
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_target_platform_bad_package() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_executionPlatform_alias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_execution_platform_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidHostPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_host_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_invalidExecutionPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_invalid_execution_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_execConstraints
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_exec_constraints() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_execConstraints_invalid
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_exec_constraints_invalid() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_noMatchingPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_no_matching_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_forceExecutionPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_force_execution_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_forceExecutionPlatform_alias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_force_execution_platform_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_forceExecutionPlatform_host
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_force_execution_platform_host() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_forceExecutionPlatform_host_alias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_force_execution_platform_host_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_forceExecutionPlatform_noRequiredToolchains
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_force_execution_platform_no_required_toolchains() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java errorProperlyReportedWhenInvalidConfigurationConfiguration
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn error_properly_reported_when_invalid_configuration_configuration() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_checkPlatformAllowedToolchains_match
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_check_platform_allowed_toolchains_match() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_checkPlatformAllowedToolchains_failsMatch
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_check_platform_allowed_toolchains_fails_match() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_checkPlatformAllowedToolchains_noneRequested_failsMatch
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_check_platform_allowed_toolchains_none_requested_fails_match() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_checkPlatformAllowedToolchains_noToolchainType_noneRequested
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_check_platform_allowed_toolchains_no_toolchain_type_none_requested() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunctionTest.java resolve_checkPlatformAllowedToolchains_noToolchainType
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn resolve_check_platform_allowed_toolchains_no_toolchain_type() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java basicToolchains
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn basic_toolchains() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java basicToolchainsWithAliasAutoExecGroups
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn basic_toolchains_with_alias_auto_exec_groups() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java basicToolchainsWithAliasNoAutoExecGroups
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn basic_toolchains_with_alias_no_auto_exec_groups() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java basicToolchainsWithAliasNoAutoExecGroups_test
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn basic_toolchains_with_alias_no_auto_exec_groups_test() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java execPlatform
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_platform() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java execPlatform_withExecConstraint
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_platform_with_exec_constraint() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java execGroups_named
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_groups_named() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java execGroups_defaultAndNamed
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn exec_groups_default_and_named() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java keepParentToolchainContext
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn keep_parent_toolchain_context() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java toolchainWithDifferentExecutionPlatforms_doesNotGenerateConflictingCoverageAction
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn toolchain_with_different_execution_platforms_does_not_generate_conflicting_coverage_action() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java targetCompatibleWith_matchesExecCompatibleWith
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn target_compatible_with_matches_exec_compatible_with() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java targetCompatibleWith_mismatchesExecCompatibleWith
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn target_compatible_with_mismatches_exec_compatible_with() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java targetCompatibleWith_mismatchesExecCompatibleInDifferentPackage
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn target_compatible_with_mismatches_exec_compatible_in_different_package() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java targetCompatibleWith_mismatchesExecCompatibleDepInDifferentPackage
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn target_compatible_with_mismatches_exec_compatible_dep_in_different_package() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainsForTargetsTest.java targetCompatibleWith_mismatchesExecCompatibleWithinAspect
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn target_compatible_with_mismatches_exec_compatible_within_aspect() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java testToolchainTypeLookup
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_type_lookup() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java testToolchainTypeLookup_toolchainAlias
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_type_lookup_toolchain_alias() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java testToolchainTypeLookup_targetNotToolchainType
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_type_lookup_target_not_toolchain_type() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java testToolchainTypeLookup_targetNotToolchainType_ignoreInvalid
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_type_lookup_target_not_toolchain_type_ignore_invalid() {}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java testToolchainTypeLookup_targetDoesNotExist
#[test]
#[ignore = "needs a BuildViewTestCase harness (analysis of BUILD files)"]
fn test_toolchain_type_lookup_target_does_not_exist() {}
