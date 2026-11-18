//! `toolchain_type` (Java `rules/ToolchainType.ToolchainTypeRule`).

use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::native_rules::toolchain_type_info_id;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::ToolchainResolutionMode;
use crate::types::Type;

/// `toolchain_type` (Java `ToolchainType.ToolchainTypeRule`).
// Java: com/google/devtools/build/lib/rules/ToolchainType.java:57 ToolchainType.ToolchainTypeRule
pub struct ToolchainTypeRule;

impl RuleDefinition for ToolchainTypeRule {
    // Java: com/google/devtools/build/lib/rules/ToolchainType.java:60 ToolchainType.ToolchainTypeRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .advertise_starlark_provider(vec![toolchain_type_info_id()])
            .remove_attribute("licenses")
            .remove_attribute("distribs")
            .remove_attribute(":action_listener")
            .add(
                attr("no_match_error", Type::String)
                    .nonconfigurable("low-level attribute, used in platform configuration"),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/ToolchainType.java:77 ToolchainType.ToolchainTypeRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("toolchain_type")
            .factory_class("ToolchainType")
            .ancestors(&[NativeBuildRule::CLASS])
    }

    fn definition_class(&self) -> String {
        "ToolchainType.ToolchainTypeRule".to_owned()
    }
}
