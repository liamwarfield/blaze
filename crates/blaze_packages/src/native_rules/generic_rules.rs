//! The rule definitions of `bazel/rules/GenericRules.java`: `environment`, `alias`,
//! `filegroup`, `test_suite`, `genquery`, `label_setting`, `label_flag`,
//! `starlark_doc_extract`.

use std::sync::Arc;
use std::sync::LazyLock;

use crate::attribute::AllowedValueSet;
use crate::attribute::AttributeMap;
use crate::attribute::ImplicitOutputsFunction;
use crate::attribute::LateBoundDefault;
use crate::attribute::LateBoundDefaultKind;
use crate::attribute::RawAttributeMapper;
use crate::attribute::TransitionFactory;
use crate::attribute::any_rule;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::ENVIRONMENT_RULE;
use crate::native_rules::base_rule_classes::NativeActionCreatingRule;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::rule_class::BuildSetting;
use crate::rule_class::MissingFragmentPolicy;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::rule_class::ToolchainResolutionMode;
use crate::rule_class::rule_class::CONFIG_SETTING_DEPS_ATTRIBUTE;
use crate::rule_class::rule_class::STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME;
use crate::rule_class::rule_class::TARGET_COMPATIBLE_WITH_ATTR;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::FileType;
use crate::types::FileTypeSet;
use crate::types::Type;

/// `environment` (Java `analysis/constraints/EnvironmentRule`).
// Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentRule.java:34 EnvironmentRule
pub struct EnvironmentRule;

impl EnvironmentRule {
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentRule.java:36 EnvironmentRule.FULFILLS_ATTRIBUTE
    pub const FULFILLS_ATTRIBUTE: &'static str = "fulfills";
}

impl RuleDefinition for EnvironmentRule {
    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentRule.java:39 EnvironmentRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .cfg(TransitionFactory::no_config_transition())
            .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
            .override_(
                attr("tags", Type::StringList)
                    .value(Some(AttrValue::StringList(vec!["manual".to_owned()])))
                    .nonconfigurable(
                        "low-level attribute, used in TargetUtils without configurations",
                    ),
            )
            .add(
                attr(Self::FULFILLS_ATTRIBUTE, Type::LabelList)
                    .allowed_rule_classes(&[ENVIRONMENT_RULE])
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable(
                        "used for defining constraint models - this shouldn't be configured",
                    ),
            )
            .exempt_from_constraint_checking("this rule *defines* a constraint")
            .set_undocumented()
            .build()
    }

    // Java: com/google/devtools/build/lib/analysis/constraints/EnvironmentRule.java:73 EnvironmentRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(ENVIRONMENT_RULE)
            .type_(RuleClassType::BuildOnly)
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("Environment")
    }

    fn definition_class(&self) -> String {
        "EnvironmentRule".to_owned()
    }
}

/// `alias` (Java `rules/Alias.AliasRule`).
// Java: com/google/devtools/build/lib/rules/Alias.java:61 Alias.AliasRule
pub struct AliasRule;

impl AliasRule {
    // Java: com/google/devtools/build/lib/rules/Alias.java:37 Alias.RULE_NAME
    pub const RULE_NAME: &'static str = "alias";
    // Java: com/google/devtools/build/lib/rules/Alias.java:38 Alias.ACTUAL_ATTRIBUTE_NAME
    pub const ACTUAL_ATTRIBUTE_NAME: &'static str = "actual";
}

/// Java `!RawAttributeMapper.of(rule).get(attr, LABEL_LIST).isEmpty()` for a present attribute.
fn has_non_empty_label_list(attr_map: &RawAttributeMapper<'_>, name: &str) -> bool {
    attr_map.has(name)
        && attr_map
            .get(name, Type::LabelList)
            .as_ref()
            .and_then(AttrValue::as_label_list)
            .is_some_and(|l| !l.is_empty())
}

impl RuleDefinition for AliasRule {
    // Java: com/google/devtools/build/lib/rules/Alias.java:63 Alias.AliasRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        // If transitions are added here, AspectFunction should be modified to follow all
        // configurations along alias chains.
        builder
            .remove_attribute("licenses")
            .remove_attribute("distribs")
            .remove_attribute(":action_listener")
            .add(
                attr(Self::ACTUAL_ATTRIBUTE_NAME, Type::Label)
                    .allowed_file_types(FileTypeSet::AnyFile)
                    .allowed_rule_classes_predicate(any_rule())
                    .mandatory(),
            )
            .can_have_any_provider()
            // Aliases themselves do not need toolchains or an execution platform, so this is
            // fine. The actual target will resolve platforms and toolchains with no issues
            // regardless of this setting. In some circumstances an alias directly needs the
            // platform:
            // - when it has a select() on a constraint_setting, or
            // - when it has a target_compatible_with attribute.
            // Special-case enable those instances too.
            .toolchain_resolution_mode(ToolchainResolutionMode::Custom(Arc::new(|rule: &Rule| {
                let attr_map = RawAttributeMapper::of(rule as &dyn RuleOrMacroInstance);
                has_non_empty_label_list(&attr_map, CONFIG_SETTING_DEPS_ATTRIBUTE)
                    || has_non_empty_label_list(&attr_map, TARGET_COMPATIBLE_WITH_ATTR)
            })))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/Alias.java:100 Alias.AliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder(Self::RULE_NAME)
            .factory_class("Alias")
            .ancestors(&[NativeBuildRule::CLASS])
    }

    fn definition_class(&self) -> String {
        "Alias.AliasRule".to_owned()
    }
}

