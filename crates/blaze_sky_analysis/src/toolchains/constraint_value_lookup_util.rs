//! Port of `skyframe/toolchains/ConstraintValueLookupUtil.java`: helper class that looks up
//! `ConstraintValueInfo` data.

use std::sync::Arc;

use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::platform::ConstraintValueInfo;
use blaze_analysis::platform::platform_provider_utils;
use blaze_analysis::toolchains::toolchain_exception::no_such_thing_java_class_name;
use blaze_analysis::toolchains::{ToolchainException, ToolchainExceptionKind};
use blaze_cmdline::Label;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_skyframe::{Environment, SkyError, SkyKey, SkyframeLookupResult};

use crate::toolchains::lookup_error::{
    ConfiguredTargetLookupError, ToolchainLookupError, catch_configured_target_error,
};

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:37 ConstraintValueLookupUtil.getConstraintValueInfo
pub fn get_constraint_value_info(
    constraint_value_keys: &[ConfiguredTargetKey],
    env: &mut Environment<'_>,
) -> Result<Option<Vec<ConstraintValueInfo>>, ToolchainLookupError> {
    let sky_keys: Vec<SkyKey> = constraint_value_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let values = env.get_values_and_exceptions(&sky_keys)?;
    let values_missing = env.values_missing();
    let mut constraint_values = Vec::new();
    for key in constraint_value_keys {
        let constraint_value_info = find_constraint_value_info(key, &values)?;
        if !values_missing && let Some(v) = constraint_value_info {
            constraint_values.push(v);
        }
    }
    if values_missing {
        return Ok(None);
    }
    Ok(Some(constraint_values))
}

/// Returns the [`ConstraintValueInfo`] provider from the configured target in the lookup
/// result, or `None` if the configured target is not present. If the configured target does not
/// have a [`ConstraintValueInfo`] provider, an `InvalidConstraintValueException` is thrown.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:63 ConstraintValueLookupUtil.findConstraintValueInfo
fn find_constraint_value_info(
    key: &ConfiguredTargetKey,
    values: &SkyframeLookupResult,
) -> Result<Option<ConstraintValueInfo>, ToolchainException> {
    match values.get_or_throw_with(key, catch_configured_target_error) {
        Ok(None) => Ok(None),
        Ok(Some(ctv)) => {
            let configured_target = ctv.get_configured_target();
            let Some(constraint_value_info) =
                platform_provider_utils::constraint_value(Some(configured_target))
            else {
                return Err(invalid_constraint_value_exception(
                    &configured_target.get_label(),
                ));
            };
            Ok(Some(constraint_value_info))
        }
        Err(ConfiguredTargetLookupError::ConfiguredValueCreation(e)) => Err(
            invalid_constraint_value_exception_with_cause(key.get_label(), e),
        ),
        Err(ConfiguredTargetLookupError::NoSuchThing(e)) => {
            Err(invalid_constraint_value_exception_no_such_thing(e))
        }
        Err(ConfiguredTargetLookupError::ActionConflict(e)) => Err(
            invalid_constraint_value_exception_with_cause(key.get_label(), e),
        ),
    }
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:118 ConstraintValueLookupUtil.InvalidConstraintValueException.formatError
fn format_error(label: &Label) -> String {
    format!(
        "Target {label} was referenced as a constraint_value, but does not provide ConstraintValueInfo"
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:96 ConstraintValueLookupUtil.InvalidConstraintValueException.<init>
pub fn invalid_constraint_value_exception(label: &Label) -> ToolchainException {
    ToolchainException::new(
        ToolchainExceptionKind::InvalidConstraintValue,
        format_error(label),
    )
}

/// Java `InvalidConstraintValueException(Label, ConfiguredValueCreationException)` (also the
/// `ActionConflictException` overload).
// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:100 ConstraintValueLookupUtil.InvalidConstraintValueException.<init>
pub fn invalid_constraint_value_exception_with_cause(
    label: &Label,
    cause: SkyError,
) -> ToolchainException {
    ToolchainException::with_cause(
        ToolchainExceptionKind::InvalidConstraintValue,
        format_error(label),
        cause,
    )
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:104 ConstraintValueLookupUtil.InvalidConstraintValueException.<init>
pub fn invalid_constraint_value_exception_no_such_thing(
    e: NoSuchThingException,
) -> ToolchainException {
    // Just propagate the inner exception, because it's directly actionable.
    let class = no_such_thing_java_class_name(&e);
    ToolchainException::from_cause(
        ToolchainExceptionKind::InvalidConstraintValue,
        class,
        Arc::new(e),
    )
}
