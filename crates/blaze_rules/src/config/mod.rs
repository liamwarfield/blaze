//! Native configured target factories of `lib/rules/config`, `lib/rules/Alias`,
//! `lib/rules/LateBoundAlias` and `lib/rules/LabelBuildSettings` (agent C;
//! crates/blaze_analysis/DESIGN.md).

pub mod alias;
pub mod config_feature_flag_provider;
pub mod config_setting;
pub mod config_starlark_common;
pub mod label_build_settings;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

/// Registers the factories under the names `blaze_packages::native_rules` gives their rule
/// classes (`RuleClass::get_configured_target_factory()`, the Java factory class simple name).
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRules.java:46 ConfigRules.init
    builder.add_factory("ConfigSetting", config_setting::factory());
    // Java: com/google/devtools/build/lib/rules/core/CoreRules.java GenericRules (alias)
    builder.add_factory("Alias", alias::alias_factory());
    // label_flag, label_setting and the native late-bound aliases.
    builder.add_factory("LateBoundAlias", alias::late_bound_alias_factory());
}
