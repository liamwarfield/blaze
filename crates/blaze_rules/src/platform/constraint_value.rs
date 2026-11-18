//! Port of `rules/platform/ConstraintValue.java`: defines a potential value of a constraint.

use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::freeze_native_info;
use blaze_analysis::platform::{ConstraintValueInfo, platform_provider_utils};
use blaze_analysis::rule::RuleContext;

use crate::platform::common::build_with_empty_providers;

// Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:35 ConstraintValueRule.CONSTRAINT_SETTING_ATTR
pub const CONSTRAINT_SETTING_ATTR: &str = "constraint_setting";

/// Java `ConstraintValue` (the `constraint_value` rule's factory).
// Java: com/google/devtools/build/lib/rules/platform/ConstraintValue.java:31 ConstraintValue
pub struct ConstraintValue;

impl RuleConfiguredTargetFactory for ConstraintValue {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintValue.java:35 ConstraintValue.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let constraint = platform_provider_utils::constraint_setting(
            rule_context
                .get_prerequisite(CONSTRAINT_SETTING_ATTR)
                .as_ref(),
        )
        // Java passes a null setting into ConstraintValueInfo.create (an error was reported by
        // the prerequisite validation).
        .expect("NullPointerException: constraint_setting does not provide ConstraintSettingInfo");

        let label = rule_context.get_label().clone();
        build_with_empty_providers(
            rule_context,
            freeze_native_info(ConstraintValueInfo::create(constraint, label)),
        )
    }
}
