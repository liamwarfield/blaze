//! Port of `rules/config/ConfigRuleClasses.java`: `$config_base_rule`, `config_setting` and
//! `config_feature_flag`, plus the allowlist attribute helper of `analysis/Allowlist.java`.

use std::sync::Arc;
use std::sync::LazyLock;

use indexmap::IndexSet;

use crate::attribute::AllowedValueSet;
use crate::attribute::AttributeBuilder;
use crate::attribute::AttributeMap;
use crate::attribute::LateBoundDefault;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::attribute::RawAttributeMapper;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::native_rules::base_rule_classes::TAGGED_TRIMMING_ATTR;
use crate::native_rules::exec_transition_factory;
use crate::native_rules::generic_rules::scope_type_allowed_attribute_values;
use crate::rule_class::AllowlistChecker;
use crate::rule_class::LocationCheck;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::rule_class::ToolchainResolutionMode;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::Type;

// Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:53 ConfigRuleClasses.NONCONFIGURABLE_ATTRIBUTE_REASON
const NONCONFIGURABLE_ATTRIBUTE_REASON: &str =
    "part of a rule class that *triggers* configurable behavior";

/// Java `Allowlist.getAttributeNameFromAllowlistName(name).iterator().next()`.
// Java: com/google/devtools/build/lib/analysis/Allowlist.java:140 Allowlist.getAttributeNameFromAllowlistName
fn allowlist_attribute_names(allowlist_name: &str) -> [String; 2] {
    [
        format!("$whitelist_{allowlist_name}"),
        format!("$allowlist_{allowlist_name}"),
    ]
}

/// An implicit attribute holding a package group allowlist (Java
/// `Allowlist.getAttributeFromAllowlistName`).
// PORT: move to the port of analysis/Allowlist.java (W4).
// Java: com/google/devtools/build/lib/analysis/Allowlist.java:49 Allowlist.getAttributeFromAllowlistName
pub fn get_attribute_from_allowlist_name(allowlist_name: &str) -> AttributeBuilder {
    let [attribute_name, _] = allowlist_attribute_names(allowlist_name);
    let mut b = attr(&attribute_name, Type::Label);
    b.cfg(exec_transition_factory())
        .mandatory_builtin_providers(vec!["PackageSpecificationProvider".to_owned()]);
    b
}

/// The abstract base of configuration rules (Java `ConfigRuleClasses.ConfigBaseRule`).
// Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:57 ConfigRuleClasses.ConfigBaseRule
pub struct ConfigBaseRule;

impl ConfigBaseRule {
    pub const CLASS: &'static str = "ConfigRuleClasses.ConfigBaseRule";
}

impl RuleDefinition for ConfigBaseRule {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:59 ConfigRuleClasses.ConfigBaseRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .override_(
                attr("tags", Type::StringList)
                    // No need to show up in ":all", etc. target patterns.
                    .value(Some(AttrValue::StringList(vec!["manual".to_owned()])))
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .exempt_from_constraint_checking(
                "these rules don't include content that gets built into their dependers",
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:72 ConfigRuleClasses.ConfigBaseRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("$config_base_rule")
            .type_(RuleClassType::Abstract)
            .ancestors(&[NativeBuildRule::CLASS])
    }

    fn definition_class(&self) -> String {
        Self::CLASS.to_owned()
    }
}

/// `config_setting` (Java `ConfigRuleClasses.ConfigSettingRule`).
// Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:130 ConfigRuleClasses.ConfigSettingRule
pub struct ConfigSettingRule;

impl ConfigSettingRule {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:132 ConfigRuleClasses.ConfigSettingRule.RULE_NAME
    pub const RULE_NAME: &'static str = "config_setting";
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:135 ConfigRuleClasses.ConfigSettingRule.SETTINGS_ATTRIBUTE
    pub const SETTINGS_ATTRIBUTE: &'static str = "values";
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:141 ConfigRuleClasses.ConfigSettingRule.FLAG_ALIAS_SETTINGS_ATTRIBUTE
    pub const FLAG_ALIAS_SETTINGS_ATTRIBUTE: &'static str = ":flag_alias_settings";
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:144 ConfigRuleClasses.ConfigSettingRule.DEFINE_SETTINGS_ATTRIBUTE
    pub const DEFINE_SETTINGS_ATTRIBUTE: &'static str = "define_values";
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:147 ConfigRuleClasses.ConfigSettingRule.FLAG_SETTINGS_ATTRIBUTE
    pub const FLAG_SETTINGS_ATTRIBUTE: &'static str = "flag_values";
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:150 ConfigRuleClasses.ConfigSettingRule.CONSTRAINT_VALUES_ATTRIBUTE
    pub const CONSTRAINT_VALUES_ATTRIBUTE: &'static str = "constraint_values";
}

/// Java `ConfigSettingRule.FLAG_ALIAS_REFERENCES` (the user-defined flags `--flag_alias`
/// maps `values` keys to, resolved during analysis).
// Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:172 ConfigRuleClasses.ConfigSettingRule.FLAG_ALIAS_REFERENCES
pub static FLAG_ALIAS_REFERENCES: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    // W4-analysis: names the analysis resolver.
    Arc::new(
        LateBoundDefault::label_list(Some("BuildConfigurationValue"))
            .with_resolver_name("ConfigSettingRule.FLAG_ALIAS_REFERENCES"),
    )
});

