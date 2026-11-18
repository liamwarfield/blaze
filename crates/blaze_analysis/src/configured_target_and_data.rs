//! Port of `skyframe/ConfiguredTargetAndData.java`.

use std::sync::Arc;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_packages::target::Target;

use crate::configured_target::ConfiguredTarget;

/// A container class for a [`ConfiguredTarget`] and associated data: the `Target` it configures
/// and its `BuildConfigurationValue` (`None` for the null configuration), plus transition keys.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:50 ConfiguredTargetAndData
#[derive(Clone, Debug)]
pub struct ConfiguredTargetAndData {
    configured_target: ConfiguredTarget,
    target: Target,
    configuration: Option<Arc<BuildConfigurationValue>>,
    /// Java `transitionKeys`: the split-transition keys that produced this dependency.
    transition_keys: Vec<String>,
}

impl ConfiguredTargetAndData {
    pub fn new(
        configured_target: ConfiguredTarget,
        target: Target,
        configuration: Option<Arc<BuildConfigurationValue>>,
        transition_keys: Vec<String>,
    ) -> Self {
        ConfiguredTargetAndData {
            configured_target,
            target,
            configuration,
            transition_keys,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:239 ConfiguredTargetAndData.getConfiguredTarget
    pub fn get_configured_target(&self) -> &ConfiguredTarget {
        &self.configured_target
    }

    /// Java `getTargetLabel()`.
    pub fn get_target_label(&self) -> blaze_cmdline::Label {
        self.target.get_label().clone()
    }

    /// Java `getConfiguration()`.
    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }

    /// Java `getTransitionKeys()`.
    pub fn get_transition_keys(&self) -> &[String] {
        &self.transition_keys
    }

    /// The configured `Target` (Java exposes its parts: `getTargetKind`, `isTargetRule`,
    /// `getRuleClass`, `getLocation`, `getTargetPackage`, ...).
    pub fn get_target(&self) -> &Target {
        &self.target
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:291 ConfiguredTargetAndData.fromExistingConfiguredTargetInSkyframe
    // PORT: the other Java helpers are ported by the agents that need them, in this file.
}
