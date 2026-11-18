//! Helpers shared by the platform rule factories.

use blaze_analysis::configured_rule_class_provider::CreateConfiguredTargetError;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::{
    FileProvider, FilesToRunProvider, RunfilesProvider,
};
use blaze_analysis::rule::RuleConfiguredTargetBuilder;
use blaze_analysis::rule::RuleContext;
use blaze_analysis::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError;
use starlark::values::OwnedFrozenValue;

/// Java `RuleConfiguredTargetBuilder.build()`'s exceptions as `RuleConfiguredTargetFactory`
/// exceptions.
pub(crate) fn build_error(e: RuleConfiguredTargetBuildError) -> CreateConfiguredTargetError {
    match e {
        RuleConfiguredTargetBuildError::ActionConflict(m) => {
            CreateConfiguredTargetError::ActionConflict(m)
        }
        RuleConfiguredTargetBuildError::Interrupted => CreateConfiguredTargetError::Interrupted,
    }
}

/// The common tail of the platform factories: `new RuleConfiguredTargetBuilder(ruleContext)
/// .addProvider(RunfilesProvider.class, RunfilesProvider.EMPTY)
/// .addProvider(FileProvider.class, FileProvider.EMPTY)
/// .addProvider(FilesToRunProvider.class, FilesToRunProvider.EMPTY)
/// .addNativeDeclaredProvider(info).build()`.
pub(crate) fn build_with_empty_providers(
    rule_context: &RuleContext,
    info: OwnedFrozenValue,
) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
    let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
    builder
        .add_provider(RunfilesProvider::empty())
        .add_provider_arc(FileProvider::empty())
        .add_provider_arc(FilesToRunProvider::empty())
        .add_native_declared_provider(info);
    builder.build().map_err(build_error)
}
