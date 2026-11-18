//! Port of `rules/ToolchainType.java`: implementation of `toolchain_type`.

use blaze_actions::analysis::runfiles::Runfiles;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::info::freeze_native_info;
use blaze_analysis::platform::ToolchainTypeInfo;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;

use crate::platform::common::build_error;

/// Java `ToolchainType` (the `toolchain_type` rule's factory).
// Java: com/google/devtools/build/lib/rules/ToolchainType.java:38 ToolchainType
pub struct ToolchainType;

impl RuleConfiguredTargetFactory for ToolchainType {
    // Java: com/google/devtools/build/lib/rules/ToolchainType.java:42 ToolchainType.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let no_match_error: String = rule_context
            .attributes()
            .get("no_match_error", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        let toolchain_type_info = ToolchainTypeInfo::create_with(
            rule_context.get_label().clone(),
            if no_match_error.is_empty() {
                None
            } else {
                Some(no_match_error)
            },
        );

        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::simple(Runfiles::empty()))
            .add_native_declared_provider(freeze_native_info(toolchain_type_info));
        builder.build().map_err(build_error)
    }
}
