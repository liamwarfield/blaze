//! The Build Event Protocol side of `build` and `test`: the BEP part of
//! `buildeventservice/BuildEventServiceModule.java` (`beforeCommand`, `afterCommand`), the event
//! subscriptions of the `BuildEventStreamer` and `metrics/MetricsCollector.java` on the command's
//! event bus, and the BEP events that `BlazeCommandDispatcher`, `CommandEnvironment`,
//! `BuildTool`, `BuildView`, `TargetPatternPhaseFunction`, `ExecutionTool` and
//! `BuildSummaryStatsModule` post at their points of the command.
//!
//! The command calls [`BepCommandSupport`] at those points (see `build.rs`):
//!
//! | call | Java |
//! |---|---|
//! | [`BepCommandSupport::before_command`] | `BuildEventServiceModule.beforeCommand`, `MetricsModule.beforeCommand`, `CommandEnvironment.beforeCommand` (`BuildMetadataEvent`, `GotOptionsEvent`) and `BlazeCommandDispatcher.execExclusively` (the command-line events) |
//! | [`BepCommandSupport::build_starting`] | `BuildTool.buildTargets` (`BuildStartingEvent`) |
//! | [`BepCommandSupport::analysis_complete`] | `TargetPatternPhaseFunction` (`TargetParsingCompleteEvent`), `BuildTool.buildTargets` (`ExecRootEvent`, `ReleaseReplaceableBuildEvent`), `AnalysisPhaseRunner` (`AnalysisPhaseStartedEvent`, `AnalysisPhaseCompleteEvent`, `AnalysisGraphStatsEvent`), `BuildView.update` (top-level configuration), `SkyframeExecutor.configureTargets` (`TargetConfiguredEvent`) |
//! | [`BepCommandSupport::convenience_symlinks_identified`] | `ExecutionTool.handleConvenienceSymlinks` |
//! | [`BepCommandSupport::build_complete`] | `BuildTool.stopRequest` (`BuildCompleteEvent`, `BuildToolLogs`), `BuildSummaryStatsModule.buildComplete` (tool log values), `UiEventHandler.buildComplete` (the progress bar while the BEP transports are open) |
//! | [`BepCommandSupport::command_precomplete`] | `MetricsCollector.onCommandPrecompleteEvent` (`BuildMetricsEvent`) |
//! | [`BepCommandSupport::after_command`] | `BuildEventServiceModule.afterCommand` |
//!
//! Events posted during the command by other crates reach the streamer through the subscriber
//! registered on the command's event bus ([`BepSubscriber`]); where the posted Rust type is not
//! the `blaze_bep` build event (Java has one class for both), the subscriber converts it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use blaze_analysis::rule::rule_error_consumer::AnalysisRootCauseEvent;
use blaze_bep::bep_module::{
    BepCommandContext, BepStreamer, BuildEventPost, BuildEventStreamerBuilder,
};
use blaze_bep::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, LocalFilesArtifactUploader, PathConverterFuture,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::transport::BuildEventTransport;
use blaze_bep::buildeventstream::{
    BuildEvent, BuildToolLogs, LocalFile, LocalFileCompression, LocalFileType, LogFileEntry,
    TransportCloseError,
};
use blaze_bep::events::actions_events::{
    ActionExecutedEvent, ActionFailure, ErrorTiming, ExecutedActionInfo, ExecutedActionOwner,
};
use blaze_bep::events::analysis_events::{
    AnalysisFailureBuildEvent, AnalysisRootCauseBuildEvent, RepositoryFailedBuildEvent,
};
use blaze_bep::events::analysis_events::{
    BuildInfoEvent, ConfiguredTargetInfo, NoBuildEvent, PackagesTestSize, TargetConfiguredEvent,
};
use blaze_bep::events::buildtool_events::{
    BuildResultData, BuildStartingData, BuildStartingEvent, ConvenienceSymlinksIdentifiedEvent,
    build_complete_event, testing_complete_event,
};
use blaze_bep::events::completion_context::{CompletionContext, ImportantInputMap};
use blaze_bep::events::pkgcache_events::{
    PatternExpandingErrorEvent, TargetParsingCompleteEvent, ThinTarget,
};
use blaze_bep::events::target_complete_event::{
    ArtifactsInOutputGroup, Cause, CompletedTargetData, TargetCompleteEvent,
};
use blaze_bep::events::target_complete_event::{
    AspectCompleteBuildEvent, AspectConfiguredBuildEvent,
};
use blaze_bep::metrics::metrics_collector::{
    AnalysisPhaseCompleteInfo, BuildMetricsFinishInputs, JvmMemoryFacts, MetricsCollector,
    PackageMetricsRecorderSnapshot, RemoteAnalysisCachingFacts, SkyframeStats,
    TotalAndConfiguredTargetOnlyMetric, WorkerProcessMetricsInfo,
};
use blaze_bep::runtime::BuildEventArtifactUploaderFactoryMap;
use blaze_bep::runtime::ReleaseReplaceableBuildEvent;
use blaze_bep::runtime::build_event_artifact_uploader_factory::BuildEventArtifactUploaderFactory;
use blaze_bep::runtime::command_line_event::{
    CanonicalCommandLineEvent, OriginalCommandLineEvent, OriginalUnstructuredCommandLineEvent,
    ToolCommandLineEvent,
};
use blaze_bep::runtime::got_options_event::GotOptionsEvent;
use blaze_bep::runtime::runtime_events::{BuildMetadataEvent, ExecRootEvent};
use blaze_bep::runtime::synchronized_output_stream::SynchronizedOutputStream;
use blaze_bep::runtime::target_summary_publisher::{
    TargetSummaryKey, TargetSummaryPublisher, TargetSummarySink, TargetSummaryTarget,
};
use blaze_bep::runtime::test_summary_event::{TargetSummaryEvent, TestSummaryEvent};
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_buildtool::buildtool::build_result::{self as tool_build_result, BuildResult};
use blaze_buildtool::buildtool::execution_tool::ExecutionEventCollector;
use blaze_buildtool::buildtool::output_directory_links_utils::SymlinkCreationResult;
use blaze_buildtool::runtime::test_command_support::{
    NoTestsFound, TestingCompleteEvent as ToolTestingCompleteEvent,
};
use blaze_buildtool::runtime::test_summary::{TestSummary, get_test_params};
use blaze_buildtool::runtime::ui_state_tracker::TransportId;
use blaze_cmdline::Label;
use blaze_events::OutErr;
use blaze_events::{Clock, Event, EventBus, EventHandler, JavaClock, Postable, Reporter};
use blaze_exec::actions::cache::PostableActionCacheStats;
use blaze_exec::analysis::actions::bazel_workspace_status_module::BuildInfoEvent as ExecBuildInfoEvent;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_optdefs::command::build_request_options::{BuildRequestOptions, ConvenienceSymlinksMode};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::{BesUploadMode, BuildEventServiceOptions};
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_optdefs::misc::metrics_module::MetricsModuleOptions;
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::build_event_stream::aborted::AbortReason;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_repo_rules::repository::repository_fetch_function::RepositoryFailedEvent;
use blaze_shell::util::shell_escaper::ShellEscaper;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionExecutedEvent as SkyActionExecutedEvent,
    ActionResultReceivedEvent, AspectCompleteEvent as SkyAspectCompleteEvent, CompletionRootCause,
    ErrorTiming as SkyErrorTiming, SomeExecutionStartedEvent,
    TargetCompleteEvent as SkyTargetCompleteEvent, TopLevelTargetPendingExecutionEvent,
    downcast_event,
};
use blaze_sky_top::analysis_failure_event::{AnalysisFailureEvent, AnalysisRootCause};
use blaze_sky_top::build_tool::{BuildEventHooks, BuildRequest};
use blaze_sky_top::build_view::{
    AnalysisResult, TopLevelConfigurationEvent, TopLevelTargetConfiguredEvent,
    construct_label_to_target_map,
};
use blaze_sky_top::execution_seam::configured_target_views;
use blaze_sky_top::pattern_expanding_error::PatternExpandingError;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_sky_top::skyframe_executor::{AspectConfiguredEvent, ToplevelAspectsIdentifiedEvent};
use blaze_sky_top::target_pattern_phase::{
    TargetPatternPhaseValue, TestsForTargetPatternKey, TestsForTargetPatternValue,
};
use blaze_skyframe::SkyKey;
use blaze_vfs::Path;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::command_environment::{CommandEnvironment, CommandEventBus};
use crate::exit_code::DetailedExitCode;
use crate::module::AbruptExitException;
use crate::options_result::OptionsParsingResult;
use crate::version_info::BlazeVersionInfo;

/// What `TargetCompleteEvent` reads from `ConfiguredTargetAndData` besides the completion
/// itself, recorded when the top-level target is configured.
#[derive(Clone)]
struct CompletedTargetFacts {
    /// `getConfiguredTarget().getOriginalLabel()`.
    original_label: Label,
    /// `configuration.toBuildEvent()` (`None` for the null configuration).
    configuration_event: Option<Arc<dyn BuildEvent>>,
    /// The target's `BuildConfigurationValue` (for `TestConfiguration.getTestTimeout()`).
    configuration: Option<Arc<BuildConfigurationValue>>,
    /// `getRuleTags()`.
    rule_tags: Vec<String>,
    /// `getTestTimeout()`: the target's categorical timeout.
    test_timeout: Option<TestTimeout>,
    /// `getProvider(TestProvider.class).getTestParams()` runs and shards (test rules only).
    test_runs_and_shards: Option<(i32, i32)>,
}

impl CompletedTargetFacts {
    /// `configuration.getFragment(TestConfiguration.class).getTestTimeout()
    /// .get(categoricalTimeout).toSeconds()`.
    // Java: com/google/devtools/build/lib/analysis/TargetCompleteEvent.java:566 TargetCompleteEvent.getTestTimeoutSeconds
    fn get_test_timeout_seconds(&self) -> i64 {
        let configuration = self
            .configuration
            .as_ref()
            .unwrap_or_else(|| panic!("NullPointerException: configuration"));
        // Java: com/google/devtools/build/lib/analysis/test/TestConfiguration.java:447 TestConfiguration.getTestTimeout
        // TestConfiguration copies `options.testTimeout` from the configuration's TestOptions.
        let options = configuration
            .get_options()
            .get::<TestConfigurationTestOptions>()
            .unwrap_or_else(|| panic!("NullPointerException: TestConfiguration"));
        let categorical_timeout = self
            .test_timeout
            .unwrap_or_else(|| panic!("NullPointerException: categoricalTimeout"));
        options
            .test_timeout
            .0
            .get(&categorical_timeout)
            .unwrap_or_else(|| panic!("NullPointerException: test timeout"))
            .as_secs() as i64
    }
}

/// The state the subscriber and the command share.
struct Shared {
    streamer: BepStreamer,
    metrics: Arc<MetricsCollector>,
    /// The completion facts of each top-level configured target, by (label, configuration
    /// checksum).
    targets: Mutex<IndexMap<(Label, Option<String>), CompletedTargetFacts>>,
    /// The `TargetSummaryPublisher` registered on the event bus (`--experimental_bep_target_summary`).
    target_summary_publisher: Option<Arc<TargetSummaryPublisher>>,
    /// `skyframeActionExecutor.publishTargetSummaries()`
    /// (`BuildEventProtocolOptions.publishTargetSummary`).
    announce_target_summary: bool,
    /// `TopLevelArtifactContext.expandFilesets()` (`BuildEventProtocolOptions.expandFilesets`).
    expand_filesets: bool,
    /// `BuildEventStreamOptions.publishAllActions`.
    publish_all_actions: bool,
    clock: Arc<dyn Clock>,
    /// `CommandEnvironment.buildInfoPosted` (set by its `gotBuildInfo` subscription).
    build_info_posted: AtomicBool,
    /// The `executionTimer` of `BuildTool.buildTargetsWithMergedAnalysisExecution` (a
    /// `Stopwatch`): `Some(start)` while running.
    execution_timer: Mutex<Option<Instant>>,
    /// `failedPatterns` of `TargetPatternPhaseFunction.compute`, in the order they failed.
    failed_target_patterns: Mutex<Vec<String>>,
}

/// The subscriber of the command's event bus: the `@Subscribe` methods of the
/// `BuildEventStreamer` and of the `MetricsCollector` for the events other crates post.
pub struct BepSubscriber {
    shared: Arc<Shared>,
}

impl EventBus for BepSubscriber {
    fn post(&self, event: Arc<dyn Postable>) {
        let shared = &self.shared;
        // `HttpDownloader` posts a `buildeventstream.FetchEvent` after each download attempt;
        // the downloader of the port posts its own copy of the class (blaze_repo_fetch is below
        // blaze_bep), which is the same event for the streamer.
        // Java: com/google/devtools/build/lib/bazel/repository/downloader/HttpDownloader.java:206 HttpDownloader.downloadWithoutChecksum
        if let Some(e) = downcast_event::<
            blaze_repo_fetch::downloader::download_progress_event::FetchEvent,
        >(&event)
        {
            let downloader = blaze_protos::build_event_stream::build_event_id::fetch_id::Downloader::from_str_name(
                e.downloader,
            )
            .unwrap_or_else(|| panic!("unknown FetchId.Downloader {}", e.downloader));
            shared.streamer.build_event(Arc::new(
                blaze_bep::buildeventstream::build_completing_event::FetchEvent::new(
                    e.url.clone(),
                    downloader,
                    e.success,
                ),
            ));
            return;
        }
        // The MetricsCollector's subscriptions.
        if let Some(e) = downcast_event::<ActionCompletionEvent>(&event) {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:240 MetricsCollector.onActionComplete
            // `getRelativeActionStartTimeNanos()` is a BlazeClock.nanoTime() reading.
            let now = Instant::now();
            let start_nanos = shared.clock.nano_time()
                - now
                    .saturating_duration_since(e.related_time_start)
                    .as_nanos() as i64;
            shared
                .metrics
                .on_action_complete(&e.action.get_mnemonic(), start_nanos);
            return;
        }
        if let Some(e) = downcast_event::<ActionResultReceivedEvent>(&event) {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:251 MetricsCollector.actionResultReceived
            shared
                .metrics
                .action_result_received(&e.action.get_mnemonic(), &e.action_result);
            return;
        }
        if let Some(e) = downcast_event::<
            blaze_exec::dynamic::dynamic_spawn_strategy::DynamicExecutionFinishedEvent,
        >(&event)
        {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:277 MetricsCollector.onDynamicExecutionFinishedEvent
            shared.metrics.on_dynamic_execution_finished_event(
                &e.mnemonic,
                &e.local_branch_name,
                &e.remote_branch_name,
                match e.winner_branch_type {
                    blaze_exec::exec::spawn_strategy_registry::DynamicMode::Local => {
                        blaze_bep::metrics::metrics_collector::DynamicMode::Local
                    }
                    blaze_exec::exec::spawn_strategy_registry::DynamicMode::Remote => {
                        blaze_bep::metrics::metrics_collector::DynamicMode::Remote
                    }
                },
            );
            return;
        }
        if let Some(e) = downcast_event::<PostableActionCacheStats>(&event) {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:326 MetricsCollector.logActionCacheStatistics
            shared.metrics.log_action_cache_statistics(e);
            return;
        }
        if let Some(e) =
            downcast_event::<blaze_bep::metrics::criticalpath::CriticalPathEvent>(&event)
        {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:318 MetricsCollector.onCriticalPath
            shared
                .metrics
                .on_critical_path(e.get_critical_path().get_aggregated_elapsed_time());
            return;
        }
        if let Some(e) = downcast_event::<SomeExecutionStartedEvent>(&event) {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:221 MetricsCollector.onSomeExecutionStarted
            // Profiler.getProfileElapsedTime() never returns null; Duration.toMillis truncates.
            shared.metrics.on_some_execution_started(
                e.counted_in_execution_time,
                Some(blaze_util::profiler::Profiler::get_profile_elapsed_time_nanos() / 1_000_000),
            );
            // ExecutionProgressReceiverSetup.setupExecutionProgressReceiver: the build's
            // executionTimer (BuildTool.buildTargetsWithMergedAnalysisExecution) starts with the
            // first execution activity counted in execution time.
            // PORT: the timer belongs to the BEP support, its only reader here (through the
            // ExecutionPhaseCompleteEvent the MetricsCollector receives).
            // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:1073 ExecutionTool.ExecutionProgressReceiverSetup.setupExecutionProgressReceiver
            if e.counted_in_execution_time {
                let mut timer = shared.execution_timer.lock();
                if timer.is_none() {
                    *timer = Some(Instant::now());
                }
            }
            return;
        }
        if let Some(e) = downcast_event::<
            blaze_sky_top::skyframe_build_view::AnalysisPhaseCompleteStatsEvent,
        >(&event)
        {
            // The merged phase's analysisFinishedCallback: AnalysisPhaseCompleteEvent and
            // AnalysisGraphStatsEvent (the path without it logs them in `analysis_complete`).
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:164 MetricsCollector.onAnalysisPhaseComplete
            log_analysis_metrics(
                &shared.metrics,
                Some(&e.stats),
                e.build_graph_metrics.as_ref(),
            );
            return;
        }
        if let Some(e) = downcast_event::<TopLevelTargetPendingExecutionEvent>(&event) {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:213 MetricsCollector.handleExecutionPhaseStart
            shared.metrics.handle_execution_phase_start();
            if let Some(publisher) = &shared.target_summary_publisher {
                publisher.populate_target(&shared.summary_target(&e.label), e.is_test);
            }
            return;
        }
        if let Some(e) = downcast_event::<ToplevelAspectsIdentifiedEvent>(&event) {
            if let Some(publisher) = &shared.target_summary_publisher {
                publisher.toplevel_aspects_identified(
                    TargetSummaryKey {
                        label: e.configured_target_key.get_label().clone(),
                        configuration_checksum: e
                            .configured_target_key
                            .get_configuration_key()
                            .map(|k| k.get_options_checksum().to_string()),
                    },
                    e.aspect_count,
                );
            }
            return;
        }
        // The build events whose Rust type is not the blaze_bep one.
        if let Some(e) = downcast_event::<SkyTargetCompleteEvent>(&event) {
            shared
                .streamer
                .build_event(Arc::new(convert_target_complete(shared, e)));
            // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:229 TargetSummaryPublisher.targetComplete
            if let Some(publisher) = &shared.target_summary_publisher {
                publisher.target_complete(
                    &TargetSummaryKey {
                        label: e.label.clone(),
                        configuration_checksum: e.configuration_checksum.clone(),
                    },
                    e.failed(),
                );
            }
            return;
        }
        if let Some(e) = downcast_event::<ExecBuildInfoEvent>(&event) {
            // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1126 CommandEnvironment.gotBuildInfo
            shared.build_info_posted.store(true, Ordering::SeqCst);
            // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:203 BazelWorkspaceStatusAction.execute
            let map: IndexMap<String, String> = e
                .get_build_info_map()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            shared
                .streamer
                .build_event(Arc::new(BuildInfoEvent::new(map)));
            return;
        }
        if let Some(summary) = downcast_event::<TestSummary>(&event) {
            shared
                .streamer
                .build_event(Arc::new(convert_test_summary(summary)));
            // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:262 TargetSummaryPublisher.testSummaryEvent
            if let Some(publisher) = &shared.target_summary_publisher {
                publisher.test_summary_event(
                    &TargetSummaryKey {
                        label: summary.get_target().get_label(),
                        configuration_checksum: summary.get_target().get_configuration_checksum(),
                    },
                    summary.get_status(),
                );
            }
            return;
        }
        if let Some(e) = downcast_event::<ToolTestingCompleteEvent>(&event) {
            // Java: com/google/devtools/build/lib/runtime/commands/TestCommand.java:227 TestCommand.doTest
            shared.streamer.build_event(Arc::new(testing_complete_event(
                e.exit_code,
                e.finish_time_millis,
            )));
            return;
        }
        if let Some(e) = downcast_event::<NoTestsFound>(&event) {
            // Java: com/google/devtools/build/lib/runtime/commands/NoTestsFound.java:25 NoTestsFound.<init>
            shared.streamer.build_event(Arc::new(
                blaze_bep::buildeventstream::BuildCompletingEvent::new(
                    e.exit_code,
                    e.finish_time_millis,
                    vec![build_event_id_util::build_metrics()],
                ),
            ));
            return;
        }
        if let Some(e) = downcast_event::<TopLevelTargetConfiguredEvent>(&event) {
            // Java: com/google/devtools/build/lib/skyframe/BuildDriverFunction.java:438 BuildDriverFunction.announceTopLevelConfiguredTargetAnalyzed
            let configuration = e.configuration.as_deref();
            shared
                .streamer
                .build_event(Arc::new(TargetConfiguredEvent::new(
                    configured_target_info(&e.target),
                    configuration.map(|c| shared.configuration_event(c)),
                    configuration.map(|c| c.checksum().to_string()),
                )));
            // What the target's TargetCompleteEvent reads of its ConfiguredTargetAndData, which
            // the merged phase posts before the analysis of the other targets is over (the path
            // without it records them in `post_targets_configured`).
            if let (Some(configured_target), Some(actual_target)) =
                (&e.configured_target, &e.actual_target)
            {
                let view = configured_target_views(std::slice::from_ref(configured_target))
                    .pop()
                    .expect("one view per configured target");
                shared.targets.lock().insert(
                    (view.get_label(), view.get_configuration_checksum()),
                    CompletedTargetFacts {
                        original_label: view.get_original_label(),
                        configuration_event: configuration.map(|c| shared.configuration_event(c)),
                        configuration: e.configuration.clone(),
                        rule_tags: actual_target.get_rule_tags().into_iter().collect(),
                        test_timeout: actual_target.get_test_timeout(),
                        test_runs_and_shards: view.get_test_params().map(|p| (p.runs, p.shards)),
                    },
                );
            }
            return;
        }
        if let Some(e) = downcast_event::<TopLevelConfigurationEvent>(&event) {
            // Java: com/google/devtools/build/lib/analysis/BuildView.java:350 BuildView.update
            shared
                .streamer
                .build_event(shared.configuration_event(&e.configuration));
            return;
        }
        if let Some(e) = downcast_event::<PatternExpandingError>(&event) {
            // TargetPatternPhaseFunction.getTargetsToBuild: each pattern that failed to parse or
            // expand is added to failedPatterns where its PatternExpandingError.failed is posted.
            // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:303 TargetPatternPhaseFunction.getTargetsToBuild
            if !e.is_skipped() {
                shared
                    .failed_target_patterns
                    .lock()
                    .extend(e.get_pattern().iter().cloned());
            }
            shared
                .streamer
                .build_event(Arc::new(PatternExpandingErrorEvent::new(
                    e.get_pattern().to_vec(),
                    e.get_message().to_string(),
                    e.is_skipped(),
                )));
            return;
        }
        if let Some(e) = downcast_event::<AnalysisRootCauseEvent>(&event) {
            // The event is a `BuildEventWithConfiguration` in Java; its build event here.
            let event = match &e.configuration {
                // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:60 AnalysisRootCauseEvent.withConfigurationValue
                Some(configuration) => AnalysisRootCauseBuildEvent::with_configuration_value(
                    e.configuration_id.as_deref(),
                    // `buildEvent(configuration.orElse(null))`: the null configuration's event
                    // for `None`.
                    configuration
                        .as_ref()
                        .map(|c| shared.configuration_event(c)),
                    e.label.clone(),
                    e.error_message.clone(),
                ),
                // Java: com/google/devtools/build/lib/analysis/AnalysisRootCauseEvent.java:69 AnalysisRootCauseEvent.withUnavailableConfiguration
                None => AnalysisRootCauseBuildEvent::with_unavailable_configuration(
                    build_event_id_util::configuration_id_message_for_key(
                        e.configuration_id.as_deref(),
                    ),
                    e.label.clone(),
                    e.error_message.clone(),
                ),
            };
            shared.streamer.build_event(Arc::new(event));
            return;
        }
        if let Some(e) = downcast_event::<RepositoryFailedEvent>(&event) {
            // Java: com/google/devtools/build/lib/bazel/repository/RepositoryFetchFunction.java:542 RepositoryFetchFunction.fetchRepository
            shared
                .streamer
                .build_event(Arc::new(RepositoryFailedBuildEvent::new(
                    e.get_repo().get_name().to_string(),
                    e.get_message().to_string(),
                )));
            return;
        }
        if let Some(e) = downcast_event::<AnalysisFailureEvent>(&event) {
            shared
                .streamer
                .build_event(Arc::new(convert_analysis_failure(e)));
            return;
        }
        if let Some(e) = downcast_event::<AspectConfiguredEvent>(&event) {
            // Java: com/google/devtools/build/lib/analysis/AspectConfiguredEvent.java:28 AspectConfiguredEvent
            let configuration = e.get_configuration();
            shared
                .streamer
                .build_event(Arc::new(AspectConfiguredBuildEvent::new(
                    e.get_target().clone(),
                    e.get_aspect_class_name().to_string(),
                    e.get_aspect_description().to_string(),
                    configuration.map(|c| c.checksum().to_string()),
                    configuration.map(|c| shared.configuration_event(c)),
                )));
            return;
        }
        if let Some(e) = downcast_event::<SkyAspectCompleteEvent>(&event) {
            let converted = convert_aspect_complete(shared, e);
            let failed = converted.failed();
            shared.streamer.build_event(Arc::new(converted));
            // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:238 TargetSummaryPublisher.aspectComplete
            if let Some(publisher) = &shared.target_summary_publisher {
                let base = aspect_key_of(&e.aspect_key).get_base_configured_target_key();
                publisher.aspect_complete(
                    &TargetSummaryKey {
                        label: base.get_label().clone(),
                        configuration_checksum: base
                            .get_configuration_key()
                            .map(|k| k.get_options_checksum().to_string()),
                    },
                    failed,
                );
            }
            return;
        }
        if let Some(e) = downcast_event::<SkyActionExecutedEvent>(&event) {
            if let Some(converted) = convert_action_executed(shared, e) {
                shared.streamer.build_event(Arc::new(converted));
            }
            return;
        }
        shared.streamer.on_post(event);
    }
}

