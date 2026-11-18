//! Port of the analysis-only part of `analysis/BuildView.java` (`update`, `createResult`,
//! `createAnalysisFailureDetail`) and `analysis/AnalysisResult.java`.
//!
//! PORT: the execution-phase parts of `createResult` (workspace status artifacts, extra actions,
//! coverage artifacts, the artifacts to build, test partitioning by exclusivity, package roots)
//! are not computed here: they belong to the build driver. `AnalysisResult` carries what the
//! analysis-only commands (cquery, aquery) read.

use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::Label;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::target::Target;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_skyframe::WalkableGraph;
use indexmap::{IndexMap, IndexSet};

use crate::skyframe_build_view::{self, ConfigureTargetsError, SkyframeAnalysisResult};
use crate::skyframe_executor::{SkyframeExecutor, TargetAndConfiguration};
use crate::target_pattern_phase::TargetPatternPhaseValue;

/// Return value for `BuildView.update`.
// Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:36 AnalysisResult
pub struct AnalysisResult {
    configuration: Arc<BuildConfigurationValue>,
    targets_to_build: Vec<ConfiguredTarget>,
    targets_to_test: Option<Vec<ConfiguredTarget>>,
    targets_to_skip: Vec<ConfiguredTarget>,
    failure_detail: Option<FailureDetail>,
    walkable_graph: Arc<dyn WalkableGraph>,
    top_level_targets_with_configs: Vec<TargetAndConfiguration>,
    /// The counters of the `AnalysisPhaseCompleteEvent`.
    analysis_phase_complete_stats: AnalysisPhaseCompleteStats,
    /// The `AnalysisGraphStatsEvent`'s metrics (`None`: conflicts were not checked, no event).
    build_graph_metrics: Option<blaze_protos::build_event_stream::build_metrics::BuildGraphMetrics>,
}

impl AnalysisResult {
    /// The analysis phase's counters (Java `AnalysisPhaseCompleteEvent`'s).
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:253 BuildView.update
    pub fn get_analysis_phase_complete_stats(&self) -> &AnalysisPhaseCompleteStats {
        &self.analysis_phase_complete_stats
    }

    /// The `BuildGraphMetrics` of the `AnalysisGraphStatsEvent` (`None` when the analysis did not
    /// check for action conflicts, so no event was posted).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:454 SkyframeBuildView.configureTargets
    pub fn get_build_graph_metrics(
        &self,
    ) -> Option<&blaze_protos::build_event_stream::build_metrics::BuildGraphMetrics> {
        self.build_graph_metrics.as_ref()
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:92 AnalysisResult.getConfiguration
    pub fn get_configuration(&self) -> &Arc<BuildConfigurationValue> {
        &self.configuration
    }

    /// Returns the configured targets to build.
    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:100 AnalysisResult.getTargetsToBuild
    pub fn get_targets_to_build(&self) -> &[ConfiguredTarget] {
        &self.targets_to_build
    }

    /// Returns the configured targets to run as tests, or `None` if testing was not requested.
    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:133 AnalysisResult.getTargetsToTest
    pub fn get_targets_to_test(&self) -> Option<&[ConfiguredTarget]> {
        self.targets_to_test.as_deref()
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:141 AnalysisResult.getTargetsToSkip
    pub fn get_targets_to_skip(&self) -> &[ConfiguredTarget] {
        &self.targets_to_skip
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:162 AnalysisResult.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }

    /// The graph the analysis ran in (Java: `SkyframeAnalysisResult.getWalkableGraph`).
    pub fn get_walkable_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.walkable_graph
    }

    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:204 AnalysisResult.getTopLevelTargetsWithConfigs
    pub fn get_top_level_targets_with_configs(&self) -> &[TargetAndConfiguration] {
        &self.top_level_targets_with_configs
    }

    /// `getAspectsMap()`.
    ///
    /// PORT: top-level aspects (`--aspects`) are not ported; the map is always empty.
    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:116 AnalysisResult.getAspectsMap
    pub fn get_aspects_map_is_empty(&self) -> bool {
        true
    }
}

/// What `BuildView.update` throws.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildViewError {
    #[error("{message}")]
    ViewCreationFailed {
        message: String,
        failure_detail: FailureDetail,
    },
    #[error("{}", .0.get_message())]
    InvalidConfiguration(InvalidConfigurationException),
    #[error("interrupted")]
    Interrupted,
}

impl From<ConfigureTargetsError> for BuildViewError {
    fn from(e: ConfigureTargetsError) -> Self {
        match e {
            ConfigureTargetsError::Interrupted => BuildViewError::Interrupted,
            ConfigureTargetsError::ViewCreationFailed {
                message,
                failure_detail,
            } => BuildViewError::ViewCreationFailed {
                message,
                failure_detail,
            },
        }
    }
}

