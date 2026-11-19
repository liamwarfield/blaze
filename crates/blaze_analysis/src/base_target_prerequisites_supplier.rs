//! Port of `skyframe/BaseTargetPrerequisitesSupplier.java` and its implementation
//! `SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl`.

use std::sync::Arc;

use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_skyframe::{Key, SkyKey, SkyValue};

use crate::configured_target_key::ConfiguredTargetKey;
use crate::configured_target_value::ConfiguredTargetValue;
use crate::toolchains::toolchain_context_key::ToolchainContextKey;

/// Java `MemoizingEvaluator.getExistingValue`: the value of a node that is done, without adding
/// a dependency edge. `blaze_sky_top` supplies it from its evaluator.
pub type ExistingValueLookup = Arc<dyn Fn(&SkyKey) -> Option<SkyValue> + Send + Sync>;

/// Looks up previously evaluated `ConfiguredTargetValue`s and `BuildConfigurationValue`s without
/// adding a dependency edge between them and the requesting node.
///
/// Mainly used by `AspectFunction` to look up the `ConfiguredTargetValue`s and
/// `BuildConfigurationValue` of its target dependencies.
// Java: com/google/devtools/build/lib/skyframe/BaseTargetPrerequisitesSupplier.java:29 BaseTargetPrerequisitesSupplier
#[derive(Clone)]
pub struct BaseTargetPrerequisitesSupplier {
    get_existing_value: ExistingValueLookup,
}

impl BaseTargetPrerequisitesSupplier {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4598 SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl
    pub fn new(get_existing_value: ExistingValueLookup) -> Self {
        BaseTargetPrerequisitesSupplier { get_existing_value }
    }

    fn get<K: Key + Clone>(&self, key: &K) -> Option<Arc<K::Value>> {
        (self.get_existing_value)(&SkyKey::new(key.clone()))
            .and_then(|value| value.downcast::<K::Value>().ok())
    }

    /// Directly retrieves configured targets from Skyframe without adding a dependency edge.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4602 SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl.getPrerequisite
    pub fn get_prerequisite(
        &self,
        key: &ConfiguredTargetKey,
    ) -> Option<Arc<ConfiguredTargetValue>> {
        self.get(key)
    }

    /// Directly retrieves configuration values from Skyframe without adding a dependency edge.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4609 SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl.getPrerequisiteConfiguration
    pub fn get_prerequisite_configuration(
        &self,
        key: &BuildConfigurationKey,
    ) -> Option<Arc<BuildConfigurationValue>> {
        self.get(key)
    }

    /// Directly retrieves unloaded toolchain contexts from Skyframe without adding a dependency
    /// edge.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:4616 SkyframeExecutor.BaseTargetPrerequisitesSupplierImpl.getUnloadedToolchainContext
    pub fn get_unloaded_toolchain_context(
        &self,
        key: &ToolchainContextKey,
    ) -> Option<Arc<<ToolchainContextKey as Key>::Value>> {
        self.get(key)
    }
}

impl std::fmt::Debug for BaseTargetPrerequisitesSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BaseTargetPrerequisitesSupplier")
    }
}
