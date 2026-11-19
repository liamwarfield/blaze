//! Rule classes: ports of `packages/RuleClass.java` (with `RuleClass.Builder`,
//! `RuleClassType`, `RuleClassNamePredicate`), `RuleClassId`, `ConfigurationFragmentPolicy`,
//! `AdvertisedProviderSet`, `RequiredProviders`, `AspectsList`, `DeclaredExecGroup`,
//! `BuildSetting`, `StarlarkProviderIdentifier`, `Provider.Key`, `RuleFunction`,
//! `RuleTransitionData`, `AllowlistChecker`, and the [`StarlarkCallable`] handle.

pub mod advertised_provider_set;
pub mod allowlist_checker;
pub mod aspects_list;
pub mod build_setting;
pub mod configuration_fragment_policy;
pub mod declared_exec_group;
pub mod provider;
pub mod required_providers;
#[allow(clippy::module_inception)]
pub mod rule_class;
pub mod rule_class_id;
pub mod rule_function;
pub mod rule_transition_data;
pub mod starlark_callable;

pub use advertised_provider_set::AdvertisedProviderSet;
pub use advertised_provider_set::AdvertisedProviderSetBuilder;
pub use allowlist_checker::AllowlistChecker;
pub use allowlist_checker::LocationCheck;
pub use aspects_list::AspectDetails;
pub use aspects_list::AspectParametersExtractor;
pub use aspects_list::AspectsList;
pub use aspects_list::AspectsListBuilder;
pub use aspects_list::StarlarkAspectInfo;
pub use build_setting::BuildSetting;
pub use configuration_fragment_policy::ConfigurationFragmentPolicy;
pub use configuration_fragment_policy::ConfigurationFragmentPolicyBuilder;
pub use configuration_fragment_policy::MissingFragmentPolicy;
pub use declared_exec_group::DEFAULT_EXEC_GROUP_NAME;
pub use declared_exec_group::DeclaredExecGroup;
pub use declared_exec_group::ToolchainTypeRequirement;
pub use provider::ProviderKey;
pub use provider::StarlarkProviderIdentifier;
pub use required_providers::RequiredProviders;
pub use required_providers::RequiredProvidersBuilder;
pub use required_providers::RequiredProvidersConstraint;
pub use rule_class::AutoExecGroupsMode;
pub use rule_class::NAME_ATTRIBUTE;
pub use rule_class::OptionReferenceFunction;
pub use rule_class::RuleClass;
pub use rule_class::RuleClassBuilder;
pub use rule_class::RuleClassCreateRuleError;
pub use rule_class::RuleClassNamePredicate;
pub use rule_class::RuleClassNamePredicateType;
pub use rule_class::RuleClassType;
pub use rule_class::ToolchainResolutionMode;
pub use rule_class_id::RuleClassId;
pub use rule_function::RuleFunction;
pub use rule_transition_data::RuleTransitionData;
pub use starlark_callable::StarlarkCallable;

#[cfg(test)]
mod tests;
