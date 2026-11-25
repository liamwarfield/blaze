//! Port of the analysis-only part of `analysis/BuildView.java` (`update`, `createResult`,
//! `createAnalysisFailureDetail`) and `analysis/AnalysisResult.java`.
//!
//! PORT: the execution-phase parts of `createResult` (workspace status artifacts, extra actions,
//! coverage artifacts, the artifacts to build, test partitioning by exclusivity, package roots)
//! are not computed here: they belong to the build driver. `AnalysisResult` carries what the
//! analysis-only commands (cquery, aquery) read.
//!
//! PORT: incompatible top-level targets. Java has two paths. Without the execution phase
//! (`includeExecutionPhase` false: cquery, aquery, `--nobuild`,
//! `--noexperimental_merged_skyframe_analysis_execution`) `BuildView.update` runs
//! `TopLevelConstraintSemantics` itself; that part is ported as written. With Skymeld (the default
//! of `build` and `test`) `analyzeAndExecuteTargets` evaluates one `BuildDriverKey` per top-level
//! target and `BuildDriverFunction` does the check, an explicitly requested incompatible target
//! being an error of that key that `SkyframeErrorProcessor.processErrors` reports. Blaze analyzes
//! every top-level target before it executes any, so for `include_execution_phase` the
//! compatibility part of `BuildDriverFunction.compute` and the handling of its
//! `TargetCompatibilityCheckException` by `SkyframeErrorProcessor` run here, after
//! `configureTargets`, in top-level target order (see `check_target_compatibility_of_build_driver_keys`).
//! In both paths the skipped targets are `AnalysisResult::get_targets_to_skip()` (Java's Skymeld
//! path tracks them through `TopLevelTargetSkippedEvent` in the `BuildResultListener` instead).

use std::sync::Arc;

use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_key_creator::{AspectKeyCreator, TopLevelAspectsKey};
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::ConfiguredTargetKey;
use blaze_analysis::constraints::top_level_constraint_semantics::{
    PlatformCompatibility, TargetLookup, TargetLookupException, TopLevelConstraintSemantics,
};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::build_options::BuildOptions;
use blaze_build_config::fragment::InvalidConfigurationException;
use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{Label, LabelRepoContextImpl, RepositoryMapping};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::aspect::{AspectClass, StarlarkAspectClass};
use blaze_packages::target::Target;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_sky_analysis::top_level_status_events::{
    TopLevelTargetAnalyzedEvent, TopLevelTargetSkippedEvent,
};
use blaze_sky_loading::package_manager::RepositoryMappingResolutionOrInterrupted;
use blaze_skyframe::WalkableGraph;
use indexmap::{IndexMap, IndexSet};

use crate::skyframe_build_view::{self, ConfigureTargetsError, SkyframeAnalysisResult};
use crate::skyframe_error_processor::ViewCreationFailedException;
use crate::skyframe_executor::{SkyframeExecutor, TargetAndConfiguration};
use crate::target_pattern_phase::TargetPatternPhaseValue;

/// Return value for `BuildView.update`.
// Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:36 AnalysisResult
pub struct AnalysisResult {
    configuration: Arc<BuildConfigurationValue>,
    targets_to_build: Vec<ConfiguredTarget>,
    targets_to_test: Option<Vec<ConfiguredTarget>>,
    targets_to_skip: Vec<ConfiguredTarget>,
    aspects: IndexMap<AspectKey, Arc<AspectValue>>,
    /// The `TopLevelAspectsKey`s `BuildView.update` analyzed (Java's `aspectKeys`, passed on to
    /// `SkyframeBuildView.analyzeAndExecuteTargets`).
    top_level_aspects_keys: Vec<TopLevelAspectsKey>,
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

    /// Returns aspects of configured targets to build.
    ///
    /// If this list is empty, build the targets returned by `getTargetsToBuild()`. Otherwise,
    /// only build these aspects of the targets returned by `getTargetsToBuild()`.
    // Java: com/google/devtools/build/lib/analysis/AnalysisResult.java:116 AnalysisResult.getAspectsMap
    pub fn get_aspects_map(&self) -> &IndexMap<AspectKey, Arc<AspectValue>> {
        &self.aspects
    }

