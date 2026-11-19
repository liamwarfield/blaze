//! Port of the analysis half of `rules/LabelBuildSettings.java`: the `:alias` late-bound
//! default of `label_flag` / `label_setting` (the rule classes are loading-phase,
//! `blaze_packages::native_rules`; their factory is `LateBoundAlias`).

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::StarlarkOptionValue;
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use blaze_packages::target::Rule;
use blaze_packages::types::{AttrValue, Type};

/// The resolver of `LabelBuildSettings.ACTUAL`: the command-line value of the flag, else its
/// `build_setting_default` (Java `LabelLateBoundDefault.fromTargetConfigurationWithRuleBasedDefault`
/// resolver lambda).
// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:62 LabelBuildSettings.ACTUAL
pub fn resolve_actual(
    rule: Option<&Rule>,
    attributes: &dyn AttributeMap,
    configuration: Option<&BuildConfigurationValue>,
) -> Option<Label> {
    let default =
        || match attributes.get(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::NodepLabel) {
            Some(AttrValue::Label(l)) => Some(l),
            _ => None,
        };
    let (Some(rule), Some(configuration)) = (rule, configuration) else {
        return default();
    };
    let Some(command_line_value) = configuration
        .get_options()
        .get_starlark_options()
        .get(rule.get_label())
    else {
        return default();
    };
    match command_line_value {
        StarlarkOptionValue::Label(l) => Some(l.clone()),
        other => panic!(
            "IllegalStateException: the value of {} should have been converted to a label already, but its type is {}",
            rule.get_label(),
            match other {
                StarlarkOptionValue::Bool(_) => "bool",
                StarlarkOptionValue::Int(_) => "int",
                StarlarkOptionValue::Str(_) => "string",
                StarlarkOptionValue::List(_) => "list",
                StarlarkOptionValue::Label(_) => "Label",
                // PORT: Starlark.type of a FeatureFlagValue class (not a Starlark value).
                StarlarkOptionValue::FeatureFlag(v) =>
                    blaze_util::unported!("Starlark.type", "FeatureFlagValue {}", v),
            }
        ),
    }
}
