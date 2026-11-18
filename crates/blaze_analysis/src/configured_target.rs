//! Port of `analysis/ConfiguredTarget.java` (the interface) as a closed enum over the Java
//! implementations in `analysis/configuredtargets/` (agent R owns their structs).

use std::sync::Arc;

use blaze_actions::ActionLookupKeyRef;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use starlark::values::OwnedFrozenValue;

use crate::configuredtargets::{
    AliasConfiguredTarget, EnvironmentGroupConfiguredTarget, InputFileConfiguredTarget,
    MergedConfiguredTarget, OutputFileConfiguredTarget, PackageGroupConfiguredTarget,
    RuleConfiguredTarget,
};
use crate::provider_map::{TransitiveInfoProvider, TransitiveInfoProviderMap};

/// The common state of every configured target (Java `AbstractConfiguredTarget`'s fields).
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:55 AbstractConfiguredTarget
#[derive(Clone, Debug)]
pub struct ConfiguredTargetBase {
    /// Java `actionLookupKey` (usually a `ConfiguredTargetKey`).
    pub lookup_key: ActionLookupKeyRef,
    /// Java `visibility` (`NestedSet<PackageGroupContents>`), as the package specifications'
    /// string forms in `NestedSet` order.
    // PORT: the visibility of a configured target only feeds visibility checks; agent C decides
    // the precise representation together with VisibilityProvider.
    pub visibility: Option<Arc<crate::providers::visibility::VisibilityProviderData>>,
}

/// A configured target (Java `ConfiguredTarget`).
// Java: com/google/devtools/build/lib/analysis/ConfiguredTarget.java:37 ConfiguredTarget
#[derive(Clone, Debug)]
pub enum ConfiguredTarget {
    Rule(Arc<RuleConfiguredTarget>),
    InputFile(Arc<InputFileConfiguredTarget>),
    OutputFile(Arc<OutputFileConfiguredTarget>),
    PackageGroup(Arc<PackageGroupConfiguredTarget>),
    EnvironmentGroup(Arc<EnvironmentGroupConfiguredTarget>),
    Alias(Arc<AliasConfiguredTarget>),
    Merged(Arc<MergedConfiguredTarget>),
}

impl ConfiguredTarget {
    fn base(&self) -> &ConfiguredTargetBase {
        match self {
            ConfiguredTarget::Rule(t) => &t.base,
            ConfiguredTarget::InputFile(t) => &t.base,
            ConfiguredTarget::OutputFile(t) => &t.base,
            ConfiguredTarget::PackageGroup(t) => &t.base,
            ConfiguredTarget::EnvironmentGroup(t) => &t.base,
            ConfiguredTarget::Alias(t) => &t.base,
            ConfiguredTarget::Merged(t) => &t.base,
        }
    }

    /// Java `getLookupKey()`.
    pub fn get_lookup_key(&self) -> &ActionLookupKeyRef {
        &self.base().lookup_key
    }

    /// Java `getLabel()` (an alias reports its actual target's label).
    // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:132 AliasConfiguredTarget.getLabel
    pub fn get_label(&self) -> Label {
        match self {
            ConfiguredTarget::Alias(a) => a.actual.get_label(),
            ConfiguredTarget::Merged(m) => m.base_configured_target.get_label(),
            _ => self
                .get_lookup_key()
                .get_label()
                .expect("configured target key has a label"),
        }
    }

    /// Java `getConfigurationKey()` (`None` for the null configuration).
    pub fn get_configuration_key(&self) -> Option<BuildConfigurationKey> {
        self.get_lookup_key()
            .as_any()
            .downcast_ref::<crate::configured_target_key::ConfiguredTargetKey>()
            .and_then(|k| k.get_configuration_key().cloned())
    }

    /// Java `getConfigurationChecksum()`.
    pub fn get_configuration_checksum(&self) -> Option<String> {
        self.get_lookup_key().get_configuration_checksum()
    }

    /// Java `getVisibility()`.
    pub fn get_visibility(
        &self,
    ) -> Option<&Arc<crate::providers::visibility::VisibilityProviderData>> {
        self.base().visibility.as_ref()
    }

    /// Java `isRuleConfiguredTarget()`.
    pub fn is_rule_configured_target(&self) -> bool {
        matches!(self, ConfiguredTarget::Rule(_))
    }

    /// Java `getActual()`: follows aliases.
    pub fn get_actual(&self) -> ConfiguredTarget {
        match self {
            ConfiguredTarget::Alias(a) => a.get_actual(),
            _ => self.clone(),
        }
    }

    /// Java `getOriginalLabel()`.
    pub fn get_original_label(&self) -> Label {
        match self {
            ConfiguredTarget::Alias(a) => a.get_original_label(),
            _ => self.get_label(),
        }
    }

    /// Java `unwrapIfMerged()`.
    pub fn unwrap_if_merged(&self) -> ConfiguredTarget {
        match self {
            ConfiguredTarget::Merged(m) => m.base_configured_target.clone(),
            _ => self.clone(),
        }
    }

    /// The provider map of this target (Java: each class's `getProvider` / `rawGetStarlarkProvider`
    /// sources). Agent R implements `providers()` on each struct.
    pub fn providers(&self) -> &TransitiveInfoProviderMap {
        match self {
            ConfiguredTarget::Rule(t) => t.providers(),
            ConfiguredTarget::InputFile(t) => t.providers(),
            ConfiguredTarget::OutputFile(t) => t.providers(),
            ConfiguredTarget::PackageGroup(t) => t.providers(),
            ConfiguredTarget::EnvironmentGroup(t) => t.providers(),
            ConfiguredTarget::Alias(t) => t.providers(),
            ConfiguredTarget::Merged(t) => t.providers(),
        }
    }

    /// Java `getProvider(Class<P>)`.
    pub fn get_provider<P: TransitiveInfoProvider>(&self) -> Option<&P> {
        self.providers().get_provider::<P>()
    }

    /// Java `get(Provider.Key)` (`TransitiveInfoCollection.get`).
    pub fn get(&self, key: &ProviderKey) -> Option<OwnedFrozenValue> {
        crate::configuredtargets::get_info(self, key)
    }
}