/// The parameters of `BuildView.update` that the analysis-only path reads.
pub struct BuildViewUpdateOptions {
    pub keep_going: bool,
    pub check_for_action_conflicts: bool,
    /// `--aspects`.
    pub aspects: Vec<String>,
}

/// The BuildView is Bazel's main analysis entry point.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:216 BuildView.update
pub fn update(
    skyframe_executor: &SkyframeExecutor,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
    options: &BuildViewUpdateOptions,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<AnalysisResult, BuildViewError> {
    // AnalysisPhaseRunner.runAnalysisPhase: Stopwatch timer = Stopwatch.createStarted().
    // PORT: the analysis timer runs over BuildView.update (Java starts it just before).
    let timer = std::time::Instant::now();
    let label_to_target_map = construct_label_to_target_map(skyframe_executor, loading_result);
    // PORT: AnalysisPhaseStartedEvent is not posted (event bus).

    // Configuration creation.
    // TODO(gregce): Consider dropping this phase and passing on-the-fly target / exec configs as
    // needed. This requires cleaning up the invalidation in SkyframeBuildView.setConfigurations.
    skyframe_executor.set_baseline_configuration(target_options);
    let top_level_config = skyframe_executor
        .create_configuration(
            event_handler.clone(),
            target_options.clone(),
            options.keep_going,
        )
        .map_err(BuildViewError::InvalidConfiguration)?;
    // PORT: getTopLevelConfigurationTrimmedOfTestOptions only feeds TopLevelConfigRequestedEvent
    // and remote analysis caching (not ported); MakeEnvironmentEvent and the configuration's
    // build event are not posted (event bus).
    // Java: SkyframeBuildView.setConfiguration
    if let Some(services) = skyframe_executor.get_analysis_services() {
        services
            .artifact_factory
            .set_sibling_repository_layout(top_level_config.sibling_repository_layout());
    }

    // Lightly chastize the user for disabling visibility checking. (Previously, we spammed them
    // for every visibility failure; #16767.)
    // Java: BuildConfigurationValue.checkVisibility() (CoreOptions.checkVisibility).
    if !top_level_config
        .get_options()
        .get::<blaze_optdefs::fragment::core_options::CoreOptions>()
        .expect("CoreOptions")
        .check_visibility
    {
        event_handler.handle(&Event::warn(
            "This build has globally disabled target visibility checking (--nocheck_visibility).",
        ));
    }

    let configuration_key = top_level_config.get_key();
    let top_level_ct_keys: Vec<ConfiguredTargetKey> = label_to_target_map
        .keys()
        .map(|label| {
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(Some(configuration_key.clone()))
                .build()
        })
        .collect();

    if !options.aspects.is_empty() {
        blaze_util::unported!("BuildView.createTopLevelAspectKeys");
    }

    // PORT: ArtifactFactory.noteAnalysisStarting only resets incremental state.
    let skyframe_analysis_result = skyframe_build_view::configure_targets(
        skyframe_executor,
        event_handler.clone(),
        &label_to_target_map,
        &top_level_ct_keys,
        options.keep_going,
        options.check_for_action_conflicts,
    )?;

    let num_targets_to_analyze = label_to_target_map.len();
    let num_successful = skyframe_analysis_result.get_configured_targets().len();
    if 0 < num_successful && num_successful < num_targets_to_analyze {
        let msg = format!(
            "{} succeeded for only {} of {} top-level targets",
            "Analysis", num_successful, num_targets_to_analyze
        );
        event_handler.handle(&Event::info(msg));
    }

    // PORT: reportIncompatibleTargets (TopLevelConstraintSemantics) is not ported; cquery
    // disables it and no incompatible top-level targets are skipped.
    let targets_to_skip: Vec<ConfiguredTarget> = Vec::new();

    // The counters of AnalysisPhaseRunner's AnalysisPhaseCompleteEvent.
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:431 AnalysisPhaseRunner.runAnalysisPhase
    let analysis_phase_complete_stats = skyframe_build_view::get_evaluated_counts(
        skyframe_executor.get_evaluator(),
        timer.elapsed().as_millis() as i64,
    );

    Ok(create_result(
        loading_result,
        top_level_config,
        skyframe_analysis_result,
        targets_to_skip,
        analysis_phase_complete_stats,
    ))
}

// Java: com/google/devtools/build/lib/analysis/BuildView.java:517 BuildView.constructLabelToTargetMap
pub fn construct_label_to_target_map(
    skyframe_executor: &SkyframeExecutor,
    loading_result: &TargetPatternPhaseValue,
) -> IndexMap<Label, Target> {
    let labels: &IndexSet<Label> = loading_result.get_target_labels();
    let mut builder: IndexMap<Label, Target> = IndexMap::with_capacity(labels.len());
    for label in labels {
        let pkg = skyframe_executor
            .get_existing_package(label.get_package_identifier())
            .unwrap_or_else(|| panic!("NullPointerException: {}", label));
        let target = pkg
            .get_targets()
            .get(label.get_name())
            .cloned()
            .unwrap_or_else(|| panic!("NullPointerException: {}", label));
        if builder.insert(label.clone(), target).is_some() {
            panic!("IllegalArgumentException: Multiple entries with same key: {label}");
        }
    }
    builder
}

// Java: com/google/devtools/build/lib/analysis/BuildView.java:622 BuildView.createResult
fn create_result(
    loading_result: &TargetPatternPhaseValue,
    configuration: Arc<BuildConfigurationValue>,
    skyframe_analysis_result: SkyframeAnalysisResult,
    targets_to_skip: Vec<ConfiguredTarget>,
    analysis_phase_complete_stats: AnalysisPhaseCompleteStats,
) -> AnalysisResult {
    let tests_to_run = loading_result.get_tests_to_run_labels();
    let configured_targets: Vec<ConfiguredTarget> =
        skyframe_analysis_result.get_configured_targets().to_vec();

    let all_targets_to_test: Option<Vec<ConfiguredTarget>> =
        tests_to_run.map(|tests| filter_tests_by_targets(&configured_targets, tests));

    let failure_detail =
        create_analysis_failure_detail(loading_result, Some(&skyframe_analysis_result));

    AnalysisResult {
        configuration,
        targets_to_build: configured_targets,
        targets_to_test: all_targets_to_test,
        targets_to_skip,
        failure_detail,
        walkable_graph: skyframe_analysis_result.get_walkable_graph().clone(),
        top_level_targets_with_configs: skyframe_analysis_result
            .get_targets_with_configuration()
            .to_vec(),
        analysis_phase_complete_stats,
        build_graph_metrics: skyframe_analysis_result.get_build_graph_metrics().cloned(),
    }
}

/// Returns the collection of configured targets corresponding to any of the provided targets.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:207 BuildView.filterTestsByTargets
pub fn filter_tests_by_targets(
    targets: &[ConfiguredTarget],
    allowed_target_labels: &IndexSet<Label>,
) -> Vec<ConfiguredTarget> {
    targets
        .iter()
        .filter(|ct| allowed_target_labels.contains(&ct.get_label()))
        .cloned()
        .collect()
}

/// Check for errors in "chronological" order (acknowledge that loading and analysis are
/// interleaved, but sequential on the single target scale).
// Java: com/google/devtools/build/lib/analysis/BuildView.java:752 BuildView.createAnalysisFailureDetail
pub fn create_analysis_failure_detail(
    loading_result: &TargetPatternPhaseValue,
    skyframe_analysis_result: Option<&SkyframeAnalysisResult>,
) -> Option<FailureDetail> {
    if loading_result.has_error() {
        return Some(FailureDetail {
            message: "command succeeded, but there were errors parsing the target pattern"
                .to_owned(),
            category: Some(Category::TargetPatterns(failure_details::TargetPatterns {
                code: failure_details::target_patterns::Code::TargetPatternParseFailure as i32,
            })),
        });
    }
    if loading_result.has_post_expansion_error()
        || skyframe_analysis_result.is_some_and(|r| r.has_loading_error())
    {
        return Some(FailureDetail {
            message: "command succeeded, but there were loading phase errors".to_owned(),
            category: Some(Category::Analysis(failure_details::Analysis {
                code: failure_details::analysis::Code::GenericLoadingPhaseFailure as i32,
            })),
        });
    }
    if skyframe_analysis_result.is_some_and(|r| r.has_analysis_error()) {
        return Some(FailureDetail {
            message: "command succeeded, but not all targets were analyzed".to_owned(),
            category: Some(Category::Analysis(failure_details::Analysis {
                code: failure_details::analysis::Code::NotAllTargetsAnalyzed as i32,
            })),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/analysis/BuildViewTest.java
    // PORT: BuildViewTest drives a full BuildViewTestCase (analysis mock); not portable yet.
    #[test]
    #[ignore = "needs the BuildViewTestCase analysis harness (configured target functions)"]
    fn test_rule_configured_target() {}
}

/// The counters of `AnalysisPhaseCompleteEvent`.
// Java: com/google/devtools/build/lib/analysis/AnalysisPhaseCompleteEvent.java:30 AnalysisPhaseCompleteEvent
#[derive(Clone, Debug, Default)]
pub struct AnalysisPhaseCompleteStats {
    pub actions_constructed_total: i32,
    pub actions_constructed_configured_targets_only: i32,
    pub targets_configured_total: i32,
    pub targets_configured_configured_targets_only: i32,
    pub time_in_ms: i64,
    pub packages_successfully_loaded: i32,
    pub actions_constructed_by_mnemonic: Vec<(String, i32)>,
}
