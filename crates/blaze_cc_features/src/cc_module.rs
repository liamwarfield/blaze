//! The bodies of the native `cc_common` methods in
//! `com.google.devtools.build.lib.rules.cpp.CcModule` that rules_cc reaches through
//! `native_cc_common.bzl`. The Starlark wrapper calls [`is_called_from_starlark_cc_common`] with
//! the innermost .bzl label first.

use indexmap::IndexMap;

use crate::actions::NOOP;
use crate::cc_toolchain_variables::{CcToolchainVariables, CcToolchainVariablesRef};
use crate::errors::{EvalException, ExpansionException};
use crate::feature_configuration_for_starlark::FeatureConfigurationForStarlark;

/// `cc_common.get_tool_for_action(feature_configuration, action_name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:62 CcModule.getToolForAction
pub fn get_tool_for_action(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
) -> Result<String, EvalException> {
    feature_configuration
        .get_feature_configuration()
        .try_get_tool_path_for_action(action_name)
        // new EvalException(illegalArgumentException) carries the cause's message.
        .map_err(EvalException::new)
}

/// `cc_common.get_tool_requirement_for_action(feature_configuration, action_name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:77 CcModule.getToolRequirementForAction
pub fn get_tool_requirement_for_action(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
) -> Vec<String> {
    feature_configuration
        .get_feature_configuration()
        .get_tool_requirements_for_action(action_name)
        .iter()
        .cloned()
        .collect()
}

/// `cc_common.get_execution_requirements(feature_configuration, action_name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:88 CcModule.getExecutionRequirements
pub fn get_execution_requirements(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
) -> Vec<String> {
    get_tool_requirement_for_action(feature_configuration, action_name)
}

/// `cc_common.action_is_enabled(feature_configuration, action_name)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:99 CcModule.actionIsEnabled
pub fn action_is_enabled(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
) -> bool {
    feature_configuration
        .get_feature_configuration()
        .action_is_configured(action_name)
}

/// `cc_common.get_memory_inefficient_command_line(feature_configuration, action_name,
/// variables)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:109 CcModule.getCommandLine
pub fn get_command_line(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
    variables: &CcToolchainVariablesRef,
) -> Result<Vec<String>, ExpansionException> {
    feature_configuration
        .get_feature_configuration()
        .get_command_line(action_name, variables)
}

/// `cc_common.get_environment_variables(feature_configuration, action_name, variables)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:121 CcModule.getEnvironmentVariable
pub fn get_environment_variable(
    feature_configuration: &FeatureConfigurationForStarlark,
    action_name: &str,
    variables: &CcToolchainVariables,
) -> Result<IndexMap<String, String>, ExpansionException> {
    feature_configuration
        .get_feature_configuration()
        .get_environment_variables(action_name, variables, &NOOP)
}

/// `cc_common.empty_variables()`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:135 CcModule.getVariables
pub fn get_variables() -> CcToolchainVariablesRef {
    CcToolchainVariables::empty()
}

/// The access check of the `cc_common` natives, given
/// `BazelModuleContext.ofInnermostBzlOrThrow(thread).label().getCanonicalForm()`.
// Java: com/google/devtools/build/lib/rules/cpp/CcModule.java:202 CcModule.isCalledFromStarlarkCcCommon
pub fn is_called_from_starlark_cc_common(label_canonical_form: &str) -> Result<(), EvalException> {
    // Allow direct access to cc_common.bzl and to C++ linking code that can't use cc_common.bzl
    // directly without creating a cycle.
    if !label_canonical_form.ends_with("_builtins//:common/cc/cc_common.bzl")
        && !label_canonical_form.ends_with("_builtins//:common/cc/cc_common_bazel.bzl")
        && !label_canonical_form.contains("_builtins//:common/cc/compile")
        && !label_canonical_form.contains("_builtins//:common/cc/link")
        && !label_canonical_form.contains("_builtins//:common/cc/toolchain_config")
    {
        return Err(EvalException::new(
            "cc_common_internal can only be used by cc_common.bzl in builtins, please use \
             cc_common instead.",
        ));
    }
    Ok(())
}
