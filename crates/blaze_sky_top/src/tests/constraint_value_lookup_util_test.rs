//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `ConstraintValueLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_CONSTRAINT_VALUE_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls
//! `ConstraintValueLookupUtil.getConstraintValueInfo`; the harness cannot register extra
//! SkyFunctions in its `SkyframeExecutor`, so the tests stop at the evaluation and stay ignored.
//! The Java assertions after it are kept as comments.

use blaze_analysis::configured_target_key::ConfiguredTargetKey;

use crate::tests::toolchain_test_case::{ToolchainTestCase, label};

const NEEDS_FUNCTION: &str = "needs an extra SkyFunction in the harness's SkyframeExecutor \
                              (GET_CONSTRAINT_VALUE_INFO_FUNCTION calling \
                              ConstraintValueLookupUtil.getConstraintValueInfo)";

/// `ConfiguredTargetKey.builder().setLabel(label).setConfigurationKey(targetConfigKey).build()`.
fn ct_key(t: &mut ToolchainTestCase, label_string: &str) -> ConfiguredTargetKey {
    let config = t.get_target_configuration();
    ConfiguredTargetKey::builder()
        .set_label(label(label_string))
        .set_configuration(Some(config.as_ref()))
        .build()
}

/// Java `getConstraintValueInfo(GetConstraintValueInfoKey.create(keys))`.
// Java: ConstraintValueLookupUtilTest.getConstraintValueInfo
fn get_constraint_value_info(_t: &mut ToolchainTestCase, _keys: Vec<ConfiguredTargetKey>) -> ! {
    panic!("{NEEDS_FUNCTION}")
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_CONSTRAINT_VALUE_INFO_FUNCTION calling ConstraintValueLookupUtil.getConstraintValueInfo)"]
fn test_constraint_value_lookup() {
    let mut t = ToolchainTestCase::new();
    let linux_key = ct_key(&mut t, "//constraints:linux");
    let mac_key = ct_key(&mut t, "//constraints:mac");
    get_constraint_value_info(&mut t, vec![linux_key, mac_key]);
    // assertThatEvaluationResult(result).hasNoError();
    // assertThatEvaluationResult(result).hasEntryThat(key).isNotNull();
    // List<ConstraintValueInfo> constraintValues = result.get(key).constraintValues();
    // assertThat(constraintValues).contains(linuxConstraint);
    // assertThat(constraintValues).contains(macConstraint);
    // assertThat(constraintValues).hasSize(2);
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup_targetNotConstraintValue
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_CONSTRAINT_VALUE_INFO_FUNCTION calling ConstraintValueLookupUtil.getConstraintValueInfo)"]
fn test_constraint_value_lookup_target_not_constraint_value() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_constraint')"]);

    let target_key = ct_key(&mut t, "//invalid:not_a_constraint");
    get_constraint_value_info(&mut t, vec![target_key]);
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidConstraintValueException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasMessageThat()
    //     .contains("//invalid:not_a_constraint");
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup_targetDoesNotExist
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_CONSTRAINT_VALUE_INFO_FUNCTION calling ConstraintValueLookupUtil.getConstraintValueInfo)"]
fn test_constraint_value_lookup_target_does_not_exist() {
    let mut t = ToolchainTestCase::new();
    let target_key = ct_key(&mut t, "//fake:missing");

    t.remove_fail_fast_handler();
    get_constraint_value_info(&mut t, vec![target_key]);
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidConstraintValueException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasCauseThat()
    //     .isInstanceOf(NoSuchPackageException.class);
    // assertContainsEvent("no such package 'fake': BUILD file not found");
}
