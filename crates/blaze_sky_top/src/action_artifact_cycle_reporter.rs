//! Port of `lib/skyframe/ActionArtifactCycleReporter.java`.

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_sky_exec::keys::{
    ActionExecutionKey, ArtifactKey, AspectCompletionKey, TargetCompletionKey, TestCompletionKey,
};
use blaze_sky_loading::package_manager::SkyframePackageManager;
use blaze_skyframe::{CycleInfo, SkyFunctionName, SkyKey};

use crate::cycles_reporter::{
    AbstractLabelCycleReporter, SingleCycleReporter, label_maybe_report_cycle,
};

/// `SkyFunctions.ARTIFACT_NESTED_SET`.
const ARTIFACT_NESTED_SET: SkyFunctionName = SkyFunctionName("ARTIFACT_NESTED_SET");
/// `SkyFunctions.TARGET_COMPLETION`.
const TARGET_COMPLETION: SkyFunctionName = SkyFunctionName("TARGET_COMPLETION");
/// `SkyFunctions.TEST_COMPLETION`.
const TEST_COMPLETION: SkyFunctionName = SkyFunctionName("TEST_COMPLETION");
/// `SkyFunctions.BUILD_DRIVER`.
const BUILD_DRIVER: SkyFunctionName = SkyFunctionName("BUILD_DRIVER");

// Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:32 ActionArtifactCycleReporter.ACTION_OR_ARTIFACT_OR_TRANSITIVE_RDEP
pub fn action_or_artifact_or_transitive_rdep(key: &SkyKey) -> bool {
    matches!(
        key.function_name().0,
        "ARTIFACT"
            | "ARTIFACT_NESTED_SET"
            | "ACTION_EXECUTION"
            | "TARGET_COMPLETION"
            | "ASPECT_COMPLETION"
            | "TEST_COMPLETION"
            | "BUILD_DRIVER"
    )
}

/// Reports cycles between Actions and Artifacts. These indicates cycles within a rule.
// Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:30 ActionArtifactCycleReporter
pub struct ActionArtifactCycleReporter {
    package_provider: Arc<SkyframePackageManager>,
}

impl ActionArtifactCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:42 ActionArtifactCycleReporter.<init>
    pub fn new(package_provider: Arc<SkyframePackageManager>) -> Self {
        ActionArtifactCycleReporter { package_provider }
    }
}

/// The action lookup key of a `TopLevelActionLookupKeyWrapper` (a target or aspect completion
/// key).
fn top_level_action_lookup_key(
    key: &SkyKey,
) -> Option<&blaze_actions::actions::action_lookup::ActionLookupKeyRef> {
    if let Some(k) = key.downcast::<TargetCompletionKey>() {
        Some(&k.action_lookup_key)
    } else {
        key.downcast::<AspectCompletionKey>()
            .map(|k| &k.action_lookup_key)
    }
}

impl AbstractLabelCycleReporter for ActionArtifactCycleReporter {
    fn package_provider(&self) -> &Arc<SkyframePackageManager> {
        &self.package_provider
    }

    /// Should be kept consistent with `ACTION_OR_ARTIFACT_OR_TRANSITIVE_RDEP` and
    /// `shouldSkipOnPathToCycle`.
    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:47 ActionArtifactCycleReporter.prettyPrint
    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:57 ActionArtifactCycleReporter.prettyPrint
    fn pretty_print(&self, key: &SkyKey) -> String {
        if let Some(artifact) = key.downcast::<ArtifactKey>() {
            // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:76 ActionArtifactCycleReporter.prettyPrintArtifact
            return format!("file: {}", artifact.0.get_root_relative_path_string());
        } else if let Some(action_lookup_data) = key.downcast::<ActionExecutionKey>() {
            return format!("action from: {}", action_lookup_data.0);
        } else if let Some(action_lookup_key) = top_level_action_lookup_key(key) {
            if key.function_name() == TARGET_COMPLETION {
                return format!(
                    "configured target: {}",
                    action_lookup_key
                        .get_label()
                        .expect("a configured target key has a label")
                );
            }
            let aspect_key = action_lookup_key
                .as_any()
                .downcast_ref::<AspectKey>()
                .unwrap_or_else(|| panic!("ClassCastException: {:?} is not an AspectKey", key));
            return format!("top-level aspect: {}", aspect_key.pretty_print());
        } else if let Some(test_completion_key) = key.downcast::<TestCompletionKey>()
            && key.function_name() == TEST_COMPLETION
        {
            return format!(
                "test target: {}",
                test_completion_key
                    .configured_target_key
                    .get_label()
                    .expect("a configured target key has a label")
            );
        }
        panic!(
            "IllegalStateException: Argument is not Action, TargetCompletion, AspectCompletion, \
             or TestCompletion: {:?}",
            key
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:80 ActionArtifactCycleReporter.shouldSkipOnPathToCycle
    fn should_skip_on_path_to_cycle(&self, key: &SkyKey) -> bool {
        // BuildDriverKeys don't provide any relevant info for the end user.
        BUILD_DRIVER == key.function_name()
            // ArtifactNestedSetKeys are just an implementation detail.
            || ARTIFACT_NESTED_SET == key.function_name()
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:88 ActionArtifactCycleReporter.getLabel
    fn get_label(&self, key: &SkyKey) -> Label {
        let label = if let Some(artifact) = key.downcast::<ArtifactKey>() {
            artifact.0.get_owner()
        } else if let Some(action_lookup_data) = key.downcast::<ActionExecutionKey>() {
            action_lookup_data.0.get_label()
        } else if let Some(action_lookup_key) = top_level_action_lookup_key(key) {
            action_lookup_key.get_label()
        } else if let Some(test_completion_key) = key.downcast::<TestCompletionKey>()
            && key.function_name() == TEST_COMPLETION
        {
            test_completion_key.configured_target_key.get_label()
        } else {
            panic!(
                "IllegalStateException: Argument is not Action, TargetCompletion, \
                 AspectCompletion, or TestCompletion: {:?}",
                key
            )
        };
        // Java returns null (and getTargetForLabel then throws) for the keys without a label.
        label.unwrap_or_else(|| panic!("NullPointerException: label of {:?}", key))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:105 ActionArtifactCycleReporter.canReportCycle
    fn can_report_cycle(&self, top_level_key: &SkyKey, cycle_info: &CycleInfo) -> bool {
        action_or_artifact_or_transitive_rdep(top_level_key)
            && cycle_info
                .get_cycle()
                .iter()
                .all(action_or_artifact_or_transitive_rdep)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionArtifactCycleReporter.java:111 ActionArtifactCycleReporter.shouldSkipIntermediateKeyOnCycle
    fn should_skip_intermediate_key_on_cycle(&self, key: &SkyKey) -> bool {
        // ArtifactNestedSetKey isn't worth reporting to the user - it is just an optimization,
        // and will always be an intermediate member of a cycle. It may contain artifacts
        // irrelevant to the cycle, and may be nested several layers deep.
        ARTIFACT_NESTED_SET == key.function_name()
    }
}

impl SingleCycleReporter for ActionArtifactCycleReporter {
    fn maybe_report_cycle(
        &self,
        top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        label_maybe_report_cycle(
            self,
            top_level_key,
            cycle_info,
            already_reported,
            event_handler,
        )
    }
}
