//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `ToolchainTypeLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_TOOLCHAIN_TYPE_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls
//! `ToolchainTypeLookupUtil.resolveToolchainTypes`; here it is registered with
//! `BuildViewTestCase::set_extra_sky_functions`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_analysis::platform::ToolchainTypeInfo;
use blaze_analysis::toolchains::{
    ToolchainException, ToolchainExceptionKind, ToolchainTypeRequirement,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_packages::exceptions::{NoSuchPackageException, NoSuchThingException};
use blaze_sky_analysis::toolchains::toolchain_type_lookup_util;
use blaze_skyframe::{EvaluationResult, Key, SkyFunctionName};
use indexmap::IndexMap;

use crate::tests::toolchain_test_case::{
    ToolchainTestCase, assert_error_is_toolchain_exception, assert_has_error, assert_has_no_error,
    error_exception, error_message, get_value, label,
};

/// Java `GetToolchainTypeInfoKey(BuildConfigurationValue configuration,
/// Iterable<ToolchainTypeRequirement> toolchainTypes)`.
// Java: ToolchainTypeLookupUtilTest.GetToolchainTypeInfoKey
#[derive(Clone)]
struct GetToolchainTypeInfoKey {
    configuration: Arc<BuildConfigurationValue>,
    toolchain_types: Vec<ToolchainTypeRequirement>,
}

impl PartialEq for GetToolchainTypeInfoKey {
    fn eq(&self, other: &Self) -> bool {
        *self.configuration == *other.configuration && self.toolchain_types == other.toolchain_types
    }
}

impl Eq for GetToolchainTypeInfoKey {}

impl Hash for GetToolchainTypeInfoKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.configuration.checksum().hash(state);
        self.toolchain_types.hash(state);
    }
}

impl fmt::Debug for GetToolchainTypeInfoKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GetToolchainTypeInfoKey[configuration={:?}, toolchainTypes={:?}]",
            self.configuration, self.toolchain_types
        )
    }
}

impl Key for GetToolchainTypeInfoKey {
    type Value = GetToolchainTypeInfoValue;
    const FUNCTION: SkyFunctionName = SkyFunctionName("GET_TOOLCHAIN_TYPE_INFO_FUNCTION");
    // Java: ToolchainTypeLookupUtilTest GET_TOOLCHAIN_TYPE_INFO_FUNCTION (createHermetic)
    const HERMETICITY: blaze_skyframe::FunctionHermeticity =
        blaze_skyframe::FunctionHermeticity::Hermetic;
}

/// Java `GetToolchainTypeInfoValue`.
// Java: ToolchainTypeLookupUtilTest.GetToolchainTypeInfoValue
#[derive(Debug)]
struct GetToolchainTypeInfoValue {
    toolchain_types: IndexMap<Label, ToolchainTypeInfo>,
}

/// The test case with `GetToolchainTypeInfoFunction` registered.
fn new_test() -> ToolchainTestCase {
    let mut t = ToolchainTestCase::new();
    t.set_extra_sky_functions(|builder| {
        // Java: ToolchainTypeLookupUtilTest.GetToolchainTypeInfoFunction.compute
        builder.typed_function::<GetToolchainTypeInfoKey, _>(|key, env| {
            let toolchain_types = match toolchain_type_lookup_util::resolve_toolchain_types(
                env,
                &key.toolchain_types,
                &key.configuration,
            ) {
                Ok(v) => v,
                Err(e) => return Err(e.into_compute_error()),
            };
            if env.values_missing() {
                return Ok(None);
            }
            Ok(Some(GetToolchainTypeInfoValue {
                toolchain_types: toolchain_types.expect("values present"),
            }))
        })
    });
    t
}

/// `getToolchainTypeInfo(GetToolchainTypeInfoKey.create(targetConfig, requirements...))`.
// Java: ToolchainTypeLookupUtilTest.getToolchainTypeInfo
fn get_toolchain_type_info(
    t: &mut ToolchainTestCase,
    toolchain_types: Vec<ToolchainTypeRequirement>,
) -> (GetToolchainTypeInfoKey, EvaluationResult) {
    let key = GetToolchainTypeInfoKey {
        configuration: t.get_target_configuration(),
        toolchain_types,
    };
    let result = t.evaluate_key(&key);
    (key, result)
}