/// `TargetCompletor.createSucceeded` / `createFailed`: the `TargetCompleteEvent` of a completed
/// top-level target.
// Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:104 TargetCompletor.createSucceeded
// Java: com/google/devtools/build/lib/skyframe/TargetCompletor.java:86 TargetCompletor.createFailed
fn convert_target_complete(shared: &Shared, e: &SkyTargetCompleteEvent) -> TargetCompleteEvent {
    let facts = shared
        .targets
        .lock()
        .get(&(e.label.clone(), e.configuration_checksum.clone()))
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "IllegalStateException: no configured top-level target for the completion of {} ({:?})",
                e.label, e.configuration_checksum
            )
        });
    // CompletionContext.create(expandFilesets, importantInputMap, pathResolverFactory).
    let mut important_input_map = ImportantInputMap::new();
    for (artifact, metadata) in &e.important_input_map {
        important_input_map.put(artifact.clone(), metadata.clone());
    }
    for (tree, metadata, children) in &e.important_tree_artifacts {
        important_input_map.put_tree_artifact(tree.clone(), metadata.clone(), children.clone());
    }
    let ctx = Arc::new(CompletionContext::new(
        important_input_map,
        shared.expand_filesets,
    ));
    // artifactsToBuild.getAllArtifactsByOutputGroup() (succeeded) or the
    // SuccessfulArtifactFilter-filtered groups (failed).
    let outputs: IndexMap<String, ArtifactsInOutputGroup> = e
        .artifacts_by_output_group
        .iter()
        .map(|(name, group)| {
            (
                name.clone(),
                ArtifactsInOutputGroup::new(
                    group.artifacts.clone(),
                    group.important,
                    group.incomplete,
                ),
            )
        })
        .collect();
    let (test_runs, test_shards) = if e.is_test {
        facts
            .test_runs_and_shards
            .unwrap_or_else(|| panic!("NullPointerException: TestProvider of {}", e.label))
    } else {
        // PORT: Java keeps `testParams` null for non-tests and never reads it; the event reads
        // runs and shards only when it is a test.
        (0, 0)
    };
    let data = CompletedTargetData {
        label: e.label.clone(),
        original_label: facts.original_label.clone(),
        configuration_checksum: e.configuration_checksum.clone(),
        configuration_event: facts.configuration_event.clone(),
        rule_tags: facts.rule_tags.clone(),
        test_runs,
        test_shards,
        test_timeout_seconds: if e.is_test {
            Some(facts.get_test_timeout_seconds())
        } else {
            None
        },
    };
    if e.failed() {
        let root_causes: Vec<Cause> = e.root_cause_details.iter().map(completion_cause).collect();
        TargetCompleteEvent::create_failed(
            data,
            ctx,
            root_causes,
            outputs,
            shared.announce_target_summary,
        )
    } else if e.is_test {
        TargetCompleteEvent::successful_build_scheduling_test(
            data,
            ctx,
            outputs,
            shared.announce_target_summary,
        )
    } else {
        TargetCompleteEvent::successful_build(data, ctx, outputs, shared.announce_target_summary)
    }
}

/// `Cause.getIdProto()` of an analysis root cause.
// Java: com/google/devtools/build/lib/causes/AnalysisFailedCause.java:70 AnalysisFailedCause.getIdProto
// Java: com/google/devtools/build/lib/causes/LoadingFailedCause.java:52 LoadingFailedCause.getIdProto
// Java: com/google/devtools/build/lib/causes/LabelCause.java:55 LabelCause.getIdProto
fn analysis_root_cause_id(
    cause: &AnalysisRootCause,
) -> blaze_protos::build_event_stream::BuildEventId {
    use blaze_sky_analysis::producers::errors::Cause as SkyCause;
    match cause {
        AnalysisRootCause::Cause(SkyCause::AnalysisFailed {
            label,
            configuration,
            ..
        })
        | AnalysisRootCause::AnalysisFailed {
            label,
            configuration,
            ..
        } => build_event_id_util::configured_label_id(
            label,
            build_event_id_util::configuration_id_message(configuration),
        ),
        AnalysisRootCause::Cause(SkyCause::LoadingFailed { label, .. })
        | AnalysisRootCause::Cause(SkyCause::Label { label, .. })
        | AnalysisRootCause::Label { label, .. } => {
            build_event_id_util::unconfigured_label_id(label)
        }
    }
}

/// `AnalysisFailureEvent` as the build event it is in Java.
// Java: com/google/devtools/build/lib/analysis/AnalysisFailureEvent.java:127 AnalysisFailureEvent.getEventId
fn convert_analysis_failure(e: &AnalysisFailureEvent) -> AnalysisFailureBuildEvent {
    let failed_target = e.get_failed_target();
    let checksum = failed_target
        .get_configuration_key()
        .map(|k| k.get_options_checksum().to_string());
    let aspect = e.get_failed_aspect().map(|aspect| {
        (
            aspect
                .get_configuration_key()
                .map(|k| k.get_options_checksum().to_string()),
            aspect.get_aspect_name(),
        )
    });
    let id = AnalysisFailureBuildEvent::event_id(
        failed_target.get_label(),
        e.is_configured(),
        checksum.as_deref(),
        aspect
            .as_ref()
            .map(|(c, name)| (c.as_deref(), name.as_str())),
    );
    AnalysisFailureBuildEvent::new(
        id,
        e.get_root_causes()
            .iter()
            .map(analysis_root_cause_id)
            .collect(),
    )
}

/// `Cause.getIdProto()` and the exit code of a completion's root cause.
fn completion_cause(cause: &CompletionRootCause) -> Cause {
    match cause {
        // Java: com/google/devtools/build/lib/causes/ActionFailed.java:71 ActionFailed.getIdProto
        CompletionRootCause::ActionFailed {
            exec_path,
            label,
            configuration_checksum,
            failure_detail,
        } => Cause {
            id_proto: build_event_id_util::action_completed_with(
                exec_path,
                label.as_ref(),
                configuration_checksum.as_deref(),
            ),
            detailed_exit_code: blaze_util::DetailedExitCode::of(failure_detail.clone()),
        },
        // Java: com/google/devtools/build/lib/causes/LabelCause.java:55 LabelCause.getIdProto
        CompletionRootCause::Label {
            label,
            failure_detail,
        } => Cause {
            id_proto: build_event_id_util::unconfigured_label_id(label),
            detailed_exit_code: blaze_util::DetailedExitCode::of(failure_detail.clone()),
        },
    }
}

/// The `AspectKey` behind an aspect completion's lookup key.
fn aspect_key_of(
    key: &blaze_actions::ActionLookupKeyRef,
) -> &blaze_analysis::aspect_key_creator::AspectKey {
    key.as_any()
        .downcast_ref::<blaze_analysis::aspect_key_creator::AspectKey>()
        .unwrap_or_else(|| panic!("ClassCastException: {key} is not an AspectKey"))
}

/// The sink of the `TargetSummaryPublisher`: `eventBus.post(event)`, whose subscriber is the
/// streamer.
struct StreamerSink(Arc<blaze_bep::runtime::build_event_streamer::BuildEventStreamer>);

impl TargetSummarySink for StreamerSink {
    fn post(&self, event: TargetSummaryEvent) {
        self.0.build_event(Arc::new(event));
    }
}

impl Shared {
    /// `configuration.toBuildEvent()`.
    fn configuration_event(&self, configuration: &BuildConfigurationValue) -> Arc<dyn BuildEvent> {
        configuration_event(configuration)
    }

    /// `TopLevelTargetPendingExecutionEvent.configuredTarget()` as the publisher sees it: the
    /// configured top-level target with this (dependency) label.
    fn summary_target(&self, label: &Label) -> TargetSummaryTarget {
        let targets = self.targets.lock();
        let ((label, checksum), facts) = targets
            .iter()
            .find(|((l, _), _)| l == label)
            .unwrap_or_else(|| panic!("IllegalStateException: no top-level target {label}"));
        TargetSummaryTarget {
            key: TargetSummaryKey {
                label: label.clone(),
                configuration_checksum: checksum.clone(),
            },
            original_label: facts.original_label.clone(),
            // AliasProvider.isAlias: the configured target of an alias carries the alias as its
            // original label.
            is_alias: &facts.original_label != label,
        }
    }
}

/// `AspectCompletor.createSucceeded` / `createFailed`: the `AspectCompleteEvent` of a completed
/// top-level aspect.
// Java: com/google/devtools/build/lib/skyframe/AspectCompletor.java:78 AspectCompletor.createSucceeded
// Java: com/google/devtools/build/lib/skyframe/AspectCompletor.java:61 AspectCompletor.createFailed
fn convert_aspect_complete(
    shared: &Shared,
    e: &SkyAspectCompleteEvent,
) -> AspectCompleteBuildEvent {
    let aspect_key = aspect_key_of(&e.aspect_key);
    let mut important_input_map = ImportantInputMap::new();
    for (artifact, metadata) in &e.important_input_map {
        important_input_map.put(artifact.clone(), metadata.clone());
    }
    for (tree, metadata, children) in &e.important_tree_artifacts {
        important_input_map.put_tree_artifact(tree.clone(), metadata.clone(), children.clone());
    }
    let ctx = Arc::new(CompletionContext::new(
        important_input_map,
        shared.expand_filesets,
    ));
    let outputs: IndexMap<String, ArtifactsInOutputGroup> = e
        .artifacts_by_output_group
        .iter()
        .map(|(name, group)| {
            (
                name.clone(),
                ArtifactsInOutputGroup::new(
                    group.artifacts.clone(),
                    group.important,
                    group.incomplete,
                ),
            )
        })
        .collect();
    AspectCompleteBuildEvent::new(
        e.label.clone(),
        e.aspect_key.get_configuration_checksum(),
        aspect_key.get_aspect_descriptor().get_description(),
        e.root_cause_details.iter().map(completion_cause).collect(),
        ctx,
        outputs,
    )
}

/// `TestSummary` as the build event it is in Java (`TestSummary implements
/// BuildEventWithOrderConstraint`).
// Java: com/google/devtools/build/lib/runtime/TestSummary.java:63 TestSummary
fn convert_test_summary(summary: &TestSummary) -> TestSummaryEvent {
    let test_params = get_test_params(summary.get_target());
    TestSummaryEvent {
        label: summary.get_label(),
        configuration_checksum: summary
            .get_target()
            .get_configuration_checksum()
            .unwrap_or_else(|| panic!("NullPointerException: configuration checksum")),
        status: summary.get_status(),
        num_cached: summary.get_num_cached(),
        total_runs: summary.total_runs(),
        num_attempts: summary.get_num_attempts(),
        runs: test_params.runs,
        shards: test_params.shards,
        first_start_time_millis: summary.get_first_start_time_millis(),
        last_stop_time_millis: summary.get_last_stop_time_millis(),
        total_run_duration_millis: summary.get_total_run_duration_millis(),
        passed_logs: summary.get_passed_logs().to_vec(),
        failed_logs: summary.get_failed_logs().to_vec(),
    }
}

/// `ActionExecutedEvent` as a build event. The streamer ignores successful actions unless
/// `--build_event_publish_all_actions` or the action is an extra action (Java
/// `BuildEventStreamer.shouldPublishActionExecutedEvent`);
/// the execution phase does not record the primary output's metadata of those, so they are
/// dropped here and fail loudly when they would be published.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:63 ActionExecutedEvent.<init>
fn convert_action_executed(
    shared: &Shared,
    e: &SkyActionExecutedEvent,
) -> Option<ActionExecutedEvent> {
    // BuildEventStreamer.shouldPublishActionExecutedEvent: a successful action is published
    // only with --build_event_publish_all_actions, or when it is an extra action.
    let is_extra_action = e
        .action
        .as_any()
        .is::<blaze_analysis::extra::extra_action::ExtraAction>();
    if e.exception.is_none() && !shared.publish_all_actions && !is_extra_action {
        return None;
    }
    let failure = match &e.exception {
        None => None,
        Some(exception) => {
            let failure_detail = exception.get_failure_detail().cloned();
            let numeric_exit_code = match &failure_detail {
                Some(failure_detail) => {
                    blaze_util::DetailedExitCode::get_numeric_exit_code(failure_detail)
                }
                None => blaze_util::unported!(
                    "ActionExecutionException.getExitCode (no failure detail)"
                ),
            };
            Some(ActionFailure {
                numeric_exit_code,
                failure_detail,
            })
        }
    };
    let owner = e.action.get_owner();
    let action = ExecutedActionInfo {
        mnemonic: e.action.get_mnemonic(),
        owner: Some(ExecutedActionOwner {
            label: owner.get_label().cloned(),
            configuration_checksum: Some(owner.get_configuration_checksum().to_string()),
            build_configuration_event: owner.get_build_configuration_event().cloned(),
        }),
        arguments: e
            .action
            .as_command_action()
            .and_then(|c| c.get_arguments().ok()),
        is_extra_action,
    };
    Some(ActionExecutedEvent::new(
        e.action_id.clone(),
        action,
        failure,
        e.primary_output_path.clone(),
        e.output_artifact.clone(),
        e.primary_output_metadata.clone(),
        e.stdout.clone(),
        e.stderr.clone(),
        match e.error_timing {
            SkyErrorTiming::NO_ERROR => ErrorTiming::NoError,
            SkyErrorTiming::BEFORE_EXECUTION => ErrorTiming::BeforeExecution,
            SkyErrorTiming::AFTER_EXECUTION => ErrorTiming::AfterExecution,
        },
        e.start_time.map(timestamp_proto),
        e.end_time.map(timestamp_proto),
    ))
}

/// `Timestamp` of an `Instant`: its epoch second and nano of second.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:266 ActionExecutedEvent.timestampProto
fn timestamp_proto(time: std::time::SystemTime) -> blaze_protos::google::protobuf::Timestamp {
    // Instant.getEpochSecond() floors; getNano() is the non-negative remainder.
    let (seconds, nanos) = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos() as i32),
        Err(e) => {
            let d = e.duration();
            let mut seconds = -(d.as_secs() as i64);
            let mut nanos = d.subsec_nanos() as i32;
            if nanos > 0 {
                seconds -= 1;
                nanos = 1_000_000_000 - nanos;
            }
            (seconds, nanos)
        }
    };
    blaze_protos::google::protobuf::Timestamp { seconds, nanos }
}

