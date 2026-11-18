//! Port of the analysis-only part of `skyframe/SkyframeBuildView.java` (`configureTargets`) and
//! `skyframe/SkyframeAnalysisResult.java`.

use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_packages::target::Target;
use blaze_skyframe::{EvaluationResult, Interrupted, WalkableGraph};
use indexmap::IndexMap;

use crate::skyframe_executor::{SkyframeExecutor, TargetAndConfiguration};

/// Encapsulates the raw analysis result of top level targets and aspects coming from Skyframe.
// Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:30 SkyframeAnalysisResult
pub struct SkyframeAnalysisResult {
    has_loading_error: bool,
    has_analysis_error: bool,
    has_action_conflicts: bool,
    configured_targets: Vec<ConfiguredTarget>,
    walkable_graph: Arc<dyn WalkableGraph>,
    targets_with_configuration: Vec<TargetAndConfiguration>,
}

impl SkyframeAnalysisResult {
    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:63 SkyframeAnalysisResult.hasLoadingError
    pub fn has_loading_error(&self) -> bool {
        self.has_loading_error
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:71 SkyframeAnalysisResult.hasAnalysisError
    pub fn has_analysis_error(&self) -> bool {
        self.has_analysis_error
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:75 SkyframeAnalysisResult.hasActionConflicts
    pub fn has_action_conflicts(&self) -> bool {
        self.has_action_conflicts
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:79 SkyframeAnalysisResult.getConfiguredTargets
    pub fn get_configured_targets(&self) -> &[ConfiguredTarget] {
        &self.configured_targets
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:83 SkyframeAnalysisResult.getWalkableGraph
    pub fn get_walkable_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.walkable_graph
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeAnalysisResult.java:95 SkyframeAnalysisResult.getTargetsWithConfiguration
    pub fn get_targets_with_configuration(&self) -> &[TargetAndConfiguration] {
        &self.targets_with_configuration
    }
}

/// What `configureTargets` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigureTargetsError {
    #[error("interrupted")]
    Interrupted,
    /// `ViewCreationFailedException` (message and failure detail).
    #[error("{message}")]
    ViewCreationFailed {
        message: String,
        failure_detail: blaze_protos::failure_details::FailureDetail,
    },
}

impl From<Interrupted> for ConfigureTargetsError {
    fn from(_: Interrupted) -> Self {
        ConfigureTargetsError::Interrupted
    }
}

/// Analyzes the specified targets using Skyframe as the driving framework.
// Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:400 SkyframeBuildView.configureTargets
pub fn configure_targets(
    skyframe_executor: &SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
    label_to_target_map: &IndexMap<Label, Target>,
    ct_keys: &[ConfiguredTargetKey],
    keep_going: bool,
    check_for_action_conflicts: bool,
) -> Result<SkyframeAnalysisResult, ConfigureTargetsError> {
    // PORT: enableAnalysis(true/false) guards incremental reuse; not needed for one evaluation.
    let result = skyframe_executor.configure_targets(
        event_handler,
        label_to_target_map,
        ct_keys,
        keep_going,
    )?;

    let cts = result.configured_targets;
    let evaluation_result: EvaluationResult = result.evaluation_result;

    // PORT: ArtifactConflictFinder (inter-target output conflicts) and its AnalysisGraphStatsEvent
    // are not ported: no inter-target conflicts are detected (shouldCheckForConflicts with
    // `check_for_action_conflicts`).
    let _ = check_for_action_conflicts;

    // Intra-target conflict would mean an error in evaluationResult.
    if !evaluation_result.has_error() {
        return Ok(SkyframeAnalysisResult {
            has_loading_error: false,
            has_analysis_error: false,
            has_action_conflicts: false,
            configured_targets: cts,
            walkable_graph: skyframe_executor.get_walkable_graph(),
            targets_with_configuration: result.targets_with_configuration,
        });
    }

    blaze_util::unported!(
        "SkyframeErrorProcessor.processAnalysisErrors",
        "analysis errors in {:?}",
        evaluation_result
            .error_map()
            .iter()
            .map(|(k, _)| format!("{:?}", k))
            .collect::<Vec<_>>()
    )
}