/// Truth `containsExactlyEntriesIn(expected)`.
fn assert_entries(
    actual: &IndexMap<Label, ToolchainTypeInfo>,
    expected: &[(&Label, &ToolchainTypeInfo)],
) {
    assert_eq!(actual.len(), expected.len(), "{actual:?}");
    for (k, v) in expected {
        assert_eq!(actual.get(*k), Some(*v), "{actual:?}");
    }
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup
#[test]
fn test_toolchain_type_lookup() {
    let mut t = new_test();
    let requirement = ToolchainTypeRequirement::create(t.test_toolchain_type_label.clone());
    let (key, result) = get_toolchain_type_info(&mut t, vec![requirement]);
    assert_has_no_error(&result);
    let toolchain_types = &get_value(&result, &key).toolchain_types;
    assert_entries(
        toolchain_types,
        &[(&t.test_toolchain_type_label, &t.test_toolchain_type_info)],
    );
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_toolchainAlias
#[test]
fn test_toolchain_type_lookup_toolchain_alias() {
    let mut t = new_test();
    let line = format!(
        "alias(name = 'toolchain_type', actual = '{}')",
        t.test_toolchain_type_label
    );
    t.scratch.file("alias/BUILD", &[&line]);
    let alias_toolchain_type_label = label("//alias:toolchain_type");
    let (key, result) = get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(alias_toolchain_type_label)],
    );
    assert_has_no_error(&result);
    let toolchain_types = &get_value(&result, &key).toolchain_types;
    assert_entries(
        toolchain_types,
        &[
            (&t.test_toolchain_type_label, &t.test_toolchain_type_info),
            (
                &label("//alias:toolchain_type"),
                &t.test_toolchain_type_info,
            ),
        ],
    );
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetNotToolchainType
#[test]
fn test_toolchain_type_lookup_target_not_toolchain_type() {
    let mut t = new_test();
    t.scratch.file(
        "invalid/BUILD",
        &["filegroup(name = 'not_a_toolchain_type')"],
    );
    let (key, result) = get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(label(
            "//invalid:not_a_toolchain_type",
        ))],
    );
    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidToolchainType,
    );
    assert!(error_message(&result, &key).contains("//invalid:not_a_toolchain_type"));
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetNotToolchainType_ignoreInvalid
#[test]
fn test_toolchain_type_lookup_target_not_toolchain_type_ignore_invalid() {
    let mut t = new_test();
    t.scratch.file(
        "invalid/BUILD",
        &["filegroup(name = 'not_a_toolchain_type')"],
    );
    let (key, result) = get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement {
            toolchain_type: label("//invalid:not_a_toolchain_type"),
            mandatory: true,
            ignore_if_invalid: true,
        }],
    );
    assert_has_no_error(&result);
    assert!(get_value(&result, &key).toolchain_types.is_empty());
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetDoesNotExist
#[test]
fn test_toolchain_type_lookup_target_does_not_exist() {
    let mut t = new_test();
    t.remove_fail_fast_handler();
    let (key, result) = get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(label("//fake:missing"))],
    );
    assert_has_error(&result);
    assert_error_is_toolchain_exception(
        &result,
        &key,
        ToolchainExceptionKind::InvalidToolchainType,
    );
    let exception = error_exception(&result, &key);
    let cause = exception
        .downcast_ref::<ToolchainException>()
        .and_then(|e| e.get_cause())
        .expect("a cause");
    assert!(
        cause.downcast_ref::<NoSuchPackageException>().is_some()
            || matches!(
                cause.downcast_ref::<NoSuchThingException>(),
                Some(NoSuchThingException::Package(_))
            ),
        "expected a NoSuchPackageException cause, got {cause:?}"
    );
    t.assert_contains_event("no such package 'fake': BUILD file not found");
}
