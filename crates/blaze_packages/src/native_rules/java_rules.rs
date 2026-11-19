//! The native Java-related rule definitions of `bazel/rules/JavaRules.java` that are not
//! `EmptyRule`s: `java_plugins_flag_alias` (Java `rules/java/JavaPluginsFlagAliasRule`),
//! `extra_action` and `action_listener` (Java `rules/extra/`).

use std::sync::Arc;
use std::sync::LazyLock;

use crate::attribute::LateBoundDefault;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::MakeVariableExpandingRule;
use crate::native_rules::base_rule_classes::NativeActionCreatingRule;
use crate::native_rules::base_rule_classes::NativeBuildRule;
use crate::native_rules::exec_transition_factory;
use crate::native_rules::starlark_provider_wrapper_id;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::types::FileTypeSet;
use crate::types::Type;

/// Java `JavaSemantics.JAVA_PLUGINS` (the `--plugin` labels, resolved during analysis).
// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:50 JavaSemantics.JAVA_PLUGINS
pub static JAVA_PLUGINS: LazyLock<Arc<LateBoundDefault>> = LazyLock::new(|| {
    Arc::new(
        LateBoundDefault::label_list(Some("JavaConfiguration"))
            .with_resolver_name("JavaSemantics.JAVA_PLUGINS"),
    )
});

// Java: com/google/devtools/build/lib/rules/java/JavaSemantics.java:32 JavaSemantics.RULES_JAVA_PROVIDER_LABELS_PREFIX
const RULES_JAVA_PROVIDER_LABELS_PREFIX: &str = "@@rules_java+//";

/// `java_plugins_flag_alias` (Java `rules/java/JavaPluginsFlagAliasRule`).
// Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:40 JavaPluginsFlagAliasRule
pub struct JavaPluginsFlagAliasRule;

impl RuleDefinition for JavaPluginsFlagAliasRule {
    // Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:48 JavaPluginsFlagAliasRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        // Java: JavaPluginInfo.PROVIDER and JavaPluginInfo.RULES_JAVA_PROVIDER
        let java_plugin_info = starlark_provider_wrapper_id(
            &format!("{RULES_JAVA_PROVIDER_LABELS_PREFIX}java/private:java_info.bzl"),
            "JavaPluginInfo",
        );
        let rules_java_java_plugin_info =
            starlark_provider_wrapper_id("//java/private:java_info.bzl", "JavaPluginInfo");
        builder
            .add(
                attr(":java_plugins", Type::LabelList)
                    .cfg(exec_transition_factory())
                    .mandatory_providers_list(vec![
                        vec![java_plugin_info],
                        vec![rules_java_java_plugin_info],
                    ])
                    .silent_rule_class_filter()
                    .value_late_bound(JAVA_PLUGINS.clone()),
            )
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaPluginsFlagAliasRule.java:63 JavaPluginsFlagAliasRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("java_plugins_flag_alias")
            .ancestors(&[NativeBuildRule::CLASS])
            .factory_class("JavaPluginsFlagAlias")
    }

    fn definition_class(&self) -> String {
        "JavaPluginsFlagAliasRule".to_owned()
    }
}

/// `extra_action` (Java `rules/extra/ExtraActionRule`).
// Java: com/google/devtools/build/lib/rules/extra/ExtraActionRule.java:31 ExtraActionRule
pub struct ExtraActionRule;

impl RuleDefinition for ExtraActionRule {
    // Java: com/google/devtools/build/lib/rules/extra/ExtraActionRule.java:33 ExtraActionRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .add(
                attr("tools", Type::LabelList)
                    .cfg(exec_transition_factory())
                    .allowed_file_types(FileTypeSet::NoFile)
                    .exec(),
            )
            .add(&attr("out_templates", Type::StringList))
            .add(attr("cmd", Type::String).mandatory())
            .add(&attr("requires_action_output", Type::Boolean))
            .remove_attribute("deps")
            .remove_attribute(":action_listener")
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/extra/ExtraActionRule.java:123 ExtraActionRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("extra_action")
            .ancestors(&[
                NativeActionCreatingRule::CLASS,
                MakeVariableExpandingRule::CLASS,
            ])
            .factory_class("ExtraActionFactory")
    }

    fn definition_class(&self) -> String {
        "ExtraActionRule".to_owned()
    }
}

/// `action_listener` (Java `rules/extra/ActionListenerRule`).
// Java: com/google/devtools/build/lib/rules/extra/ActionListenerRule.java:28 ActionListenerRule
pub struct ActionListenerRule;

impl RuleDefinition for ActionListenerRule {
    // Java: com/google/devtools/build/lib/rules/extra/ActionListenerRule.java:30 ActionListenerRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .add(attr("mnemonics", Type::StringList).mandatory())
            .add(
                attr("extra_actions", Type::LabelList)
                    .mandatory()
                    .allowed_rule_classes(&["extra_action"])
                    .allowed_file_types(FileTypeSet::NoFile),
            )
            .remove_attribute("deps")
            .remove_attribute("data")
            .remove_attribute(":action_listener")
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/extra/ActionListenerRule.java:56 ActionListenerRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("action_listener")
            .ancestors(&[NativeActionCreatingRule::CLASS])
            .factory_class("ActionListener")
    }

    fn definition_class(&self) -> String {
        "ActionListenerRule".to_owned()
    }
}
