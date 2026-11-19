//! Port of
//! `src/test/java/com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtilTest.java`
//! (a `ToolchainTestCase`): tests for `ToolchainTypeLookupUtil`.
//!
//! Java injects a test SkyFunction (`GET_TOOLCHAIN_TYPE_INFO_FUNCTION`, through
//! `AnalysisMock.Delegate.getSkyFunctions`) that calls
//! `ToolchainTypeLookupUtil.resolveToolchainTypes`; the harness cannot register extra SkyFunctions
//! in its `SkyframeExecutor`, so the tests stop at the evaluation and stay ignored. The Java
//! assertions after it are kept as comments.

use blaze_analysis::toolchains::ToolchainTypeRequirement;

use crate::tests::toolchain_test_case::{ToolchainTestCase, label};

const NEEDS_FUNCTION: &str = "needs an extra SkyFunction in the harness's SkyframeExecutor \
                              (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling \
                              ToolchainTypeLookupUtil.resolveToolchainTypes)";

/// Java `getToolchainTypeInfo(GetToolchainTypeInfoKey.create(targetConfig, requirements...))`.
// Java: ToolchainTypeLookupUtilTest.getToolchainTypeInfo
fn get_toolchain_type_info(
    t: &mut ToolchainTestCase,
    _toolchain_types: Vec<ToolchainTypeRequirement>,
) -> ! {
    let _target_config = t.get_target_configuration();
    panic!("{NEEDS_FUNCTION}")
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling ToolchainTypeLookupUtil.resolveToolchainTypes)"]
fn test_toolchain_type_lookup() {
    let mut t = ToolchainTestCase::new();
    let requirement = ToolchainTypeRequirement::create(t.test_toolchain_type_label.clone());
    get_toolchain_type_info(&mut t, vec![requirement]);
    // assertThatEvaluationResult(result).hasNoError();
    // assertThatEvaluationResult(result).hasEntryThat(key).isNotNull();
    // Map<Label, ToolchainTypeInfo> toolchainTypes = result.get(key).toolchainTypes();
    // assertThat(toolchainTypes)
    //     .containsExactlyEntriesIn(ImmutableMap.of(testToolchainTypeLabel, testToolchainTypeInfo));
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_toolchainAlias
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling ToolchainTypeLookupUtil.resolveToolchainTypes)"]
fn test_toolchain_type_lookup_toolchain_alias() {
    let mut t = ToolchainTestCase::new();
    let line = format!(
        "alias(name = 'toolchain_type', actual = '{}')",
        t.test_toolchain_type_label
    );
    t.scratch.file("alias/BUILD", &[&line]);
    let alias_toolchain_type_label = label("//alias:toolchain_type");
    get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(alias_toolchain_type_label)],
    );
    // assertThatEvaluationResult(result).hasNoError();
    // assertThatEvaluationResult(result).hasEntryThat(key).isNotNull();
    // Map<Label, ToolchainTypeInfo> toolchainTypes = result.get(key).toolchainTypes();
    // assertThat(toolchainTypes).containsExactlyEntriesIn(ImmutableMap.of(
    //     testToolchainTypeLabel, testToolchainTypeInfo,
    //     aliasToolchainTypeLabel, testToolchainTypeInfo));
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetNotToolchainType
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling ToolchainTypeLookupUtil.resolveToolchainTypes)"]
fn test_toolchain_type_lookup_target_not_toolchain_type() {
    let mut t = ToolchainTestCase::new();
    t.scratch.file(
        "invalid/BUILD",
        &["filegroup(name = 'not_a_toolchain_type')"],
    );
    get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(label(
            "//invalid:not_a_toolchain_type",
        ))],
    );
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidToolchainTypeException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasMessageThat()
    //     .contains("//invalid:not_a_toolchain_type");
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetNotToolchainType_ignoreInvalid
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling ToolchainTypeLookupUtil.resolveToolchainTypes)"]
fn test_toolchain_type_lookup_target_not_toolchain_type_ignore_invalid() {
    let mut t = ToolchainTestCase::new();
    t.scratch.file(
        "invalid/BUILD",
        &["filegroup(name = 'not_a_toolchain_type')"],
    );
    get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement {
            toolchain_type: label("//invalid:not_a_toolchain_type"),
            mandatory: true,
            ignore_if_invalid: true,
        }],
    );
    // assertThatEvaluationResult(result).hasNoError();
    // Map<Label, ToolchainTypeInfo> toolchainTypes = result.get(key).toolchainTypes();
    // assertThat(toolchainTypes).isEmpty();
}

// Java: ToolchainTypeLookupUtilTest.testToolchainTypeLookup_targetDoesNotExist
#[test]
#[ignore = "needs an extra SkyFunction in the harness's SkyframeExecutor \
            (GET_TOOLCHAIN_TYPE_INFO_FUNCTION calling ToolchainTypeLookupUtil.resolveToolchainTypes)"]
fn test_toolchain_type_lookup_target_does_not_exist() {
    let mut t = ToolchainTestCase::new();
    t.remove_fail_fast_handler();
    get_toolchain_type_info(
        &mut t,
        vec![ToolchainTypeRequirement::create(label("//fake:missing"))],
    );
    // assertThatEvaluationResult(result).hasError();
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat()
    //     .isInstanceOf(InvalidToolchainTypeException.class);
    // ...hasErrorEntryForKeyThat(key).hasExceptionThat().hasCauseThat()
    //     .isInstanceOf(NoSuchPackageException.class);
    // assertContainsEvent("no such package 'fake': BUILD file not found");
}