/// `filegroup` (Java `rules/filegroup/FilegroupRule`).
// Java: com/google/devtools/build/lib/rules/filegroup/FilegroupRule.java:28 FilegroupRule
pub struct FilegroupRule;

impl RuleDefinition for FilegroupRule {
    // Java: com/google/devtools/build/lib/rules/filegroup/FilegroupRule.java:30 FilegroupRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        // filegroup ignores any filtering set with setSrcsAllowedFiles.
        builder
            .add(attr("srcs", Type::LabelList).allowed_file_types(FileTypeSet::AnyFile))
            .add(&attr("output_group", Type::String))
            .add(
                attr("data", Type::LabelList)
                    .allowed_file_types(FileTypeSet::AnyFile)
                    .dont_check_constraints(),
            )
            .add(&attr("output_licenses", Type::StringList))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/filegroup/FilegroupRule.java:75 FilegroupRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("filegroup")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("Filegroup")
    }

    fn definition_class(&self) -> String {
        "FilegroupRule".to_owned()
    }
}

/// `test_suite` (Java `rules/test/TestSuiteRule`).
// Java: com/google/devtools/build/lib/rules/test/TestSuiteRule.java:28 TestSuiteRule
pub struct TestSuiteRule;

impl RuleDefinition for TestSuiteRule {
    // Java: com/google/devtools/build/lib/rules/test/TestSuiteRule.java:30 TestSuiteRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .requires_configuration_fragments(&["TestConfiguration"])
            // Test suite's don't actually use the test configuration, so don't fail if missing
            .set_missing_fragment_policy("TestConfiguration", MissingFragmentPolicy::Ignore)
            .override_(
                attr("testonly", Type::Boolean)
                    .value(Some(AttrValue::Bool(true)))
                    .nonconfigurable("policy decision: should be consistent across configurations"),
            )
            .add(
                attr("tests", Type::LabelList)
                    .order_independent()
                    .allowed_file_types(FileTypeSet::NoFile)
                    .nonconfigurable("policy decision: should be consistent across configurations"),
            )
            // This magic attribute contains all *test rules in the package, iff
            // tests=[].
            .add(
                attr("$implicit_tests", Type::LabelList)
                    .order_independent()
                    .nonconfigurable("Accessed in TestTargetUtils without config context"),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/test/TestSuiteRule.java:114 TestSuiteRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("test_suite")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("TestSuite")
    }

    fn definition_class(&self) -> String {
        "TestSuiteRule".to_owned()
    }
}

/// `genquery` (Java `rules/genquery/GenQueryRule`).
// Java: com/google/devtools/build/lib/rules/genquery/GenQueryRule.java:30 GenQueryRule
pub struct GenQueryRule;

impl RuleDefinition for GenQueryRule {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryRule.java:38 GenQueryRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .add(
                attr("scope", Type::GenqueryScopeTypeList)
                    .mandatory()
                    .legacy_allow_any_file_type(),
            )
            .add(attr("strict", Type::Boolean).value(Some(AttrValue::Bool(true))))
            .add(attr("expression", Type::String).mandatory())
            .add(&attr("opts", Type::StringList))
            .add(attr("compressed_output", Type::Boolean).value(Some(AttrValue::Bool(false))))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQueryRule.java:79 GenQueryRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("genquery")
            .ancestors(&[NativeActionCreatingRule::CLASS])
            .factory_class("GenQuery")
    }

    fn definition_class(&self) -> String {
        "GenQueryRule".to_owned()
    }
}

// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:58 LabelBuildSettings.NONCONFIGURABLE_ATTRIBUTE_REASON
const LABEL_BUILD_SETTINGS_NONCONFIGURABLE_ATTRIBUTE_REASON: &str =
    "part of a rule class that *triggers* configurable behavior";

/// Java `Scope.ScopeType.allowedAttributeValues()`: the lower-case scope names except
/// `"default"`.
// PORT: move to the port of analysis/config/Scope.java (W4).
// Java: com/google/devtools/build/lib/analysis/config/Scope.java:54 Scope.ScopeType.allowedAttributeValues
pub fn scope_type_allowed_attribute_values() -> Vec<AttrValue> {
    ["universal", "target", "project"]
        .iter()
        .map(|s| AttrValue::String((*s).to_owned()))
        .collect()
}