    /// The `TopLevelAspectsKey`s of the request, in order: the keys Java's `BuildView.update`
    /// hands `SkyframeBuildView.analyzeAndExecuteTargets` for its
    /// `BuildDriverKey.ofTopLevelAspect`s (`execution_seam::build_driver_aspect_keys`).
    ///
    /// PORT: Java's `AnalysisResult` does not carry them; Blaze runs the execution phase after
    /// `BuildView.update` returns, so the keys travel with the result.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:372 BuildView.update
    pub fn get_top_level_aspects_keys(&self) -> &[TopLevelAspectsKey] {
        &self.top_level_aspects_keys
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
    /// `explicitTargetPatterns`: the labels of the target patterns of the command line that name
    /// single targets (`AnalysisPhaseRunner.getExplicitTargetPatterns`).
    pub explicit_target_patterns: IndexSet<Label>,
    /// `skipIncompatibleExplicitTargets` (`AnalysisOptions.skipIncompatibleExplicitTargets`).
    pub skip_incompatible_explicit_targets: bool,
    /// `reportIncompatibleTargets` (`BuildRequest.reportIncompatibleTargets()`).
    pub report_incompatible_targets: bool,
    /// `includeExecutionPhase`: the request merges analysis and execution in Java
    /// (`CommandEnvironment.withMergedAnalysisAndExecutionSourceOfTruth()`, Skymeld).
    pub include_execution_phase: bool,
}

/// The BuildView is Bazel's main analysis entry point.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:216 BuildView.update
/// `eventBus.post(topLevelConfig.toBuildEvent())` of `BuildView.update`: the top-level
/// configuration, whose build event the command's build event stream builds
/// (`BuildConfigurationValue.toBuildEvent`).
// Java: com/google/devtools/build/lib/analysis/BuildView.java:350 BuildView.update
#[derive(Clone, Debug)]
pub struct TopLevelConfigurationEvent {
    pub configuration: Arc<BuildConfigurationValue>,
}

impl blaze_events::Postable for TopLevelConfigurationEvent {}

/// `new TargetConfiguredEvent(target, configuration)` as `BuildDriverFunction
/// .announceTopLevelConfiguredTargetAnalyzed` posts it (the merged analysis and execution path):
/// the target of the configured target's original label and the configuration of its key.
///
/// PORT: the event's class lives in blaze_bep, which this crate does not depend on; the build
/// event stream converts this one (blaze_runtime commands/bep.rs).
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
#[derive(Clone, Debug)]
pub struct TopLevelTargetConfiguredEvent {
    pub target: Target,
    pub configuration: Option<Arc<BuildConfigurationValue>>,
}

impl blaze_events::Postable for TopLevelTargetConfiguredEvent {}

/// `announceTopLevelConfiguredTargetAnalyzed`'s `TargetConfiguredEvent` (once per key: the
/// function posts it when `TOP_LEVEL_TARGET_CONFIGURED` is first added to its posted types).
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
fn announce_top_level_target_configured(
    graph: &dyn blaze_skyframe::WalkableGraph,
    configured_target: &ConfiguredTarget,
    target_lookup: &dyn TargetLookup,
    event_handler: &dyn ExtendedEventHandler,
) {
    let label = configured_target.get_original_label();
    // ((PackageValue) env.getValue(label.getPackageIdentifier())).getPackage()
    // .getTarget(label.getName())
    let target = match target_lookup.get_target(&label) {
        Ok(Some((target, _))) => target,
        _ => panic!(
            "IllegalStateException: Target should already be verified and available for top level ConfiguredTarget: {}",
            label
        ),
    };
    event_handler.post(Arc::new(TopLevelTargetConfiguredEvent {
        target,
        // getConfigurationValue(env, configuredTarget.getConfigurationKey())
        configuration: crate::skyframe_executor::get_configuration_from_graph(
            graph,
            configured_target.get_configuration_key().as_ref(),
        ),
    }));
}

pub fn update(
    skyframe_executor: &SkyframeExecutor,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
    options: &BuildViewUpdateOptions,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<AnalysisResult, BuildViewError> {
    update_with_aspects_parameters(
        skyframe_executor,
        loading_result,
        target_options,
        options,
        &IndexMap::new(),
        event_handler,
    )
}

/// [`update`] with Java's `aspectsParameters` argument (`BuildRequest.getAspectsParameters()`,
/// `--aspects_parameters`).
///
/// PORT: a separate entry point rather than a field of [`BuildViewUpdateOptions`], so that the
/// callers that build the options literally keep compiling; [`update`] passes no parameters
/// (`ImmutableMap.of()`).
// Java: com/google/devtools/build/lib/analysis/BuildView.java:216 BuildView.update
pub fn update_with_aspects_parameters(
    skyframe_executor: &SkyframeExecutor,
    loading_result: &TargetPatternPhaseValue,
    target_options: &Arc<BuildOptions>,
    options: &BuildViewUpdateOptions,
    aspects_parameters: &IndexMap<String, String>,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<AnalysisResult, BuildViewError> {
    // AnalysisPhaseRunner.runAnalysisPhase: Stopwatch timer = Stopwatch.createStarted().
    // PORT: the analysis timer runs over BuildView.update (Java starts it just before).
    let timer = std::time::Instant::now();
    let label_to_target_map = construct_label_to_target_map(skyframe_executor, loading_result);
    // PORT: AnalysisPhaseStartedEvent is not posted: Blaze has no port of the event; its Java
    // subscribers are the UI's progress state and the metrics collector's analysis timer.

    // Configuration creation.
    // TODO(gregce): Consider dropping this phase and passing on-the-fly target / exec configs as
    // needed. This requires cleaning up the invalidation in SkyframeBuildView.setConfigurations.
    skyframe_executor
        .set_baseline_configuration(target_options, event_handler.clone())
        .map_err(BuildViewError::InvalidConfiguration)?;
    let top_level_config = skyframe_executor
        .create_configuration(
            event_handler.clone(),
            target_options.clone(),
            options.keep_going,
        )
        .map_err(BuildViewError::InvalidConfiguration)?;
    // PORT: getTopLevelConfigurationTrimmedOfTestOptions only feeds TopLevelConfigRequestedEvent
    // (the `config` command's collector) and remote analysis caching, neither ported.
    // MakeEnvironmentEvent is not posted (no port of the event).
    // eventBus.post(topLevelConfig.toBuildEvent()): the build event stream converts it.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:350 BuildView.update
    event_handler.post(Arc::new(TopLevelConfigurationEvent {
        configuration: top_level_config.clone(),
    }));
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

    let aspect_keys = create_top_level_aspect_keys(
        skyframe_executor,
        &options.aspects,
        aspects_parameters,
        &label_to_target_map
            .keys()
            .cloned()
            .collect::<IndexSet<Label>>(),
        &top_level_config,
        options.keep_going,
        event_handler.clone(),
    )?;

    // PORT: ArtifactFactory.noteAnalysisStarting only resets incremental state.
    // With the execution phase, Java's SkyframeBuildView.analyzeAndExecuteTargets requests the
    // workspace status artifacts together with the top-level BuildDriverKeys, right after
    // `executionSetupCallback.prepareForExecution()`; the workspace status action then runs while
    // the top-level targets are analyzed.
    // PORT: Blaze analyzes the top-level targets (configureTargets below) before it evaluates
    // the BuildDriverKeys (module comment of blaze_runtime commands/build.rs), so the workspace
    // status artifacts are evaluated here, before the analysis; the build command requests them
    // again with the BuildDriverKeys (done nodes, or their error). The symlink forest of
    // prepareForExecution is planted by the build command before the BuildDriverKeys; the
    // workspace status action writes only to the output tree.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:654 SkyframeBuildView.analyzeAndExecuteTargets
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2327 SkyframeExecutor.configureTargets
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2492 SkyframeExecutor.evaluateBuildDriverKeys
    // PORT: both of Java's paths post ConfigurationPhaseStartedEvent before their evaluation
    // (configureTargets without, evaluateBuildDriverKeys with the execution phase); Blaze's one
    // path posts it here, before the workspace status artifacts and configureTargets below.
    event_handler.post(Arc::new(
        blaze_buildtool::buildtool::buildevent::ConfigurationPhaseStartedEvent::new(Arc::new(
            crate::skyframe_progress_receiver::AnalysisProgressReceiverView(
                &blaze_sky_analysis::analysis_progress_receiver::ANALYSIS_PROGRESS,
            ),
        )),
    ));
    if options.include_execution_phase {
        // For the workspace status actions.
        event_handler.post(Arc::new(
            blaze_sky_exec::events::SomeExecutionStartedEvent::not_counted_in_execution_time(),
        ));
    }
    if options.include_execution_phase
        && let Some(execution) = skyframe_executor.get_execution()
    {
        blaze_buildtool::buildtool::execution_tool::evaluate_build_driver_keys(
            skyframe_executor.get_evaluator(),
            &[],
            &execution.seam.get_workspace_status_artifacts(),
            &execution.config,
            event_handler.clone(),
        )
        .map_err(|_| BuildViewError::Interrupted)?;
    }
    let mut skyframe_analysis_result = skyframe_build_view::configure_targets(
        skyframe_executor,
        event_handler.clone(),
        &label_to_target_map,
        &top_level_ct_keys,
        &aspect_keys,
        options.keep_going,
        options.check_for_action_conflicts,
    )?;
    // Java: setArtifactRoots(skyframeAnalysisResult.getPackageRoots()).
    set_artifact_roots(skyframe_executor);

    // PORT: with the execution phase, Java's `analyzeAndExecuteTargets` has checked the
    // compatibility of each top-level target in its `BuildDriverFunction` by now (module comment).
    let mut targets_to_skip: Vec<ConfiguredTarget> = Vec::new();
    if options.include_execution_phase {
        // PORT: a target dropped for an action conflict was analyzed too: Java's
        // BuildDriverFunction announces it before the conflict check of the merged phase.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
        let target_lookup = SkyframeTargetLookup {
            skyframe_executor,
            event_handler: event_handler.clone(),
        };
        let graph = skyframe_executor.get_walkable_graph();
        for configured_target in
            skyframe_analysis_result.get_configured_targets_with_action_conflicts()
        {
            announce_top_level_target_configured(
                &*graph,
                configured_target,
                &target_lookup,
                event_handler.as_ref(),
            );
            event_handler.post(Arc::new(TopLevelTargetAnalyzedEvent::create(
                configured_target.clone(),
            )));
        }
        let (result, skipped) = check_target_compatibility_of_build_driver_keys(
            skyframe_executor,
            skyframe_analysis_result,
            options,
            event_handler.as_ref(),
            &target_lookup,
        )?;
        skyframe_analysis_result = result;
        targets_to_skip = skipped;
        // PORT: Java's BuildDriverFunction of each TopLevelAspectsKey announces its aspects as
        // analyzed (`announceTopLevelAspectAnalyzed`); the ToplevelAspectsIdentifiedEvent and
        // AspectConfiguredEvents were posted by `configureTargets`, the AspectAnalyzedEvents
        // are posted here, in the order of the aspects map.
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:449 BuildDriverFunction.announceTopLevelAspectAnalyzed
        for (aspect_key, aspect_value) in skyframe_analysis_result.get_aspects() {
            event_handler.post(Arc::new(
                crate::analysis_phase_runner::AspectAnalyzedEvent::create(
                    aspect_key.clone(),
                    aspect_value.clone(),
                ),
            ));
        }
        // PORT: Java's SkyframeBuildView.analyzeAndExecuteTargets creates a
        // `BuildDriverKey.ofTopLevelAspect` per TopLevelAspectsKey next to the configured-target
        // keys; the build command creates them from the analysis result
        // (`AnalysisResult::get_top_level_aspects_keys`,
        // `execution_seam::build_driver_aspect_keys`).
        crate::execution_seam::note_top_level_aspects_analyzed_for_execution(aspect_keys.len());
    }

    let num_targets_to_analyze = label_to_target_map.len();
    let num_successful = skyframe_analysis_result.get_configured_targets().len();
    // PORT: with the execution phase the line is "Build succeeded for only ..." and counts the
    // targets that were built (Java: SkyframeBuildView.analyzeAndExecuteTargets); the build
    // command prints it after execution (blaze_runtime commands/build.rs).
    if !options.include_execution_phase
        && 0 < num_successful
        && num_successful < num_targets_to_analyze
    {
        let msg = format!(
            "Analysis succeeded for only {} of {} top-level targets",
            num_successful, num_targets_to_analyze
        );
        event_handler.handle(&Event::info(msg));
    }

    // With the execution phase:
    // TODO(b/199053098): Also consider targets with errors like below.
    // PORT: Java passes `targetsToSkip = ImmutableSet.of()` to createResult then: the skipped
    // targets of a Skymeld build are the `BuildResultListener`'s (module comment).
    if !options.include_execution_phase && options.report_incompatible_targets {
        let top_level_constraint_semantics =
            TopLevelConstraintSemantics::new(event_handler.as_ref());

        let platform_restrictions = top_level_constraint_semantics
            .check_platform_restrictions(
                skyframe_analysis_result.get_configured_targets(),
                &options.explicit_target_patterns,
                options.keep_going,
                options.skip_incompatible_explicit_targets,
            )
            .map_err(|e| BuildViewError::ViewCreationFailed {
                message: e.message,
                failure_detail: e.failure_detail,
            })?;

        if !platform_restrictions.targets_with_errors().is_empty() {
            // If there are any errored targets (e.g. incompatible targets that are explicitly
            // specified on the command line), remove them from the list of targets to be built.
            skyframe_analysis_result = skyframe_analysis_result
                .with_additional_errored_targets(platform_restrictions.targets_with_errors());
        }

        let graph = skyframe_executor.get_walkable_graph();
        let bad_environment_targets = top_level_constraint_semantics
            .check_target_environment_restrictions(
                skyframe_analysis_result.get_configured_targets(),
                // Java: TopLevelConstraintSemantics.getConfigurationValue
                // (`evaluator.getExistingValue(key)`).
                &|target: &ConfiguredTarget| {
                    crate::skyframe_executor::get_configuration_from_graph(
                        &*graph,
                        target.get_configuration_key().as_ref(),
                    )
                },
                // Java: TopLevelConstraintSemantics.getOrLoadTarget
                &SkyframeTargetLookup {
                    skyframe_executor,
                    event_handler: event_handler.clone(),
                },
                event_handler.as_ref(),
            )
            .map_err(|e| BuildViewError::ViewCreationFailed {
                message: e.message,
                failure_detail: e.failure_detail,
            })?;
        // Sets.union(checkTargetEnvironmentRestrictions(..), platformRestrictions.targetsToSkip())
        targets_to_skip = bad_environment_targets;
        for target in platform_restrictions.targets_to_skip() {
            if !contains_configured_target(&targets_to_skip, target) {
                targets_to_skip.push(target.clone());
            }
        }
    }

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
        aspect_keys,
        targets_to_skip,
        analysis_phase_complete_stats,
    ))
}

/// Set membership of a configured target (Java: `ConfiguredTarget` identity; one instance per
/// lookup key in a graph).
fn contains_configured_target(targets: &[ConfiguredTarget], target: &ConfiguredTarget) -> bool {
    targets
        .iter()
        .any(|t| t.get_lookup_key() == target.get_lookup_key())
}

/// The target compatibility part of `BuildDriverFunction.compute` for every analyzed top-level
/// target, and `SkyframeErrorProcessor.processErrors` for the `TargetCompatibilityCheckException`
/// it throws for an incompatible target that was explicitly requested (module comment).
///
/// Returns the analysis result without the errored targets, and the skipped targets.
///
/// PORT: Java evaluates the `BuildDriverKey`s concurrently; without `--keep_going` the first
/// error to occur ends the build. Here the targets are checked in top-level order, so the first
/// explicitly requested incompatible target in that order is the one reported.
// Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:254 BuildDriverFunction.compute
fn check_target_compatibility_of_build_driver_keys(
    skyframe_executor: &SkyframeExecutor,
    skyframe_analysis_result: SkyframeAnalysisResult,
    options: &BuildViewUpdateOptions,
    event_handler: &dyn ExtendedEventHandler,
    target_lookup: &dyn TargetLookup,
) -> Result<(SkyframeAnalysisResult, Vec<ConfiguredTarget>), BuildViewError> {
    let graph = skyframe_executor.get_walkable_graph();
    let mut skipped: Vec<ConfiguredTarget> = Vec::new();
    let mut targets_with_errors: Vec<ConfiguredTarget> = Vec::new();
    for configured_target in skyframe_analysis_result.get_configured_targets() {
        // At this point, the target is considered "analyzed".
        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:415 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
        announce_top_level_target_configured(
            &*graph,
            configured_target,
            target_lookup,
            event_handler,
        );
        event_handler.post(Arc::new(TopLevelTargetAnalyzedEvent::create(
            configured_target.clone(),
        )));

        // `buildDriverKey.isExplicitlyRequested()`: `explicitTargetPatterns.contains(k.getLabel())`
        // for the target's key (SkyframeBuildView.analyzeAndExecuteTargets).
        let top_level_label = configured_target.get_original_label();
        let is_explicitly_requested = options.explicit_target_patterns.contains(&top_level_label);

        // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:523 BuildDriverFunction.isConfiguredTargetCompatible
        let is_configured_target_compatible =
            match TopLevelConstraintSemantics::compatibility_with_platform_restrictions(
                configured_target,
                event_handler,
                /* eagerly_throw_error= */ true,
                is_explicitly_requested,
                options.skip_incompatible_explicit_targets,
            ) {
                Ok(PlatformCompatibility::INCOMPATIBLE_EXPLICIT)
                | Ok(PlatformCompatibility::INCOMPATIBLE_IMPLICIT) => Ok(false),
                Ok(PlatformCompatibility::COMPATIBLE) => {
                    let build_configuration_value =
                        crate::skyframe_executor::get_configuration_from_graph(
                            &*graph,
                            configured_target.get_configuration_key().as_ref(),
                        );
                    TopLevelConstraintSemantics::compatibility_with_target_environment(
                        configured_target,
                        build_configuration_value.as_deref(),
                        target_lookup,
                        event_handler,
                    )
                    .map(|environment_compatibility| {
                        // PORT: the lookup loads packages outside Skyframe, so a value is never
                        // missing (Java: BuildDriverFunction returns null then).
                        environment_compatibility
                            .expect("environment compatibility")
                            .is_compatible()
                    })
                }
                Err(e) => Err(e),
            };
        match is_configured_target_compatible {
            Ok(true) => {}
            Ok(false) => {
                event_handler.post(Arc::new(TopLevelTargetSkippedEvent::create(
                    configured_target.clone(),
                )));
                // We consider the evaluation of this BuildDriverKey successful at this point,
                // even when the target is skipped.
                skipped.push(configured_target.clone());
            }
            // The analysis of the target technically succeeded, just that it was incompatible
            // and can't be executed.
            Err(cause) => {
                // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:302 SkyframeErrorProcessor.processErrors
                // (`processIndividualError`: a TargetCompatibilityCheckException has no root
                // causes and is an analysis error).
                // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:411 SkyframeErrorProcessor.maybePostFailureEventsForNonConflictError
                // (the effective error key is the BuildDriverKey's ActionLookupKey; analysis
                // review R2-6).
                if let Some(ct_key) = configured_target
                    .get_lookup_key()
                    .as_any()
                    .downcast_ref::<ConfiguredTargetKey>()
                {
                    event_handler.post(Arc::new(
                        crate::analysis_failure_event::AnalysisFailureEvent::while_analyzing_target(
                            ct_key.clone(),
                            Vec::new(),
                        ),
                    ));
                }
                if options.keep_going {
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:665 SkyframeErrorProcessor.logOrPrintWarningsKeepGoing
                    let mut message = format!(
                        "errors encountered while analyzing target '{}', it will not be built.",
                        top_level_label
                    );
                    message.push_str(&format!("\n{}", cause.get_message()));
                    event_handler.handle(&Event::warn(message));
                    targets_with_errors.push(configured_target.clone());
                } else {
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:472 SkyframeErrorProcessor.throwOrReturnAspectAnalysisException
                    let error_msg = format!(
                        "Analysis of target '{}' failed; build aborted",
                        top_level_label
                    );
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeErrorProcessor.java:736 SkyframeErrorProcessor.maybeContextualizeFailureDetail
                    let original_failure_detail = cause.get_failure_detail();
                    let failure_detail = FailureDetail {
                        message: format!("{}: {}", error_msg, original_failure_detail.message),
                        category: original_failure_detail.category.clone(),
                    };
                    // ViewCreationFailedException.combineMessages
                    let message = if cause.get_message().is_empty() {
                        error_msg
                    } else {
                        format!("{}: {}", error_msg, cause.get_message())
                    };
                    return Err(BuildViewError::ViewCreationFailed {
                        message,
                        failure_detail,
                    });
                }
            }
        }
    }
    let skyframe_analysis_result = if targets_with_errors.is_empty() {
        skyframe_analysis_result
    } else {
        // The errored BuildDriverKeys have no value: their targets are not among the result's
        // configured targets and the result has an analysis error.
        skyframe_analysis_result.with_additional_errored_targets(&targets_with_errors)
    };
    Ok((skyframe_analysis_result, skipped))
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

/// `createAnalysisFailureDetail(String errorMessage, Analysis.Code code)`.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:740 BuildView.createAnalysisFailureDetail
fn create_analysis_failure_detail_with_code(
    error_message: &str,
    code: failure_details::analysis::Code,
) -> FailureDetail {
    FailureDetail {
        message: error_message.to_owned(),
        category: Some(Category::Analysis(failure_details::Analysis {
            code: code as i32,
        })),
    }
}

/// The aspect class part of `createTopLevelAspectKeys` for one `--aspects` value: `label%aspect`
/// (with the legacy forms of the label part) or the name of a native aspect.
///
/// `native_aspect_class_map` stands for `ruleClassProvider.getNativeAspectClassMap()`.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:548 BuildView.createTopLevelAspectKeys
pub fn create_top_level_aspect_class(
    aspect: &str,
    main_repo_mapping: &RepositoryMapping,
    native_aspect_class_map: &dyn Fn(&str) -> Option<AspectClass>,
) -> Result<AspectClass, ViewCreationFailedException> {
    // Syntax: label%aspect
    if let Some(delimiter_position) = aspect.find('%') {
        // TODO(jfield): For consistency with Starlark loads, the aspect should be specified
        // as an absolute label.
        // We convert it for compatibility reasons (this will be removed in the future).
        let mut bzl_file_load_like_string = aspect[..delimiter_position].to_owned();
        if !bzl_file_load_like_string.starts_with("//")
            && !bzl_file_load_like_string.starts_with('@')
        {
            // "Legacy" behavior of '--aspects' parameter.
            if let Some(rest) = bzl_file_load_like_string.strip_prefix('/') {
                bzl_file_load_like_string = rest.to_owned();
            }
            match bzl_file_load_like_string.rfind('/') {
                Some(last_slash_position) => {
                    bzl_file_load_like_string = format!(
                        "//{}:{}",
                        &bzl_file_load_like_string[..last_slash_position],
                        &bzl_file_load_like_string[last_slash_position + 1..]
                    );
                }
                None => {
                    bzl_file_load_like_string = format!("//:{bzl_file_load_like_string}");
                }
            }
            if !bzl_file_load_like_string.ends_with(".bzl") {
                bzl_file_load_like_string.push_str(".bzl");
            }
        }
        let starlark_file_label = match Label::parse_with_repo_context(
            &bzl_file_load_like_string,
            &LabelRepoContextImpl::of(MAIN.clone(), main_repo_mapping.clone()),
        ) {
            Ok(label) => label,
            Err(e) => {
                let error_message = format!("Invalid aspect '{}': {}", aspect, e.get_message());
                return Err(ViewCreationFailedException::with_cause(
                    error_message.clone(),
                    create_analysis_failure_detail_with_code(
                        &error_message,
                        failure_details::analysis::Code::AspectLabelSyntaxError,
                    ),
                    Some(e.get_message()),
                ));
            }
        };
        let starlark_function_name = &aspect[delimiter_position + 1..];
        // PORT: Java's StarlarkAspectClass holds `keyForBuild(starlarkFileLabel)` (a BzlLoadValue
        // key); the ported class holds the label.
        return Ok(AspectClass::Starlark(StarlarkAspectClass::new(
            starlark_file_label,
            starlark_function_name,
        )));
    }
    match native_aspect_class_map(aspect) {
        Some(aspect_factory_class) => Ok(aspect_factory_class),
        None => {
            let error_message = format!("Aspect '{aspect}' is unknown");
            Err(ViewCreationFailedException::new(
                error_message.clone(),
                create_analysis_failure_detail_with_code(
                    &error_message,
                    failure_details::analysis::Code::AspectNotFound,
                ),
            ))
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/BuildView.java:530 BuildView.createTopLevelAspectKeys
#[allow(clippy::too_many_arguments)]
fn create_top_level_aspect_keys(
    skyframe_executor: &SkyframeExecutor,
    aspects: &[String],
    aspects_parameters: &IndexMap<String, String>,
    top_level_targets: &IndexSet<Label>,
    configuration: &BuildConfigurationValue,
    keep_going: bool,
    event_handler: Arc<dyn ExtendedEventHandler>,
) -> Result<Vec<TopLevelAspectsKey>, BuildViewError> {
    // PORT: Java's getMainRepoMapping(eventHandler) reads the build's keep_going and thread
    // count from the executor; the ported one takes them.
    let main_repo_mapping = match skyframe_executor.get_main_repo_mapping(
        keep_going,
        crate::skyframe_executor::DEFAULT_THREAD_COUNT,
        event_handler,
    ) {
        Ok(mapping) => mapping,
        Err(RepositoryMappingResolutionOrInterrupted::Interrupted) => {
            return Err(BuildViewError::Interrupted);
        }
        Err(RepositoryMappingResolutionOrInterrupted::Resolution(e)) => {
            let error_message = format!(
                "Failed to get main repo mapping for aspect label canonicalization: {}",
                e
            );
            let e = ViewCreationFailedException::with_cause(
                error_message.clone(),
                create_analysis_failure_detail_with_code(
                    &error_message,
                    failure_details::analysis::Code::UnexpectedAnalysisException,
                ),
                Some(&e.to_string()),
            );
            return Err(BuildViewError::ViewCreationFailed {
                message: e.get_message().to_owned(),
                failure_detail: e.get_failure_detail().clone(),
            });
        }
    };

    let mut aspect_classes: Vec<AspectClass> = Vec::new();
    for aspect in aspects {
        let aspect_class = create_top_level_aspect_class(aspect, &main_repo_mapping, &|name| {
            // ruleClassProvider.getNativeAspectClassMap().get(aspect)
            // PORT: ConfiguredRuleClassProvider's native aspect class map is not ported.
            blaze_util::unported!(
                "ConfiguredRuleClassProvider.getNativeAspectClassMap",
                "native aspect '{name}'"
            )
        })
        .map_err(|e| BuildViewError::ViewCreationFailed {
            message: e.get_message().to_owned(),
            failure_detail: e.get_failure_detail().clone(),
        })?;
        aspect_classes.push(aspect_class);
    }
    if aspect_classes.is_empty() {
        return Ok(Vec::new());
    }

    Ok(top_level_targets
        .iter()
        .map(|target| {
            AspectKeyCreator::create_top_level_aspects_key(
                aspect_classes.clone(),
                target.clone(),
                Some(configuration),
                aspects_parameters.clone(),
            )
        })
        .collect())
}

// Java: com/google/devtools/build/lib/analysis/BuildView.java:622 BuildView.createResult
fn create_result(
    loading_result: &TargetPatternPhaseValue,
    configuration: Arc<BuildConfigurationValue>,
    skyframe_analysis_result: SkyframeAnalysisResult,
    top_level_aspects_keys: Vec<TopLevelAspectsKey>,
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
        aspects: skyframe_analysis_result.get_aspects().clone(),
        top_level_aspects_keys,
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

/// Java `PackageRoots.MapAsPackageRoots.getPackageRootLookup()`: the root of a package of the
/// map, null for any other package.
// Java: com/google/devtools/build/lib/skyframe/MapAsPackageRoots.java:39 MapAsPackageRoots.getPackageRootLookup
struct MapAsPackageRoots {
    package_roots_map: IndexMap<blaze_cmdline::PackageIdentifier, blaze_vfs::Root>,
}

impl blaze_actions::actions::artifact_factory::PackageRootLookup for MapAsPackageRoots {
    fn get_root_for_package(
        &self,
        package_identifier: &blaze_cmdline::PackageIdentifier,
    ) -> Option<blaze_vfs::Root> {
        self.package_roots_map.get(package_identifier).cloned()
    }
}

/// Sets the possible artifact roots in the artifact factory. This allows the factory to resolve
/// paths with unknown roots to artifacts.
///
/// PORT: Java passes the package roots of the top-level targets' and aspects' transitive
/// packages (`SkyframeBuildView.collectPackageRoots`); Blaze passes
/// [`SkyframeExecutor::get_package_roots`], the superset the symlink forest is planted from
/// (every package this command loaded; see there).
// Java: com/google/devtools/build/lib/analysis/BuildView.java:911 BuildView.setArtifactRoots
fn set_artifact_roots(skyframe_executor: &SkyframeExecutor) {
    let Some(services) = skyframe_executor.get_analysis_services() else {
        return;
    };
    services
        .artifact_factory
        .set_package_roots(Arc::new(MapAsPackageRoots {
            package_roots_map: skyframe_executor.get_package_roots(),
        }));
}

/// `TopLevelConstraintSemantics.TargetLookup` of a build: the loaded package, or else the
/// package loaded through the package manager (Java: `TopLevelConstraintSemantics.getOrLoadTarget`;
/// BuildDriverFunction's lookup requests the package from Skyframe, which the blocking evaluator
/// computes the same way).
// Java: com/google/devtools/build/lib/analysis/constraints/TopLevelConstraintSemantics.java:427 TopLevelConstraintSemantics.getOrLoadTarget
struct SkyframeTargetLookup<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
}

impl TargetLookup for SkyframeTargetLookup<'_> {
    fn get_target(
        &self,
        label: &Label,
    ) -> Result<
        Option<(
            Target,
            Vec<Arc<blaze_packages::target::environment_group::EnvironmentGroup>>,
        )>,
        TargetLookupException,
    > {
        let package = match self
            .skyframe_executor
            .get_existing_package(label.get_package_identifier())
        {
            Some(package) => package,
            None => {
                // Fall back to loading the target. Top-level targets are already in the graph,
                // but referenced environment targets may not yet be loaded.
                match self
                    .skyframe_executor
                    .get_package_manager()
                    .get_package(self.event_handler.clone(), label.get_package_identifier())
                {
                    Ok(package) => package,
                    Err(blaze_sky_loading::package_manager::NoSuchPackageOrInterrupted::NoSuchPackage(e)) => {
                        return Err(TargetLookupException {
                            message: e.get_message().to_string(),
                            failure_detail: crate::skyframe_error_processor::packages_failure_detail(
                                &e.get_detailed_exit_code(),
                            ),
                        });
                    }
                    Err(blaze_sky_loading::package_manager::NoSuchPackageOrInterrupted::Interrupted) => {
                        blaze_util::unported!(
                            "TopLevelConstraintSemantics.getOrLoadTarget",
                            "interrupted while loading {}",
                            label
                        )
                    }
                }
            }
        };
        let target = match package.get_target(label.get_name()) {
            Ok(target) => target.clone(),
            Err(e) => {
                return Err(TargetLookupException {
                    message: e.get_message().to_string(),
                    failure_detail: crate::skyframe_error_processor::packages_failure_detail(
                        &e.get_detailed_exit_code(),
                    ),
                });
            }
        };
        let groups = package
            .get_targets()
            .values()
            .filter_map(|t| t.as_environment_group().cloned())
            .collect();
        Ok(Some((target, groups)))
    }
}

/// `BuildView.constructCoverageArtifacts`: the coverage report actions of the build (the
/// command's `CoverageReportActionFactory`, `BlazeRuntime.getCoverageReportActionFactory`),
/// injected into Skyframe (`SkyframeExecutor.injectCoverageReportData`), and their outputs. The
/// wrapper is returned too: the caller registers what Java's factory registers on the event bus
/// (the `CoverageReportCollector`).
///
/// PORT: Java memoizes the result in the `BuildView` (`memoizedGetCoverageArtifactsHelper`);
/// the caller calls this once per build.
// Java: com/google/devtools/build/lib/analysis/BuildView.java:954 BuildView.constructCoverageArtifacts
pub fn construct_coverage_artifacts(
    skyframe_executor: &SkyframeExecutor,
    coverage_report_action_factory: Option<
        &dyn blaze_analysis::test::coverage_report_action_factory::CoverageReportActionFactory,
    >,
    configured_targets: &[ConfiguredTarget],
    all_targets_to_test: &[ConfiguredTarget],
    event_handler: &dyn blaze_events::EventHandler,
) -> (
    Vec<blaze_actions::Artifact>,
    Option<blaze_analysis::test::coverage_report_action_factory::CoverageReportActionsWrapper>,
) {
    let Some(coverage_report_action_factory) = coverage_report_action_factory else {
        return (Vec::new(), None);
    };
    let services = skyframe_executor
        .get_analysis_services()
        .expect("IllegalStateException: the executor has no analysis services");
    let actions_wrapper = coverage_report_action_factory.create_coverage_report_actions_wrapper(
        event_handler,
        &services.directories,
        configured_targets,
        all_targets_to_test,
        &services.artifact_factory,
        &services.action_key_context,
        &blaze_analysis::coverage_report_value::CoverageReportKey::action_lookup_key(),
        // ruleClassProvider.getRunfilesPrefix()
        &services.directories.workspace_name,
    );
    let Some(actions_wrapper) = actions_wrapper else {
        return (Vec::new(), None);
    };
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:1513 SkyframeExecutor.injectCoverageReportData
    blaze_analysis::coverage_report_value::COVERAGE_REPORT_ACTIONS.set_on(
        skyframe_executor.get_evaluator(),
        blaze_analysis::coverage_report_value::CoverageReportActions(
            actions_wrapper.get_actions().to_vec(),
        ),
    );
    // ImmutableSet.copyOf(actionsWrapper.getCoverageOutputs())
    let mut outputs: IndexSet<blaze_actions::Artifact> = IndexSet::new();
    outputs.extend(actions_wrapper.get_coverage_outputs());
    (outputs.into_iter().collect(), Some(actions_wrapper))
}
