//! `$genrule_base` (Java `rules/genrule/GenRuleBaseRule`) and `genrule` (Java
//! `bazel/rules/genrule/BazelGenRuleRule`).

use std::sync::Arc;
use std::sync::LazyLock;

use crate::attribute::AttributeMap;
use crate::attribute::ComputedDefault;
use crate::attribute::attr;
use crate::native_rules::RuleDefinition;
use crate::native_rules::RuleDefinitionEnvironment;
use crate::native_rules::RuleDefinitionMetadata;
use crate::native_rules::base_rule_classes::MakeVariableExpandingRule;
use crate::native_rules::base_rule_classes::NativeActionCreatingRule;
use crate::native_rules::exec_transition_factory;
use crate::native_rules::template_variable_info_id;
use crate::native_rules::toolchain_type_info_id;
use crate::rule_class::AutoExecGroupsMode;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::types::AttrValue;
use crate::types::FileTypeSet;
use crate::types::TriState;
use crate::types::Type;

/// Java `Rule.IS_EXECUTABLE_ATTRIBUTE_NAME`.
// Java: com/google/devtools/build/lib/packages/Rule.java:83 Rule.IS_EXECUTABLE_ATTRIBUTE_NAME
const IS_EXECUTABLE_ATTRIBUTE_NAME: &str = "$is_executable";

/// The `$is_executable` computed default: one output and `executable = True`.
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBaseRule.java:47 GenRuleBaseRule.build
static IS_EXECUTABLE_DEFAULT: LazyLock<Arc<ComputedDefault>> = LazyLock::new(|| {
    Arc::new(ComputedDefault::new_native(
        Vec::new(),
        Arc::new(|rule: &dyn AttributeMap| {
            let outs = rule.get("outs", Type::OutputList);
            let one_out = outs
                .as_ref()
                .and_then(AttrValue::as_label_list)
                .is_some_and(|l| l.len() == 1);
            let executable = rule
                .get("executable", Type::Boolean)
                .as_ref()
                .and_then(AttrValue::as_bool)
                .unwrap_or(false);
            Some(AttrValue::Bool(one_out && executable))
        }),
    ))
});

/// The abstract base of genrules (Java `GenRuleBaseRule`).
// Java: com/google/devtools/build/lib/rules/genrule/GenRuleBaseRule.java:44 GenRuleBaseRule
pub struct GenRuleBaseRule;

impl GenRuleBaseRule {
    pub const CLASS: &'static str = "GenRuleBaseRule";
}

impl RuleDefinition for GenRuleBaseRule {
    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBaseRule.java:47 GenRuleBaseRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        _env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        let executable_reason = format!(
            "Used in computed default for {IS_EXECUTABLE_ATTRIBUTE_NAME} which is itself \
             non-configurable (and thus expects its dependencies to be non-configurable), because \
             {IS_EXECUTABLE_ATTRIBUTE_NAME} is called from  RunCommand.isExecutable, which has no \
             configuration context"
        );
        builder
            .auto_exec_groups_mode(AutoExecGroupsMode::Disabled)
            .add(
                attr("srcs", Type::LabelList)
                    .direct_compile_time_input()
                    .allowed_file_types(FileTypeSet::AnyFile),
            )
            .add(
                attr("tools", Type::LabelList)
                    .cfg(exec_transition_factory())
                    .allowed_file_types(FileTypeSet::AnyFile),
            )
            .add(attr("outs", Type::OutputList).mandatory())
            .add(&attr("cmd", Type::String))
            .add(&attr("cmd_bash", Type::String))
            .add(&attr("cmd_bat", Type::String))
            .add(&attr("cmd_ps", Type::String))
            .add(
                attr("output_to_bindir", Type::Boolean)
                    .value(Some(AttrValue::Bool(false)))
                    .nonconfigurable(
                        "policy decision: no reason for this to depend on the configuration",
                    ),
            )
            .add(attr("local", Type::Boolean).value(Some(AttrValue::Bool(false))))
            .add(&attr("message", Type::String))
            .add(&attr("output_licenses", Type::StringList))
            .add(
                attr("executable", Type::Boolean)
                    .value(Some(AttrValue::Bool(false)))
                    .nonconfigurable(&executable_reason),
            )
            .add(
                attr(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
                    .nonconfigurable("Called from RunCommand.isExecutable, which takes a Target")
                    .value_computed_default(IS_EXECUTABLE_DEFAULT.clone()),
            )
            .add(
                attr("heuristic_label_expansion", Type::Boolean)
                    .value(Some(AttrValue::Bool(false))),
            )
            // This is a misfeature, so don't document it. We would like to get rid of it, but
            // that would require a cleanup of existing rules.
            .override_(
                attr("toolchains", Type::LabelList)
                    .allowed_file_types(FileTypeSet::NoFile)
                    .mandatory_providers_list(vec![
                        vec![template_variable_info_id()],
                        vec![toolchain_type_info_id()],
                    ])
                    .dont_check_constraints()
                    .nonconfigurable("Don't allow dynamic toolchain types"),
            )
            .remove_attribute("data")
            .remove_attribute("deps")
            .build()
    }

    // Java: com/google/devtools/build/lib/rules/genrule/GenRuleBaseRule.java:340 GenRuleBaseRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("$genrule_base")
            .type_(RuleClassType::Abstract)
            .ancestors(&[
                NativeActionCreatingRule::CLASS,
                MakeVariableExpandingRule::CLASS,
            ])
    }

    fn definition_class(&self) -> String {
        Self::CLASS.to_owned()
    }
}

/// `genrule` (Java `BazelGenRuleRule`).
// Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:30 BazelGenRuleRule
pub struct BazelGenRuleRule;

impl BazelGenRuleRule {
    // Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:31 BazelGenRuleRule.GENRULE_SETUP_LABEL
    const GENRULE_SETUP_LABEL: &'static str = "//tools/genrule:genrule-setup.sh";
}

impl RuleDefinition for BazelGenRuleRule {
    // Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:34 BazelGenRuleRule.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass {
        builder
            .set_output_to_genfiles()
            .add(
                attr("$genrule_setup", Type::Label)
                    .cfg(exec_transition_factory())
                    .value(Some(AttrValue::Label(
                        env.get_tools_label(Self::GENRULE_SETUP_LABEL),
                    ))),
            )
            // TODO(bazel-team): stamping doesn't seem to work. Fix it or remove attribute.
            .add(attr("stamp", Type::Tristate).value(Some(AttrValue::TriState(TriState::No))))
            .build()
    }

    // Java: com/google/devtools/build/lib/bazel/rules/genrule/BazelGenRuleRule.java:52 BazelGenRuleRule.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata::builder("genrule")
            .ancestors(&[GenRuleBaseRule::CLASS])
            .factory_class("BazelGenRule")
    }

    fn definition_class(&self) -> String {
        "BazelGenRuleRule".to_owned()
    }
}
