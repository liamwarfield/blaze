//! The rule definitions of `rules/platform/PlatformRules.java`: `constraint_setting`,
//! `constraint_value`, `platform`, `toolchain`.

use indexmap::IndexMap;

use crate::attribute::TransitionFactory;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::native_rules::constraint_setting_info_id;
use crate::native_rules::constraint_value_info_id;
use crate::native_rules::platform_info_id;
use crate::native_rules::toolchain_type_info_id;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::ToolchainResolutionMode;
use crate::rule_class::rule_class::APPLICABLE_METADATA_ATTR;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::Type;

fn manual_tags() -> Option<AttrValue> {
    Some(AttrValue::StringList(vec!["manual".to_owned()]))
}

/// `constraint_setting` (Java `rules/platform/ConstraintSettingRule`).
// Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:33 ConstraintSettingRule
pub struct ConstraintSettingRule;

impl ConstraintSettingRule {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:34 ConstraintSettingRule.RULE_NAME
    pub const RULE_NAME: &'static str = "constraint_setting";
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:35 ConstraintSettingRule.DEFAULT_CONSTRAINT_VALUE_ATTR
    pub const DEFAULT_CONSTRAINT_VALUE_ATTR: &'static str = "default_constraint_value";
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:36 ConstraintSettingRule.REFINES_CONSTRAINT_VALUE_ATTR
    pub const REFINES_CONSTRAINT_VALUE_ATTR: &'static str = "refines_constraint_value";
}

impl RuleDefinition for ConstraintSettingRule {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:39 ConstraintSettingRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .advertise_starlark_provider(vec![constraint_setting_info_id()])
            .cfg(TransitionFactory::no_config_transition())
            .exempt_from_constraint_checking("this rule helps *define* a constraint")
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .remove_attribute(":action_listener")
            .remove_attribute(APPLICABLE_METADATA_ATTR)
            .override_(
                attr("tags", Type::StringList)
                    .value(manual_tags())
                    .nonconfigurable("low-level attribute, used in platform configuration"),
            )
            .add(
                attr(Self::DEFAULT_CONSTRAINT_VALUE_ATTR, Type::NodepLabel)
                    .nonconfigurable("constants must be consistent across configurations"),
            )
            .add(
                attr(Self::REFINES_CONSTRAINT_VALUE_ATTR, Type::Label)
                    .allowed_rule_classes(&[ConstraintValueRule::RULE_NAME])
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers(vec![constraint_value_info_id()])
                    .nonconfigurable("constants must be consistent across configurations"),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/platform/ConstraintSettingRule.java:90 ConstraintSettingRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("ConstraintSetting")
    }

    fn definition_class(&self) -> String {
        "ConstraintSettingRule".to_owned()
    }
}

/// `constraint_value` (Java `rules/platform/ConstraintValueRule`).
// Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:33 ConstraintValueRule
pub struct ConstraintValueRule;

impl ConstraintValueRule {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:34 ConstraintValueRule.RULE_NAME
    pub const RULE_NAME: &'static str = "constraint_value";
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:35 ConstraintValueRule.CONSTRAINT_SETTING_ATTR
    pub const CONSTRAINT_SETTING_ATTR: &'static str = "constraint_setting";
}

impl RuleDefinition for ConstraintValueRule {
    // Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:38 ConstraintValueRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .advertise_starlark_provider(vec![constraint_value_info_id()])
            .cfg(TransitionFactory::no_config_transition())
            .exempt_from_constraint_checking("this rule helps *define* a constraint")
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .remove_attribute(":action_listener")
            .remove_attribute(APPLICABLE_METADATA_ATTR)
            .override_(
                attr("tags", Type::StringList)
                    .value(manual_tags())
                    .nonconfigurable("low-level attribute, used in platform configuration"),
            )
            .add(
                attr(Self::CONSTRAINT_SETTING_ATTR, Type::Label)
                    .mandatory()
                    .allowed_rule_classes(&[ConstraintSettingRule::RULE_NAME])
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers(vec![constraint_setting_info_id()])
                    .nonconfigurable("constants must be consistent across configurations"),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/platform/ConstraintValueRule.java:66 ConstraintValueRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("ConstraintValue")
    }

