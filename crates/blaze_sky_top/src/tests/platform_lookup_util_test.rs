//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `PlatformLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_PLATFORM_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls `PlatformLookupUtil.getPlatformInfo`; the
//! harness cannot register extra SkyFunctions in its `SkyframeExecutor`, so the tests stop at the
//! evaluation and stay ignored. The Java assertions after it are kept as comments.

use blaze_analysis::configured_target_key::ConfiguredTargetKey;

use crate::tests::toolchain_test_case::{ToolchainTestCase, label};

const NEEDS_FUNCTION: &str = "needs an extra SkyFunction in the harness's SkyframeExecutor \
                              (GET_PLATFORM_INFO_FUNCTION calling \
                              PlatformLookupUtil.getPlatformInfo)";

/// `ConfiguredTargetKey.builder().setLabel(label).setConfigurationKey(targetConfigKey).build()`.
fn ct_key(t: &mut ToolchainTestCase, label_string: &str) -> ConfiguredTargetKey {
    let config = t.get_target_configuration();
    ConfiguredTargetKey::builder()
        .set_label(label(label_string))
        .set_configuration(Some(config.as_ref()))
        .build()
}

/// Java `getPlatformInfo(GetPlatformInfoKey.create(keys))`.
// Java: PlatformLookupUtilTest.getPlatformInfo
fn get_platform_info(_t: &mut ToolchainTestCase, _keys: Vec<ConfiguredTargetKey>) -> ! {
    panic!("{NEEDS_FUNCTION}")
}

// Java: PlatformLookupUtilTest.testPlatformLookup
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_PLATFORM_INFO_FUNCTION calling PlatformLookupUtil.getPlatformInfo)"]
fn test_platform_lookup() {
    let mut t = ToolchainTestCase::new();
    let linux_key = ct_key(&mut t, "//platforms:linux");
    let mac_key = ct_key(&mut t, "//platforms:mac");
    get_platform_info(&mut t, vec![linux_key, mac_key]);
    // assertThatEvaluationResult(result).hasNoError();
    // assertThatEvaluationResult(result).hasEntryThat(key).isNotNull();
    // Map<ConfiguredTargetKey, PlatformInfo> platforms = result.get(key).platforms();
    // assertThat(platforms).containsKey(linuxKey);
    // assertThat(platforms.get(linuxKey).label()).isEqualTo(linuxPlatform.label());
    // assertThat(platforms).containsKey(macKey);
    // assertThat(platforms.get(macKey).label()).isEqualTo(macPlatform.label());
    // assertThat(platforms).hasSize(2);
}

// Java: PlatformLookupUtilTest.testPlatformLookup_targetNotPlatform
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_PLATFORM_INFO_FUNCTION calling PlatformLookupUtil.getPlatformInfo)"]
fn test_platform_lookup_target_not_platform() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_platform')"]);

    let target_key = ct_key(&mut t, "//invalid:not_a_platform");
    get_platform_info(&mut t, vec![target_key]);
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidPlatformException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasMessageThat()
    //     .contains("//invalid:not_a_platform");
}

// Java: PlatformLookupUtilTest.testPlatformLookup_targetDoesNotExist
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_PLATFORM_INFO_FUNCTION calling PlatformLookupUtil.getPlatformInfo)"]
fn test_platform_lookup_target_does_not_exist() {
    let mut t = ToolchainTestCase::new();
    let target_key = ct_key(&mut t, "//fake:missing");
    get_platform_info(&mut t, vec![target_key]);
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidPlatformException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasMessageThat()
    //     .contains("no such package 'fake': BUILD file not found");
}