impl RuleDefinition for ConfigSettingRule {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:193 ConfigRuleClasses.ConfigSettingRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .set_ignore_licenses()
            .toolchain_resolution_mode(ToolchainResolutionMode::Custom(Arc::new(|rule: &Rule| {
                let attr_map = RawAttributeMapper::of(rule as &dyn RuleOrMacroInstance);
                attr_map.has(Self::CONSTRAINT_VALUES_ATTRIBUTE)
                    && attr_map
                        .get(Self::CONSTRAINT_VALUES_ATTRIBUTE, Type::LabelList)
                        .as_ref()
                        .and_then(AttrValue::as_label_list)
                        .is_some_and(|l| !l.is_empty())
            })))
            .add(
                attr(Self::SETTINGS_ATTRIBUTE, Type::StringDict)
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .add(
                attr(Self::DEFINE_SETTINGS_ATTRIBUTE, Type::StringDict)
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .add(
                attr(Self::FLAG_SETTINGS_ATTRIBUTE, Type::LabelKeyedStringDict)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .add(
                attr(Self::FLAG_ALIAS_SETTINGS_ATTRIBUTE, Type::LabelList)
                    .value_late_bound(FLAG_ALIAS_REFERENCES.clone()),
            )
            .add(
                attr(Self::CONSTRAINT_VALUES_ATTRIBUTE, Type::LabelList)
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON)
                    .allowed_file_types(FileTypeSet::NoFile),
            )
            .set_option_reference_function_for_config_setting_only(Arc::new(|rule: &Rule| {
                NonconfigurableAttributeMapper::of(rule as &dyn RuleOrMacroInstance)
                    .get(Self::SETTINGS_ATTRIBUTE, Type::StringDict)
                    .as_ref()
                    .and_then(AttrValue::as_string_dict)
                    .map(|d| d.keys().cloned().collect::<IndexSet<String>>())
                    .unwrap_or_default()
            }))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:326 ConfigRuleClasses.ConfigSettingRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .type_(RuleClassType::Normal)
            .ancestors(&[ConfigBaseRule::CLASS])
            .factory_class("ConfigSetting")
    }

    fn definition_class(&self) -> String {
        "ConfigRuleClasses.ConfigSettingRule".to_owned()
    }
}

/// `config_feature_flag` (Java `ConfigRuleClasses.ConfigFeatureFlagRule`).
// Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:457 ConfigRuleClasses.ConfigFeatureFlagRule
pub struct ConfigFeatureFlagRule;

impl ConfigFeatureFlagRule {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:458 ConfigRuleClasses.ConfigFeatureFlagRule.RULE_NAME
    pub const RULE_NAME: &'static str = "config_feature_flag";
}

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:54 ConfigFeatureFlag.ALLOWLIST_NAME
const CONFIG_FEATURE_FLAG_ALLOWLIST_NAME: &str = "config_feature_flag";
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:57 ConfigFeatureFlag.ALLOWLIST_LABEL
const CONFIG_FEATURE_FLAG_ALLOWLIST_LABEL: &str =
    "//tools/allowlists/config_feature_flag:config_feature_flag";

/// Java `ConfigFeatureFlag.getAllowlistAttribute(env)`.
// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlag.java:61 ConfigFeatureFlag.getAllowlistAttribute
fn config_feature_flag_allowlist_attribute(
    env: &dyn RuleDefinitionEnvironment,
) -> AttributeBuilder {
    let mut b = get_attribute_from_allowlist_name(CONFIG_FEATURE_FLAG_ALLOWLIST_NAME);
    b.value(Some(AttrValue::Label(
        env.get_tools_label(CONFIG_FEATURE_FLAG_ALLOWLIST_LABEL),
    )));
    b
}

impl RuleDefinition for ConfigFeatureFlagRule {
    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:469 ConfigRuleClasses.ConfigFeatureFlagRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        // Java: ConfigRuleClasses.ConfigFeatureFlagRule.ALWAYS_CHECK_ALLOWLIST
        let always_check_allowlist = AllowlistChecker {
            allowlist_attr: CONFIG_FEATURE_FLAG_ALLOWLIST_NAME.to_owned(),
            error_message: "the config_feature_flag rule is not available in this package"
                .to_owned(),
            location_check: LocationCheck::Instance,
            attribute_set_trigger: None,
        };
        builder
            .set_undocumented(/* No documentation */)
            .requires_configuration_fragments(&["ConfigFeatureFlagConfiguration"])
            .add(
                attr("allowed_values", Type::StringList)
                    .mandatory()
                    .non_empty()
                    .order_independent()
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .add(
                attr("default_value", Type::String)
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON),
            )
            .add(
                attr("scope", Type::String)
                    .value(Some(AttrValue::String("universal".to_owned())))
                    .nonconfigurable(NONCONFIGURABLE_ATTRIBUTE_REASON)
                    .allowed_values(Arc::new(AllowedValueSet::new(
                        scope_type_allowed_attribute_values(),
                    ))),
            )
            .add(&config_feature_flag_allowlist_attribute(env))
            .add_allowlist_checker(always_check_allowlist)
            .remove_attribute(TAGGED_TRIMMING_ATTR)
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigRuleClasses.java:494 ConfigRuleClasses.ConfigFeatureFlagRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .ancestors(&[ConfigBaseRule::CLASS])
            .factory_class("ConfigFeatureFlag")
    }

    fn definition_class(&self) -> String {
        "ConfigRuleClasses.ConfigFeatureFlagRule".to_owned()
    }
}
