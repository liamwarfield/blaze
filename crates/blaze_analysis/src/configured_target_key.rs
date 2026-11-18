//! Port of `skyframe/ConfiguredTargetKey.java`: the `CONFIGURED_TARGET` SkyKey.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::ActionLookupKey;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::Label;
use blaze_skyframe::{Key, SkyFunctionName};

use crate::configured_target_value::ConfiguredTargetValue;

/// `SkyFunctions.CONFIGURED_TARGET`.
pub const CONFIGURED_TARGET: SkyFunctionName = SkyFunctionName("CONFIGURED_TARGET");

/// In simple form, a (Label, BuildConfigurationValue) pair used to trigger immediate dependency
/// resolution and the rule analysis.
///
/// PORT: Java's four subclasses (`ConfiguredTargetKeyWithFinalConfiguration`,
/// `ToolchainDependencyConfiguredTargetKey`, ...) are the combinations of
/// `execution_platform_label` and `should_apply_rule_transition`; `Debug` prints the subclass's
/// simple name like Java's `toString`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:47 ConfiguredTargetKey
#[derive(Clone, PartialEq, Eq)]
pub struct ConfiguredTargetKey {
    label: Label,
    configuration_key: Option<BuildConfigurationKey>,
    execution_platform_label: Option<Label>,
    should_apply_rule_transition: bool,
}

impl Hash for ConfiguredTargetKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.label.hash(state);
        self.configuration_key.hash(state);
        self.execution_platform_label.hash(state);
        self.should_apply_rule_transition.hash(state);
    }
}

impl ConfiguredTargetKey {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:79 ConfiguredTargetKey.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:85 ConfiguredTargetKey.getConfigurationKey
    pub fn get_configuration_key(&self) -> Option<&BuildConfigurationKey> {
        self.configuration_key.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:90 ConfiguredTargetKey.getExecutionPlatformLabel
    pub fn get_execution_platform_label(&self) -> Option<&Label> {
        self.execution_platform_label.as_ref()
    }

    /// True if the target's incoming rule transition should be applied. False if the target may
    /// already have been transitioned (for example the top-level configured target with
    /// `--trim_test_configuration`).
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:101 ConfiguredTargetKey.shouldApplyRuleTransition
    pub fn should_apply_rule_transition(&self) -> bool {
        self.should_apply_rule_transition
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:105 ConfiguredTargetKey.prettyPrint
    pub fn pretty_print(&self) -> String {
        format!(
            "{} ({})",
            self.label,
            Self::format_configuration_key(self.configuration_key.as_ref())
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:375 ConfiguredTargetKey.formatConfigurationKey
    fn format_configuration_key(key: Option<&BuildConfigurationKey>) -> String {
        match key {
            None => "null".to_owned(),
            Some(k) => k.get_options().checksum().to_owned(),
        }
    }

    /// The configuration checksum (`getConfigurationKey().getOptions().checksum()`), if any.
    pub fn get_configuration_checksum(&self) -> Option<&str> {
        self.configuration_key
            .as_ref()
            .map(|k| k.get_options().checksum())
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:211 ConfiguredTargetKey.toBuilder
    pub fn to_builder(&self) -> ConfiguredTargetKeyBuilder {
        ConfiguredTargetKey::builder()
            .set_configuration_key(self.configuration_key.clone())
            .set_label(self.label.clone())
            .set_execution_platform_label(self.execution_platform_label.clone())
            .set_should_apply_rule_transition(self.should_apply_rule_transition)
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:220 ConfiguredTargetKey.builder
    pub fn builder() -> ConfiguredTargetKeyBuilder {
        ConfiguredTargetKeyBuilder {
            label: None,
            configuration_key: None,
            execution_platform_label: None,
            should_apply_rule_transition: true,
        }
    }

    fn java_class_simple_name(&self) -> &'static str {
        match (
            self.execution_platform_label.is_some(),
            self.should_apply_rule_transition,
        ) {
            (false, true) => "ConfiguredTargetKey",
            (false, false) => "ConfiguredTargetKeyWithFinalConfiguration",
            (true, true) => "ToolchainDependencyConfiguredTargetKey",
            (true, false) => "ToolchainDependencyConfiguredTargetKeyWithFinalConfiguration",
        }
    }
}

/// Java `ConfiguredTargetKey.Builder`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:229 ConfiguredTargetKey.Builder
#[derive(Clone, Debug)]
pub struct ConfiguredTargetKeyBuilder {
    label: Option<Label>,
    configuration_key: Option<BuildConfigurationKey>,
    execution_platform_label: Option<Label>,
    should_apply_rule_transition: bool,
}

impl ConfiguredTargetKeyBuilder {
    pub fn set_label(mut self, label: Label) -> Self {
        self.label = Some(label);
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:246 ConfiguredTargetKey.Builder.setConfiguration
    pub fn set_configuration(self, build_configuration: Option<&BuildConfigurationValue>) -> Self {
        self.set_configuration_key(build_configuration.map(|c| c.get_key()))
    }

    pub fn set_configuration_key(
        mut self,
        configuration_key: Option<BuildConfigurationKey>,
    ) -> Self {
        self.configuration_key = configuration_key;
        self
    }

    pub fn set_execution_platform_label(mut self, label: Option<Label>) -> Self {
        self.execution_platform_label = label;
        self
    }

    pub fn set_should_apply_rule_transition(mut self, v: bool) -> Self {
        self.should_apply_rule_transition = v;
        self
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:280 ConfiguredTargetKey.Builder.build
    pub fn build(self) -> ConfiguredTargetKey {
        ConfiguredTargetKey {
            label: self.label.expect("ConfiguredTargetKey.Builder: label"),
            configuration_key: self.configuration_key,
            execution_platform_label: self.execution_platform_label,
            should_apply_rule_transition: self.should_apply_rule_transition,
        }
    }
}

impl fmt::Debug for ConfiguredTargetKey {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:132 ConfiguredTargetKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{{label={}, config={}",
            self.java_class_simple_name(),
            self.label,
            match &self.configuration_key {
                Some(k) => format!("{:?}", k),
                None => "null".to_owned(),
            }
        )?;
        if let Some(p) = &self.execution_platform_label {
            write!(f, ", executionPlatformLabel={}", p)?;
        }
        f.write_str("}")
    }
}

impl fmt::Display for ConfiguredTargetKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl Key for ConfiguredTargetKey {
    type Value = ConfiguredTargetValue;
    const FUNCTION: SkyFunctionName = CONFIGURED_TARGET;
}

impl ActionLookupKey for ConfiguredTargetKey {
    fn get_label(&self) -> Option<Label> {
        Some(self.label.clone())
    }

    fn get_configuration_checksum(&self) -> Option<String> {
        self.get_configuration_checksum().map(str::to_owned)
    }

    fn function_name(&self) -> &'static str {
        CONFIGURED_TARGET.0
    }

    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<ConfiguredTargetKey>()
            .is_some_and(|o| o == self)
    }

    fn key_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
