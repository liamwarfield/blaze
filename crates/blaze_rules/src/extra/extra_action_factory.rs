//! Port of `rules/extra/ExtraActionFactory.java`.

use std::sync::Arc;

use blaze_actions::analysis::runfiles::Runfiles;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::extra::extra_action_spec::ExtraActionSpec;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::command_helper::CommandHelper;
use blaze_analysis::rule::configuration_make_variable_context::ConfigurationMakeVariableContext;
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;

use super::build_error;
use crate::generic::target_utils;

/// Factory for 'extra_action'.
// Java: com/google/devtools/build/lib/rules/extra/ExtraActionFactory.java:38 ExtraActionFactory
pub struct ExtraActionFactory;

impl RuleConfiguredTargetFactory for ExtraActionFactory {
    // Java: com/google/devtools/build/lib/rules/extra/ExtraActionFactory.java:41 ExtraActionFactory.create
    fn create(
        &self,
        context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        // This rule doesn't produce any output when listed as a build target.
        // Only when used via the --experimental_action_listener flag,
        // this rule instructs the build system to add additional outputs.
        let context: &RuleContext = context;

        let command_helper = CommandHelper::builder(context)
            .add_tool_dependencies_attribute("tools")
            .build();

        let resolved_data = context.get_prerequisite_artifacts("data").list().to_vec();
        let output_templates: Vec<String> = context
            .attributes()
            .get("out_templates", Type::StringList)
            .and_then(|v| v.as_string_list().map(|l| l.to_vec()))
            .unwrap_or_else(|| panic!("IllegalArgumentException: no attribute 'out_templates'"));

        let mut command: String = context
            .attributes()
            .get("cmd", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| panic!("IllegalArgumentException: no attribute 'cmd'"));
        // This is a bit of a hack. We want to run the MakeVariableExpander first, so we expand $
        // on variables that are expanded below with $$, which gets reverted to $ by the
        // MakeVariableExpander. This allows us to expand package-specific make variables in the
        // package where the extra action is defined, and then later replace the owner-specific
        // make variables when the extra action is instantiated.
        command = command.replace("$(EXTRA_ACTION_FILE)", "$$(EXTRA_ACTION_FILE)");
        command = command.replace("$(ACTION_ID)", "$$(ACTION_ID)");
        command = command.replace("$(OWNER_LABEL_DIGEST)", "$$(OWNER_LABEL_DIGEST)");
        command = command.replace("$(output ", "$$(output ");
        let make_variable_context = ConfigurationMakeVariableContext::new(
            context.get_rule().get_package_declarations(),
            context.get_configuration(),
            context.get_default_template_variable_providers(),
        );
        command = context
            .get_expander_for(Arc::new(make_variable_context))
            .with_data_exec_locations()
            .expand(Some("cmd"), &command);

        let requires_action_output = context
            .attributes()
            .get("requires_action_output", Type::Boolean)
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                panic!("IllegalArgumentException: no attribute 'requires_action_output'")
            });

        if context.has_errors() {
            return Ok(None);
        }
        let spec = ExtraActionSpec::new(
            command_helper.get_resolved_tools().clone(),
            resolved_data,
            output_templates,
            command,
            context.get_label().clone(),
            target_utils::get_execution_info(context.get_rule()),
            requires_action_output,
        );
        let mut builder = RuleConfiguredTargetBuilder::new(context);
        builder
            .add_provider(spec)
            .add_provider(RunfilesProvider::simple(Runfiles::empty()));
        builder.build().map_err(build_error)
    }
}