/// Java `LabelBuildSettings.ACTUAL`: the `build_setting_default` value, or the command-line
/// value during analysis.
// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:62 LabelBuildSettings.ACTUAL
static LABEL_BUILD_SETTINGS_ACTUAL: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    // Java: LabelLateBoundDefault.fromTargetConfigurationWithRuleBasedDefault: the loading-phase
    // default is `RawAttributeMapper.of(rule).get("build_setting_default", NODEP_LABEL)`.
    Arc::new(LateBoundDefault::new(
        Some("BuildConfigurationValue".to_owned()),
        Arc::new(|rule: Option<&dyn RuleOrMacroInstance>| {
            let rule = rule?;
            RawAttributeMapper::of(rule)
                .get(STARLARK_BUILD_SETTING_DEFAULT_ATTR_NAME, Type::NodepLabel)
        }),
        LateBoundDefaultKind::Label,
    ))
});

// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:86 LabelBuildSettings.buildRuleClass
fn label_build_settings_build_rule_class(builder: &mut RuleClassBuilder, flag: bool) -> RuleClass {
    builder
        .remove_attribute("licenses")
        .remove_attribute("distribs")
        .remove_attribute(":action_listener")
        .add(attr(":alias", Type::Label).value_late_bound(LABEL_BUILD_SETTINGS_ACTUAL.clone()))
        .add(
            attr("scope", Type::String)
                .value(Some(AttrValue::String("universal".to_owned())))
                .nonconfigurable(LABEL_BUILD_SETTINGS_NONCONFIGURABLE_ATTRIBUTE_REASON)
                .allowed_values(Arc::new(AllowedValueSet::new(
                    scope_type_allowed_attribute_values(),
                ))),
        )
        .set_build_setting(BuildSetting::create(flag, Type::NodepLabel))
        .can_have_any_provider()
        .toolchain_resolution_mode(ToolchainResolutionMode::Disabled)
        .build()
}

/// `label_setting` (Java `LabelBuildSettings.LabelBuildSettingRule`).
// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:104 LabelBuildSettings.LabelBuildSettingRule
pub struct LabelBuildSettingRule;

impl RuleDefinition for LabelBuildSettingRule {
    // Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:111 LabelBuildSettings.LabelBuildSettingRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        label_build_settings_build_rule_class(builder, /* flag= */ false)
    }

    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:138 LateBoundAlias.AbstractAliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("label_setting")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("LateBoundAlias")
    }

    fn definition_class(&self) -> String {
        "LabelBuildSettings.LabelBuildSettingRule".to_owned()
    }
}

/// `label_flag` (Java `LabelBuildSettings.LabelBuildFlagRule`).
// Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:117 LabelBuildSettings.LabelBuildFlagRule
pub struct LabelBuildFlagRule;

impl RuleDefinition for LabelBuildFlagRule {
    // Java: com/google/devtools/build/lib/rules/LabelBuildSettings.java:124 LabelBuildSettings.LabelBuildFlagRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        label_build_settings_build_rule_class(builder, /* flag= */ true)
    }

    // Java: com/google/devtools/build/lib/rules/LateBoundAlias.java:138 LateBoundAlias.AbstractAliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("label_flag")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("LateBoundAlias")
    }

    fn definition_class(&self) -> String {
        "LabelBuildSettings.LabelBuildFlagRule".to_owned()
    }
}

/// `starlark_doc_extract` (Java `rules/starlarkdocextract/StarlarkDocExtractRule`).
// Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractRule.java:33 StarlarkDocExtractRule
pub struct StarlarkDocExtractRule;

impl RuleDefinition for StarlarkDocExtractRule {
    // Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractRule.java:36 StarlarkDocExtractRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        let bzl_or_scl = || FileTypeSet::of(vec![FileType::of(&[".bzl"]), FileType::of(&[".scl"])]);
        builder
            .add(
                attr("src", Type::Label)
                    .allowed_file_types(bzl_or_scl())
                    .single_artifact()
                    .mandatory(),
            )
            .override_(attr("deps", Type::LabelList).allowed_file_types(bzl_or_scl()))
            .add(
                attr("symbol_names", Type::StringList)
                    .value(Some(AttrValue::StringList(Vec::new()))),
            )
            .add(attr("render_main_repo_name", Type::Boolean).value(Some(AttrValue::Bool(false))))
            .add(
                attr("allow_unused_doc_comments", Type::Boolean)
                    .value(Some(AttrValue::Bool(false))),
            )
            // Java: StarlarkDocExtract.BINARYPROTO_OUT, StarlarkDocExtract.TEXTPROTO_OUT
            .set_implicit_outputs_function(ImplicitOutputsFunction::from_functions(vec![
                ImplicitOutputsFunction::from_templates(&["%{name}.binaryproto"]),
                ImplicitOutputsFunction::from_templates(&["%{name}.textproto"]),
            ]))
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractRule.java:131 StarlarkDocExtractRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("starlark_doc_extract")
            .ancestors(&[NativeActionCreatingRule::CLASS])
            .factory_class("StarlarkDocExtract")
    }

    fn definition_class(&self) -> String {
        "StarlarkDocExtractRule".to_owned()
    }
}
