//! Ports of `skyframe/ConfiguredTargetValue.java` (an interface in Java),
//! `RuleConfiguredTargetValue.java` and `NonRuleConfiguredTargetValue.java`.

use blaze_actions::ActionRef;
use blaze_collect::NestedSet;
use blaze_packages::target::Target;

use crate::configured_target::ConfiguredTarget;

/// The value of `CONFIGURED_TARGET`.
///
/// PORT: Java's two classes become one struct; `actions` is empty for non-rule targets.
/// `target_data` (Java `NonRuleConfiguredTargetValue.targetData`, used by cquery for remote
/// values) is replaced by the loaded `Target` itself, which cquery and aquery read.
// Java: com/google/devtools/build/lib/skyframe/RuleConfiguredTargetValue.java:32 RuleConfiguredTargetValue
// Java: com/google/devtools/build/lib/skyframe/NonRuleConfiguredTargetValue.java:31 NonRuleConfiguredTargetValue
#[derive(Clone, Debug)]
pub struct ConfiguredTargetValue {
    configured_target: ConfiguredTarget,
    /// Java `RuleConfiguredTargetValue.actions` (`configuredTarget.getActions()`).
    actions: Vec<ActionRef>,
    /// Java `transitivePackages` (`--track_transitive_package_loading`); `None` when not
    /// tracked.
    transitive_packages: Option<
        NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    >,
    /// The target this value configures (`None` only for targets created without one).
    target: Option<Target>,
}

impl ConfiguredTargetValue {
    pub fn new(
        configured_target: ConfiguredTarget,
        actions: Vec<ActionRef>,
        transitive_packages: Option<
            NestedSet<
                blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
            >,
        >,
        target: Option<Target>,
    ) -> Self {
        ConfiguredTargetValue {
            configured_target,
            actions,
            transitive_packages,
            target,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetValue.java:28 ConfiguredTargetValue.getConfiguredTarget
    pub fn get_configured_target(&self) -> &ConfiguredTarget {
        &self.configured_target
    }

    // Java: com/google/devtools/build/lib/skyframe/RuleConfiguredTargetValue.java:46 RuleConfiguredTargetValue.getActions
    pub fn get_actions(&self) -> &[ActionRef] {
        &self.actions
    }

    // Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetValue.java:34 ConfiguredTargetValue.getTransitivePackages
    pub fn get_transitive_packages(
        &self,
    ) -> Option<
        &NestedSet<
            blaze_actions::analysis::repo_mapping_manifest_action::RepoMappingPackageMetadata,
        >,
    > {
        self.transitive_packages.as_ref()
    }

    pub fn get_target(&self) -> Option<&Target> {
        self.target.as_ref()
    }
}
