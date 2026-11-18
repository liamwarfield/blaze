//! The native C++ rule definitions of `bazel/rules/CcRules.java` that are not `EmptyRule`s:
//! `cc_toolchain_alias` (Java `rules/cpp/CcToolchainAliasRule`) and `cc_libc_top_alias`
//! (Java `rules/cpp/CcLibcTopAlias`, a `LateBoundAlias.CommonAliasRule`).

use std::sync::Arc;

use crate::attribute::LateBoundDefault;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::types::Type;

/// `cc_toolchain_alias` (Java `rules/cpp/CcToolchainAliasRule`).
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainAliasRule.java:27 CcToolchainAliasRule
pub struct CcToolchainAliasRule;

impl RuleDefinition for CcToolchainAliasRule {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainAliasRule.java:29 CcToolchainAliasRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder.build()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainAliasRule.java:35 CcToolchainAliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("cc_toolchain_alias")
            .factory_class("EmptyRuleConfiguredTargetFactory")
            .ancestors(&[NativeBuildRule::CLASS])
    }

    fn definition_class(&self) -> String {
        "CcToolchainAliasRule".to_owned()
    }
}

/// Java `LateBoundAlias.CommonAliasRule.build`: the `:alias` attribute with `label_resolver`'s
/// default, requiring `fragment_class`.
// Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:115 LateBoundAlias.CommonAliasRule.build
pub fn common_alias_rule_build(
    builder: &mut RuleClassBuilder,
    alias_default: Arc<LateBoundDefault>,
    fragment_class: &str,
) -> RuleClass {
    // Java: LateBoundAlias.CommonAliasRule.makeAttribute
    let attribute = attr(":alias", Type::Label)
        .value_late_bound(alias_default)
        .build();
    blaze_util::check_argument!(attribute.get_name() == ":alias");
    blaze_util::check_argument!(attribute.get_type() == Type::Label);
    builder
        .requires_configuration_fragments(&[fragment_class])
        .remove_attribute("licenses")
        .remove_attribute("distribs")
        .advertise_provider(&["AliasProvider.LateBoundAliasProvider"])
        .add_attribute(attribute)
        .build()
}

/// `cc_libc_top_alias` (Java `rules/cpp/CcLibcTopAlias`).
// Java: com/google/devtools/build/lib/rules/cpp/CcLibcTopAlias.java:21 CcLibcTopAlias
pub struct CcLibcTopAlias;

impl RuleDefinition for CcLibcTopAlias {
    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:115 LateBoundAlias.CommonAliasRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        // Java: CcLibcTopAlias.getSysrootAttribute (resolved from CppConfiguration during
        // analysis; no default).
        // W4-analysis: names the analysis resolver.
        let sysroot = Arc::new(
            LateBoundDefault::label_from_target_configuration("CppConfiguration", None)
                .with_resolver_name("CcLibcTopAlias.SYSROOT"),
        );
        common_alias_rule_build(builder, sysroot, "CppConfiguration")
    }

    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:138 LateBoundAlias.AbstractAliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("cc_libc_top_alias")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("LateBoundAlias")
    }

    fn definition_class(&self) -> String {
        "CcLibcTopAlias".to_owned()
    }
}