/// The BEP of one `build` / `test` command.
pub struct BepCommandSupport {
    shared: Arc<Shared>,
    event_bus: Arc<CommandEventBus>,
    reporter: Arc<Reporter>,
    exec_root: Path,
    /// `BuildTool.shouldStopOnFailure(request)` (`None` for a command without `ExecutionOptions`).
    stop_on_first_failure: Option<bool>,
    /// The uploader factory map of the file transports with path conversion.
    uploader_factory_map: Arc<DeferredUploaderFactory>,
    /// The UI's subscriber on the event bus: `reporter.post` of `AnnounceBuildEventTransportsEvent`
    /// and `BuildEventTransportClosedEvent` reaches its `@Subscribe` methods.
    ui_subscriber:
        Mutex<Option<Arc<blaze_buildtool::runtime::ui_event_subscriber::UiEventSubscriber>>>,
    /// `--experimental_ui_debug_all_events`.
    ui_debug_all_events: bool,
    /// `getOutputListener()`: the command's stdout and stderr for the progress events.
    output_listener: OutErr,
    /// Whether `CommandPrecompleteEvent` was handled (see [`BepCommandSupport::command_precomplete`]).
    precompleted: AtomicBool,
    /// Whether afterCommand ran (see [`BepCommandSupport::after_command`]).
    after_command_done: AtomicBool,
    /// What `BuildStartingEvent.create(env, request)` reads from the command environment, for
    /// the commands whose `BuildTool` posts it through the hooks (`cquery`, `aquery`).
    build_starting_context: Mutex<BuildStartingData>,
    /// Whether the command's `BuildStartingEvent` was posted.
    build_starting_posted: AtomicBool,
    /// `--aspects` is not empty (`TargetSummaryPublisher.buildStarting`).
    has_aspects: bool,
    /// The build tool log files of the `CoverageReportCollector` that
    /// `BazelCoverageReportModule`'s factory registered on the event bus (see
    /// [`BepCommandSupport::register_coverage_report_collector`]).
    coverage_report_log_files: Mutex<Vec<(&'static str, Path)>>,
    /// The execution log `SpawnLogModule.buildComplete` publishes (see
    /// [`BepCommandSupport::add_execution_log`]).
    execution_log_file: Mutex<Option<(String, Path)>>,
    /// The Build Event Service part of the module's state.
    bes: BesCommandState,
}

/// What `BuildEventServiceModule` keeps of the command for the Build Event Service.
struct BesCommandState {
    /// `!Strings.isNullOrEmpty(besOptions.besBackend)`.
    bes_backend_set: bool,
    /// Whether the Build Event Service transport is one of the transports.
    #[allow(dead_code)]
    bes_transport_created: bool,
    bes_options: BuildEventServiceOptions,
    /// `invocationId`: `cmdEnv.getCommandId().toString()`.
    invocation_id: String,
    /// `uiUsesColor`.
    ui_uses_color: bool,
    /// `--keep_backend_connections`.
    keep_backend_connections: bool,
}

/// `createBesTransport(cmdEnv, uploaderSupplier, artifactGroupNamer)`: the transport, or `None`
/// without `--bes_backend` or when it cannot be created (then the error is reported and the
/// command exits once its modules ran: `reportError`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:737 BuildEventServiceModule.createBesTransport
fn create_bes_transport(
    env: &CommandEnvironment,
    bes_options: &BuildEventServiceOptions,
    bep_options: Arc<BuildEventProtocolOptions>,
    uploader_supplier: &dyn Fn() -> Arc<
        dyn blaze_bep::buildeventstream::artifact_uploader::BuildEventArtifactUploader,
    >,
    artifact_group_namer: Arc<
        dyn blaze_bep::buildeventstream::build_event_context::ArtifactGroupNamer,
    >,
) -> Option<Arc<dyn blaze_bep::buildeventstream::transport::BuildEventTransport>> {
    let parsing_result = env.get_options();
    let auth_and_tls_options: AuthAndTLSOptions = parsing_result
        .get_options()
        .unwrap_or_else(|| panic!("NullPointerException: AuthAndTLSOptions"));
    // isRunsPerTestOverTheLimit
    let is_runs_per_test_over_the_limit = parsing_result
        .get_options::<TestConfigurationTestOptions>()
        .is_some_and(|test_options| {
            test_options.runs_per_test.iter().any(|per_label_options| {
                let options = per_label_options.get_options();
                blaze_util::check_argument!(options.len() == 1);
                options[0]
                    .parse::<i32>()
                    .unwrap_or_else(|e| panic!("NumberFormatException: {e}"))
                    > blaze_bes::module::RUNS_PER_TEST_LIMIT
            })
        });
    let file_system = env.get_output_base().get_file_system().clone();
    let workspace = env.get_workspace().cloned();
    let command_line_path_factory =
        blaze_repo_fetch::authandtls::credentialhelper::command_line_path_factory::CommandLinePathFactory::create_for(
            file_system.clone(),
            workspace.clone(),
            Some(env.get_directories().get_install_base().clone()),
        );
    let reporter: Arc<dyn EventHandler> = env.get_reporter().clone();
    let event_bus: Arc<dyn blaze_events::EventBus> = env.get_event_bus().clone();
    // BlazeVersionInfo.instance().getVersion(), which TracingMetadataUtils.buildMetadata reads
    // for the calls of the client (the remote module sets it too; the first call wins).
    blaze_remote::util::tracing_metadata_utils::set_tool_version(
        &BlazeVersionInfo::instance().get_version(),
    );
    let result = blaze_bes::module::create_bes_transport(
        blaze_bes::module::BesTransportEnvironment {
            credentials: blaze_bes::module::CredentialsEnvironment {
                reporter,
                workspace,
                client_env: env.get_client_env().clone(),
                command_line_path_factory: &command_line_path_factory,
                file_system: &file_system,
            },
            bes_options,
            auth_and_tls_options: &auth_and_tls_options,
            bep_options,
            is_runs_per_test_over_the_limit,
            build_request_id: crate::commands::build::build_remote::command_build_request_id(
                parsing_result,
            ),
            invocation_id: env.get_command_id().to_string(),
            attempt_number: env.get_attempt_number(),
            command_name: env.get_command_name().to_string(),
            command_start_time_millis: env.get_command_start_time(),
            clock: Arc::new(blaze_bes::uploader::JavaClock),
            event_bus,
            artifact_group_namer,
        },
        || Ok(uploader_supplier()),
    );
    match result {
        Ok(transport) => transport.map(|t| {
            Arc::new(t) as Arc<dyn blaze_bep::buildeventstream::transport::BuildEventTransport>
        }),
        Err(error) => {
            // reportError: logging is not ported.
            if error.report_event {
                // reportCommandLineError(commandLineReporter, exception)
                env.get_reporter()
                    .handle(&Event::error(error.exception_message.clone()));
            }
            env.exit(create_abrupt_exit_exception(
                &error.exception_message,
                &error.message,
                error.code,
            ));
            None
        }
    }
}

/// `BazelBuildEventServiceModule.ALLOWED_COMMANDS`: the commands that stream build events.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:189 BazelBuildEventServiceModule.ALLOWED_COMMANDS
pub(crate) const ALLOWED_COMMANDS: &[&str] = &[
    "fetch",
    "build",
    "test",
    "run",
    "query",
    "aquery",
    "cquery",
    "coverage",
    "mobile-install",
];

/// `registerOutAndErrOutputStreams`: the module's stdout and stderr streams, whose contents the
/// streamer reads into its progress events.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:425 BuildEventServiceModule.registerOutAndErrOutputStreams
fn register_out_and_err_output_streams(
    streamer: &Arc<blaze_bep::runtime::build_event_streamer::BuildEventStreamer>,
    bes_options: &BuildEventServiceOptions,
) -> OutErr {
    let buffer_size = bes_options.bes_outerr_buffer_size;
    let chunk_size = bes_options.bes_outerr_chunk_size;
    let out = Arc::new(SynchronizedOutputStream::new(
        buffer_size,
        chunk_size,
        /* is_stderr= */ false,
    ));
    let err = Arc::new(SynchronizedOutputStream::new(
        buffer_size,
        chunk_size,
        /* is_stderr= */ true,
    ));
    let out_err = OutErr::create(out.clone(), err.clone());
    streamer.register_out_err_provider(Arc::new(SynchronizedOutErrProvider {
        out: out.clone(),
        err: err.clone(),
    }));
    err.register_streamer(streamer.clone());
    out.register_streamer(streamer.clone());
    out_err
}

/// The `OutErrProvider` of [`register_out_and_err_output_streams`].
struct SynchronizedOutErrProvider {
    out: Arc<SynchronizedOutputStream>,
    err: Arc<SynchronizedOutputStream>,
}

impl blaze_bep::runtime::build_event_streamer::OutErrProvider for SynchronizedOutErrProvider {
    fn get_out(&self) -> Option<Vec<String>> {
        Some(self.out.read_and_reset())
    }

    fn get_err(&self) -> Option<Vec<String>> {
        Some(self.err.read_and_reset())
    }
}

/// `CommandEnvironment.makeMapFromMapEntries`: a `TreeMap`, later entries winning.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:598 CommandEnvironment.makeMapFromMapEntries
fn make_map_from_map_entries(entries: &[(String, String)]) -> IndexMap<String, String> {
    let mut result = BTreeMap::new();
    for (k, v) in entries {
        result.insert(k.clone(), v.clone());
    }
    result.into_iter().collect()
}

/// `System.getProperty("user.name")` (`UserUtils.getUserName()`): the JVM sets it from
/// `getpwuid(getuid())->pw_name`, or "?" when there is no passwd entry.
// Java: com/google/devtools/build/lib/util/UserUtils.java:37 UserUtils.getUserName
fn user_name() -> String {
    // SAFETY: getpwuid_r writes into the provided buffers; result is checked before use.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16384];
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(
            libc::getuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        );
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return "?".to_string();
        }
        // A JVM property from the platform: an internal string in the server's Latin-1 locale.
        blaze_util::string_encoding::platform_to_internal(std::os::unix::ffi::OsStrExt::from_bytes(
            std::ffi::CStr::from_ptr(pwd.pw_name).to_bytes(),
        ))
        .into_owned()
    }
}

/// `OptionsUtils.asShellEscapedString(options)`.
// Java: com/google/devtools/build/lib/util/OptionsUtils.java:54 OptionsUtils.asShellEscapedString
pub(crate) fn as_shell_escaped_string(options: &OptionsParsingResult) -> String {
    let mut result = String::new();
    for option in options.as_complete_list_of_parsed_options() {
        if option.is_hidden() {
            continue;
        }
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(&option.get_canonical_form_with_value_escaper(ShellEscaper::escape_string));
    }
    result
}

/// `TargetConfiguredEvent.bepTestSize` input: `packages.TestSize` as the event's enum.
fn packages_test_size(size: Option<TestSize>) -> Option<PackagesTestSize> {
    size.map(|size| match size {
        TestSize::SMALL => PackagesTestSize::Small,
        TestSize::MEDIUM => PackagesTestSize::Medium,
        TestSize::LARGE => PackagesTestSize::Large,
        TestSize::ENORMOUS => PackagesTestSize::Enormous,
    })
}

/// What `TargetConfiguredEvent` reads from its `Target`.
// Java: com/google/devtools/build/lib/analysis/TargetConfiguredEvent.java:76 TargetConfiguredEvent.asStreamProto
fn configured_target_info(target: &Target) -> ConfiguredTargetInfo {
    let tags = target.get_associated_rule().and_then(|rule| {
        let mapper = RawAttributeMapper::of(&**rule);
        // Not every rule has tags, as, due to the "external" package we also have to expect
        // repository rules at this place.
        if !mapper.has("tags") {
            return None;
        }
        match mapper.get_merged_values("tags", Type::StringList) {
            Some(AttrValue::StringList(tags)) => Some(tags),
            other => panic!("ClassCastException: tags value {other:?}"),
        }
    });
    let test_size = target_utils::is_test_rule(target).then(|| {
        // Java: com/google/devtools/build/lib/packages/TestSize.java:70 TestSize.getTestSize
        let attr = target_utils::get_string_attr(target, "size")
            .unwrap_or_else(|| panic!("NullPointerException: size of {}", target.get_label()));
        packages_test_size(TestSize::get_test_size(&attr))
    });
    ConfiguredTargetInfo {
        label: target.get_label().clone(),
        target_kind: target.get_target_kind(),
        tags,
        test_size,
    }
}

/// `TargetParsingCompleteEvent.asThinTarget`.
// Java: com/google/devtools/build/lib/pkgcache/TargetParsingCompleteEvent.java:45 ThinTarget.<init>
fn thin_target(target: &Target) -> ThinTarget {
    ThinTarget::new(
        target.get_label().clone(),
        target.get_target_kind(),
        target.is_rule().then(|| target.get_rule_class()),
    )
}

/// `BuildConfigurationValue.toBuildEvent()`.
fn configuration_event(configuration: &BuildConfigurationValue) -> Arc<dyn BuildEvent> {
    Arc::new(blaze_analysis::rule::config_util::to_build_event(
        configuration,
    ))
}

/// `BuildToolLogCollection`'s `LocalFileType` as the BEP's.
fn bep_local_file_type(type_: tool_build_result::LocalFileType) -> LocalFileType {
    use tool_build_result::LocalFileType as T;
    match type_ {
        T::OUTPUT_FILE => LocalFileType::OutputFile,
        T::OUTPUT_DIRECTORY => LocalFileType::OutputDirectory,
        T::OUTPUT_SYMLINK => LocalFileType::OutputSymlink,
        T::SUCCESSFUL_TEST_OUTPUT => LocalFileType::SuccessfulTestOutput,
        T::FAILED_TEST_OUTPUT => LocalFileType::FailedTestOutput,
        T::COVERAGE_OUTPUT => LocalFileType::CoverageOutput,
        T::QUERY_OUTPUT => LocalFileType::QueryOutput,
        T::STDOUT => LocalFileType::Stdout,
        T::STDERR => LocalFileType::Stderr,
        T::LOG => LocalFileType::Log,
        T::PERFORMANCE_LOG => LocalFileType::PerformanceLog,
    }
}

/// `BuildToolLogCollection.toEvent()`.
// Java: com/google/devtools/build/lib/buildtool/BuildResult.java:355 BuildToolLogCollection.toEvent
fn build_tool_logs_event(collection: &tool_build_result::BuildToolLogCollection) -> BuildToolLogs {
    BuildToolLogs::new(
        collection.get_direct_values().to_vec(),
        collection
            .get_uris()
            .iter()
            .map(|(name, uri)| (name.clone(), Some(uri.clone())))
            .collect(),
        collection
            .get_local_files()
            .iter()
            .map(|entry| LogFileEntry {
                name: entry.name.clone(),
                local_file: LocalFile::with_compression(
                    entry.local_file.path.clone(),
                    bep_local_file_type(entry.local_file.type_),
                    match entry.local_file.compression {
                        tool_build_result::LocalFileCompression::NONE => LocalFileCompression::None,
                        tool_build_result::LocalFileCompression::GZIP => LocalFileCompression::Gzip,
                    },
                    None,
                ),
            })
            .collect(),
    )
}

/// `BuildEventServiceModule.createAbruptExitException`: the message is `message + " " +
/// e.getMessage()`.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:934 BuildEventServiceModule.createAbruptExitException
fn create_abrupt_exit_exception(
    exception_message: &str,
    message: &str,
    bes_code: failure_details::build_progress::Code,
) -> AbruptExitException {
    AbruptExitException::new(DetailedExitCode::of(FailureDetail {
        message: format!("{message} {exception_message}"),
        category: Some(failure_detail::Category::BuildProgress(
            failure_details::BuildProgress {
                code: bes_code as i32,
            },
        )),
    }))
}

// Events of Bazel that no module of Bazel subscribes to are not posted:
// PORT: skipped, no subscriber in Bazel: UpdateOptionsEvent.<init>, UpdateOptionsEvent.getOptions
// PORT: skipped, no subscriber in Bazel: StartingAqueryDumpAfterBuildEvent.<init>, StartingAqueryDumpAfterBuildEvent.getStreamingContext
// PORT: skipped, no subscriber in Bazel: StartingAqueryDumpAfterBuildEvent.getLocalAqueryDumpPath, StartingAqueryDumpAfterBuildEvent.getAqueryDumpName
// The ExecutionStartingEvent of the non-merged ExecutionTool reaches its subscribers
// (MetricsCollector.logExecutionStartingEvent, BuildSummaryStatsModule.executionPhaseStarting)
// as direct calls (blaze_buildtool execution_tool.rs).
// PORT: skipped, direct calls to the subscribers: ExecutionStartingEvent.<init>, ExecutionStartingEvent.getTargets

/// `e.getMessage()` of the `ExecutionException` whose cause is the `TimeoutException` of
/// `Futures.withTimeout`.
///
/// The exception is Guava's `TimeoutFuture.TimeoutFutureException` (oracle, a backend that
/// closes after `--bes_timeout`: `...TimeoutFutureException: Timed out:
/// NonCancellationPropagatingFuture@489f1984[status=PENDING, info=[delegate=[SettableFuture@...`).
///
/// PORT: Guava's message ends with the `toString()` of the timed-out future (object identity
/// hashes and its state), which cannot be reproduced; the port gives the exception's class and
/// the fixed start of its message.
// Java: com/google/common/util/concurrent/TimeoutFuture.java TimeoutFuture.TimeoutFutureException
const TIMEOUT_EXCEPTION_MESSAGE: &str =
    "com.google.common.util.concurrent.TimeoutFuture$TimeoutFutureException: Timed out";

/// What waiting for a close future with its timeout gave.
#[derive(Clone, Debug)]
enum TimedOutcome {
    Done(Result<(), blaze_bep::buildeventstream::transport::TransportCloseError>),
    Cancelled,
    TimedOut,
}

/// A transport's close (or half-close) future with the timeout of the transport attached
/// (`constructCloseFuturesMapWithTimeouts`): `Futures.withTimeout(nonCancellationPropagating(
/// closeFuture), transport.getTimeout())`, whose `cancel` cancels the close future itself.
///
/// `Futures.withTimeout` fails the returned future with a `TimeoutException` when its timer fires
/// before the close future is done, and that failure stays, whatever the close future does
/// later: `timed_out` latches it. Its timer is a thread that waits for the close future until
/// the deadline (Java: a single-thread scheduled executor per transport,
/// `bes-close-<name>-%d`).
#[derive(Clone)]
struct TimedCloseFuture {
    future: blaze_bep::buildeventstream::transport::CloseFuture,
    /// `None` for a zero (or negative) timeout.
    deadline: Option<std::time::Instant>,
    /// The timer fired while the close future was pending.
    timed_out: Arc<AtomicBool>,
}

impl TimedCloseFuture {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:585 BuildEventServiceModule.constructCloseFuturesMapWithTimeouts
    fn new(
        future: blaze_bep::buildeventstream::transport::CloseFuture,
        timeout: std::time::Duration,
        name: &str,
    ) -> TimedCloseFuture {
        let deadline = (!timeout.is_zero()).then(|| std::time::Instant::now() + timeout);
        let timed_out = Arc::new(AtomicBool::new(false));
        if deadline.is_some() && !future.is_done() {
            let timer_future = future.clone();
            let timer_flag = timed_out.clone();
            // Java's scheduled executor starts its thread here; a JVM that cannot start it
            // throws (OutOfMemoryError "unable to create native thread"), so a timer that does
            // not run cannot latch a late completion: fail as the JVM does (review round 2, S4).
            std::thread::Builder::new()
                .name(format!("bes-close-{name}-0"))
                .spawn(move || {
                    if let blaze_bep::buildeventstream::transport::CloseOutcome::TimedOut =
                        timer_future.get_outcome(Some(timeout))
                    {
                        timer_flag.store(true, Ordering::SeqCst);
                    }
                })
                .unwrap_or_else(|e| {
                    panic!("OutOfMemoryError: unable to create native thread: {e}")
                });
        }
        TimedCloseFuture {
            future,
            deadline,
            timed_out,
        }
    }

    fn outcome(&self, wait_until: Option<std::time::Instant>) -> Option<TimedOutcome> {
        use blaze_bep::buildeventstream::transport::CloseOutcome;
        if self.timed_out.load(Ordering::SeqCst) {
            return Some(TimedOutcome::TimedOut);
        }
        // The earlier of the caller's wait and the timeout.
        let until = match (wait_until, self.deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let timeout = until.map(|u| u.saturating_duration_since(std::time::Instant::now()));
        match self.future.get_outcome(timeout) {
            // A close that completes after the timer fired is still a timeout.
            _ if self.timed_out.load(Ordering::SeqCst) => Some(TimedOutcome::TimedOut),
            CloseOutcome::Done(result) => Some(TimedOutcome::Done(result)),
            CloseOutcome::Cancelled => Some(TimedOutcome::Cancelled),
            CloseOutcome::TimedOut => match self.deadline {
                Some(deadline) if std::time::Instant::now() >= deadline => {
                    // The timer fires at the deadline.
                    self.timed_out.store(true, Ordering::SeqCst);
                    Some(TimedOutcome::TimedOut)
                }
                _ => None,
            },
        }
    }

    /// Whether the future is done, without waiting (`None`: still pending).
    fn poll(&self) -> Option<TimedOutcome> {
        self.outcome(Some(std::time::Instant::now()))
    }

    /// `cancel(true)`: cancels the close future.
    fn cancel(&self) {
        self.future.cancel();
    }
}

/// A transport with its close and half-close futures (the entries of
/// `closeFuturesWithTimeoutsMap` and `halfCloseFuturesWithTimeoutsMap`).
#[derive(Clone)]
struct PendingUpload {
    transport: Arc<dyn BuildEventTransport>,
    /// How the UI knows the transport (`None` when the UI is not told).
    id: Option<TransportId>,
    close: TimedCloseFuture,
    half_close: TimedCloseFuture,
}

impl PendingUpload {
    fn new(
        transport: Arc<dyn BuildEventTransport>,
        id: Option<TransportId>,
        close: blaze_bep::buildeventstream::transport::CloseFuture,
        half_close: blaze_bep::buildeventstream::transport::CloseFuture,
    ) -> PendingUpload {
        let timeout = transport.get_timeout();
        let name = transport.name();
        PendingUpload {
            transport,
            id,
            close: TimedCloseFuture::new(close, timeout, &name),
            half_close: TimedCloseFuture::new(half_close, timeout, &name),
        }
    }

