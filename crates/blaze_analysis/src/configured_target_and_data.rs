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
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:217 ConfiguredTargetAndData.getTargetLabel
    pub fn get_target_label(&self) -> blaze_cmdline::Label {
        self.target.get_label().clone()
    }

    /// Java `getConfiguration()`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:199 ConfiguredTargetAndData.getConfiguration
    pub fn get_configuration(&self) -> Option<&Arc<BuildConfigurationValue>> {
        self.configuration.as_ref()
    }

    /// Java `getTransitionKeys()`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:213 ConfiguredTargetAndData.getTransitionKeys
    pub fn get_transition_keys(&self) -> &[String] {
        &self.transition_keys
    }

    /// The configured `Target` (Java exposes its parts: `getTargetKind`, `isTargetRule`,
    /// `getRuleClass`, `getLocation`, `getTargetPackage`, ...).
    pub fn get_target(&self) -> &Target {
        &self.target
    }

    /// For use with `MergedConfiguredTarget` and similar, where we create a virtual
    /// `ConfiguredTarget` corresponding to the same `Target`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:179 ConfiguredTargetAndData.fromConfiguredTarget
    pub fn from_configured_target(&self, maybe_new: ConfiguredTarget) -> ConfiguredTargetAndData {
        // Java: `configuredTarget.equals(maybeNew)` is reference equality.
        if self.configured_target.identity() == maybe_new.identity() {
            return self.clone();
        }
        // PORT: Java's constructor checks the label and configuration consistency; a merged
        // target has its base target's label and configuration.
        blaze_util::check_state!(
            maybe_new.get_label() == *self.target.get_label(),
            "Unable to construct ConfiguredTargetAndData: ConfiguredTarget's label {} is not equal to Target's label {}",
            maybe_new.get_label(),
            self.target.get_label()
        );
        let inner_configuration_key = maybe_new.get_configuration_key();
        match &self.configuration {
            None => blaze_util::check_state!(
                inner_configuration_key.is_none(),
                "Non-null configuration key for {} but configuration is null ({})",
                crate::configuredtargets::to_string(&maybe_new),
                self.target
            ),
            // PORT: Java compares the key's options with the configuration's options; a
            // configuration's key is made of its options.
            Some(configuration) => blaze_util::check_state!(
                inner_configuration_key.as_ref() == Some(&configuration.get_key()),
                "Configurations don't match: {:?} {} ({} {})",
                inner_configuration_key,
                configuration.checksum(),
                crate::configuredtargets::to_string(&maybe_new),
                self.target
            ),
        }
        ConfiguredTargetAndData::new(
            maybe_new,
            self.target.clone(),
            self.configuration.clone(),
            self.transition_keys.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:205 ConfiguredTargetAndData.getConfigurationKey
    pub fn get_configuration_key(
        &self,
    ) -> Option<blaze_build_config::build_configuration_key::BuildConfigurationKey> {
        self.configured_target.get_configuration_key()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:221 ConfiguredTargetAndData.getLocation
    pub fn get_location(&self) -> blaze_starlark::Location {
        self.target.get_location()
    }

    /// Returns the rule class object if the target is a rule and `None` otherwise.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:240 ConfiguredTargetAndData.getRuleClassObject
    pub fn get_rule_class_object(&self) -> Option<Arc<blaze_packages::rule_class::RuleClass>> {
        match &self.target {
            Target::Rule(rule) => Some(rule.get_rule_class_object().clone()),
            _ => None,
        }
    }

    /// Returns the rule tags attribute value if the target is a rule and `None` otherwise.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:249 ConfiguredTargetAndData.getOnlyTagsAttribute
    pub fn get_only_tags_attribute(&self) -> Option<Vec<String>> {
        match &self.target {
            Target::Rule(rule) => Some(rule.get_only_tags_attribute()),
            _ => None,
        }
    }

    /// Returns the rule tags if the target is a rule and an empty set otherwise.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:257 ConfiguredTargetAndData.getRuleTags
    pub fn get_rule_tags(&self) -> indexmap::IndexSet<String> {
        self.target.get_rule_tags()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:261 ConfiguredTargetAndData.isTargetRule
    pub fn is_target_rule(&self) -> bool {
        self.target.is_rule()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:273 ConfiguredTargetAndData.isTargetOutputFile
    pub fn is_target_output_file(&self) -> bool {
        self.target.is_output_file()
    }

    /// The generating rule's label if the target is an `OutputFile` otherwise `None`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:279 ConfiguredTargetAndData.getGeneratingRuleLabel
    pub fn get_generating_rule_label(&self) -> Option<blaze_cmdline::Label> {
        self.target.get_generating_rule_label()
    }

    /// Any deprecation warning of the associated rule (maybe generating) otherwise `None`.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:291 ConfiguredTargetAndData.getDeprecationWarning
    pub fn get_deprecation_warning(&self) -> Option<String> {
        self.target.get_deprecation_warning()
    }

    /// True if the target is a testonly rule or an output file generated by a testonly rule.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:296 ConfiguredTargetAndData.isTestOnly
    pub fn is_test_only(&self) -> bool {
        self.target.is_test_only()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:300 ConfiguredTargetAndData.getTargetAdvertisedProviders
    pub fn get_target_advertised_providers(
        &self,
    ) -> blaze_packages::rule_class::AdvertisedProviderSet {
        // TODO(shahan): If this is an output file, refers to the providers of the generating
        // rule. However, in such cases, aspects are not permitted to have required providers.
        // Consider short-circuiting the logic for that case.
        self.target.get_advertised_providers()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:312 ConfiguredTargetAndData.copyWithClearedTransitionKeys
    pub fn copy_with_cleared_transition_keys(&self) -> ConfiguredTargetAndData {
        if self.transition_keys.is_empty() {
            return self.clone();
        }
        self.copy_with_transition_keys(Vec::new())
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:319 ConfiguredTargetAndData.copyWithTransitionKeys
    pub fn copy_with_transition_keys(&self, keys: Vec<String>) -> ConfiguredTargetAndData {
        ConfiguredTargetAndData::new(
            self.configured_target.clone(),
            self.target.clone(),
            self.configuration.clone(),
            keys,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:291 ConfiguredTargetAndData.fromExistingConfiguredTargetInSkyframe
    // PORT: the other Java helpers are ported by the agents that need them, in this file.
}
