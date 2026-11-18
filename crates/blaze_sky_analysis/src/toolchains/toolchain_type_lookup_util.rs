//! Port of `skyframe/toolchains/ToolchainTypeLookupUtil.java`: helper class that looks up
//! `ToolchainTypeInfo` data.

use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::ToolchainTypeInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::toolchains::toolchain_exception::{
    CONFIGURED_VALUE_CREATION_EXCEPTION_CLASS, no_such_thing_java_class_name,
};
use blaze_analysis::toolchains::{
    ToolchainException, ToolchainExceptionKind, ToolchainTypeRequirement,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_skyframe::{Environment, SkyError, SkyKey, SkyframeLookupResult};
use indexmap::IndexMap;

use crate::toolchains::lookup_error::{
    ConfiguredTargetLookupError, ToolchainLookupError, catch_configured_target_error,
};

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:45 ToolchainTypeLookupUtil.resolveToolchainTypes
pub fn resolve_toolchain_types(
    env: &mut Environment<'_>,
    toolchain_types: &[ToolchainTypeRequirement],
    configuration: &BuildConfigurationValue,
) -> Result<Option<IndexMap<Label, ToolchainTypeInfo>>, ToolchainLookupError> {
    // Java: toImmutableMap (throws on duplicate keys; the requirements are distinct by type
    // label in practice).
    let mut toolchain_types_by_key: IndexMap<ConfiguredTargetKey, &ToolchainTypeRequirement> =
        IndexMap::new();
    for requirement in toolchain_types {
        let key = ConfiguredTargetKey::builder()
            .set_label(requirement.toolchain_type.clone())
            .set_configuration(Some(configuration))
            .build();
        if toolchain_types_by_key
            .insert(key.clone(), requirement)
            .is_some()
        {
            panic!("IllegalArgumentException: Multiple entries with same key: {key}");
        }
    }

    let sky_keys: Vec<SkyKey> = toolchain_types_by_key
        .keys()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let values = env.get_values_and_exceptions(&sky_keys)?;
    let mut values_missing = env.values_missing();
    let mut results: IndexMap<Label, ToolchainTypeInfo> = IndexMap::new();
    for (key, toolchain_type_requirement) in &toolchain_types_by_key {
        let original_label = key.get_label();
        let toolchain_type_info =
            find_toolchain_type_info(toolchain_type_requirement, key, &values)?;
        let Some(toolchain_type_info) = toolchain_type_info else {
            // Null means skyframe restart.
            values_missing = true;
            continue;
        };
        if !values_missing && let Some(info) = toolchain_type_info {
            // Save the response by both the original label and the type label.
            results.insert(original_label.clone(), info.clone());
            results.insert(info.type_label().clone(), info);
        }
    }
    if values_missing {
        return Ok(None);
    }

    Ok(Some(results))
}

/// Returns `None` to signal a Skyframe restart, `Some(None)` if the toolchain type is invalid
/// but ignored, and `Some(Some(info))` with the toolchain type info otherwise.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:100 ToolchainTypeLookupUtil.findToolchainTypeInfo
fn find_toolchain_type_info(
    toolchain_type_requirement: &ToolchainTypeRequirement,
    key: &ConfiguredTargetKey,
    values: &SkyframeLookupResult,
) -> Result<Option<Option<ToolchainTypeInfo>>, ToolchainException> {
    match values.get_or_throw_with(key, catch_configured_target_error) {
        Ok(None) => Ok(None),
        Ok(Some(ctv)) => {
            let configured_target = ctv.get_configured_target();
            let toolchain_type_info = platform_provider_utils::toolchain_type(configured_target);
            if toolchain_type_info.is_none() && !toolchain_type_requirement.ignore_if_invalid {
                if platform_provider_utils::declared_toolchain_info(Some(configured_target))
                    .is_some()
                {
                    return Err(invalid_toolchain_type_exception_with_error(
                        &configured_target.get_label(),
                        "is a toolchain instance. Is the rule definition for the target you're building setting \"toolchains =\" to a toolchain() instead of the expected toolchain_type()?",
                    ));
                }
                return Err(invalid_toolchain_type_exception(
                    &configured_target.get_label(),
                ));
            }

            Ok(Some(toolchain_type_info))
        }
        Err(ConfiguredTargetLookupError::ConfiguredValueCreation(e)) => {
            Err(invalid_toolchain_type_exception_from_cvce(e))
        }
        Err(ConfiguredTargetLookupError::NoSuchThing(e)) => {
            Err(invalid_toolchain_type_exception_no_such_thing(e))
        }
    }
}

const INVALID_TOOLCHAIN_TYPE_DEFAULT_ERROR: &str = "does not provide ToolchainTypeInfo";

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:174 ToolchainTypeLookupUtil.InvalidToolchainTypeException.formatError
fn format_error(label: &Label, error: &str) -> String {
    format!("Target {label} was referenced as a toolchain type, but {error}")
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:147 ToolchainTypeLookupUtil.InvalidToolchainTypeException.<init>
pub fn invalid_toolchain_type_exception(label: &Label) -> ToolchainException {
    invalid_toolchain_type_exception_with_error(label, INVALID_TOOLCHAIN_TYPE_DEFAULT_ERROR)
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:151 ToolchainTypeLookupUtil.InvalidToolchainTypeException.<init>
fn invalid_toolchain_type_exception_from_cvce(cause: SkyError) -> ToolchainException {
    // Just propagate the inner exception, because it's directly actionable.
    ToolchainException::from_cause(
        ToolchainExceptionKind::InvalidToolchainType,
        CONFIGURED_VALUE_CREATION_EXCEPTION_CLASS,
        cause,
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:156 ToolchainTypeLookupUtil.InvalidToolchainTypeException.<init>
pub fn invalid_toolchain_type_exception_no_such_thing(
    e: NoSuchThingException,
) -> ToolchainException {
    // Just propagate the inner exception, because it's directly actionable.
    let class = no_such_thing_java_class_name(&e);
    ToolchainException::from_cause(
        ToolchainExceptionKind::InvalidToolchainType,
        class,
        Arc::new(e),
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:165 ToolchainTypeLookupUtil.InvalidToolchainTypeException.<init>
fn invalid_toolchain_type_exception_with_error(label: &Label, error: &str) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::InvalidToolchainType,
        format_error(label, error),
    )
}
