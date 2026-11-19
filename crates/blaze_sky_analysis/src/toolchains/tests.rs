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

    let action_conflict: SkyError = Arc::new(
        crate::producers::errors::action_conflict_for_testing("conflict"),
    );
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

// The harness-based tests of these classes are in blaze_sky_top/src/tests/ (one file per Java class).
