//! The `ConfiguredTargetAndData` accessors analysis uses that the lead's struct does not have.
//!
//! PORT: move to `crate::configured_target_and_data` (lead-owned); an extension trait meanwhile.

use std::sync::Arc;

use blaze_packages::rule_class::RuleClass;
use blaze_packages::target::Target;
use blaze_vfs::Path;

use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;

/// Java `ConfiguredTargetAndData` getters over the configured `Target`.
pub trait ConfiguredTargetAndDataExt {
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:260 ConfiguredTargetAndData.getTargetKind
    fn get_target_kind(&self) -> String;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:264 ConfiguredTargetAndData.isTargetRule
    fn is_target_rule(&self) -> bool;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:268 ConfiguredTargetAndData.isTargetFile
    fn is_target_file(&self) -> bool;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:272 ConfiguredTargetAndData.isTargetInputFile
    fn is_target_input_file(&self) -> bool;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:280 ConfiguredTargetAndData.getRuleClass
    fn get_rule_class(&self) -> String;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:284 ConfiguredTargetAndData.getRuleClassObject
    fn get_rule_class_object(&self) -> Option<Arc<RuleClass>>;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:292 ConfiguredTargetAndData.getInputPath
    fn get_input_path(&self) -> Option<Path>;
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:300 ConfiguredTargetAndData.isForDependencyResolution
    fn is_for_dependency_resolution(&self) -> bool;
    /// Returns a copy with the configured target replaced, without checking that it configures
    /// the same target.
    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetAndData.java:226 ConfiguredTargetAndData.fromConfiguredTargetNoCheck
    fn from_configured_target_no_check(
        &self,
        maybe_new: ConfiguredTarget,
    ) -> ConfiguredTargetAndData;
}

impl ConfiguredTargetAndDataExt for ConfiguredTargetAndData {
    fn get_target_kind(&self) -> String {
        self.get_target().get_target_kind()
    }

    fn is_target_rule(&self) -> bool {
        self.get_target().is_rule()
    }

    fn is_target_file(&self) -> bool {
        self.get_target().is_file()
    }

    fn is_target_input_file(&self) -> bool {
        self.get_target().is_input_file()
    }

    fn get_rule_class(&self) -> String {
        // Java: the target data's rule class, "" for non-rules.
        match self.get_target() {
            Target::Rule(r) => r.get_rule_class().to_owned(),
            _ => String::new(),
        }
    }

    fn get_rule_class_object(&self) -> Option<Arc<RuleClass>> {
        match self.get_target() {
            Target::Rule(r) => Some(r.get_rule_class_object().clone()),
            _ => None,
        }
    }

    fn get_input_path(&self) -> Option<Path> {
        self.get_target().get_input_path()
    }

    fn is_for_dependency_resolution(&self) -> bool {
        self.get_target().is_for_dependency_resolution()
    }

    fn from_configured_target_no_check(
        &self,
        maybe_new: ConfiguredTarget,
    ) -> ConfiguredTargetAndData {
        ConfiguredTargetAndData::new(
            maybe_new,
            self.get_target().clone(),
            self.get_configuration().cloned(),
            self.get_transition_keys().to_vec(),
        )
    }
}