    fn definition_class(&self) -> String {
        "ConstraintValueRule".to_owned()
    }
}

/// `platform` (Java `rules/platform/PlatformRule`).
// Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:34 PlatformRule
pub struct PlatformRule;

impl PlatformRule {
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:35 PlatformRule.DEFAULT_MISSING_TOOLCHAIN_ERROR
    pub const DEFAULT_MISSING_TOOLCHAIN_ERROR: &'static str = "For more information on platforms or toolchains see https://bazel.build/concepts/platforms-intro.";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:39 PlatformRule.RULE_NAME
    pub const RULE_NAME: &'static str = "platform";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:40 PlatformRule.CONSTRAINT_VALUES_ATTR
    pub const CONSTRAINT_VALUES_ATTR: &'static str = "constraint_values";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:41 PlatformRule.PARENTS_PLATFORM_ATTR
    pub const PARENTS_PLATFORM_ATTR: &'static str = "parents";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:42 PlatformRule.REMOTE_EXECUTION_PROPS_ATTR
    pub const REMOTE_EXECUTION_PROPS_ATTR: &'static str = "remote_execution_properties";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:43 PlatformRule.EXEC_PROPS_ATTR
    pub const EXEC_PROPS_ATTR: &'static str = "exec_properties";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:44 PlatformRule.FLAGS_ATTR
    pub const FLAGS_ATTR: &'static str = "flags";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:45 PlatformRule.REQUIRED_SETTINGS_ATTR
    pub const REQUIRED_SETTINGS_ATTR: &'static str = "required_settings";
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:46 PlatformRule.MISSING_TOOLCHAIN_ERROR_ATTR
    pub const MISSING_TOOLCHAIN_ERROR_ATTR: &'static str = "missing_toolchain_error";
}

impl RuleDefinition for PlatformRule {
    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:49 PlatformRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .advertise_starlark_provider(vec![platform_info_id()])
            .exempt_from_constraint_checking("this rule helps *define* a constraint")
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .remove_attribute(":action_listener")
            .remove_attribute(APPLICABLE_METADATA_ATTR)
            .override_(
                attr("tags", Type::StringList)
                    .value(manual_tags())
                    .nonconfigurable("low-level attribute, used in platform configuration"),
            )
            .add(
                attr(Self::CONSTRAINT_VALUES_ATTR, Type::LabelList)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers(vec![constraint_value_info_id()])
                    .nonconfigurable("Part of the configuration"),
            )
            .add(
                attr(Self::PARENTS_PLATFORM_ATTR, Type::LabelList)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers(vec![platform_info_id()])
                    .nonconfigurable("Part of the configuration"),
            )
            .add(
                attr(Self::REMOTE_EXECUTION_PROPS_ATTR, Type::String)
                    .nonconfigurable("Part of the configuration"),
            )
            .add(
                attr(Self::EXEC_PROPS_ATTR, Type::StringDict)
                    .value(Some(AttrValue::StringDict(IndexMap::new())))
                    .nonconfigurable("Part of the configuration"),
            )
            .add(
                attr(Self::FLAGS_ATTR, Type::StringList)
                    .value(Some(AttrValue::StringList(Vec::new())))
                    .nonconfigurable("Part of the configuration"),
            )
            .add(
                attr(Self::REQUIRED_SETTINGS_ATTR, Type::LabelList)
                    .allowed_rule_classes(&["config_setting"])
                    .allowed_file_types(FileTypeSet::NoFile),
            )
            .add(
                attr(Self::MISSING_TOOLCHAIN_ERROR_ATTR, Type::String)
                    .value(Some(AttrValue::String(
                        Self::DEFAULT_MISSING_TOOLCHAIN_ERROR.to_owned(),
                    )))
                    .nonconfigurable("Part of the configuration"),
            )
            .add(attr("check_toolchain_types", Type::Boolean).value(Some(AttrValue::Bool(false))))
            .add(&attr("allowed_toolchain_types", Type::NodepLabelList))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/platform/PlatformRule.java:155 PlatformRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("Platform")
    }

    fn definition_class(&self) -> String {
        "PlatformRule".to_owned()
    }
}

