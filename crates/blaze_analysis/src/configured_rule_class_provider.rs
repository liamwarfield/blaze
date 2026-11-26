//! The analysis half of `analysis/ConfiguredRuleClassProvider.java`: native configured target
//! factories by name (Java `RuleClass.getConfiguredTargetFactory()`), plus what analysis reads
//! from the provider (the Starlark environment, the tools repository).

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use indexmap::IndexMap;

use crate::configured_target::ConfiguredTarget;
use crate::rule::{RuleContext, RuleErrorException};

/// Errors of `RuleConfiguredTargetFactory.create` (Java: `InterruptedException`,
/// `RuleErrorException`, `ActionConflictException`).
#[derive(Debug, thiserror::Error)]
pub enum CreateConfiguredTargetError {
    #[error(transparent)]
    RuleError(#[from] RuleErrorException),
    /// Java `ActionConflictException` (message).
    #[error("{0}")]
    ActionConflict(crate::rule::action_conflict_exception::ActionConflictException),
    #[error("interrupted")]
    Interrupted,
}

/// Integration adapter: `RuleConfiguredTargetBuilder.build()`'s exceptions propagate unchanged
/// out of `create` in Java.
impl From<crate::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError>
    for CreateConfiguredTargetError
{
    fn from(
        e: crate::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError,
    ) -> Self {
        use crate::rule::rule_configured_target_builder::RuleConfiguredTargetBuildError as E;
        match e {
            E::ActionConflict(m) => CreateConfiguredTargetError::ActionConflict(m),
            E::Interrupted => CreateConfiguredTargetError::Interrupted,
        }
    }
}

/// A native rule's analysis implementation (Java `RuleConfiguredTargetFactory`).
// Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetFactory.java:124 RuleConfiguredTargetFactory
pub trait RuleConfiguredTargetFactory: Send + Sync {
    /// Java `create(RuleContext)`: `Ok(None)` where Java returns null (errors were reported on
    /// the rule context).
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError>;

    /// Adds any rule implementation-specific requirements to the given builder (Java's default
    /// does nothing).
    // Java: com/google/devtools/build/lib/analysis/RuleConfiguredTargetFactory.java:129 RuleConfiguredTargetFactory.addRuleImplSpecificRequiredConfigFragments
    fn add_rule_impl_specific_required_config_fragments(
        &self,
        _required_fragments: &mut crate::providers::required_config_fragments_provider::RequiredConfigFragmentsProviderBuilder,
        _attributes: &crate::config::configured_attribute_mapper::ConfiguredAttributeMapper,
        _configuration: &blaze_build_config::build_configuration_value::BuildConfigurationValue,
    ) {
    }
}

/// The analysis view of Bazel's `ConfiguredRuleClassProvider`.
pub struct ConfiguredRuleClassProvider {
    factories: IndexMap<String, Arc<dyn RuleConfiguredTargetFactory>>,
    /// Java `nativeAspectClassMap`: the native aspect classes by key (`getName()`), with the
    /// `ConfiguredAspectFactory` the Java class also implements.
    native_aspect_classes: IndexMap<String, NativeAspectEntry>,
    /// Java `getToolsRepository()` (`@bazel_tools`).
    pub tools_repository: RepositoryName,
}

impl ConfiguredRuleClassProvider {
    pub fn builder() -> ConfiguredRuleClassProviderBuilder {
        ConfiguredRuleClassProviderBuilder {
            factories: IndexMap::new(),
            native_aspect_classes: IndexMap::new(),
        }
    }

    /// The native aspect classes by key.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:767 ConfiguredRuleClassProvider.getNativeAspectClassMap
    pub fn get_native_aspect_class_map(
        &self,
    ) -> IndexMap<String, Arc<dyn blaze_packages::aspect::NativeAspectClass>> {
        self.native_aspect_classes
            .iter()
            .map(|(k, e)| (k.clone(), e.aspect_class.clone()))
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:772 ConfiguredRuleClassProvider.getNativeAspectClass
    pub fn get_native_aspect_class(
        &self,
        key: &str,
    ) -> Option<Arc<dyn blaze_packages::aspect::NativeAspectClass>> {
        self.native_aspect_classes
            .get(key)
            .map(|e| e.aspect_class.clone())
    }

    /// `(ConfiguredAspectFactory) nativeAspectClass`: the factory of a registered native aspect
    /// class (Java's class implements both interfaces).
    pub fn get_native_aspect_factory(
        &self,
        aspect_class: &dyn blaze_packages::aspect::NativeAspectClass,
    ) -> Option<Arc<dyn crate::configured_aspect_factory::ConfiguredAspectFactory>> {
        self.native_aspect_classes
            .get(&aspect_class.get_key())
            .map(|e| e.factory.clone())
    }

    /// The factory registered under `name` (the native rule class's
    /// `get_configured_target_factory()`).
    pub fn get_configured_target_factory(
        &self,
        name: &str,
    ) -> Option<&Arc<dyn RuleConfiguredTargetFactory>> {
        self.factories.get(name)
    }
}

pub struct ConfiguredRuleClassProviderBuilder {
    factories: IndexMap<String, Arc<dyn RuleConfiguredTargetFactory>>,
    native_aspect_classes: IndexMap<String, NativeAspectEntry>,
}

/// A registered native aspect: its class and its factory.
#[derive(Clone)]
struct NativeAspectEntry {
    aspect_class: Arc<dyn blaze_packages::aspect::NativeAspectClass>,
    factory: Arc<dyn crate::configured_aspect_factory::ConfiguredAspectFactory>,
}

impl ConfiguredRuleClassProviderBuilder {
    pub fn add_factory(
        &mut self,
        name: &str,
        factory: Arc<dyn RuleConfiguredTargetFactory>,
    ) -> &mut Self {
        self.factories.insert(name.to_owned(), factory);
        self
    }

    /// Adds a native aspect class (Java's `NativeAspectClass` that is also its
    /// `ConfiguredAspectFactory`).
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:272 ConfiguredRuleClassProvider.Builder.addNativeAspectClass
    pub fn add_native_aspect_class(
        &mut self,
        aspect_class: Arc<dyn blaze_packages::aspect::NativeAspectClass>,
        factory: Arc<dyn crate::configured_aspect_factory::ConfiguredAspectFactory>,
    ) -> &mut Self {
        self.native_aspect_classes.insert(
            aspect_class.get_key(),
            NativeAspectEntry {
                aspect_class,
                factory,
            },
        );
        self
    }

    pub fn build(self) -> ConfiguredRuleClassProvider {
        ConfiguredRuleClassProvider {
            factories: self.factories,
            native_aspect_classes: self.native_aspect_classes,
            tools_repository: blaze_cmdline::repository_name::BAZEL_TOOLS.clone(),
        }
    }
}
