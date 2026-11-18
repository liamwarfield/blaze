//! Port of `rules/platform/Toolchain.java`: defines a toolchain that can be used by rules.

use blaze_analysis::config::config_matching_provider::ConfigMatchingProvider;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::platform::{
    ConstraintValueInfo, DeclaredToolchainInfo, platform_provider_utils,
};
use blaze_analysis::providers::file_provider::{
    FileProvider, FilesToRunProvider, RunfilesProvider,
};
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext, RuleErrorConsumer};
use blaze_cmdline::Label;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;

use crate::platform::common::build_error;

// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:36 ToolchainRule.TOOLCHAIN_TYPE_ATTR
pub const TOOLCHAIN_TYPE_ATTR: &str = "toolchain_type";
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:37 ToolchainRule.EXEC_COMPATIBLE_WITH_ATTR
pub const EXEC_COMPATIBLE_WITH_ATTR: &str = "exec_compatible_with";
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:38 ToolchainRule.TARGET_COMPATIBLE_WITH_ATTR
pub const TARGET_COMPATIBLE_WITH_ATTR: &str = "target_compatible_with";
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:39 ToolchainRule.TARGET_SETTING_ATTR
pub const TARGET_SETTING_ATTR: &str = "target_settings";
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:40 ToolchainRule.TOOLCHAIN_ATTR
pub const TOOLCHAIN_ATTR: &str = "toolchain";
// Java: com/google/devtools/build/lib/rules/platform/ToolchainRule.java:41 ToolchainRule.USE_TARGET_PLATFORM_CONSTRAINTS_ATTR
pub const USE_TARGET_PLATFORM_CONSTRAINTS_ATTR: &str = "use_target_platform_constraints";

/// Java `Toolchain` (the `toolchain` rule's factory).
// Java: com/google/devtools/build/lib/rules/platform/Toolchain.java:39 Toolchain
pub struct Toolchain;

impl RuleConfiguredTargetFactory for Toolchain {
    // Java: com/google/devtools/build/lib/rules/platform/Toolchain.java:43 Toolchain.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let toolchain_type = rule_context
            .get_prerequisite(TOOLCHAIN_TYPE_ATTR)
            .as_ref()
            .and_then(platform_provider_utils::toolchain_type);
        let exec_constraints: Vec<ConstraintValueInfo> = platform_provider_utils::constraint_values(
            rule_context
                .get_prerequisites(EXEC_COMPATIBLE_WITH_ATTR)
                .iter(),
        );
        let target_constraints: Vec<ConstraintValueInfo> =
            platform_provider_utils::constraint_values(
                rule_context
                    .get_prerequisites(TARGET_COMPATIBLE_WITH_ATTR)
                    .iter(),
            );
        let target_settings: Vec<ConfigMatchingProvider> = rule_context
            .get_prerequisites(TARGET_SETTING_ATTR)
            .iter()
            .map(|target| {
                // Java: ImmutableList rejects the null of a target without the provider.
                target
                    .get_provider::<ConfigMatchingProvider>()
                    .expect("NullPointerException: target_settings without ConfigMatchingProvider")
                    .clone()
            })
            .collect();
        let resolved_toolchain_label: Label = rule_context
            .attributes()
            .get(TOOLCHAIN_ATTR, Type::NodepLabel)
            .and_then(|v| v.as_label().cloned())
            .expect("NullPointerException: toolchain");
        let target_to_exec_constraints: bool = rule_context
            .attributes()
            .get(USE_TARGET_PLATFORM_CONSTRAINTS_ATTR, Type::Boolean)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if target_to_exec_constraints
            && !(exec_constraints.is_empty() && target_constraints.is_empty())
        {
            rule_context.attribute_error(
                USE_TARGET_PLATFORM_CONSTRAINTS_ATTR,
                "Cannot set use_target_platform_constraints to True and also set exec_compatible_with or target_compatible_with",
            );
            return Ok(None);
        }

        let mut registered_toolchain_builder = DeclaredToolchainInfo::builder();
        registered_toolchain_builder
            .toolchain_type(toolchain_type)
            .add_target_settings(target_settings)
            .resolved_toolchain_label(resolved_toolchain_label)
            .target_label(rule_context.get_label().clone());
        let registered_toolchain = if target_to_exec_constraints {
            registered_toolchain_builder.build_with_target_to_exec_constraints()
        } else {
            match registered_toolchain_builder
                .add_exec_constraints(exec_constraints)
                .add_target_constraints(target_constraints)
                .build()
            {
                Ok(t) => t,
                Err(e) => {
                    if let Some(exec) = e.exec_constraints_exception() {
                        rule_context.attribute_error(EXEC_COMPATIBLE_WITH_ATTR, exec.get_message());
                    }
                    if let Some(target) = e.target_constraints_exception() {
                        rule_context
                            .attribute_error(TARGET_COMPATIBLE_WITH_ATTR, target.get_message());
                    }
                    return Ok(None);
                }
            }
        };

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::empty())
            .add_provider_arc(FileProvider::empty())
            .add_provider_arc(FilesToRunProvider::empty())
            .add_provider(registered_toolchain);
        builder.build().map_err(build_error)
    }
}
