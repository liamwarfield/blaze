//! The native extra-action rules (`lib/rules/extra/`): the factories of `action_listener`
//! ([`action_listener::ActionListener`]) and `extra_action`
//! ([`extra_action_factory::ExtraActionFactory`]). The rule definitions are in
//! `blaze_packages::native_rules::java_rules`.

use std::sync::Arc;

use blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder;

pub mod action_listener;
pub mod extra_action_factory;

/// `RuleConfiguredTargetBuilder.build()`'s exceptions, propagated unchanged by `create`.
fn build_error(
    e: blaze_analysis::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError,
) -> blaze_analysis::configured_rule_class_provider::CreateConfiguredTargetError {
    e.into()
}

/// Adds the factories (Java: `ExtraActionRule` and `ActionListenerRule` with their
/// `factoryClass`, registered by `BazelRuleClassProvider`).
pub fn register(builder: &mut ConfiguredRuleClassProviderBuilder) {
    builder.add_factory(
        "ExtraActionFactory",
        Arc::new(extra_action_factory::ExtraActionFactory),
    );
    builder.add_factory("ActionListener", Arc::new(action_listener::ActionListener));
}
