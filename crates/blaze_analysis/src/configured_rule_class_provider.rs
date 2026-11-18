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
    ActionConflict(String),
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
}

/// The analysis view of Bazel's `ConfiguredRuleClassProvider`.
pub struct ConfiguredRuleClassProvider {
    factories: IndexMap<String, Arc<dyn RuleConfiguredTargetFactory>>,
    /// Java `getToolsRepository()` (`@bazel_tools`).
    pub tools_repository: RepositoryName,
}

impl ConfiguredRuleClassProvider {
    pub fn builder() -> ConfiguredRuleClassProviderBuilder {
        ConfiguredRuleClassProviderBuilder {
            factories: IndexMap::new(),
        }
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

    pub fn build(self) -> ConfiguredRuleClassProvider {
        ConfiguredRuleClassProvider {
            factories: self.factories,
            tools_repository: blaze_cmdline::repository_name::BAZEL_TOOLS.clone(),
        }
    }
}
