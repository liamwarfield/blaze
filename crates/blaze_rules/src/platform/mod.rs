//! Native platform rule factories (agent P; crates/blaze_analysis/DESIGN.md): ports of
//! `rules/platform/{Platform, ConstraintSetting, ConstraintValue, Toolchain}.java` and
//! `rules/ToolchainType.java`.

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

mod common;
pub mod constraint_setting;
pub mod constraint_value;
// Java package `rules/platform` holds class `Platform`.
#[allow(clippy::module_inception)]
pub mod platform;
pub mod toolchain;
pub mod toolchain_type;

#[cfg(test)]
mod tests;

/// Registers the factories under the names `blaze_packages::native_rules` gives the rule
/// classes (`RuleClass::get_configured_target_factory()`).
// Java: com/google/devtools/build/lib/rules/platform/PlatformRules.java:34 PlatformRules.init
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder
        .add_factory("Platform", Arc::new(platform::Platform))
        .add_factory(
            "ConstraintSetting",
            Arc::new(constraint_setting::ConstraintSetting),
        )
        .add_factory(
            "ConstraintValue",
            Arc::new(constraint_value::ConstraintValue),
        )
        .add_factory("Toolchain", Arc::new(toolchain::Toolchain))
        .add_factory("ToolchainType", Arc::new(toolchain_type::ToolchainType));
}
