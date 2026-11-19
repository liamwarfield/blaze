//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `PlatformLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_PLATFORM_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls `PlatformLookupUtil.getPlatformInfo`; here
//! it is registered with `BuildViewTestCase::set_extra_sky_functions`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::PlatformInfo;
use blaze_analysis::toolchains::ToolchainExceptionKind;
use blaze_sky_analysis::toolchains::platform_lookup_util;
use blaze_skyframe::{EvaluationResult, Key, SkyFunctionName};

use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_error_is_toolchain_exception, assert_has_error, assert_has_no_error,
    error_message, get_value, label,
};

/// `ConfiguredTargetKey.builder().setLabel(label).setConfigurationKey(targetConfigKey).build()`.
fn ct_key(t: &mut ToolchainTestCase, label_string: &str) -> ConfiguredTargetKey {
    let config = t.get_target_configuration();
    ConfiguredTargetKey::builder()
        .set_label(label(label_string))
        .set_configuration(Some(config.as_ref()))
        .build()
}

/// Java `GetPlatformInfoKey`.
// Java: PlatformLookupUtilTest.GetPlatformInfoKey
#[derive(Clone, PartialEq, Eq, Hash)]
struct GetPlatformInfoKey {
    platform_keys: Vec<ConfiguredTargetKey>,
}

impl fmt::Debug for GetPlatformInfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GetPlatformInfoKey[platformKeys={:?}]",
            self.platform_keys
        )
    }
}

impl Key for GetPlatformInfoKey {
    type Value = GetPlatformInfoValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("GET_PLATFORM_INFO_FUNCTION");
}

/// Java `GetPlatformInfoValue`.
// Java: PlatformLookupUtilTest.GetPlatformInfoValue
#[derive(Debug)]
struct GetPlatformInfoValue {
    platforms: HashMap<ConfiguredTargetKey, Arc<PlatformInfo>>,
}

/// The test case with `GetPlatformInfoFunction` registered.
fn new_test() -> ToolchainTestCase {
    let mut t = ToolchainTestCase::new();
    t.set_extra_sky_functions(|builder| {
        // Java: PlatformLookupUtilTest.GetPlatformInfoFunction.compute
        builder.typed_function::<GetPlatformInfoKey, _>(|key, env| {
            let platforms = match platform_lookup_util::get_platform_info(&key.platform_keys, env) {
                Ok(v) => v,
                Err(e) => return Err(e.into_compute_error()),
            };
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(GetPlatformInfoValue {
                platforms: platforms.expect("values present"),
            }))
        })
    });
    t
}

// Java: PlatformLookupUtilTest.getPlatformInfo
fn get_platform_info(t: &mut ToolchainTestCase, key: &GetPlatformInfoKey) -> EvaluationResult {
    t.evaluate_key(key)
}

// Java: PlatformLookupUtilTest.testPlatformLookup
#[test]
fn test_platform_lookup() {
    let mut t = new_test();
    let linux_key = ct_key(&mut t, "//platforms:linux");
    let mac_key = ct_key(&mut t, "//platforms:mac");
    let key = GetPlatformInfoKey {
        platform_keys: vec![linux_key.clone(), mac_key.clone()],
    };
    let result = get_platform_info(&mut t, &key);

    assert_has_no_error(&result);
    let platforms = &get_value(&result, &key).platforms;
    assert!(platforms.contains_key(&linux_key));
    assert_eq!(platforms[&linux_key].label(), t.linux_platform.label());
    assert!(platforms.contains_key(&mac_key));
    assert_eq!(platforms[&mac_key].label(), t.mac_platform.label());
    assert_eq!(platforms.len(), 2);
}

// Java: PlatformLookupUtilTest.testPlatformLookup_targetNotPlatform
#[test]
fn test_platform_lookup_target_not_platform() {
    let mut t = new_test();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_platform')"]);

    let target_key = ct_key(&mut t, "//invalid:not_a_platform");
    let key = GetPlatformInfoKey {
        platform_keys: vec![target_key],
    };
    let result = get_platform_info(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    assert!(error_message(&result, &key).contains("//invalid:not_a_platform"));
}

// Java: PlatformLookupUtilTest.testPlatformLookup_targetDoesNotExist
#[test]
fn test_platform_lookup_target_does_not_exist() {
    let mut t = new_test();
    let target_key = ct_key(&mut t, "//fake:missing");
    let key = GetPlatformInfoKey {
        platform_keys: vec![target_key],
    };
    let result = get_platform_info(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(&result, &key, ToolchainExceptionKind::InvalidPlatform);
    assert!(error_message(&result, &key).contains("no such package 'fake': BUILD file not found"));
}