    fn same_transport(&self, other: &PendingUpload) -> bool {
        Arc::ptr_eq(&self.transport, &other.transport)
    }
}

/// The module's `closeFuturesWithTimeoutsMap` (with the half-close futures): the uploads the
/// next command waits for. They live as long as the server, as the module does.
static PENDING_UPLOADS: parking_lot::Mutex<Vec<PendingUpload>> =
    parking_lot::Mutex::new(Vec::new());

/// Cancels and interrupts any in-flight threads closing BES transports, then resets the maps
/// tracking in-flight close operations.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:211 BuildEventServiceModule.cancelAndResetPendingUploads
fn cancel_and_reset_pending_uploads() {
    let pending = std::mem::take(&mut *PENDING_UPLOADS.lock());
    for upload in pending {
        upload.close.cancel();
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:218 BuildEventServiceModule.removeFromPendingUploads
fn remove_from_pending_uploads(uploads: &[PendingUpload]) {
    for upload in uploads {
        upload.close.cancel();
    }
    PENDING_UPLOADS
        .lock()
        .retain(|pending| !uploads.iter().any(|u| u.same_transport(pending)));
}

/// The part of `closeBepTransports` after the close futures are known: they become the pending
/// uploads; the blocking ones are waited for, the UI is told at once of the others.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:637 BuildEventServiceModule.closeBepTransports
fn close_pending_uploads(
    pending: Vec<PendingUpload>,
    notify: &dyn Fn(Option<&TransportId>),
) -> Result<(), AbruptExitException> {
    *PENDING_UPLOADS.lock() = pending.clone();
    let mut blocking = Vec::new();
    for upload in pending {
        let bes_upload_mode_is_synchronous =
            upload.transport.get_bes_upload_mode() == BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE;
        if !upload.transport.may_be_slow() || bes_upload_mode_is_synchronous {
            blocking.push(upload);
        } else {
            // When running asynchronously notify the UI immediately since we won't wait for the
            // uploads to close.
            notify(upload.id.as_ref());
        }
    }
    if !blocking.is_empty() {
        wait_for_build_event_transports_to_close(&blocking, notify)?;
    }
    Ok(())
}

/// Waits for `uploads` to close: the UI is told of each one as it finishes; a timed-out
/// upload fails with `BES_UPLOAD_TIMEOUT_ERROR`, a failed one with its own failure (the
/// first to fail, as `Futures.allAsList` does). The waited futures are cancelled and removed
/// from the pending uploads afterwards (`removeFromPendingUploads`).
///
/// PORT: Java's listeners notify the UI from a `bes-notify-ui` thread as each future
/// completes; here the waiting thread polls the futures (every 10 ms) and notifies. A
/// cancelled upload ends the wait (`CancellationException`), as in Java.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:533 BuildEventServiceModule.waitForBuildEventTransportsToClose
fn wait_for_build_event_transports_to_close(
    uploads: &[PendingUpload],
    notify: &dyn Fn(Option<&TransportId>),
) -> Result<(), AbruptExitException> {
    let mut notified = vec![false; uploads.len()];
    let result = loop {
        let mut all_done = true;
        let mut outcome = None;
        for (index, upload) in uploads.iter().enumerate() {
            match upload.close.poll() {
                None => all_done = false,
                Some(result) => {
                    if !notified[index] {
                        notified[index] = true;
                        notify(upload.id.as_ref());
                    }
                    match result {
                        TimedOutcome::Done(Ok(())) => {}
                        other => {
                            outcome = Some(other);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(outcome) = outcome {
            break match outcome {
                // catch (CancellationException e): expected if the upload needs to be
                // cancelled for some reason, e.g. an error interrupting the build.
                TimedOutcome::Cancelled => Ok(()),
                // Futures.withTimeout wraps the TimeoutException in an ExecutionException
                // when the future times out (isTimeoutException).
                // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:233 BuildEventServiceModule.isTimeoutException
                TimedOutcome::TimedOut => Err(create_abrupt_exit_exception(
                    TIMEOUT_EXCEPTION_MESSAGE,
                    "The Build Event Protocol upload timed out.",
                    failure_details::build_progress::Code::BesUploadTimeoutError,
                )),
                // Throwables.throwIfInstanceOf(e.getCause(), AbruptExitException.class)
                TimedOutcome::Done(Err(e)) => Err(transport_close_exception(&e)),
                TimedOutcome::Done(Ok(())) => unreachable!(),
            };
        }
        if all_done {
            // PORT: BuildEventServiceUploadCompleteEvent (the time the wait took) has no
            // subscriber in Bazel; it is not posted.
            break Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    // finally: removeFromPendingUploads(transportFutures)
    remove_from_pending_uploads(uploads);
    result
}

/// `blazeShutdown()`: waits (at most `getMaxWaitForPreviousInvocation()`, five seconds) for the
/// uploads the last command left running, then cancels what is left. A failure or a timeout is
/// only logged by Java (logging is not ported).
///
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:512 BuildEventServiceModule.blazeShutdown
pub(crate) fn blaze_shutdown_pending_uploads() {
    let pending = PENDING_UPLOADS.lock().clone();
    if pending.is_empty() {
        return;
    }
    // Uninterruptibles.getUninterruptibly(Futures.allAsList(...), 5, SECONDS): the first failure
    // ends the wait.
    let until = std::time::Instant::now() + max_wait_for_previous_invocation();
    'wait: loop {
        let mut all_done = true;
        for upload in &pending {
            match upload.close.poll() {
                None => all_done = false,
                Some(TimedOutcome::Done(Ok(()))) => {}
                Some(_) => break 'wait,
            }
        }
        if all_done || std::time::Instant::now() >= until {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // finally: cancelAndResetPendingUploads(), then uploaderFactoryToCleanup.shutdown()
    cancel_and_reset_pending_uploads();
    if let Some(factory) = &*UPLOADER_FACTORY_TO_CLEANUP.lock() {
        factory.shutdown();
    }
}

/// The module's `uploaderFactoryToCleanup`: the uploader factory the last command selected.
static UPLOADER_FACTORY_TO_CLEANUP: parking_lot::Mutex<Option<Arc<DeferredUploaderFactory>>> =
    parking_lot::Mutex::new(None);

/// `waitForPreviousInvocation(isShutdown)`: waits (at most five seconds,
/// `getMaxWaitForPreviousInvocation`) for the uploads the previous command left running, and
/// reports how long it waited or why it gave up.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:237 BuildEventServiceModule.waitForPreviousInvocation
pub(crate) fn wait_for_previous_invocation(reporter: &dyn EventHandler, is_shutdown: bool) {
    let pending = PENDING_UPLOADS.lock().clone();
    if pending.is_empty() {
        return;
    }
    // The connectivity status (ConnectivityStatusProvider) is always OK in Bazel.

    // A FULLY_ASYNC transport is waited for until it half-closed (fully closed for shutdown).
    let waiting: Vec<TimedCloseFuture> = pending
        .iter()
        .map(|upload| {
            if upload.transport.get_bes_upload_mode() == BesUploadMode::FULLY_ASYNC && !is_shutdown
            {
                upload.half_close.clone()
            } else {
                upload.close.clone()
            }
        })
        .collect();
    let mut cancel_close_futures: Vec<PendingUpload> = pending
        .iter()
        .filter(|upload| upload.transport.get_bes_upload_mode() != BesUploadMode::FULLY_ASYNC)
        .cloned()
        .collect();

    let started = std::time::Instant::now();
    let until = started + max_wait_for_previous_invocation();
    // Uninterruptibles.getUninterruptibly(Futures.allAsList(...), 5s): the first failure ends
    // the wait.
    let mut failure: Option<TimedOutcome> = None;
    let mut timed_out = false;
    'wait: loop {
        let mut all_done = true;
        for future in &waiting {
            match future.poll() {
                None => all_done = false,
                Some(TimedOutcome::Done(Ok(()))) => {}
                Some(other) => {
                    failure = Some(other);
                    break 'wait;
                }
            }
        }
        if all_done {
            break;
        }
        if std::time::Instant::now() >= until {
            timed_out = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let waited_millis = started.elapsed().as_millis();
    if timed_out {
        reporter.handle(&Event::warn(format!(
            "The background upload of the Build Event Protocol for the previous invocation \
             failed to complete in {}.{:03} seconds. Cancelling and starting a new invocation...",
            waited_millis / 1000,
            waited_millis % 1000
        )));
        cancel_close_futures = pending.clone();
    } else if let Some(failure) = failure {
        let msg = match failure {
            // Futures.withTimeout wraps the TimeoutException in an ExecutionException when the
            // future times out.
            TimedOutcome::TimedOut => "The background upload of the Build Event Protocol for \
                the previous invocation failed due to a network timeout. Ignoring the failure \
                and starting a new invocation..."
                .to_string(),
            // PORT: a cancelled future (CancellationException) is not an ExecutionException in
            // Java and leaves this method; the port reports it as a failure.
            TimedOutcome::Cancelled => "The background upload of the Build Event Protocol for \
                the previous invocation failed with the following exception: 'Task was \
                cancelled.'. Ignoring the failure and starting a new invocation..."
                .to_string(),
            // e.getMessage() of the ExecutionException: the cause's toString().
            TimedOutcome::Done(Err(e)) => format!(
                "The background upload of the Build Event Protocol for the previous invocation \
                 failed with the following exception: \
                 'com.google.devtools.build.lib.util.AbruptExitException: {}'. Ignoring the \
                 failure and starting a new invocation...",
                e.message
            ),
            TimedOutcome::Done(Ok(())) => unreachable!(),
        };
        reporter.handle(&Event::warn(msg));
        cancel_close_futures = pending.clone();
    } else if waited_millis > 100 {
        reporter.handle(&Event::info(format!(
            "Waited for the background upload of the Build Event Protocol for {}.{:03} seconds.",
            waited_millis / 1000,
            waited_millis % 1000
        )));
    }
    // finally
    for upload in &cancel_close_futures {
        upload.close.cancel();
    }
    reset_pending_uploads();
}

/// Maximum duration Bazel waits for the previous invocation to finish before cancelling it.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:175 BuildEventServiceModule.getMaxWaitForPreviousInvocation
fn max_wait_for_previous_invocation() -> std::time::Duration {
    // BazelBuildEventServiceModuleTest overrides the method (WAIT_FOR_LAST_INVOCATION_TIMEOUT).
    #[cfg(test)]
    if let Some(wait) = bes_module_tests::MAX_WAIT_FOR_PREVIOUS_INVOCATION.with(|w| w.get()) {
        return wait;
    }
    std::time::Duration::from_secs(5)
}

/// Resets the maps tracking the state of closing/half-closing BES transports.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:204 BuildEventServiceModule.resetPendingUploads
fn reset_pending_uploads() {
    PENDING_UPLOADS.lock().clear();
}

/// The `AbruptExitException` a transport's close future fails with
/// (`createAbruptExitException(e, message, BuildProgress.Code)`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:533 BuildEventServiceModule.waitForBuildEventTransportsToClose
fn transport_close_exception(e: &TransportCloseError) -> AbruptExitException {
    let code = failure_details::build_progress::Code::from_str_name(e.code)
        .unwrap_or_else(|| panic!("unknown BuildProgress.Code {}", e.code));
    AbruptExitException::new(DetailedExitCode::of(FailureDetail {
        message: e.message.clone(),
        category: Some(failure_detail::Category::BuildProgress(
            failure_details::BuildProgress { code: code as i32 },
        )),
    }))
}

impl BepCommandSupport {
    /// `BuildEventServiceModule.beforeCommand` (the BEP part: file transports and streamer, its
    /// registration on the event bus) and `MetricsModule.beforeCommand`, run by the dispatcher in
    /// module order (`crate::bazel_modules::BazelBuildEventServiceModule`) for every command.
    /// Returns `None` when the command does not stream build events (`allowedCommands`) or no
    /// transport was requested.
    ///
    /// The events Java posts before the command runs are posted by the dispatcher at their own
    /// points: [`BepCommandSupport::post_before_command_events`] (`CommandEnvironment.beforeCommand`),
    /// [`BepCommandSupport::post_command_line_events`] and
    /// [`BepCommandSupport::post_no_build_event`] (`BlazeCommandDispatcher.execExclusively`).
    ///
    /// The uploader of the file transports with path conversion comes from the runtime's
    /// `BuildEventArtifactUploaderFactoryMap`, selected by
    /// `--experimental_build_event_upload_strategy`; see [`DeferredUploaderFactory`] for when the
    /// remote module's entry is known.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:337 BuildEventServiceModule.beforeCommand
    pub fn module_before_command(env: &CommandEnvironment) -> Option<Arc<BepCommandSupport>> {
        // We need to wait for the previous invocation before we check the list of allowed
        // commands to allow completing previous runs using BES, for example:
        //   bazel build (..run with async BES..)
        //   bazel info <-- Doesn't run with BES unless we wait before checking allowedCommands.
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:377 BuildEventServiceModule.beforeCommand
        let command_is_shutdown = env.get_command_name() == "shutdown";
        wait_for_previous_invocation(env.get_reporter().as_ref(), command_is_shutdown);
        if command_is_shutdown && let Some(factory) = &*UPLOADER_FACTORY_TO_CLEANUP.lock() {
            factory.shutdown();
        }

        // Exit early if the running command isn't supported.
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:383 BuildEventServiceModule.beforeCommand
        if !ALLOWED_COMMANDS.contains(&env.get_command_name()) {
            return None;
        }
        let uploader_factory_map = DeferredUploaderFactory::new(
            crate::commands::build::build_remote::build_event_artifact_uploader_factory_map(None),
        );
        // this.uploaderFactoryToCleanup = uploaderFactory
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:395 BuildEventServiceModule.beforeCommand
        *UPLOADER_FACTORY_TO_CLEANUP.lock() = Some(uploader_factory_map.clone());
        let parsing_result = env.get_options();
        let bep_options: BuildEventProtocolOptions = parsing_result
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: BuildEventProtocolOptions"));
        let bes_stream_options: BuildEventStreamOptions = parsing_result
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: BuildEventStreamOptions"));
        let common_options: CommonCommandOptions = parsing_result
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: CommonCommandOptions"));
        let ui_options: UiOptions = parsing_result
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: UiOptions"));

        // BuildEventOutputStreamFactoryImpl.create: DestinationRelativeTo.WORKSPACE_OR_HOME.
        // Java: com/google/devtools/build/lib/runtime/InstrumentationOutputFactory.java:164 InstrumentationOutputFactory.createInstrumentationOutput
        let workspace_or_home = match env.get_workspace() {
            Some(workspace) => workspace.clone(),
            None => blaze_util::unported!(
                "InstrumentationOutputFactory.createInstrumentationOutput (no workspace: home directory)"
            ),
        };
        // The connectivity status (ConnectivityStatusProvider) is always OK in Bazel, so the
        // strategy is --experimental_build_event_upload_strategy.
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:366 BuildEventServiceModule.beforeCommand
        uploader_factory_map.set_strategy(bep_options.build_event_upload_strategy.clone());
        let uploader_factory: Arc<dyn BuildEventArtifactUploaderFactory> =
            uploader_factory_map.clone();
        let bes_options: BuildEventServiceOptions = parsing_result
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: BuildEventServiceOptions"));
        let ui_uses_color = ui_options.use_color();
        let mut bes_transport_created = false;
        let (streamer, errors) = BuildEventStreamerBuilder::from_options_with_bes_transport(
            &bep_options,
            &bes_stream_options,
            &BepCommandContext {
                workspace_or_home,
                oom_message: common_options.oom_message.clone(),
            },
            uploader_factory,
            |uploader_supplier, artifact_group_namer| {
                // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:917 BuildEventServiceModule.createBepTransports
                let transport = create_bes_transport(
                    env,
                    &bes_options,
                    Arc::new(bep_options.clone()),
                    uploader_supplier,
                    artifact_group_namer,
                )?;
                bes_transport_created = true;
                blaze_bes::module::construct_and_maybe_report_invocation_id_url(
                    env.get_reporter().as_ref(),
                    &bes_options,
                    env.get_command_id(),
                    ui_uses_color,
                );
                blaze_bes::module::construct_and_maybe_report_build_request_id_url(
                    env.get_reporter().as_ref(),
                    &crate::commands::build::build_remote::command_build_request_id(
                        env.get_options(),
                    ),
                );
                Some(transport)
            },
        );
        // createBepTransports: each transport that cannot be opened is omitted and reported.
        for error in errors {
            // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:180 BuildEventServiceModule.reportError
            // logger.atSevere().withCause(exception).log("%s", msg): logging is not ported.
            // reportCommandLineError(commandLineReporter, exception)
            // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:168 BuildEventServiceModule.reportCommandLineError
            env.get_reporter()
                .handle(&Event::error(error.exception.to_string()));
            // moduleEnvironment.exit(createAbruptExitException(exception, msg, besCode))
            env.exit(create_abrupt_exit_exception(
                &error.exception.to_string(),
                &error.message,
                failure_details::build_progress::Code::BesLocalWriteError,
            ));
        }
        let ui_debug_all_events = ui_options.experimental_ui_debug_all_events;
        // PORT: with no transports Java still posts AnnounceBuildEventTransportsEvent with the
        // empty set ("so that interested parties always get this event"); its only effect on the
        // UI is the --experimental_ui_debug_all_events line "Transports announced:", which is not
        // written here (no BEP support exists for the command to replay it).
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:407 BuildEventServiceModule.beforeCommand
        let streamer = streamer?;

        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:417 BuildEventServiceModule.beforeCommand
        let target_summary_publisher = bep_options.publish_target_summary.then(|| {
            Arc::new(TargetSummaryPublisher::new(Arc::new(StreamerSink(
                streamer.streamer().clone(),
            ))))
        });

        // MetricsModule.beforeCommand: MetricsCollector.installInEnv(env, numAnalyses,
        // numBuilds), with the module's counters (they live as long as the server). A command
        // without a BEP transport gets its collector from `MetricsOnlyCommandSupport`.
        // Java: com/google/devtools/build/lib/metrics/MetricsModule.java:78 MetricsModule.beforeCommand
        let clock: Arc<dyn Clock> = Arc::new(JavaClock);
        let metrics_options: Option<MetricsModuleOptions> = parsing_result.get_options();
        let (num_analyses, num_builds) = crate::bazel_modules::MetricsModule::counters();
        let metrics = Arc::new(MetricsCollector::install_in_env(
            metrics_options.as_ref(),
            clock.clone(),
            num_analyses,
            num_builds,
        ));
        // MetricsCollector.<init>: WorkerProcessMetricsCollector.instance().setClock(env.getClock()).
        // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:133 MetricsCollector.<init>
        blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector::instance()
            .set_clock(clock.clone());

        let shared = Arc::new(Shared {
            streamer,
            metrics,
            targets: Mutex::new(IndexMap::new()),
            announce_target_summary: bep_options.publish_target_summary,
            target_summary_publisher,
            expand_filesets: bep_options.expand_filesets,
            publish_all_actions: bes_stream_options.publish_all_actions,
            clock,
            build_info_posted: AtomicBool::new(false),
            execution_timer: Mutex::new(None),
            failed_target_patterns: Mutex::new(Vec::new()),
        });
        // cmdEnv.getEventBus().register(streamer) (and the MetricsCollector's registration).
        env.get_event_bus().register(Arc::new(BepSubscriber {
            shared: shared.clone(),
        }));
        let output_listener = register_out_and_err_output_streams(
            shared.streamer.streamer(),
            &parsing_result
                .get_options::<BuildEventServiceOptions>()
                .unwrap_or_else(|| panic!("NullPointerException: BuildEventServiceOptions")),
        );

        let options = env.get_options();
        let keep_going = options
            .get_options::<KeepGoingOption>()
            .map(|o| o.keep_going)
            .unwrap_or(false);
        // Read by BuildTool (build, test, run, fetch with targets) only; the other commands have
        // no ExecutionOptions.
        let test_keep_going = options
            .get_options::<ExecutionOptions>()
            .map(|o| o.test_keep_going);
        let support = Arc::new(BepCommandSupport {
            shared,
            event_bus: env.get_event_bus().clone(),
            reporter: env.get_reporter().clone(),
            exec_root: env.get_exec_root(),
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1127 BuildTool.shouldStopOnFailure
            stop_on_first_failure: test_keep_going
                .map(|test_keep_going| !(keep_going && test_keep_going)),
            uploader_factory_map,
            output_listener,
            precompleted: AtomicBool::new(false),
            after_command_done: AtomicBool::new(false),
            build_starting_context: Mutex::new(BuildStartingData {
                uuid: env.get_command_id().to_string(),
                // request.getStartTime(): the requests are built with env.getCommandStartTime().
                start_time_millis: env.get_command_start_time(),
                build_tool_version: BlazeVersionInfo::instance().get_version(),
                options_description: as_shell_escaped_string(&options),
                command_name: env.get_command_name().to_string(),
                server_pid: std::process::id() as i64,
                pwd: env.get_working_directory().get_path_string().to_string(),
                workspace: env
                    .get_directories()
                    .get_workspace()
                    .map(|w| w.get_path_string().to_string()),
                host: blaze_sky_top::skyframe_executor::short_host_name(),
                user: user_name(),
                targets: Vec::new(),
            }),
            build_starting_posted: AtomicBool::new(false),
            coverage_report_log_files: Mutex::new(Vec::new()),
            execution_log_file: Mutex::new(None),
            has_aspects: options
                .get_options::<BuildRequestOptions>()
                .is_some_and(|o| !o.aspects.is_empty()),
            ui_subscriber: Mutex::new(None),
            ui_debug_all_events,
            bes: BesCommandState {
                bes_backend_set: !bes_options.bes_backend.is_empty(),
                bes_transport_created,
                bes_options: bes_options.clone(),
                invocation_id: env.get_command_id().to_string(),
                ui_uses_color,
                keep_backend_connections: bes_stream_options.keep_backend_connections,
            },
        });
        // reporter.post(new AnnounceBuildEventTransportsEvent(bepTransports)) is posted at the
        // end of beforeCommand; it reaches the UI when the dispatcher replays the events stored
        // during the modules' beforeCommand: see `post_before_command_events`.
        Some(support)
    }

    /// The streamer's transports as the UI tells them apart (`BuildEventTransport` identity and
    /// `name()`), in the streamer's order.
    fn transport_ids(&self) -> Vec<TransportId> {
        self.shared
            .streamer
            .streamer()
            .get_transports()
            .iter()
            .map(|transport| TransportId {
                id: Arc::as_ptr(transport) as *const () as usize as u64,
                name: transport.name(),
            })
            .collect()
    }

    /// The `AnnounceBuildEventTransportsEvent` as the build's own state tracker receives it.
    ///
    /// PORT: a build replaces the UI's state tracker after the modules' `beforeCommand`
    /// (`crate::commands::build`: the `SkymeldUiStateTracker` is chosen once the build knows
    /// whether analysis and execution are merged; Java creates the right tracker before
    /// `beforeCommand`). The replacement has not seen the announcement, so it is told again when
    /// the build starts, without the handler's debug line, which Java writes once.
    fn reannounce_transports_to_build_tracker(&self) {
        if let Some(subscriber) = &*self.ui_subscriber.lock() {
            subscriber
                .get_state_tracker()
                .build_event_transports_announced(&self.transport_ids());
        }
    }

    /// `BuildEventServiceModule.getOutputListener()`: the dispatcher tees the command's output
    /// into it (`BlazeCommandDispatcher.tee`).
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:448 BuildEventServiceModule.getOutputListener
    pub(crate) fn get_output_listener(&self) -> &OutErr {
        &self.output_listener
    }

    /// `BuildEventArtifactUploaderFactoryDelegate.init(...)` of the command's remote module: the
    /// factory map with the command's `RemoteModule` entry. Called by the commands that set the
    /// remote module up themselves (`crate::commands::build::run_build`).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1351 BuildEventArtifactUploaderFactoryDelegate.init
    pub(crate) fn set_uploader_factory_map(&self, map: BuildEventArtifactUploaderFactoryMap) {
        self.uploader_factory_map.set_map(map);
    }

    /// The `NoBuildEvent` of a command that stops before it runs
    /// (`new NoBuildEvent(commandName, firstContactTime, false, true, commandId)`), and of the
    /// commands that do not build (`separate_finished_event`: followed by a
    /// `NoBuildRequestFinishedEvent`).
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:520 BlazeCommandDispatcher.execExclusively
    pub(crate) fn post_no_build_event(
        &self,
        command: Option<String>,
        start_time_millis: Option<i64>,
        separate_finished_event: bool,
        show_progress: bool,
        id: Option<String>,
    ) {
        self.post(Arc::new(BuildEventPost(Arc::new(NoBuildEvent::new(
            command,
            start_time_millis,
            separate_finished_event,
            show_progress,
            id,
            BlazeVersionInfo::instance().get_version(),
            std::process::id() as i64,
        )))));
    }

    /// `env.getEventBus().post(event)` of a build event (a `BuildEvent` the streamer
    /// subscribes to).
    pub(crate) fn post_build_event(&self, event: Arc<dyn BuildEvent>) {
        self.post(Arc::new(BuildEventPost(event)));
    }

    /// `NoBuildRequestFinishedEvent(exitCode, finishTimeMillis)`: a `BuildCompletingEvent` with
    /// no children.
    // Java: com/google/devtools/build/lib/analysis/NoBuildRequestFinishedEvent.java:22 NoBuildRequestFinishedEvent.<init>
    pub(crate) fn post_no_build_request_finished_event(
        &self,
        exit_code: blaze_util::ExitCode,
        finish_time_millis: i64,
    ) {
        self.post(Arc::new(BuildEventPost(Arc::new(
            blaze_bep::buildeventstream::BuildCompletingEvent::new(
                exit_code,
                finish_time_millis,
                Vec::new(),
            ),
        ))));
    }

    fn post(&self, event: Arc<dyn Postable>) {
        self.event_bus.post(event);
    }

    /// `CommandEnvironment.beforeCommand`: the build metadata and the parsed options.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:919 CommandEnvironment.beforeCommand
    pub(crate) fn post_before_command_events(&self, env: &CommandEnvironment) {
        // BlazeCommandDispatcher.execExclusively: the events stored while the modules ran
        // beforeCommand (storedEventHandler) are replayed once the UI's handler exists, just
        // before CommandEnvironment.beforeCommand. The BES module's
        // AnnounceBuildEventTransportsEvent is one of them: its @Subscribe in the UI is
        // UiEventHandler.buildEventTransportsAnnounced.
        // PORT: the dispatcher does not replay posts; the announcement is delivered here, the
        // first point after the replay where the UI's subscriber is known.
        // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:581 BlazeCommandDispatcher.execExclusively
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:442 BuildEventServiceModule.beforeCommand
        // The event is the UI subscriber's call with the transports.
        // Java: com/google/devtools/build/lib/buildeventstream/AnnounceBuildEventTransportsEvent.java:28 AnnounceBuildEventTransportsEvent.<init>
        // Java: com/google/devtools/build/lib/buildeventstream/AnnounceBuildEventTransportsEvent.java:35 AnnounceBuildEventTransportsEvent.transports
        if let Some(subscriber) = env.get_ui_event_subscriber() {
            *self.ui_subscriber.lock() = Some(subscriber.clone());
            subscriber
                .build_event_transports_announced(&self.transport_ids(), self.ui_debug_all_events);
        }
        let common_options: CommonCommandOptions = env
            .get_options()
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: CommonCommandOptions"));
        self.post(Arc::new(BuildMetadataEvent::new(
            make_map_from_map_entries(&common_options.build_metadata),
        )));
        self.post(Arc::new(GotOptionsEvent::new(
            env.get_runtime().get_startup_options_provider_arc(),
            env.get_options().clone(),
            env.get_invocation_policy().clone(),
        )));
    }

    /// The `CanonicalCommandLineEvent` that `BuildTool.buildTargets` posts once a project file's
    /// flags were parsed into the command's options parser: not replaceable, it replaces the
    /// tentative one of the dispatcher. The `UpdateOptionsEvent` posted next has no subscriber
    /// at the pin.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:375 BuildTool.buildTargets
    pub(crate) fn post_project_canonical_command_line(
        &self,
        product_name: &str,
        active_startup_options: Arc<OptionsParsingResult>,
        command_name: &str,
        options_parser: &OptionsParsingResult,
    ) {
        let include_residue_in_run_bep_event = options_parser
            .get_options::<BuildEventProtocolOptions>()
            .expect("NullPointerException: BuildEventProtocolOptions")
            .include_residue_in_run_bep_event;
        self.post(Arc::new(CanonicalCommandLineEvent::new(
            product_name.to_string(),
            active_startup_options,
            command_name.to_string(),
            options_parser.get_residue(),
            include_residue_in_run_bep_event,
            options_parser
                .get_explicit_starlark_options(OriginalCommandLineEvent::command_line_priority),
            options_parser.get_starlark_options().clone(),
            options_parser.as_list_of_canonical_options(),
            // This replaces the tentative CanonicalCommandLineEvent posted earlier in the build
            // in BlazeCommandDispatcher.
            /* replaceable= */
            false,
        )));
    }

    /// The command-line events of `BlazeCommandDispatcher.execExclusively`.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:779 BlazeCommandDispatcher.execExclusively
    pub(crate) fn post_command_line_events(
        &self,
        env: &CommandEnvironment,
        options: &OptionsParsingResult,
    ) {
        let runtime = env.get_runtime();
        let command_name = env.get_command_name();
        let include_residue_in_run_bep_event = env
            .get_options()
            .get_options::<BuildEventProtocolOptions>()
            .is_some_and(|o| o.include_residue_in_run_bep_event);
        let original_command_line_event = OriginalCommandLineEvent::new(
            runtime.get_product_name().to_string(),
            runtime.get_startup_options_provider_arc(),
            command_name.to_string(),
            options.get_residue(),
            include_residue_in_run_bep_event,
            options.as_list_of_explicit_options(),
            options.get_explicit_starlark_options(OriginalCommandLineEvent::command_line_priority),
            env.get_startup_options_tagged_with_bazel_rc()
                .map(|tagged| tagged.to_vec()),
        );
        let canonical_command_line_event = CanonicalCommandLineEvent::new(
            runtime.get_product_name().to_string(),
            runtime.get_startup_options_provider_arc(),
            command_name.to_string(),
            options.get_residue(),
            include_residue_in_run_bep_event,
            options.get_explicit_starlark_options(OriginalCommandLineEvent::command_line_priority),
            options.get_starlark_options().clone(),
            options.as_list_of_canonical_options(),
            // If this is a command that analyzes with BuildTool, PROJECT.scl might set extra
            // canonical flags. In that case give BuildTool a chance to post a final updated
            // CanonicalCommandLineEvent. Then this one is dropped. But if that event doesn't post
            // for any reason, including a build error or crash, post this one so the build still
            // registers a canonical command line. That guarantees BuildEventStream always posts
            // exactly one CanonicalCommandLineEvent message for all builds.
            env.get_command().build_phase.analyzes(),
        );
        let unstructured_server_command_line_event =
            if command_name == "run" && !include_residue_in_run_bep_event {
                OriginalUnstructuredCommandLineEvent::redacted_unstructured_command_line_event()
            } else {
                OriginalUnstructuredCommandLineEvent::new(env.get_client_args().to_vec())
            };
        let common_options: CommonCommandOptions = options
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: CommonCommandOptions"));
        self.post(Arc::new(unstructured_server_command_line_event));
        self.post(Arc::new(original_command_line_event));
        self.post(Arc::new(canonical_command_line_event));
        self.post(Arc::new(ToolCommandLineEvent::from_option_value(
            &common_options.tool_command_line,
        )));
    }

    /// `request.getOptionsDescription()` of a request whose `BuildStartingEvent` is posted
    /// through the hooks: the options the command was given (with the flag aliases the
    /// dispatcher injected).
    // Java: com/google/devtools/build/lib/buildtool/BuildRequest.java:268 BuildRequest.getOptionsDescription
    pub(crate) fn set_build_request_options(&self, options: &OptionsParsingResult) {
        self.build_starting_context.lock().options_description = as_shell_escaped_string(options);
    }

    /// `BuildTool.buildTargets`: `BuildStartingEvent.create(env, request)`.
    ///
    /// `targets` is `request.getTargets()`, `start_time_millis` `request.getStartTime()`,
    /// `host` `NetUtil.getCachedShortHostName()`.
    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:67 BuildStartingEvent.create
    pub fn build_starting(
        &self,
        env: &CommandEnvironment,
        options: &OptionsParsingResult,
        targets: &[String],
        start_time_millis: i64,
        host: String,
    ) {
        self.build_starting_posted.store(true, Ordering::SeqCst);
        self.reannounce_transports_to_build_tracker();
        // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:80 TargetSummaryPublisher.buildStarting
        if let Some(publisher) = &self.shared.target_summary_publisher {
            publisher.build_starting(
                options
                    .get_options::<BuildRequestOptions>()
                    .is_some_and(|o| !o.aspects.is_empty()),
            );
        }
        let data = BuildStartingData {
            uuid: env.get_command_id().to_string(),
            start_time_millis,
            build_tool_version: BlazeVersionInfo::instance().get_version(),
            // BuildRequest: OptionsUtils.asShellEscapedString(options).
            options_description: as_shell_escaped_string(options),
            command_name: env.get_command_name().to_string(),
            server_pid: std::process::id() as i64,
            pwd: env.get_working_directory().get_path_string().to_string(),
            workspace: env
                .get_directories()
                .get_workspace()
                .map(|w| w.get_path_string().to_string()),
            host,
            user: user_name(),
            targets: targets.to_vec(),
        };
        self.post(Arc::new(BuildStartingEvent::create(data)));
    }

    /// `TargetPatternPhaseFunction.compute`: the `TargetParsingCompleteEvent`.
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:76 TargetPatternPhaseFunction.compute
    fn post_target_parsing_complete(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        loading_result: &TargetPatternPhaseValue,
    ) {
        // ImmutableList.copyOf(failedPatterns): the value does not keep them; they are the
        // patterns of the PatternExpandingError.failed events posted while it was computed.
        let failed_target_patterns = self.shared.failed_target_patterns.lock().clone();
        let target_of = |label: &Label| -> Target {
            let pkg = skyframe_executor
                .get_existing_package(label.get_package_identifier())
                .unwrap_or_else(|| panic!("NullPointerException: {}", label));
            pkg.get_targets()
                .get(label.get_name())
                .cloned()
                .unwrap_or_else(|| panic!("NullPointerException: {}", label))
        };
        // targets.getTargets(): the targets before test suite expansion.
        let mut targets: Vec<ThinTarget> = Vec::new();
        let mut test_suite_expansions: Vec<(Label, Vec<Label>)> = Vec::new();
        for label in loading_result.get_non_expanded_labels() {
            let target = target_of(label);
            if target_utils::is_test_suite_rule(&target)
                && request.loading_options.expand_test_suites
            {
                let key = SkyKey::new(TestsForTargetPatternKey::key(IndexSet::from([
                    label.clone()
                ])));
                let value = skyframe_executor
                    .get_evaluator()
                    .get_existing_value(&key)
                    .unwrap_or_else(|| panic!("NullPointerException: {:?}", key))
                    .downcast::<TestsForTargetPatternValue>()
                    .unwrap_or_else(|_| panic!("TEST_SUITE_EXPANSION value"));
                test_suite_expansions.push((
                    label.clone(),
                    value.get_labels().get_targets().iter().cloned().collect(),
                ));
            }
            targets.push(thin_target(&target));
        }
        // expandedTargets: TestsForTargetPatternFunction.labelsToTargets(targetLabels).
        let expanded_targets: Vec<ThinTarget> =
            construct_label_to_target_map(skyframe_executor, loading_result)
                .values()
                .map(thin_target)
                .collect();
        // PORT: the filtered targets and the pattern-to-labels map are not part of the value;
        // they feed only the event's Java consumers (none of them reach the stream).
        self.post(Arc::new(TargetParsingCompleteEvent::new(
            targets,
            Vec::new(),
            Vec::new(),
            request.targets.clone(),
            expanded_targets,
            failed_target_patterns,
            Vec::new(),
            test_suite_expansions,
        )));
    }

    /// `AnalysisPhaseStartedEvent`, `AnalysisPhaseCompleteEvent` and `AnalysisGraphStatsEvent`
    /// as the `MetricsCollector` receives them.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:253 BuildView.update
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:431 AnalysisPhaseRunner.execute
    ///
    /// `analysis_phase_completed`: whether the analysis posted `AnalysisPhaseCompleteEvent` (and
    /// `AnalysisGraphStatsEvent`); with merged analysis and execution not when there was no
    /// top-level key (`blaze_sky_top::analysis_phase_runner::analysis_phase_complete_event_posted`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:911 SkyframeBuildView.analysisFinishedCallback
    fn analysis_metrics(&self, analysis_result: &AnalysisResult, analysis_phase_completed: bool) {
        log_analysis_metrics(
            &self.shared.metrics,
            analysis_phase_completed.then(|| analysis_result.get_analysis_phase_complete_stats()),
            analysis_result.get_build_graph_metrics(),
        );
    }
}

/// `AnalysisPhaseStartedEvent`, `AnalysisPhaseCompleteEvent` (`stats`: `None` when it was not
/// posted) and `AnalysisGraphStatsEvent` as the `MetricsCollector` receives them.
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:145 MetricsCollector.logAnalysisStartingEvent
fn log_analysis_metrics(
    metrics: &MetricsCollector,
    stats: Option<&blaze_sky_top::build_view::AnalysisPhaseCompleteStats>,
    build_graph_metrics: Option<
        &blaze_protos::build_event_stream::build_metrics::BuildGraphMetrics,
    >,
) {
    metrics.log_analysis_starting_event();
    let Some(stats) = stats else {
        return;
    };
    // PackageMetricsPackageLoadingListener.getInstance(): the recorder's metrics, read here
    // for the MetricsCollector.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:164 MetricsCollector.onAnalysisPhaseComplete
    let listener = crate::package_metrics::PackageMetricsPackageLoadingListener::get_instance();
    let package_metrics_recorder = if listener.get_publish_package_metrics_in_bep() {
        listener
            .get_package_metrics_recorder()
            .map(|recorder| PackageMetricsRecorderSnapshot {
                package_load_metrics: recorder.get_package_load_metrics(),
                only_extremes_num_packages_to_track: match recorder.get_recorder_type() {
                    crate::package_metrics::RecorderType::OnlyExtremes => Some(
                        recorder
                            .as_extrema()
                            .expect("ClassCastException: ExtremaPackageMetricsRecorder")
                            .get_num_packages_to_track(),
                    ),
                    crate::package_metrics::RecorderType::All => None,
                },
            })
    } else {
        None
    };
    metrics.on_analysis_phase_complete(&AnalysisPhaseCompleteInfo {
        actions_constructed: TotalAndConfiguredTargetOnlyMetric::create(
            stats.actions_constructed_total,
            stats.actions_constructed_configured_targets_only,
        ),
        targets_configured: TotalAndConfiguredTargetOnlyMetric::create(
            stats.targets_configured_total,
            stats.targets_configured_configured_targets_only,
        ),
        time_in_ms: stats.time_in_ms,
        packages_successfully_loaded: stats.packages_successfully_loaded,
        actions_constructed_by_mnemonic: stats.actions_constructed_by_mnemonic.clone(),
        package_metrics_recorder,
    });
    // AnalysisGraphStatsEvent: posted only when the analysis checked for action conflicts.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:454 SkyframeBuildView.configureTargets
    if let Some(build_graph_metrics) = build_graph_metrics {
        metrics.log_analysis_graph_stats(build_graph_metrics);
    }
}

impl BepCommandSupport {
    /// `SkyframeExecutor.configureTargets`: a `TargetConfiguredEvent` per configured top-level
    /// target (the path without merged analysis and execution; with it, `BuildDriverFunction`
    /// posts them, see `blaze_sky_top::build_view::TopLevelTargetConfiguredEvent`); also records
    /// what the target's `TargetCompleteEvent` reads of it.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2317 SkyframeExecutor.configureTargets
    fn post_targets_configured(
        &self,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
        post_events: bool,
    ) {
        let mut by_label: IndexMap<Label, (Target, Option<Arc<BuildConfigurationValue>>)> =
            IndexMap::new();
        for target_and_configuration in analysis_result.get_top_level_targets_with_configs() {
            let target = target_and_configuration.get_target();
            let configuration = target_and_configuration.get_configuration().cloned();
            let checksum = configuration.as_ref().map(|c| c.checksum().to_string());
            if post_events {
                self.post(Arc::new(TargetConfiguredEvent::new(
                    configured_target_info(target),
                    configuration
                        .as_deref()
                        .map(|c| self.shared.configuration_event(c)),
                    checksum,
                )));
            }
            by_label.insert(target.get_label().clone(), (target.clone(), configuration));
        }

        let mut facts = self.shared.targets.lock();
        for view in configured_target_views(analysis_result.get_targets_to_build()) {
            let original_label = view.get_original_label();
            let Some((target, configuration)) = by_label.get(&original_label) else {
                panic!(
                    "IllegalStateException: no target for the configured target {}",
                    original_label
                );
            };
            facts.insert(
                (view.get_label(), view.get_configuration_checksum()),
                CompletedTargetFacts {
                    original_label,
                    configuration_event: configuration
                        .as_deref()
                        .map(|c| self.shared.configuration_event(c)),
                    configuration: configuration.clone(),
                    // Java: the event's ConfiguredTargetAndData holds the target of
                    // `configuredTarget.getLabel()`: for an alias, the actual target.
                    rule_tags: actual_target(skyframe_executor, &view.get_label(), target)
                        .get_rule_tags()
                        .into_iter()
                        .collect(),
                    test_timeout: actual_target(skyframe_executor, &view.get_label(), target)
                        .get_test_timeout(),
                    test_runs_and_shards: view.get_test_params().map(|p| (p.runs, p.shards)),
                },
            );
        }
    }

    /// `AnalysisPhaseRunner.postAbortedEventsForSkippedTargets`: an `AbortedEvent`
    /// (`AbortReason.SKIPPED`) with the `targetCompleted` id of each skipped top-level target.
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:343 AnalysisPhaseRunner.postAbortedEventsForSkippedTargets
    pub(crate) fn post_aborted_events_for_skipped_targets(
        &self,
        targets_to_skip: &[blaze_analysis::configured_target::ConfiguredTarget],
    ) {
        for target in targets_to_skip {
            // configurationId(skyframeExecutor.getConfiguration(reporter, configurationKey)):
            // the configuration's event id is its options checksum, the key's.
            let checksum = target
                .get_configuration_key()
                .map(|k| k.get_options_checksum().to_string());
            let label = target.get_original_label();
            self.post_build_event(Arc::new(
                blaze_bep::buildeventstream::generic_build_event::AbortedEvent::with_label(
                    build_event_id_util::target_completed(
                        &label,
                        &build_event_id_util::configuration_id_for_key(checksum.as_deref()),
                    ),
                    AbortReason::Skipped,
                    format!("Target {} build was skipped.", label),
                    Some(label.clone()),
                ),
            ));
        }
    }

    /// `env.withMergedAnalysisAndExecutionSourceOfTruth()` once `SkymeldModule` decided it (the
    /// `TargetSummaryPublisher`'s supplier).
    pub(crate) fn set_merged_analysis_and_execution(&self, value: bool) {
        if let Some(publisher) = &self.shared.target_summary_publisher {
            publisher.set_merged_skyframe_analysis_execution(value);
        }
    }

    /// `ExecutionTool.handleConvenienceSymlinks`: the `ConvenienceSymlinksIdentifiedEvent`.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:705 ExecutionTool.handleConvenienceSymlinks
    pub fn convenience_symlinks_identified(
        &self,
        build_request_options: &BuildRequestOptions,
        convenience_symlinks: &SymlinkCreationResult,
    ) {
        let protos = if build_request_options.experimental_convenience_symlinks
            != ConvenienceSymlinksMode::IGNORE
        {
            convenience_symlinks
                .get_convenience_symlink_protos()
                .to_vec()
        } else {
            // OutputDirectoryLinksUtils.EMPTY_SYMLINK_CREATION_RESULT.
            Vec::new()
        };
        if build_request_options.experimental_convenience_symlinks_bep_event {
            self.post(Arc::new(ConvenienceSymlinksIdentifiedEvent::new(protos)));
        }
    }

    /// `SpawnLogModule.buildComplete`: `event.getResult().getBuildToolLogCollection()
    /// .addLocalFile(EXEC_LOG_FILENAME, outputPath)`, added by [`BepCommandSupport::build_complete`]
    /// after the entries of `BuildSummaryStatsModule` and before those of the
    /// `CoverageReportCollector` (the order of the oracle's `buildToolLogs`).
    // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:218 SpawnLogModule.buildComplete
    pub fn add_execution_log(&self, name: String, path: Path) {
        *self.execution_log_file.lock() = Some((name, path));
    }

    /// `eventBus.register(new CoverageReportCollector(wrapper))`: the build tool log files the
    /// collector adds when the build completes.
    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:113 BazelCoverageReportModule.getCoverageReportFactory
    pub fn register_coverage_report_collector(&self, files: Vec<(&'static str, Path)>) {
        self.coverage_report_log_files.lock().extend(files);
    }

    /// `BuildTool.stopRequest` from the BEP's side: the `BuildCompleteEvent` reaches
    /// `BuildSummaryStatsModule` (which adds its tool log values to the result), the UI (which
    /// draws its progress bar while the BEP transports are still open) and the streamer; then
    /// the `BuildToolLogs` event is posted.
    ///
    /// Call right after the UI's `Build completed ...` line, with the result's stop time set.
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1155 BuildTool.stopRequest
    pub fn build_complete(
        &self,
        build_result: &mut BuildResult,
        detailed_exit_code: &blaze_util::DetailedExitCode,
        collector: &ExecutionEventCollector,
        action_executor: Option<&blaze_sky_exec::skyframe_action_executor::SkyframeActionExecutor>,
    ) {
        // ExecutionTool.unconditionalExecutionPhaseFinalizations, as the MetricsCollector receives
        // it (posted whenever the build's ExecutionTool exists, before stopRequest).
        if collector.executor_initialized() {
            self.unconditional_execution_phase_finalizations(action_executor);
        }
        // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
        let elapsed_seconds = build_result.get_elapsed_seconds();
        build_result
            .get_build_tool_log_collection()
            // String.format("%f", ...): six digits, Java's half-up rounding of the shortest
            // decimal representation.
            .add_direct_value(
                "elapsed time",
                blaze_events::java_format::java_format_fixed(elapsed_seconds, 6).as_bytes(),
            );
        // criticalPath.toString(), when executorInit registered a CriticalPathComputer.
        if let Some(critical_path) = collector.critical_path_tool_log() {
            build_result
                .get_build_tool_log_collection()
                .add_direct_value("critical path", critical_path.as_bytes());
        }
        // Stop the profiler and list its file ("command.profile.gz").
        // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:196 BuildSummaryStatsModule.buildComplete
        if let Some(message) = crate::profiler_init::stop_and_publish_profile(
            build_result.get_build_tool_log_collection(),
        ) {
            self.reporter.handle(&Event::error(message));
        }
        let spawn_summary_string =
            SpawnStats::convert_summary_to_string(&collector.spawn_stats.get_summary());
        build_result
            .get_build_tool_log_collection()
            .add_direct_value("process stats", spawn_summary_string.as_bytes());
        // SpawnLogModule.buildComplete.
        // Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:218 SpawnLogModule.buildComplete
        if let Some((name, path)) = self.execution_log_file.lock().take() {
            build_result
                .get_build_tool_log_collection()
                .add_local_file(&name, path);
        }
        // CoverageReportCollector.buildComplete (registered on the event bus during the build,
        // after the modules).
        // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:144 CoverageReportCollector.buildComplete
        for (name, path) in self.coverage_report_log_files.lock().iter() {
            build_result
                .get_build_tool_log_collection()
                .add_local_file(name, path.clone());
        }

        self.post(Arc::new(blaze_bep::bep_module::BuildEventPost(Arc::new(
            build_complete_event(
                &BuildResultData {
                    detailed_exit_code: detailed_exit_code.clone(),
                    stop_time_millis: build_result.get_stop_time(),
                    has_unhandled_throwable: build_result.get_unhandled_throwable().is_some(),
                    was_catastrophe: build_result.was_catastrophe(),
                    success: build_result.get_success(),
                    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1121 BuildTool.maybeSetStopOnFirstFailure
                    stop_on_first_failure: build_result.get_stop_on_first_failure()
                        || self
                            .stop_on_first_failure
                            .unwrap_or_else(|| panic!("NullPointerException: ExecutionOptions")),
                },
                vec![
                    build_event_id_util::build_tool_logs(),
                    build_event_id_util::build_metrics(),
                ],
            ),
        ))));
        // TargetSummaryPublisher.buildCompleteEvent (a subscriber of the BuildCompleteEvent; its
        // TargetSummaryEvents reach the streamer after the BuildCompleteEvent).
        // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:185 TargetSummaryPublisher.buildCompleteEvent
        if let Some(publisher) = &self.shared.target_summary_publisher {
            let key_of = |t: &blaze_buildtool::buildtool::build_result::ConfiguredTargetRef| {
                TargetSummaryKey {
                    label: t.get_label(),
                    configuration_checksum: t.get_configuration_checksum(),
                }
            };
            let actual: Option<Vec<TargetSummaryKey>> = build_result
                .get_actual_targets()
                .map(|ts| ts.iter().map(key_of).collect());
            let successful: Option<Vec<TargetSummaryKey>> = build_result
                .get_successful_targets()
                .map(|ts| ts.iter().map(key_of).collect());
            let aspects: Vec<TargetSummaryKey> = build_result
                .get_successful_aspects()
                .map(|aspects| {
                    aspects
                        .iter()
                        .map(|key| {
                            let base = aspect_key_of(key).get_base_configured_target_key();
                            TargetSummaryKey {
                                label: base.get_label().clone(),
                                configuration_checksum: base
                                    .get_configuration_key()
                                    .map(|k| k.get_options_checksum().to_string()),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            publisher.build_complete_event(actual.as_deref(), successful.as_deref(), &aspects);
        }
        // Post the build tool logs event; the corresponding local files may be contributed from
        // modules, and this has to happen after posting the BuildCompleteEvent because that's
        // when modules add their data to the collection.
        let collection = build_result.get_build_tool_log_collection().freeze();
        self.post(Arc::new(blaze_bep::bep_module::BuildEventPost(Arc::new(
            build_tool_logs_event(collection),
        ))));
    }

    /// `ExecutionTool.unconditionalExecutionPhaseFinalizations` from the `MetricsCollector`'s
    /// side: the `ExecutionFinishedEvent` (`onExecutionComplete`, the artifact metrics) and, when
    /// some execution activity started the timer, the `ExecutionPhaseCompleteEvent`
    /// (`handleExecutionPhaseComplete`).
    ///
    /// PORT: called from `build_complete` (right after the point Java posts the events: the
    /// ExecutionTool's finalizations precede `BuildTool.stopRequest`, and nothing between them
    /// reads the metrics). The `ExecutionFinishedEvent`'s artifact counts are those of the
    /// action executor's `FilesMetricConsumer`s (`SkyframeExecutor.createExecutionFinishedEvent`;
    /// see `blaze_sky_exec::metadata_consumer_for_metrics`); without an executor (no execution
    /// state) nothing was counted and the consumers would hold zeros.
    // Java: com/google/devtools/build/lib/buildtool/ExecutionTool.java:592 ExecutionTool.unconditionalExecutionPhaseFinalizations
    fn unconditional_execution_phase_finalizations(
        &self,
        action_executor: Option<&blaze_sky_exec::skyframe_action_executor::SkyframeActionExecutor>,
    ) {
        // env.getEventBus().post(skyframeExecutor.createExecutionFinishedEvent())
        // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:267 MetricsCollector.onExecutionComplete
        // The event is this call with the four artifact counts (its other subscriber,
        // BuildSummaryStatsModule.executionPhaseFinish, is blaze_buildtool's ExecutionTool).
        // PORT: skipped, read by no subscriber at the pin: ExecutionFinishedEvent.builderWithDefaults Builder.setOutputDirtyFiles Builder.setOutputDirtyFileExecPathSample Builder.setOutputModifiedFilesDuringPreviousBuild Builder.setSourceDiffCheckingDuration Builder.setNumSourceFilesCheckedBecauseOfMissingDiffs Builder.setOutputTreeDiffCheckingDuration
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:38 ExecutionFinishedEvent.<init>
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:64 ExecutionFinishedEvent.builder
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:85 ExecutionFinishedEvent.Builder.setSourceArtifactsRead
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:87 ExecutionFinishedEvent.Builder.setOutputArtifactsSeen
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:89 ExecutionFinishedEvent.Builder.setOutputArtifactsFromActionCache
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:91 ExecutionFinishedEvent.Builder.setTopLevelArtifacts
        // Java: com/google/devtools/build/lib/skyframe/ExecutionFinishedEvent.java:93 ExecutionFinishedEvent.Builder.build
        let metrics = action_executor
            .map(|e| {
                e.artifact_metrics()
                    .to_execution_finished_metrics_and_reset()
            })
            .unwrap_or_default();
        self.shared.metrics.on_execution_complete(
            &blaze_bep::metrics::metrics_collector::ExecutionFinishedInfo {
                source_artifacts_read: metrics.source_artifacts_read,
                output_artifacts_seen: metrics.output_artifacts_seen,
                output_artifacts_from_action_cache: metrics.output_artifacts_from_action_cache,
                top_level_artifacts: metrics.top_level_artifacts,
            },
        );
        if let Some(start) = self.shared.execution_timer.lock().take() {
            // new ExecutionPhaseCompleteEvent(executionTimer.stop().elapsed().toMillis()): the
            // event is this call with its time.
            // Java: com/google/devtools/build/lib/buildtool/buildevent/ExecutionPhaseCompleteEvent.java:28 ExecutionPhaseCompleteEvent.<init>
            // Java: com/google/devtools/build/lib/buildtool/buildevent/ExecutionPhaseCompleteEvent.java:32 ExecutionPhaseCompleteEvent.getTimeInMs
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:233 MetricsCollector.handleExecutionPhaseComplete
            self.shared
                .metrics
                .handle_execution_phase_complete(start.elapsed().as_millis() as i64);
        }
    }

    /// `env.precompleteCommand`: `MetricsCollector.onCommandPrecompleteEvent` posts the
    /// `BuildMetricsEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:313 MetricsCollector.onCommandPrecompleteEvent
    pub fn command_precomplete(&self) {
        // PORT: `run_build` precompletes before its remote module's afterCommand (Java's order:
        // precompleteCommand, then the modules' afterCommand); the dispatcher's call for the
        // other commands is then a no-op.
        if self.precompleted.swap(true, Ordering::SeqCst) {
            return;
        }
        // createBuildMetrics: WorkerProcessMetricsCollector.instance().collectMetrics().
        let worker_process_metrics =
            blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector::instance()
                .collect_metrics();
        // Java: com/google/devtools/build/lib/profiler/Profiler.java:420 Profiler.getProfileElapsedTime
        // Duration.toMillis() of a non-negative duration truncates.
        let profile_elapsed_time_ms =
            blaze_util::profiler::Profiler::get_profile_elapsed_time_nanos() / 1_000_000;
        // Java: com/google/devtools/build/lib/profiler/Profiler.java:433 Profiler.getServerProcessCpuTime
        let server_process_cpu_time_ms =
            blaze_util::profiler::Profiler::get_server_process_cpu_time_nanos() / 1_000_000;
        let event = self
            .shared
            .metrics
            .on_command_precomplete_event(BuildMetricsFinishInputs {
            worker_process_metrics: worker_process_metrics
                .iter()
                .map(|wpm| WorkerProcessMetricsInfo::of(wpm))
                .collect(),
            worker_metrics: worker_process_metrics
                .iter()
                .map(|wpm| wpm.to_proto())
                .collect(),
            // PORT: env.getPendingException() is always null when a command precompletes in
            // Blaze: the pending exception is thrown before the command returns.
            has_pending_exception: false,
            // getSkyframeStats is asked for only with --experimental_record_skyframe_metrics.
            skyframe_stats: &|| -> Option<SkyframeStats> {
                blaze_util::unported!("SkyframeExecutor.getSkyframeStats")
            },
            // PORT: remote analysis caching is not ported (no cache hits, misses or stores).
            remote_analysis_caching: RemoteAnalysisCachingFacts::default(),
            // MemoryProfiler's heap at FINISH (D15: resident memory after malloc_trim). PORT:
            // the PostGCMemoryUseRecorder figures stay absent: they come from the JVM's GC
            // notifications, and Blaze has no garbage collector.
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:508 MetricsCollector.createMemoryMetrics
            memory: JvmMemoryFacts {
                heap_used_memory_at_finish:
                    blaze_util::profiler::memory_profiler::MemoryProfiler::instance()
                        .get_heap_used_memory_at_finish(),
                ..JvmMemoryFacts::default()
            },
            // Profiler.getProfileElapsedTime() and getServerProcessCpuTime() never return
            // null.
            profile_elapsed_time_ms: Some(profile_elapsed_time_ms),
            server_process_cpu_time_ms: Some(server_process_cpu_time_ms),
            network_metrics:
                blaze_util::profiler::network_metrics_collector::NetworkMetricsCollector::instance()
                    .collect_metrics(),
        });
        self.post(Arc::new(event));
    }

    /// `BuildEventServiceModule.closeBepTransports`: the transports that may not be slow, and
    /// the slow ones in `WAIT_FOR_UPLOAD_COMPLETE` mode, are waited for
    /// (`waitForBuildEventTransportsToClose`); the UI is told of each transport that finished
    /// closing (`BuildEventTransportClosedEvent`), and at once of the asynchronous ones, whose
    /// uploads the next command waits for (`waitForPreviousInvocation`). A failed close is the
    /// module's `AbruptExitException`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:637 BuildEventServiceModule.closeBepTransports
    fn close_bep_transports(&self) -> Result<(), AbruptExitException> {
        let ids = self.transport_ids();
        let streamer = self.shared.streamer.streamer();
        let transports: Vec<Arc<dyn BuildEventTransport>> = streamer.get_transports().to_vec();
        let close_futures = streamer.get_close_futures();
        // closeFuturesWithTimeoutsMap / halfCloseFuturesWithTimeoutsMap
        let pending: Vec<PendingUpload> = transports
            .iter()
            .zip(close_futures)
            .enumerate()
            .map(|(index, (transport, (_name, close_future)))| {
                PendingUpload::new(
                    transport.clone(),
                    ids.get(index).cloned(),
                    close_future,
                    transport.get_half_close_future(),
                )
            })
            .collect();
        close_pending_uploads(pending, &|id| self.notify_transport_closed(id))
    }

    /// `reporter.post(new BuildEventTransportClosedEvent(bepTransport))` to the UI: the event is
    /// the subscriber's call with the transport.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransportClosedEvent.java:25 BuildEventTransportClosedEvent.<init>
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransportClosedEvent.java:32 BuildEventTransportClosedEvent.transport
    fn notify_transport_closed(&self, id: Option<&TransportId>) {
        if let (Some(subscriber), Some(id)) = (&*self.ui_subscriber.lock(), id) {
            subscriber.build_event_transport_closed(id, self.ui_debug_all_events);
        }
    }

    /// `BuildEventServiceModule.forceShutdownBuildEventStreamer`: closes the streamer with the
    /// abort reason (each announced event not yet posted gets an `Aborted` event, then the last
    /// message) and waits for the transports to close.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:476 BuildEventServiceModule.forceShutdownBuildEventStreamer
    pub(crate) fn force_shutdown_build_event_streamer(&self, reason: AbortReason) {
        let streamer = &self.shared.streamer;
        streamer.close_on_abort(reason);
        // closeFuturesWithTimeoutsMap = constructCloseFuturesMapWithTimeouts(...)
        let transports: Vec<Arc<dyn BuildEventTransport>> =
            streamer.streamer().get_transports().to_vec();
        let uploads: Vec<PendingUpload> = transports
            .iter()
            .zip(streamer.streamer().get_close_futures())
            .map(|(transport, (_name, close_future))| {
                PendingUpload::new(
                    transport.clone(),
                    None,
                    close_future,
                    transport.get_half_close_future(),
                )
            })
            .collect();
        *PENDING_UPLOADS.lock() = uploads.clone();
        // GC thrashing during severe OOMs may prevent future completion, so don't wait forever
        // (--bes_oom_finish_upload_timeout); otherwise getUninterruptibly(besClosedFuture).
        let deadline = (reason == AbortReason::OutOfMemory).then(|| {
            std::time::Instant::now() + self.bes.bes_options.bes_oom_finish_upload_timeout
        });
        // A failed close is only logged by Java (`logger.atSevere()...log("Failed to close a
        // build event transport")`); logging is not ported, so the results are not read.
        // Futures.allAsList(...).get(): the first failure (or the deadline) ends the wait.
        'wait: loop {
            let mut all_done = true;
            for upload in &uploads {
                match upload.close.poll() {
                    None => all_done = false,
                    Some(TimedOutcome::Done(Ok(()))) => {}
                    Some(_) => break 'wait,
                }
            }
            if all_done || deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // finally: cancelAndResetPendingUploads()
        cancel_and_reset_pending_uploads();
    }

    /// `BuildEventServiceModule.blazeShutdownOnCrash`: the module's streamer is closed with
    /// `INTERNAL` (`OUT_OF_MEMORY` for an OOM crash), then the uploader factory is shut down.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:500 BuildEventServiceModule.blazeShutdownOnCrash
    pub(crate) fn blaze_shutdown_on_crash(&self, exit_code: &DetailedExitCode) {
        // logger.atWarning().log("Attempting to close BES streamer on crash")
        self.force_shutdown_build_event_streamer(
            if exit_code.get_exit_code() == blaze_util::ExitCode::OOM_ERROR {
                AbortReason::OutOfMemory
            } else {
                AbortReason::Internal
            },
        );
        // uploaderFactoryToCleanup.shutdown()
        self.uploader_factory_map.shutdown();
    }

    /// `BuildEventServiceModule.afterCommand`: closes the streamer if it did not close itself,
    /// waits for the transports, and reports the files as produced. A transport's close failure
    /// is the module's `AbruptExitException`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:662 BuildEventServiceModule.afterCommand
    pub fn after_command(&self) -> Result<(), AbruptExitException> {
        // PORT: the commands that build run the module's afterCommand themselves, between their
        // remote module's and the dispatcher's (see crate::commands::build::run_build); the
        // dispatcher's call is then a no-op.
        if self.after_command_done.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let streamer = &self.shared.streamer;
        if !streamer.is_closed() {
            // This should not occur, but close with an internal error if a {@link
            // BuildEventStreamer} bug manifests as an unclosed streamer.
            self.reporter
                .handle(&Event::warn("BES was not properly closed"));
            self.force_shutdown_build_event_streamer(AbortReason::Internal);
        }

        self.close_bep_transports()?;

        if self.bes.bes_backend_set {
            blaze_bes::module::construct_and_maybe_report_invocation_id_url(
                self.reporter.as_ref(),
                &self.bes.bes_options,
                &self.bes.invocation_id,
                self.bes.ui_uses_color,
            );
        } else {
            // !bepTransports.isEmpty(): a support exists only with transports.
            self.reporter.handle(&Event::info(
                "Build Event Protocol files produced successfully.",
            ));
        }

        // besStreamOptions can be null if we are crashing. Don't crash here too.
        if !self.bes.keep_backend_connections {
            blaze_bes::module::clear_bes_client();
        }
        Ok(())
    }
}

/// The posting points of loading and analysis (`blaze_sky_top`'s `BuildTool` calls them where
/// Java posts the events, so they are posted whether or not analysis succeeds or anything
/// executes).
impl BuildEventHooks for BepCommandSupport {
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:288 BuildTool.buildTargets
    // Java: com/google/devtools/build/lib/buildtool/buildevent/BuildStartingEvent.java:67 BuildStartingEvent.create
    fn build_starting(&self, request: &BuildRequest) {
        // PORT: build, test and run posted it before BuildTool.processRequest (see the trait).
        if self.build_starting_posted.swap(true, Ordering::SeqCst) {
            return;
        }
        self.reannounce_transports_to_build_tracker();
        // Java: com/google/devtools/build/lib/runtime/TargetSummaryPublisher.java:80 TargetSummaryPublisher.buildStarting
        if let Some(publisher) = &self.shared.target_summary_publisher {
            publisher.build_starting(self.has_aspects);
        }
        let data = BuildStartingData {
            targets: request.targets.clone(),
            ..self.build_starting_context.lock().clone()
        };
        self.post(Arc::new(BuildStartingEvent::create(data)));
    }

    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:76 TargetPatternPhaseFunction.compute
    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:311 BuildTool.buildTargets
    fn target_patterns_evaluated(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        target_pattern_phase_value: &TargetPatternPhaseValue,
    ) {
        self.post_target_parsing_complete(request, skyframe_executor, target_pattern_phase_value);
        self.post(Arc::new(ExecRootEvent::new(self.exec_root.clone())));
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:343 BuildTool.buildTargets
    fn project_evaluated(&self, project_flags_applied: bool) {
        // With PROJECT.scl flag updates, the final CanonicalCommandLineEvent was posted when the
        // flags were applied (`post_project_canonical_command_line`).
        if project_flags_applied {
            return;
        }
        // No PROJECT.scl flag updates. Release the original CanonicalCommandLineEvent for
        // posting.
        self.post(Arc::new(ReleaseReplaceableBuildEvent::new(
            build_event_id_util::structured_commandline_id(CanonicalCommandLineEvent::LABEL),
        )));
    }

    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1113 CommandEnvironment.ensureBuildInfoPosted
    fn ensure_build_info_posted(&self) {
        if self.shared.build_info_posted.load(Ordering::SeqCst) {
            return;
        }
        // workspace.getWorkspaceStatusActionFactory().createDummyWorkspaceStatus(
        // workspaceInfoFromDiff): Bazel's factory (BazelWorkspaceStatusModule).
        use blaze_actions::analysis::workspace_status_action::WorkspaceStatusActionFactory as _;
        let factory =
            blaze_actions::bazel::bazel_workspace_status_module::BazelStatusActionFactory {
                username: user_name(),
                hostname: blaze_sky_top::skyframe_executor::short_host_name(),
            };
        let workspace_status: IndexMap<String, String> = factory
            .create_dummy_workspace_status()
            .into_iter()
            .collect();
        self.post(Arc::new(BuildEventPost(Arc::new(BuildInfoEvent::new(
            workspace_status,
        )))));
        // gotBuildInfo(BuildInfoEvent)
        self.shared.build_info_posted.store(true, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:560 BuildTool.buildTargetsWithoutMergedAnalysisExecution
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:168 AnalysisPhaseRunner.execute
    fn no_analyze(&self) {
        self.post(Arc::new(
            blaze_bep::events::buildtool_events::NoAnalyzeEvent,
        ));
    }

    fn no_execution(&self) {
        self.post(Arc::new(
            blaze_bep::events::buildtool_events::NoExecutionEvent,
        ));
    }

    /// The analysis metrics (`AnalysisPhaseRunner`) and a `TargetConfiguredEvent` per top-level
    /// target (`SkyframeExecutor.configureTargets`).
    fn analysis_complete(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        _target_pattern_phase_value: &TargetPatternPhaseValue,
        analysis_result: &AnalysisResult,
    ) {
        // With merged analysis and execution the metrics arrive with the
        // AnalysisPhaseCompleteStatsEvent of analysisFinishedCallback.
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            self.analysis_metrics(
                analysis_result,
                blaze_sky_top::analysis_phase_runner::analysis_phase_complete_event_posted(
                    request,
                    analysis_result,
                ),
            );
        }
        self.post_targets_configured(
            skyframe_executor,
            analysis_result,
            !request.with_merged_analysis_and_execution_source_of_truth(),
        );
        // The path without merged analysis and execution: after reportTargets.
        // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:165 AnalysisPhaseRunner.execute
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            self.post_aborted_events_for_skipped_targets(analysis_result.get_targets_to_skip());
        }
    }
}

/// The target of `label` (an alias's actual target), or `original` when it is that target.
fn actual_target(skyframe_executor: &SkyframeExecutor, label: &Label, original: &Target) -> Target {
    if original.get_label() == label {
        return original.clone();
    }
    skyframe_executor
        .get_existing_package(label.get_package_identifier())
        .and_then(|p| p.get_targets().get(label.get_name()).cloned())
        .unwrap_or_else(|| panic!("NullPointerException: target {label}"))
}

/// The build events of a command that does not build (`QueryEnvironmentBasedCommand.exec`,
/// `FetchCommand.exec`, `ModCommand.exec`): `NoBuildEvent(commandName, commandStartTime,
/// separateFinishedEvent=true, showProgress=true, id=null)` before the command's work and
/// `NoBuildRequestFinishedEvent(exitCode, now)` after it.
// Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:80 QueryEnvironmentBasedCommand.exec
pub(crate) fn exec_with_no_build_events(
    env: &mut CommandEnvironment,
    exec_internal: impl FnOnce(&mut CommandEnvironment) -> crate::command::BlazeCommandResult,
) -> crate::command::BlazeCommandResult {
    if let Some(bep) = env.get_bep() {
        bep.post_no_build_event(
            Some(env.get_command_name().to_string()),
            Some(env.get_command_start_time()),
            true,
            true,
            None,
        );
    }
    // The UI's subscriber of NoBuildEvent (showProgress = true).
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:777 UiEventHandler.noBuild
    if let Some(handler) = env.get_ui_event_handler() {
        handler.no_build(true);
    }
    let result = exec_internal(env);
    // Java: com/google/devtools/build/lib/runtime/commands/QueryEnvironmentBasedCommand.java:91 QueryEnvironmentBasedCommand.exec
    // PORT: FetchCommand and ModCommand do not mark the phase; neither analyzes, so the profiler
    // is inactive for them (tracerEnabled) and this is a no-op.
    blaze_util::profiler::Profiler::instance()
        .mark_phase(blaze_util::profiler::ProfilePhase::FINISH);
    if let Some(bep) = env.get_bep() {
        bep.post_no_build_request_finished_event(
            result.get_exit_code(),
            JavaClock.current_time_millis(),
        );
    }
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:788 UiEventHandler.noBuildFinished
    if let Some(handler) = env.get_ui_event_handler() {
        handler.no_build_finished();
    }
    result
}

/// The runtime's `BuildEventArtifactUploaderFactoryMap` as `BuildEventServiceModule.beforeCommand`
/// selects from it.
///
/// PORT: in Java the map's "remote" entry is `RemoteModule`'s factory delegate, which
/// `RemoteModule.beforeCommand` initialises before the BES module's `beforeCommand` runs (module
/// order). Blaze sets the remote module up inside the commands that build
/// (`crate::commands::build::run_build`), after the BES module created its transports, so the map
/// is replaceable ([`DeferredUploaderFactory::set_map`]) until the first upload, and the uploader
/// the transports hold ([`DeferredUploader`]) is created from the map at that first upload. The
/// events posted before a command's remote module is set up (options, command lines, metadata,
/// `NoBuildEvent`) reference no local files.
pub(crate) struct DeferredUploaderFactory {
    state: Arc<DeferredUploaderState>,
}

/// The map and strategy a [`DeferredUploaderFactory`] and its uploaders share.
struct DeferredUploaderState {
    map: Mutex<BuildEventArtifactUploaderFactoryMap>,
    /// `--experimental_build_event_upload_strategy`.
    strategy: Mutex<Option<String>>,
}

impl DeferredUploaderState {
    /// `getBuildEventArtifactUploaderFactoryMap().select(buildEventUploadStrategy)`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:390 BuildEventServiceModule.beforeCommand
    fn select(&self) -> Arc<dyn BuildEventArtifactUploaderFactory> {
        let strategy = self.strategy.lock().clone();
        self.map.lock().select(strategy.as_deref())
    }
}

impl DeferredUploaderFactory {
    fn new(map: BuildEventArtifactUploaderFactoryMap) -> Arc<Self> {
        Arc::new(DeferredUploaderFactory {
            state: Arc::new(DeferredUploaderState {
                map: Mutex::new(map),
                strategy: Mutex::new(None),
            }),
        })
    }

    fn set_strategy(&self, strategy: Option<String>) {
        *self.state.strategy.lock() = strategy;
    }

    fn set_map(&self, map: BuildEventArtifactUploaderFactoryMap) {
        *self.state.map.lock() = map;
    }
}

impl BuildEventArtifactUploaderFactory for DeferredUploaderFactory {
    fn create(&self) -> Arc<dyn BuildEventArtifactUploader> {
        Arc::new(DeferredUploader {
            state: self.state.clone(),
            delegate: Mutex::new(None),
            pending_retains: Mutex::new(0),
        })
    }

    /// `uploaderFactoryToCleanup.shutdown()`: the selected factory's.
    // Java: com/google/devtools/build/lib/runtime/BuildEventArtifactUploaderFactory.java:37 BuildEventArtifactUploaderFactory.shutdown
    fn shutdown(&self) {
        self.state.select().shutdown();
    }
}

/// The uploader [`DeferredUploaderFactory`] creates: the selected factory's uploader, created at
/// the first upload of a file (see there).
pub(crate) struct DeferredUploader {
    state: Arc<DeferredUploaderState>,
    delegate: Mutex<Option<Arc<dyn BuildEventArtifactUploader>>>,
    /// `retain()` calls before the delegate exists.
    pending_retains: Mutex<i32>,
}

impl DeferredUploader {
    fn delegate(&self) -> Arc<dyn BuildEventArtifactUploader> {
        let mut delegate = self.delegate.lock();
        if let Some(d) = &*delegate {
            return d.clone();
        }
        let created = self.state.select().create();
        for _ in 0..std::mem::take(&mut *self.pending_retains.lock()) {
            created.retain();
        }
        *delegate = Some(created.clone());
        created
    }
}

impl BuildEventArtifactUploader for DeferredUploader {
    fn upload(
        &self,
        files: std::collections::HashMap<Path, blaze_bep::buildeventstream::LocalFile>,
    ) -> PathConverterFuture {
        if files.is_empty() && self.delegate.lock().is_none() {
            // Nothing to upload: no uploader is needed yet (Java's would answer the same way).
            return LocalFilesArtifactUploader.upload(files);
        }
        self.delegate().upload(files)
    }

    fn may_be_slow(&self) -> bool {
        match &*self.delegate.lock() {
            Some(d) => d.may_be_slow(),
            // PORT: before the first upload the remote module may not be set up yet (see
            // DeferredUploaderFactory); the events sent until then reference no files.
            None => false,
        }
    }

    fn retain(&self) {
        match &*self.delegate.lock() {
            Some(d) => d.retain(),
            None => *self.pending_retains.lock() += 1,
        }
    }

    fn release(&self) {
        let delegate = self.delegate.lock().clone();
        match delegate {
            Some(d) => d.release(),
            None => {
                let mut pending = self.pending_retains.lock();
                if *pending > 0 {
                    *pending -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_metadata_map_is_sorted_and_later_entries_win() {
        let map = make_map_from_map_entries(&[
            ("b".to_string(), "1".to_string()),
            ("a".to_string(), "2".to_string()),
            ("b".to_string(), "3".to_string()),
        ]);
        assert_eq!(
            map.into_iter().collect::<Vec<_>>(),
            vec![
                ("a".to_string(), "2".to_string()),
                ("b".to_string(), "3".to_string())
            ]
        );
    }

    #[test]
    fn test_sizes_map_to_the_event_enum() {
        assert_eq!(
            packages_test_size(Some(TestSize::SMALL)),
            Some(PackagesTestSize::Small)
        );
        assert_eq!(
            packages_test_size(Some(TestSize::ENORMOUS)),
            Some(PackagesTestSize::Enormous)
        );
        assert_eq!(packages_test_size(None), None);
    }
}

impl BepCommandSupport {
    /// `SkyframeGraphStatsEvent` as the command's `MetricsCollector` receives it
    /// (`onSkyframeGraphStats`). Java posts it from `SkyframeExecutor.notifyCommandComplete` in
    /// `BlazeRuntime.afterCommand`, right before `buildtool.CommandPrecompleteEvent`, the event
    /// whose subscriber builds the `BuildMetricsEvent`; the build driver hands it here before
    /// [`BepCommandSupport::command_precomplete`].
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:299 MetricsCollector.onSkyframeGraphStats
    pub fn skyframe_graph_stats(
        &self,
        event: &blaze_skyframe::evaluation_stats::SkyframeGraphStatsEvent,
    ) {
        self.shared
            .metrics
            .on_skyframe_graph_stats(&skyframe_graph_stats_info(event));
    }
}

/// The event's statistics in Java's order: `ImmutableMap.toImmutableMap` over the entries of a
/// `ConcurrentHashMultiset` whose backing map is `new ConcurrentHashMap<>(availableProcessors(),
/// 0.75f)` (`InMemoryMemoizingEvaluator.ProgressReceiver.createMultiset`), keyed by
/// `SkyFunctionName` (`hashCode()` is its name's).
///
/// PORT: java-hash-order. The map's iteration order is replayed with the function names inserted
/// in first-occurrence order (Java's concurrent insertion order is not deterministic; it only
/// matters within one bin).
// Java: com/google/devtools/build/skyframe/InMemoryMemoizingEvaluator.java:76 InMemoryMemoizingEvaluator.ProgressReceiver.fromMultiset
fn skyframe_graph_stats_info(
    event: &blaze_skyframe::evaluation_stats::SkyframeGraphStatsEvent,
) -> blaze_bep::metrics::metrics_collector::SkyframeGraphStatsInfo {
    use blaze_query_engine::java_concurrent_hash_map::JavaConcurrentHashMap;
    // Runtime.getRuntime().availableProcessors()
    let available_processors = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(1);
    let ordered = |entries: &[(&'static str, i32)]| -> Vec<(String, i32)> {
        let mut map: JavaConcurrentHashMap<String, i32> =
            JavaConcurrentHashMap::with_capacity(available_processors, 0.75, 1);
        for (name, count) in entries {
            map.put((*name).to_owned(), *count);
        }
        map.entries().map(|(k, v)| (k.clone(), *v)).collect()
    };
    let stats = &event.evaluation_stats;
    blaze_bep::metrics::metrics_collector::SkyframeGraphStatsInfo {
        dirtied: ordered(&stats.dirtied),
        changed: ordered(&stats.changed),
        built: ordered(&stats.built),
        cleaned: ordered(&stats.cleaned),
        evaluated: ordered(&stats.evaluated),
        graph_size: event.graph_size,
    }
}

/// `MetricsModule.beforeCommand` for a command that has no build event transport: Java installs
/// the `MetricsCollector` for every command (`MetricsCollector.installInEnv`), so the command
/// still counts its analysis and build in the server's cumulative metrics, although no
/// `BuildMetricsEvent` reaches a stream.
///
/// PORT: with a transport the collector is part of [`BepCommandSupport`]; without one this holds
/// it, and the build driver gives it the analysis hook ([`BuildEventHooks::analysis_complete`],
/// where Blaze delivers `AnalysisPhaseStartedEvent`) while its subscriber receives the bus
/// events that change the counters.
// Java: com/google/devtools/build/lib/metrics/MetricsModule.java:78 MetricsModule.beforeCommand
pub struct MetricsOnlyCommandSupport {
    metrics: Arc<MetricsCollector>,
}

impl MetricsOnlyCommandSupport {
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:139 MetricsCollector.installInEnv
    pub fn before_command(env: &CommandEnvironment) -> Arc<MetricsOnlyCommandSupport> {
        let clock: Arc<dyn Clock> = Arc::new(JavaClock);
        let metrics_options: Option<MetricsModuleOptions> = env.get_options().get_options();
        let (num_analyses, num_builds) = crate::bazel_modules::MetricsModule::counters();
        let metrics = Arc::new(MetricsCollector::install_in_env(
            metrics_options.as_ref(),
            clock.clone(),
            num_analyses,
            num_builds,
        ));
        // MetricsCollector.<init>: WorkerProcessMetricsCollector.instance().setClock(env.getClock()).
        // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:133 MetricsCollector.<init>
        blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector::instance()
            .set_clock(clock);
        // env.getEventBus().register(this)
        env.get_event_bus()
            .register(Arc::new(MetricsOnlySubscriber {
                metrics: metrics.clone(),
            }));
        Arc::new(MetricsOnlyCommandSupport { metrics })
    }
}

impl BuildEventHooks for MetricsOnlyCommandSupport {
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:145 MetricsCollector.logAnalysisStartingEvent
    fn analysis_complete(
        &self,
        request: &BuildRequest,
        _skyframe_executor: &SkyframeExecutor,
        _target_pattern_phase_value: &TargetPatternPhaseValue,
        _analysis_result: &AnalysisResult,
    ) {
        // With merged analysis and execution: the subscriber, at the
        // AnalysisPhaseCompleteStatsEvent.
        if !request.with_merged_analysis_and_execution_source_of_truth() {
            self.metrics.log_analysis_starting_event();
        }
    }
}

/// The `MetricsCollector`'s subscriptions that change the cumulative counters (the others only
/// feed the `BuildMetricsEvent`, which no transport receives).
struct MetricsOnlySubscriber {
    metrics: Arc<MetricsCollector>,
}

impl EventBus for MetricsOnlySubscriber {
    fn post(&self, event: Arc<dyn Postable>) {
        if downcast_event::<blaze_sky_top::skyframe_build_view::AnalysisPhaseCompleteStatsEvent>(
            &event,
        )
        .is_some()
        {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:145 MetricsCollector.logAnalysisStartingEvent
            self.metrics.log_analysis_starting_event();
        }
        if downcast_event::<TopLevelTargetPendingExecutionEvent>(&event).is_some() {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:213 MetricsCollector.handleExecutionPhaseStart
            self.metrics.handle_execution_phase_start();
        }
    }
}

/// Port of `BazelBuildEventServiceModuleTest`.
///
/// PORT: Java runs whole builds (`BuildIntegrationTestCase`) against an in-process
/// `DelayingPublishBuildEventService`. Blaze has no in-process build harness; the tests of the
/// three upload modes, `--bes_timeout` and the second command's wait drive the module's closing
/// and waiting code (`close_pending_uploads`, `wait_for_previous_invocation`) with a fake
/// Build Event Service transport whose close and half-close futures behave as the delaying
/// service makes the real transport's behave: `setDelayBeforeClosingStream` (half-closed at once,
/// closed late), `setDelayBeforeHalfClosingStream` (both late) and `setErrorMessage` (the close
/// fails). The uploader itself is covered by `blaze_bes`' transport tests. Times are scaled
/// down: `--bes_timeout` 1s -> 200ms, `WAIT_FOR_LAST_INVOCATION_TIMEOUT` 2s -> 500ms, the
/// service's 10s delays -> 3s. The tests that need a build are ignored with the reason.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModuleTest.java
#[cfg(test)]
pub(crate) mod bes_module_tests {
    use std::cell::Cell;
    use std::time::Duration;

    use blaze_bep::buildeventstream::artifact_uploader::BuildEventArtifactUploader;
    use blaze_bep::buildeventstream::build_event::BuildEvent;
    use blaze_bep::buildeventstream::transport::{CloseFuture, TransportCloseError};
    use blaze_events::{EventKind, StoredEventHandler};

    use super::*;

    thread_local! {
        /// `getMaxWaitForPreviousInvocation()` of the test's module subclass.
        pub(crate) static MAX_WAIT_FOR_PREVIOUS_INVOCATION: Cell<Option<Duration>> =
            const { Cell::new(None) };
    }

    /// The pending uploads are the module's (a process global): one test at a time.
    static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    const BES_TIMEOUT: Duration = Duration::from_millis(200);
    const SLOW: Duration = Duration::from_secs(3);

    /// What the delaying service does to the stream.
    #[derive(Clone, Copy)]
    enum Service {
        /// Closes the stream at once.
        Fast,
        /// `setDelayBeforeClosingStream(10s)`: the half-close is done, the close is late.
        SlowFullClose,
        /// `setDelayBeforeHalfClosingStream(10s)`: both are late.
        SlowHalfClose,
        /// `setErrorMessage(message)`: the upload fails.
        Error(&'static str),
    }

    struct FakeBesTransport {
        close: CloseFuture,
        half_close: CloseFuture,
        timeout: Duration,
        mode: BesUploadMode,
    }

    impl BuildEventTransport for FakeBesTransport {
        fn name(&self) -> String {
            "Build Event Service".to_string()
        }
        fn send_build_event(&self, _event: &Arc<dyn BuildEvent>) {}
        fn close(&self) -> CloseFuture {
            self.close.clone()
        }
        fn get_half_close_future(&self) -> CloseFuture {
            self.half_close.clone()
        }
        fn get_timeout(&self) -> Duration {
            self.timeout
        }
        fn may_be_slow(&self) -> bool {
            true
        }
        fn get_bes_upload_mode(&self) -> BesUploadMode {
            self.mode
        }
        fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>> {
            None
        }
    }

    fn complete_later(
        future: &CloseFuture,
        after: Duration,
        result: Result<(), TransportCloseError>,
    ) {
        let future = future.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            future.set(result);
        });
    }

    /// A transport closing as `service` makes it close. The shapes of its futures are those
    /// the real `BuildEventServiceTransport` shows against a port of Java's
    /// `DelayingPublishBuildEventService`: `blaze_bes`' `tests/delaying_service_test.rs`
    /// observes them, one test per service (review BES-5).
    fn transport(service: Service, mode: BesUploadMode, timeout: Duration) -> PendingUpload {
        let close = CloseFuture::create();
        let half_close = CloseFuture::create();
        match service {
            Service::Fast => {
                half_close.set(Ok(()));
                close.set(Ok(()));
            }
            Service::SlowFullClose => {
                half_close.set(Ok(()));
                complete_later(&close, SLOW, Ok(()));
            }
            // The uploader half-closes as soon as it has sent the final event
            // (`halfCloseEventUploadingStream`); the service's delay holds back the acks and so
            // the close only.
            Service::SlowHalfClose => {
                half_close.set(Ok(()));
                complete_later(&close, SLOW, Ok(()));
            }
            // The service fails the call when the client half-closes: the half-close is done,
            // the close fails.
            Service::Error(message) => {
                let error = TransportCloseError {
                    message: format!("The Build Event Protocol upload failed: {message}"),
                    code: "BES_STREAM_NOT_RETRYING_FAILURE",
                };
                half_close.set(Ok(()));
                close.set(Err(error));
            }
        }
        let transport: Arc<dyn BuildEventTransport> = Arc::new(FakeBesTransport {
            close: close.clone(),
            half_close: half_close.clone(),
            timeout,
            mode,
        });
        PendingUpload::new(transport, None, close, half_close)
    }

    /// The first command: the module's `afterCommand` (closeBepTransports), with the error the
    /// dispatcher reports.
    fn after_build_command(upload: PendingUpload, events: &StoredEventHandler) {
        if let Err(e) = close_pending_uploads(vec![upload], &|_| {}) {
            events.handle(&Event::error(e.get_message().to_string()));
        }
    }

    /// The second command's `beforeCommand`.
    fn before_second_command(events: &StoredEventHandler) {
        MAX_WAIT_FOR_PREVIOUS_INVOCATION.with(|w| w.set(Some(Duration::from_millis(500))));
        wait_for_previous_invocation(events, /* is_shutdown= */ false);
    }

    fn messages(events: &StoredEventHandler, kind: EventKind) -> Vec<String> {
        events
            .get_events()
            .iter()
            .filter(|e| e.get_kind() == kind)
            .map(|e| e.get_message().to_string())
            .collect()
    }

    fn assert_no_warnings_or_errors(events: &StoredEventHandler) {
        assert!(
            messages(events, EventKind::Warning).is_empty(),
            "{:?}",
            events.get_events()
        );
        assert!(
            messages(events, EventKind::Error).is_empty(),
            "{:?}",
            events.get_events()
        );
    }

    fn assert_contains(events: &StoredEventHandler, kind: EventKind, text: &str) {
        assert!(
            messages(events, kind).iter().any(|m| m.contains(text)),
            "no {kind:?} containing {text:?} in {:?}",
            events.get_events()
        );
    }

    /// Runs one test case: `before` sets the scene, then the first command's afterCommand, then
    /// (when `second` is set) the second command's beforeCommand.
    fn run(
        service: Service,
        mode: BesUploadMode,
        timeout: Duration,
        second: bool,
    ) -> StoredEventHandler {
        let _serial = SERIAL.lock();
        reset_pending_uploads();
        let events = StoredEventHandler::new();
        after_build_command(transport(service, mode, timeout), &events);
        if second {
            before_second_command(&events);
        }
        cancel_and_reset_pending_uploads();
        events
    }

    const WAIT: BesUploadMode = BesUploadMode::WAIT_FOR_UPLOAD_COMPLETE;
    const NOWAIT: BesUploadMode = BesUploadMode::NOWAIT_FOR_UPLOAD_COMPLETE;
    const ASYNC: BesUploadMode = BesUploadMode::FULLY_ASYNC;
    const NO_TIMEOUT: Duration = Duration::ZERO;

    #[test]
    fn test_after_command_wait_for_upload_complete() {
        assert_no_warnings_or_errors(&run(Service::Fast, WAIT, NO_TIMEOUT, false));
    }

    #[test]
    #[ignore = "BuildEventServiceUploadCompleteEvent has no subscriber in Bazel and is not posted (bep.rs)"]
    fn test_after_command_wait_for_upload_complete_posts_event() {}

    #[test]
    fn test_after_command_wait_for_upload_complete_slow_full_close_error() {
        let events = run(Service::SlowFullClose, WAIT, BES_TIMEOUT, false);
        assert_contains(
            &events,
            EventKind::Error,
            "The Build Event Protocol upload timed out",
        );
    }

    #[test]
    fn test_after_command_wait_for_upload_complete_slow_half_close_error() {
        let events = run(Service::SlowHalfClose, WAIT, BES_TIMEOUT, false);
        assert_contains(
            &events,
            EventKind::Error,
            "The Build Event Protocol upload timed out",
        );
    }

    #[test]
    fn test_after_command_no_wait_for_upload_complete() {
        assert_no_warnings_or_errors(&run(Service::Fast, NOWAIT, NO_TIMEOUT, false));
    }

    #[test]
    fn test_after_command_no_wait_for_upload_complete_slow_full_close_ignored() {
        assert_no_warnings_or_errors(&run(Service::SlowFullClose, NOWAIT, NO_TIMEOUT, false));
    }

    #[test]
    fn test_after_command_no_wait_for_upload_complete_slow_half_close_ignored() {
        assert_no_warnings_or_errors(&run(Service::SlowHalfClose, NOWAIT, NO_TIMEOUT, false));
    }

    const FAILED_TO_COMPLETE: &str = "The background upload of the Build Event Protocol for the previous invocation failed to complete in";
    const NETWORK_TIMEOUT: &str = "The background upload of the Build Event Protocol for the previous invocation failed due to a network timeout";

    #[test]
    fn test_before_second_command_no_wait_for_upload_complete_slow_full_close_warning() {
        let events = run(Service::SlowFullClose, NOWAIT, NO_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, FAILED_TO_COMPLETE);
    }

    #[test]
    fn test_before_second_command_no_wait_for_upload_complete_slow_half_close_warning() {
        let events = run(Service::SlowHalfClose, NOWAIT, NO_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, FAILED_TO_COMPLETE);
    }

    #[test]
    fn test_before_second_command_no_wait_for_upload_complete_bes_timeout_slow_full_close_warning()
    {
        let events = run(Service::SlowFullClose, NOWAIT, BES_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, NETWORK_TIMEOUT);
    }

    #[test]
    fn test_before_second_command_no_wait_for_upload_bes_timeout_slow_half_close_warning() {
        let events = run(Service::SlowHalfClose, NOWAIT, BES_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, NETWORK_TIMEOUT);
    }

    /// Review 2026-11-26 BES-1 (not a Java test): the upload outlives `--bes_timeout` but
    /// finishes before the next command; `Futures.withTimeout` has failed for good, so the
    /// next command still reports the network timeout.
    #[test]
    fn before_second_command_no_wait_bes_timeout_late_completion_warning() {
        let _serial = SERIAL.lock();
        reset_pending_uploads();
        let events = StoredEventHandler::new();
        let close = CloseFuture::create();
        let half_close = CloseFuture::create();
        half_close.set(Ok(()));
        let transport: Arc<dyn BuildEventTransport> = Arc::new(FakeBesTransport {
            close: close.clone(),
            half_close: half_close.clone(),
            timeout: BES_TIMEOUT,
            mode: NOWAIT,
        });
        after_build_command(
            PendingUpload::new(transport, None, close.clone(), half_close),
            &events,
        );
        // The upload completes after the timeout, before the next command.
        std::thread::sleep(BES_TIMEOUT * 2);
        close.set(Ok(()));
        before_second_command(&events);
        cancel_and_reset_pending_uploads();
        assert_contains(&events, EventKind::Warning, NETWORK_TIMEOUT);
    }

    #[test]
    fn test_after_command_fully_async() {
        assert_no_warnings_or_errors(&run(Service::Fast, ASYNC, NO_TIMEOUT, false));
    }

    #[test]
    #[ignore = "needs a build with a file transport whose output stream closes late (BuildIntegrationTestCase); the mode logic is covered by the tests above"]
    fn test_after_command_build_event_file_wait_for_upload_complete() {}

    #[test]
    fn test_after_command_fully_async_slow_half_close_ignored() {
        assert_no_warnings_or_errors(&run(Service::SlowHalfClose, ASYNC, NO_TIMEOUT, false));
    }

    #[test]
    fn test_before_second_command_fully_async_slow_full_close_ignored() {
        assert_no_warnings_or_errors(&run(Service::SlowFullClose, ASYNC, NO_TIMEOUT, true));
    }

    #[test]
    #[ignore = "@Ignore(\"b/246912214\") in Java"]
    fn test_before_second_command_fully_async_slow_half_close_warning() {
        let events = run(Service::SlowHalfClose, ASYNC, NO_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, FAILED_TO_COMPLETE);
    }

    #[test]
    fn test_before_second_command_fully_async_bes_timeout_slow_full_close_ignored() {
        assert_no_warnings_or_errors(&run(Service::SlowFullClose, ASYNC, BES_TIMEOUT, true));
    }

    #[test]
    #[ignore = "@Ignore(\"b/246912214\") in Java"]
    fn test_before_second_command_fully_async_bes_timeout_slow_half_close_warning() {
        let events = run(Service::SlowHalfClose, ASYNC, BES_TIMEOUT, true);
        assert_contains(&events, EventKind::Warning, NETWORK_TIMEOUT);
    }

    #[test]
    fn test_after_command_wait_for_upload_complete_error_on_complete() {
        let events = run(Service::Error("Boom1"), WAIT, NO_TIMEOUT, false);
        assert_contains(
            &events,
            EventKind::Error,
            "The Build Event Protocol upload failed: Boom1",
        );
    }

    #[test]
    fn test_after_command_wait_for_upload_complete_bes_timeout_error_on_complete() {
        let events = run(Service::Error("Boom2"), WAIT, Duration::from_secs(5), false);
        assert_contains(
            &events,
            EventKind::Error,
            "The Build Event Protocol upload failed: Boom2",
        );
    }

    #[test]
    fn test_after_command_no_wait_for_upload_complete_error_on_complete() {
        assert_no_warnings_or_errors(&run(Service::Error("Boom3"), NOWAIT, NO_TIMEOUT, false));
    }

    #[test]
    fn test_before_second_command_no_wait_for_upload_complete_error_on_complete() {
        let events = run(Service::Error("Boom4"), NOWAIT, NO_TIMEOUT, true);
        assert_contains(
            &events,
            EventKind::Warning,
            "The Build Event Protocol upload failed: Boom4",
        );
    }

    #[test]
    fn test_after_command_fully_async_error_on_complete() {
        assert_no_warnings_or_errors(&run(Service::Error("Boom5"), ASYNC, NO_TIMEOUT, false));
    }

    #[test]
    fn test_before_second_command_fully_async_error_on_complete() {
        // The second command waits for the half-close, which is done before the failing service
        // fails the stream (as `Service::Error` has it; kept as Java writes the case).
        let _serial = SERIAL.lock();
        reset_pending_uploads();
        let events = StoredEventHandler::new();
        let close = CloseFuture::create();
        let half_close = CloseFuture::create();
        half_close.set(Ok(()));
        close.set(Err(TransportCloseError {
            message: "The Build Event Protocol upload failed: Boom6".to_string(),
            code: "BES_STREAM_NOT_RETRYING_FAILURE",
        }));
        let transport: Arc<dyn BuildEventTransport> = Arc::new(FakeBesTransport {
            close: close.clone(),
            half_close: half_close.clone(),
            timeout: NO_TIMEOUT,
            mode: ASYNC,
        });
        after_build_command(
            PendingUpload::new(transport, None, close, half_close),
            &events,
        );
        before_second_command(&events);
        cancel_and_reset_pending_uploads();
        assert_no_warnings_or_errors(&events);
    }

    // The tests below run builds with BuildIntegrationTestCase (transport creation from the
    // options, the results URL lines, retries against a server, OOM crashes, flagsets); Blaze
    // has no in-process build harness. The parts they check are covered as noted.

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase); transport creation: blaze_bep's file transport tests and the oracle comparison of BES runs"]
    fn test_creates_streamer_for_text_format_file_transport() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_creates_streamer_for_binary_format_file_transport() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_creates_streamer_for_json_format_file_transport() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_creates_streamer_for_bes_transport() {}

    #[test]
    #[ignore = "needs an in-process build; the retry limit and its message: blaze_bes transport tests and the oracle comparison (unreachable backend)"]
    fn test_retry_count() {}

    #[test]
    #[ignore = "ConnectivityStatusProvider is not ported: Bazel's status is always OK"]
    fn test_connectivity_failure_disables_bes_streaming() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_creates_streamer_for_grpc_bes_results_url() {}

    #[test]
    #[ignore = "needs an in-process build; the check itself is blaze_bes::module::create_bes_transport"]
    fn test_creates_streamer_for_grpc_runs_per_test_too_high_disables_streaming() {}

    #[test]
    #[ignore = "needs an in-process build; the line is compared with the oracle (BES runs)"]
    fn test_before_command_grpc_reports_bes_results_url() {}

    #[test]
    #[ignore = "needs an in-process build; the line is compared with the oracle (BES runs)"]
    fn test_after_command_grpc_reports_bes_results_url() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_after_command_streamer_is_closed_no_warning() {}

    #[test]
    #[ignore = "needs an in-process build; retries and codes: blaze_bes transport tests"]
    fn test_after_command_wait_for_upload_complete_retryable_error_early_in_stream() {}

    #[test]
    #[ignore = "needs an in-process build; retries and codes: blaze_bes transport tests"]
    fn test_after_command_wait_for_upload_complete_permission_denied_error_early_in_stream() {}

    #[test]
    #[ignore = "needs an in-process build; retries and codes: blaze_bes transport tests"]
    fn test_after_command_wait_for_upload_complete_invalid_argument_error_early_in_stream() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_creates_streamer_for_all_transports() {}

    #[test]
    #[ignore = "needs an in-process build; the supplier is blaze_bep::bep_module's ThrowingBuildEventArtifactUploaderSupplier"]
    fn test_uploader_sharing() {}

    #[test]
    #[ignore = "needs an in-process build (BuildIntegrationTestCase)"]
    fn test_does_not_creates_streamer_without_transports() {}

    #[test]
    fn test_keywords() {
        let bes_options = BuildEventServiceOptions {
            bes_keywords: vec!["keyword0".into(), "keyword1".into(), "keyword0".into()],
            bes_system_keywords: vec![
                "sys_keyword0".into(),
                "sys_keyword1".into(),
                "sys_keyword0".into(),
            ],
            ..BuildEventServiceOptions::default()
        };
        let keywords: Vec<String> = blaze_bes::module::get_bes_keywords("build", &bes_options)
            .into_iter()
            .collect();
        // containsExactly (any order).
        let mut sorted = keywords.clone();
        sorted.sort();
        let mut expected = vec![
            "protocol_name=BEP",
            "command_name=build",
            "user_keyword=keyword0",
            "user_keyword=keyword1",
            "sys_keyword0",
            "sys_keyword1",
        ];
        expected.sort();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn test_make_grpc_metadata() {
        let bes_options = BuildEventServiceOptions {
            bes_backend: "bes-backend".into(),
            bes_proxy: Some("bes-proxy".into()),
            bes_headers: vec![
                ("key1".into(), "val1".into()),
                ("key2".into(), "val2".into()),
                ("key3".into(), "val3".into()),
                ("key1".into(), "val4".into()),
            ],
            ..BuildEventServiceOptions::default()
        };
        let config =
            blaze_bes::module::BackendConfig::create(&bes_options, &AuthAndTLSOptions::default());
        let metadata = blaze_bes::module::make_grpc_metadata(&config);
        let get = |k: &str| {
            metadata
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("key1"), Some("val4"));
        assert_eq!(get("key2"), Some("val2"));
        assert_eq!(get("key3"), Some("val3"));
    }

    #[test]
    #[ignore = "OOM handling (GcThrashingDetector, BugReport.handleCrash) is JVM-only"]
    fn oom_first_reported_via_handle_crash() {}

    #[test]
    #[ignore = "OOM handling (OutOfMemoryError thrown from Skyframe) is JVM-only"]
    fn oom_first_thrown_from_skyframe() {}

    #[test]
    #[ignore = "OOM handling (OutOfMemoryError thrown from Skyframe) is JVM-only"]
    fn oom_bes_closes_after_special_case_timeout_thrown_from_skyframe() {}

    #[test]
    #[ignore = "needs an in-process build with flagsets (BuildIntegrationTestCase)"]
    fn command_line_events_includes_flags_from_flagsets() {}
}
