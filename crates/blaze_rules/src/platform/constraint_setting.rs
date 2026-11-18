//! Port of `rules/platform/ConstraintSetting.java`: defines a category of constraint that can be
//! fulfilled by a constraint_value rule in a platform definition.

use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::freeze_native_info;
use blaze_analysis::platform::{ConstraintSettingInfo, platform_provider_utils};
use blaze_analysis::rule::{RuleContext, RuleErrorConsumer};
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;

use crate::platform::common::build_with_empty_providers;

// Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:35 ConstraintSettingRule.DEFAULT_CONSTRAINT_VALUE_ATTR
pub const DEFAULT_CONSTRAINT_VALUE_ATTR: &str = "default_constraint_value";
// Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:36 ConstraintSettingRule.REFINES_CONSTRAINT_VALUE_ATTR
pub const REFINES_CONSTRAINT_VALUE_ATTR: &str = "refines_constraint_value";

/// Java `ConstraintSetting` (the `constraint_setting` rule's factory).
// Java: com/google/devtools/build/lib/rules/platform/ConstraintSetting.java:41 ConstraintSetting
pub struct ConstraintSetting;

impl RuleConfiguredTargetFactory for ConstraintSetting {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSetting.java:44 ConstraintSetting.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let constraint_setting: Label = rule_context.get_label().clone();
        let default_constraint_value: Option<Label> = rule_context
            .attributes()
            .get(DEFAULT_CONSTRAINT_VALUE_ATTR, Type::NodepLabel)
            .and_then(|v| v.as_label().cloned());
        let refines_constraint_value_attr: Option<Label> = rule_context
            .attributes()
            .get(REFINES_CONSTRAINT_VALUE_ATTR, Type::Label)
            .and_then(|v| v.as_label().cloned());

        validate_default_constraint_value(
            rule_context,
            &constraint_setting,
            default_constraint_value.as_ref(),
        )?;

        // Before using the refines_constraint_value attribute, ensure it's actually set and was
        // a valid constraint value (the RuleContext would have already reported an error if it
        // wasn't).
        let mut refined_value = None;
        if let Some(refines) = &refines_constraint_value_attr {
            refined_value = platform_provider_utils::constraint_value(
                rule_context
                    .get_prerequisite(REFINES_CONSTRAINT_VALUE_ATTR)
                    .as_ref(),
            );
            if refined_value.is_none() {
                // Must check for null since the attribute could have been an alias.
                return Err(rule_context
                    .throw_with_attribute_error(
                        REFINES_CONSTRAINT_VALUE_ATTR,
                        &format!(
                            "The refined constraint value '{refines}' is not a constraint_value."
                        ),
                    )
                    .into());
            }
        }

        build_with_empty_providers(
            rule_context,
            freeze_native_info(ConstraintSettingInfo::create_with(
                constraint_setting,
                default_constraint_value,
                refined_value,
            )),
        )
    }
}

// Java: com/google/devtools/build/lib/rules/platform/ConstraintSetting.java:86 ConstraintSetting.validateDefaultConstraintValue
fn validate_default_constraint_value(
    rule_context: &RuleContext,
    constraint_setting: &Label,
    default_constraint_value: Option<&Label>,
) -> Result<(), CreateConfiguredTargetError> {
    let Some(default_constraint_value) = default_constraint_value else {
        return Ok(());
    };

    // Make sure the default value is in the same package.
    if constraint_setting.get_package_identifier()
        != default_constraint_value.get_package_identifier()
    {
        return Err(rule_context
            .throw_with_attribute_error(
                DEFAULT_CONSTRAINT_VALUE_ATTR,
                "The default constraint value must be defined in the same package as the constraint setting itself.",
            )
            .into());
    }

    // Verify that the target actually exists, even though we cannot have a direct dependency
    // because it will cause a cycle.
    // PORT: Java reads the PackageValue through `getSkyframeEnv()`; the package is the current
    // target's, which "must have already been loaded".
    let pkg = rule_context
        .get_analysis_environment()
        .get_already_loaded_package(constraint_setting.get_package_identifier());
    if pkg.get_target(default_constraint_value.get_name()).is_err() {
        return Err(rule_context
            .throw_with_attribute_error(
                DEFAULT_CONSTRAINT_VALUE_ATTR,
                &format!(
                    "The default constraint value '{default_constraint_value}' does not exist"
                ),
            )
            .into());
    }
    Ok(())
}