/// `toolchain` (Java `rules/platform/ToolchainRule`).
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:34 ToolchainRule
pub struct ToolchainRule;

impl ToolchainRule {
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:35 ToolchainRule.RULE_NAME
    pub const RULE_NAME: &'static str = "toolchain";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:36 ToolchainRule.TOOLCHAIN_TYPE_ATTR
    pub const TOOLCHAIN_TYPE_ATTR: &'static str = "toolchain_type";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:37 ToolchainRule.EXEC_COMPATIBLE_WITH_ATTR
    pub const EXEC_COMPATIBLE_WITH_ATTR: &'static str = "exec_compatible_with";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:38 ToolchainRule.TARGET_COMPATIBLE_WITH_ATTR
    pub const TARGET_COMPATIBLE_WITH_ATTR: &'static str = "target_compatible_with";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:39 ToolchainRule.TARGET_SETTING_ATTR
    pub const TARGET_SETTING_ATTR: &'static str = "target_settings";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:40 ToolchainRule.TOOLCHAIN_ATTR
    pub const TOOLCHAIN_ATTR: &'static str = "toolchain";
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:41 ToolchainRule.USE_TARGET_PLATFORM_CONSTRAINTS_ATTR
    pub const USE_TARGET_PLATFORM_CONSTRAINTS_ATTR: &'static str =
        "use_target_platform_constraints";
}

impl RuleDefinition for ToolchainRule {
    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:45 ToolchainRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .advertise_provider(&["DeclaredToolchainInfo"])
            .override_(
                attr("tags", Type::StringList)
                    .value(manual_tags())
                    .nonconfigurable("low-level attribute, used in platform configuration"),
            )
            .remove_attribute(":action_listener")
            .exempt_from_constraint_checking("this rule *defines* a constraint")
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .add(
                attr(Self::TOOLCHAIN_TYPE_ATTR, Type::Label)
                    .mandatory()
                    .allowed_file_types(FileTypeSet::NoFile)
                    .allowed_rule_classes(&["toolchain_type"])
                    .mandatory_providers(vec![toolchain_type_info_id()])
                    .nonconfigurable("part of toolchain configuration"),
            )
            .add(
                attr(Self::EXEC_COMPATIBLE_WITH_ATTR, Type::LabelList)
                    .mandatory_providers(vec![constraint_value_info_id()])
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable("part of toolchain configuration"),
            )
            .add(
                attr(Self::TARGET_COMPATIBLE_WITH_ATTR, Type::LabelList)
                    .mandatory_providers(vec![constraint_value_info_id()])
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable("part of toolchain configuration"),
            )
            .add(
                attr(Self::USE_TARGET_PLATFORM_CONSTRAINTS_ATTR, Type::Boolean)
                    .value(Some(AttrValue::Bool(false)))
                    .nonconfigurable("part of toolchain configuration"),
            )
            .add(
                attr(Self::TARGET_SETTING_ATTR, Type::LabelList)
                    .allowed_rule_classes(&["config_setting"])
                    .allowed_file_types(FileTypeSet::NoFile),
            )
            .add(attr(Self::TOOLCHAIN_ATTR, Type::NodepLabel).mandatory())
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:115 ToolchainRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("Toolchain")
    }

    fn definition_class(&self) -> String {
        "ToolchainRule".to_owned()
    }
}
