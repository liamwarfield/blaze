//! Port of `analysis/ConfiguredAspectFactory.java`.

use blaze_cmdline::{Label, RepositoryName};
use blaze_packages::aspect::AspectParameters;

use crate::configured_aspect::ConfiguredAspect;
use crate::configured_rule_class_provider::CreateConfiguredTargetError;
use crate::configured_target::ConfiguredTarget;
use crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProviderBuilder;
use crate::rule::rule_context::RuleContext;

/// Creates the Skyframe node of an aspect.
// Java: com/google/devtools/build/lib/analysis/ConfiguredAspectFactory.java:25 ConfiguredAspectFactory
pub trait ConfiguredAspectFactory: Send + Sync {
    /// Creates the aspect based on the configured target of the associated rule.
    ///
    /// - `target_label`: label of the associated target
    /// - `ct`: the ConfiguredTarget of the associated rule
    /// - `context`: the context of the associated configured target plus all the attributes the
    ///   aspect itself has defined
    /// - `parameters`: information from attributes of the rule that have requested this
    /// - `tools_repository`: the name of the tools repository such as "@bazel_tools"
    ///
    /// `Ok(None)` where Java returns null (errors were reported on the rule context). The error
    /// is Java's `ActionConflictException`, `InterruptedException` or `RuleErrorException`
    /// (the same set as `RuleConfiguredTargetFactory.create`).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspectFactory.java:36 ConfiguredAspectFactory.create
    fn create(
        &self,
        target_label: &Label,
        ct: &ConfiguredTarget,
        context: &mut RuleContext,
        parameters: &AspectParameters,
        tools_repository: &RepositoryName,
    ) -> Result<Option<ConfiguredAspect>, CreateConfiguredTargetError>;

    /// Adds any aspect implementation-specific requirements to the given builder.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredAspectFactory.java:45 ConfiguredAspectFactory.addAspectImplSpecificRequiredConfigFragments
    fn add_aspect_impl_specific_required_config_fragments(
        &self,
        _required_fragments: &mut RequiredConfigFragmentsProviderBuilder,
    ) {
    }
}
