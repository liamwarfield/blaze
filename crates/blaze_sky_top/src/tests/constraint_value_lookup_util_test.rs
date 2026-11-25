//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `ConstraintValueLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_CONSTRAINT_VALUE_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls
//! `ConstraintValueLookupUtil.getConstraintValueInfo`; here it is registered with
//! `BuildViewTestCase::set_extra_sky_functions`.

use std::fmt;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::ConstraintValueInfo;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_sky_analysis::toolchains::constraint_value_lookup_util;
use blaze_skyframe::{EvaluationResult, Key, SkyFunctionName};

use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_error_is_toolchain_exception, assert_has_error, assert_has_no_error,
    error_exception, error_message, get_value, label,
};

/// `ConfiguredTargetKey.builder().setLabel(label).setConfigurationKey(targetConfigKey).build()`.
fn ct_key(t: &mut ToolchainTestCase, label_string: &str) -> ConfiguredTargetKey {
    let config = t.get_target_configuration();
    ConfiguredTargetKey::builder()
        .set_label(label(label_string))
        .set_configuration(Some(config.as_ref()))
        .build()
}

/// Java `GetConstraintValueInfoKey`.
// Java: ConstraintValueLookupUtilTest.GetConstraintValueInfoKey
#[derive(Clone, PartialEq, Eq, Hash)]
struct GetConstraintValueInfoKey {
    constraint_value_keys: Vec<ConfiguredTargetKey>,
}

impl fmt::Debug for GetConstraintValueInfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GetConstraintValueInfoKey[constraintValueKeys={:?}]",
            self.constraint_value_keys
        )
    }
}

impl Key for GetConstraintValueInfoKey {
    type Value = GetConstraintValueInfoValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("GET_CONSTRAINT_VALUE_INFO_FUNCTION");
    // Java: ConstraintValueLookupUtilTest GET_CONSTRAINT_VALUE_INFO_FUNCTION (createHermetic)
    const HERMETICITY: blaze_skyframe::FunctionHermeticity =
        blaze_skyframe::FunctionHermeticity::Hermetic;
}

/// Java `GetConstraintValueInfoValue`.
// Java: ConstraintValueLookupUtilTest.GetConstraintValueInfoValue
#[derive(Debug)]
struct GetConstraintValueInfoValue {
    constraint_values: Vec<ConstraintValueInfo>,
}

/// The test case with `GetConstraintValueInfoFunction` registered (Java
/// `AnalysisMockWithGetPlatformInfoFunction`).
fn new_test() -> ToolchainTestCase {
    let mut t = ToolchainTestCase::new();
    t.set_extra_sky_functions(|builder| {
        // Java: ConstraintValueLookupUtilTest.GetConstraintValueInfoFunction.compute
        builder.typed_function::<GetConstraintValueInfoKey, _>(|key, env| {
            let constraint_values = match constraint_value_lookup_util::get_constraint_value_info(
                &key.constraint_value_keys,
                env,
            ) {
                Ok(v) => v,
                Err(e) => return Err(e.into_compute_error()),
            };
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(GetConstraintValueInfoValue {
                constraint_values: constraint_values.expect("values present"),
            }))
        })
    });
    t
}

// Java: ConstraintValueLookupUtilTest.getConstraintValueInfo
fn get_constraint_value_info(
    t: &mut ToolchainTestCase,
    key: &GetConstraintValueInfoKey,
) -> EvaluationResult {
    t.evaluate_key(key)
}

/// `hasExceptionThat().hasCauseThat().isInstanceOf(NoSuchPackageException.class)`.
fn assert_cause_is_no_such_package(exception: &blaze_skyframe::SkyError) {
    let toolchain_exception = exception
        .downcast_ref::<ToolchainException>()
        .unwrap_or_else(|| panic!("expected a ToolchainException, got {exception:?}"));
    let cause = toolchain_exception.get_cause().expect("a cause");
    assert!(
        cause.downcast_ref::<NoSuchPackageException>().is_some()
            || matches!(
                cause.downcast_ref::<NoSuchThingException>(),
                Some(NoSuchThingException::Package(_))
            ),
        "expected a NoSuchPackageException cause, got {cause:?}"
    );
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup
#[test]
fn test_constraint_value_lookup() {
    let mut t = new_test();
    let linux_key = ct_key(&mut t, "//constraints:linux");
    let mac_key = ct_key(&mut t, "//constraints:mac");
    let key = GetConstraintValueInfoKey {
        constraint_value_keys: vec![linux_key, mac_key],
    };
    let result = get_constraint_value_info(&mut t, &key);

    assert_has_no_error(&result);
    let constraint_values = &get_value(&result, &key).constraint_values;
    assert!(constraint_values.contains(&t.linux_constraint));
    assert!(constraint_values.contains(&t.mac_constraint));
    assert_eq!(constraint_values.len(), 2);
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup_targetNotConstraintValue
#[test]
fn test_constraint_value_lookup_target_not_constraint_value() {
    let mut t = new_test();
    t.scratch
        .file("invalid/BUILD", &["filegroup(name = 'not_a_constraint')"]);

    let target_key = ct_key(&mut t, "//invalid:not_a_constraint");
    let key = GetConstraintValueInfoKey {
        constraint_value_keys: vec![target_key],
    };
    let result = get_constraint_value_info(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidConstraintValue,
    );
    assert!(error_message(&result, &key).contains("//invalid:not_a_constraint"));
}

// Java: ConstraintValueLookupUtilTest.testConstraintValueLookup_targetDoesNotExist
#[test]
fn test_constraint_value_lookup_target_does_not_exist() {
    let mut t = new_test();
    let target_key = ct_key(&mut t, "//fake:missing");
    let key = GetConstraintValueInfoKey {
        constraint_value_keys: vec![target_key],
    };

    t.remove_fail_fast_handler();
    let result = get_constraint_value_info(&mut t, &key);

    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidConstraintValue,
    );
    assert_cause_is_no_such_package(&error_exception(&result, &key));
    t.assert_contains_event("no such package 'fake': BUILD file not found");
}
