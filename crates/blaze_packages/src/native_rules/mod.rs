//! Native rule classes and the rule class provider (crates/blaze_packages/DESIGN.md, owner C).
//!
//! Ports of `analysis/RuleDefinition.java`, `analysis/RuleDefinitionEnvironment.java`,
//! `analysis/BaseRuleClasses.java`, the loading-phase subset of
//! `analysis/ConfiguredRuleClassProvider.java` and `bazel/rules/BazelRuleClassProvider.java`
//! (with its rule sets), and the native `RuleDefinition`s Bazel 9.3.2 registers (`bazel info
//! build-language` lists the 43 non-abstract ones). The attribute lists, types, defaults and
//! flags are Bazel's exactly, because `query --output=streamed_jsonproto` prints every
//! attribute of a native rule; `scripts/w3/native-rule-attrs` compares them with the oracle.
//!
//! Configured-target factories are named by their Java class simple name until analysis (W4)
//! ports them (`RuleClassBuilder::factory`).

pub mod base_rule_classes;
pub mod builtin_rule_function;
pub mod cc_rules;
pub mod config_rule_classes;
pub mod generic_rules;
pub mod genrule;
pub mod java_rules;
pub mod platform_rules;
pub mod rule_class_provider;
pub mod toolchain_type;

pub use builtin_rule_function::BuiltinRuleFunctionValue;
pub use rule_class_provider::RuleClassProvider;
pub use rule_class_provider::RuleClassProviderBuilder;

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::RepositoryName;

use crate::attribute::TransitionFactory;
use crate::rule_class::ProviderKey;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassBuilder;
use crate::rule_class::RuleClassType;
use crate::rule_class::StarlarkProviderIdentifier;

/// Ancestors and properties of a native rule class (Java `RuleDefinition.Metadata`).
// Java: com/google/devtools/build/lib/analysis/RuleDefinition.java:65 RuleDefinition.Metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDefinitionMetadata {
    pub name: String,
    pub type_: RuleClassType,
    /// Java `factoryClass`: the simple name of the `RuleConfiguredTargetFactory` class; `None`
    /// is Java's default `RuleConfiguredTargetFactory.class` (no factory, abstract rules).
    pub factory_class: Option<&'static str>,
    /// Java `ancestors`: the [`RuleDefinition::definition_class`] of each ancestor.
    pub ancestors: Vec<String>,
}

impl RuleDefinitionMetadata {
    /// Java `Metadata.builder()` with its defaults (type NORMAL, no factory, no ancestors).
    // Java: com/google/devtools/build/lib/analysis/RuleDefinition.java:77 RuleDefinition.Metadata.builder
    pub fn builder(name: &str) -> RuleDefinitionMetadata {
        RuleDefinitionMetadata {
            name: name.to_owned(),
            type_: RuleClassType::Normal,
            factory_class: None,
            ancestors: Vec::new(),
        }
    }

    pub fn type_(mut self, type_: RuleClassType) -> Self {
        self.type_ = type_;
        self
    }

    pub fn factory_class(mut self, factory_class: &'static str) -> Self {
        self.factory_class = Some(factory_class);
        self
    }

    pub fn ancestors(mut self, ancestors: &[&str]) -> Self {
        self.ancestors = ancestors.iter().map(|a| (*a).to_owned()).collect();
        self
    }
}

/// A native rule class definition (Java `RuleDefinition`).
// Java: com/google/devtools/build/lib/analysis/RuleDefinition.java:33 RuleDefinition
pub trait RuleDefinition: Send + Sync {
    /// Builds the rule class from `builder` (which already holds the ancestors' attributes).
    // Java: com/google/devtools/build/lib/analysis/RuleDefinition.java:46 RuleDefinition.build
    fn build(
        &self,
        builder: &mut RuleClassBuilder,
        env: &dyn RuleDefinitionEnvironment,
    ) -> RuleClass;

    // Java: com/google/devtools/build/lib/analysis/RuleDefinition.java:51 RuleDefinition.getMetadata
    fn get_metadata(&self) -> RuleDefinitionMetadata;

    /// Java `ruleDefinition.getClass().getName()`: the identity of this definition in the
    /// provider's dependency graph. Each `EmptyRule` instance is its own anonymous class in
    /// Java, so it returns a per-instance name.
    fn definition_class(&self) -> String;
}

