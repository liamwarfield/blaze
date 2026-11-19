//! Port of `analysis/constraints/Environment.java`: the configured target factory of the
//! `environment` rule.

use std::sync::Arc;

use crate::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use crate::configured_target::ConfiguredTarget;
use crate::constraints::environment_collection::EnvironmentCollectionBuilder;
use crate::constraints::supported_environments::SupportedEnvironments;
use crate::constraints::top_level_constraint_semantics::get_environment_group;
use crate::providers::file_provider::{FileProvider, FilesToRunProvider, RunfilesProvider};
use crate::rule::rule_configured_target_builder::{
    RuleConfiguredTargetBuildError, RuleConfiguredTargetBuilder,
};
use crate::rule::rule_context::RuleContext;
use crate::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_packages::target::Target;

/// Implementation for the environment rule.
// Java: com/google/devtools/build/lib/analysis/constraints/Environment.java:33 Environment
pub struct Environment;

impl RuleConfiguredTargetFactory for Environment {
    // Java: com/google/devtools/build/lib/analysis/constraints/Environment.java:37 Environment.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        // The main analysis work to do here is to simply fill in SupportedEnvironmentsProvider to
        // pass the environment itself to depending rules.
        let label = rule_context.get_label().clone();
        // ConstraintSemantics.getEnvironmentGroup(ruleContext.getRule()): the groups of the
        // rule's package.
        let package = rule_context
            .get_analysis_environment()
            .get_already_loaded_package(label.get_package_identifier())
            .clone();
        let groups: Vec<_> = package
            .get_targets()
            .values()
            .filter_map(|t| t.as_environment_group().cloned())
            .collect();
        let group =
            match get_environment_group(&Target::Rule(rule_context.get_rule().clone()), &groups) {
                Ok(group) => group,
                Err(e) => {
                    rule_context.rule_error(&e.message);
                    return Ok(None);
                }
            };
        let env = EnvironmentCollectionBuilder::new()
            .put(&Arc::new(group.get_environment_labels().clone()), label)
            .build();
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(SupportedEnvironments::create(
                env.clone(),
                env,
                indexmap::IndexMap::new(),
            ))
            .add_provider(RunfilesProvider::empty())
            .add_provider_arc(FileProvider::empty())
            .add_provider_arc(FilesToRunProvider::empty());
        builder.build().map_err(|e| match e {
            RuleConfiguredTargetBuildError::ActionConflict(m) => {
                CreateConfiguredTargetError::ActionConflict(m)
            }
            RuleConfiguredTargetBuildError::Interrupted => CreateConfiguredTargetError::Interrupted,
        })
    }
}