/// What a rule definition may ask of the provider while building (Java
/// `RuleDefinitionEnvironment`).
// Java: com/google/devtools/build/lib/analysis/RuleDefinitionEnvironment.java:32 RuleDefinitionEnvironment
pub trait RuleDefinitionEnvironment {
    // Java: com/google/devtools/build/lib/analysis/RuleDefinitionEnvironment.java:35 RuleDefinitionEnvironment.getToolsRepository
    fn get_tools_repository(&self) -> RepositoryName;

    /// Prepends the tools repository to `label_value` and parses the result.
    // Java: com/google/devtools/build/lib/analysis/RuleDefinitionEnvironment.java:44 RuleDefinitionEnvironment.getToolsLabel
    fn get_tools_label(&self, label_value: &str) -> Label {
        Label::parse_canonical_unchecked(&format!("{}{}", self.get_tools_repository(), label_value))
    }

    /// Java `getNetworkAllowlistForTests()` (default: none).
    // Java: com/google/devtools/build/lib/analysis/RuleDefinitionEnvironment.java:50 RuleDefinitionEnvironment.getNetworkAllowlistForTests
    fn get_network_allowlist_for_tests(&self) -> Option<Label> {
        None
    }
}

/// Java `ExecutionTransitionFactory.createFactory()`.
// Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:55 ExecutionTransitionFactory.createFactory
pub fn exec_transition_factory() -> TransitionFactory {
    TransitionFactory::Exec { exec_group: None }
}

/// The id of a builtin provider (Java `BuiltinProvider.id()`): `name` is the Starlark name,
/// `provider_class` the Java value class simple name.
// Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:100 BuiltinProvider.id
pub fn builtin_provider_id(name: &str, provider_class: &str) -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Builtin {
        name: name.to_owned(),
        provider_class: provider_class.to_owned(),
    })
}

/// The id of a provider defined in Starlark, wrapped by Java (Java
/// `StarlarkProviderWrapper.id()` over `keyForBuild(label)`).
// Java: com/google/devtools/build/lib/packages/StarlarkProviderWrapper.java:60 StarlarkProviderWrapper.id
pub fn starlark_provider_wrapper_id(bzl_label: &str, name: &str) -> StarlarkProviderIdentifier {
    StarlarkProviderIdentifier::for_key(ProviderKey::Starlark {
        extension_label: Label::parse_canonical_unchecked(bzl_label),
        exported_name: name.to_owned(),
    })
}

/// Java `ConstraintValueInfo.PROVIDER.id()`.
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintValueInfo.java:38 ConstraintValueInfo.PROVIDER
pub fn constraint_value_info_id() -> StarlarkProviderIdentifier {
    builtin_provider_id("ConstraintValueInfo", "ConstraintValueInfo")
}

/// Java `ConstraintSettingInfo.PROVIDER.id()`.
// Java: com/google/devtools/build/lib/analysis/platform/ConstraintSettingInfo.java:35 ConstraintSettingInfo.PROVIDER
pub fn constraint_setting_info_id() -> StarlarkProviderIdentifier {
    builtin_provider_id("ConstraintSettingInfo", "ConstraintSettingInfo")
}

/// Java `PlatformInfo.PROVIDER.id()`.
// Java: com/google/devtools/build/lib/analysis/platform/PlatformInfo.java:71 PlatformInfo.PROVIDER
pub fn platform_info_id() -> StarlarkProviderIdentifier {
    builtin_provider_id("PlatformInfo", "PlatformInfo")
}

/// Java `ToolchainTypeInfo.PROVIDER.id()`.
// Java: com/google/devtools/build/lib/analysis/platform/ToolchainTypeInfo.java:35 ToolchainTypeInfo.PROVIDER
pub fn toolchain_type_info_id() -> StarlarkProviderIdentifier {
    builtin_provider_id("ToolchainTypeInfo", "ToolchainTypeInfo")
}

/// Java `TemplateVariableInfo.PROVIDER.id()`.
// Java: com/google/devtools/build/lib/analysis/TemplateVariableInfo.java:34 TemplateVariableInfo.PROVIDER
pub fn template_variable_info_id() -> StarlarkProviderIdentifier {
    builtin_provider_id("TemplateVariableInfo", "TemplateVariableInfo")
}

/// A shared definition handle.
pub type SharedRuleDefinition = Arc<dyn RuleDefinition>;
