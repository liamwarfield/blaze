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
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Instant;

use blaze_bep::bep_module::{
    BepCommandContext, BepStreamer, BuildEventPost, BuildEventStreamerBuilder,
};
use blaze_bep::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, LocalFilesArtifactUploader, PathConverterFuture,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::{
    BuildEvent, BuildToolLogs, LocalFile, LocalFileCompression, LocalFileType, LogFileEntry,
    TransportCloseError,
};
use blaze_bep::events::actions_events::{
    ActionExecutedEvent, ActionFailure, ErrorTiming, ExecutedActionInfo, ExecutedActionOwner,
};
use blaze_bep::events::analysis_events::AnalysisFailureBuildEvent;
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
    RemoteAnalysisCachingFacts, SkyframeStats, TotalAndConfiguredTargetOnlyMetric,
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
use blaze_cmdline::Label;
use blaze_events::OutErr;
use blaze_events::{Clock, Event, EventBus, EventHandler, JavaClock, Postable, Reporter};
use blaze_exec::actions::cache::PostableActionCacheStats;
use blaze_exec::analysis::actions::bazel_workspace_status_module::BuildInfoEvent as ExecBuildInfoEvent;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_optdefs::command::blaze_server_startup_options::BlazeServerStartupOptions;
use blaze_optdefs::command::build_request_options::{BuildRequestOptions, ConvenienceSymlinksMode};
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_optdefs::misc::metrics_module::MetricsModuleOptions;
use blaze_optdefs::misc::package_metrics_module::PackageMetricsModuleOptions;
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::build_event_stream::aborted::AbortReason;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_shell::util::shell_escaper::ShellEscaper;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionExecutedEvent as SkyActionExecutedEvent,
    ActionResultReceivedEvent, AspectCompleteEvent as SkyAspectCompleteEvent, CompletionRootCause,
    ErrorTiming as SkyErrorTiming, TargetCompleteEvent as SkyTargetCompleteEvent,
    TopLevelTargetPendingExecutionEvent, downcast_event,
};
use blaze_sky_top::analysis_failure_event::{AnalysisFailureEvent, AnalysisRootCause};
use blaze_sky_top::build_tool::BuildRequest;
use blaze_sky_top::build_view::{AnalysisResult, construct_label_to_target_map};
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
}

/// The subscriber of the command's event bus: the `@Subscribe` methods of the
/// `BuildEventStreamer` and of the `MetricsCollector` for the events other crates post.
pub struct BepSubscriber {
    shared: Arc<Shared>,
}

impl EventBus for BepSubscriber {
    fn post(&self, event: Arc<dyn Postable>) {
        let shared = &self.shared;
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
        if let Some(e) = downcast_event::<PatternExpandingError>(&event) {
            shared
                .streamer
                .build_event(Arc::new(PatternExpandingErrorEvent::new(
                    e.get_pattern().to_vec(),
                    e.get_message().to_string(),
                    e.is_skipped(),
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
                    configuration.map(|c| configuration_event(c)),
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
/// `--build_event_publish_all_actions` (Java `BuildEventStreamer.shouldIgnoreBuildEvent`);
/// the execution phase does not record the primary output's metadata of those, so they are
/// dropped here and fail loudly when they would be published.
// Java: com/google/devtools/build/lib/actions/ActionExecutedEvent.java:63 ActionExecutedEvent.<init>
fn convert_action_executed(
    shared: &Shared,
    e: &SkyActionExecutedEvent,
) -> Option<ActionExecutedEvent> {
    let Some(exception) = &e.exception else {
        if shared.publish_all_actions {
            blaze_util::unported!(
                "ActionExecutedEvent (successful actions with --build_event_publish_all_actions: primary output metadata)"
            );
        }
        return None;
    };
    let failure_detail = exception.get_failure_detail().cloned();
    let numeric_exit_code = match &failure_detail {
        Some(failure_detail) => blaze_util::DetailedExitCode::get_numeric_exit_code(failure_detail),
        None => blaze_util::unported!("ActionExecutionException.getExitCode (no failure detail)"),
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
        // PORT: Blaze has no extra actions (`ExtraAction` is not ported).
        is_extra_action: false,
    };
    Some(ActionExecutedEvent::new(
        e.action_id.clone(),
        action,
        Some(ActionFailure {
            numeric_exit_code,
            failure_detail,
        }),
        e.primary_output_path.clone(),
        e.output_artifact.clone(),
        None,
        e.stdout.clone(),
        e.stderr.clone(),
        match e.error_timing {
            SkyErrorTiming::NO_ERROR => ErrorTiming::NoError,
            SkyErrorTiming::BEFORE_EXECUTION => ErrorTiming::BeforeExecution,
            SkyErrorTiming::AFTER_EXECUTION => ErrorTiming::AfterExecution,
        },
        // PORT: the execution phase does not record the action's start and end times on the
        // event (wall-clock values).
        None,
        None,
    ))
}

/// What the UI reads to draw its progress bar after the build completed.
struct UiFacts {
    /// `UiOptions.showProgress`.
    show_progress: bool,
    /// `UiEventHandler.quiet` (`--quiet`).
    quiet: bool,
    /// `UiOptions.useCursorControl()`.
    cursor_control: bool,
    /// `UiOptions.showTimestamp`.
    show_timestamp: bool,
}

/// The BEP of one `build` / `test` command.
pub struct BepCommandSupport {
    shared: Arc<Shared>,
    event_bus: Arc<CommandEventBus>,
    reporter: Arc<Reporter>,
    exec_root: Path,
    ui: UiFacts,
    /// `BuildTool.shouldStopOnFailure(request)` (`None` for a command without `ExecutionOptions`).
    stop_on_first_failure: Option<bool>,
    publish_package_metrics_in_bep: bool,
    /// The uploader factory map of the file transports with path conversion.
    uploader_factory_map: Arc<DeferredUploaderFactory>,
    /// `getOutputListener()`: the command's stdout and stderr for the progress events.
    output_listener: OutErr,
    /// Whether `CommandPrecompleteEvent` was handled (see [`BepCommandSupport::command_precomplete`]).
    precompleted: AtomicBool,
    /// Whether afterCommand ran (see [`BepCommandSupport::after_command`]).
    after_command_done: AtomicBool,
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
        // Exit early if the running command isn't supported.
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:383 BuildEventServiceModule.beforeCommand
        if !ALLOWED_COMMANDS.contains(&env.get_command_name()) {
            return None;
        }
        let uploader_factory_map = DeferredUploaderFactory::new(
            crate::commands::build::build_remote::build_event_artifact_uploader_factory_map(None),
        );
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
            Some(workspace) => std::path::PathBuf::from(
                blaze_util::string_encoding::internal_to_platform(workspace.get_path_string())
                    .into_owned(),
            ),
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
        let (streamer, errors) = BuildEventStreamerBuilder::from_options_with_uploader_factory(
            &bep_options,
            &bes_stream_options,
            &BepCommandContext {
                workspace_or_home,
                oom_message: common_options.oom_message.clone(),
            },
            uploader_factory,
        );
        if let Some(error) = errors.first() {
            // Java reports the IOException's message and exits the command through the module
            // environment (BES_LOCAL_WRITE_ERROR).
            blaze_util::unported!("BuildEventServiceModule.reportError", "{}", error);
        }
        let streamer = streamer?;

        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:417 BuildEventServiceModule.beforeCommand
        let target_summary_publisher = bep_options.publish_target_summary.then(|| {
            Arc::new(TargetSummaryPublisher::new(Arc::new(StreamerSink(
                streamer.streamer().clone(),
            ))))
        });

        // MetricsModule.beforeCommand: MetricsCollector.installInEnv(env, numAnalyses,
        // numBuilds).
        // PORT: the module's counters live as long as the server; Blaze runs one command per
        // server process.
        let clock: Arc<dyn Clock> = Arc::new(JavaClock);
        let metrics_options: Option<MetricsModuleOptions> = parsing_result.get_options();
        let metrics = Arc::new(MetricsCollector::install_in_env(
            metrics_options.as_ref(),
            clock.clone(),
            Arc::new(AtomicI32::new(0)),
            Arc::new(AtomicI32::new(0)),
        ));
        let publish_package_metrics_in_bep = parsing_result
            .get_options::<PackageMetricsModuleOptions>()
            .is_some_and(|o| o.publish_package_metrics_in_bep);

        let shared = Arc::new(Shared {
            streamer,
            metrics,
            targets: Mutex::new(IndexMap::new()),
            announce_target_summary: bep_options.publish_target_summary,
            target_summary_publisher,
            expand_filesets: bep_options.expand_filesets,
            publish_all_actions: bes_stream_options.publish_all_actions,
            clock,
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
        let quiet = env
            .get_runtime()
            .get_startup_options_provider()
            .get_options::<BlazeServerStartupOptions>()
            .is_some_and(|o| o.quiet);
        let support = Arc::new(BepCommandSupport {
            shared,
            event_bus: env.get_event_bus().clone(),
            reporter: env.get_reporter().clone(),
            exec_root: env.get_exec_root(),
            ui: UiFacts {
                show_progress: ui_options.show_progress,
                quiet,
                cursor_control: ui_options.use_cursor_control(),
                show_timestamp: ui_options.show_timestamp,
            },
            // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:1127 BuildTool.shouldStopOnFailure
            stop_on_first_failure: test_keep_going
                .map(|test_keep_going| !(keep_going && test_keep_going)),
            publish_package_metrics_in_bep,
            uploader_factory_map,
            output_listener,
            precompleted: AtomicBool::new(false),
            after_command_done: AtomicBool::new(false),
        });
        Some(support)
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

    /// The BEP events of loading and analysis, posted once the analysis result is known:
    /// `TargetParsingCompleteEvent` (`TargetPatternPhaseFunction`), `ExecRootEvent` and the
    /// release of the tentative canonical command line (`BuildTool.buildTargets`), the analysis
    /// metrics (`AnalysisPhaseRunner`), the top-level configuration (`BuildView.update`) and a
    /// `TargetConfiguredEvent` per top-level target (`SkyframeExecutor.configureTargets`).
    ///
    /// PORT: Java posts these while loading and analysis run; Blaze's `BuildTool` hands the
    /// loading and analysis results to the execution phase, which calls this first. The
    /// streamer orders the events by their announcements, not by posting time. When analysis
    /// fails, the execution phase is not reached and these events are not posted.
    pub fn analysis_complete(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        loading_result: &TargetPatternPhaseValue,
        analysis_result: &AnalysisResult,
    ) {
        self.post_target_parsing_complete(request, skyframe_executor, loading_result);

        // Java: com/google/devtools/build/lib/buildtool/BuildTool.java:313 BuildTool.buildTargets
        self.post(Arc::new(ExecRootEvent::new(self.exec_root.clone())));
        // No PROJECT.scl flag updates. Release the original CanonicalCommandLineEvent for
        // posting.
        // PORT: PROJECT.scl evaluation is not ported, so there is never a replacement.
        self.post(Arc::new(ReleaseReplaceableBuildEvent::new(
            build_event_id_util::structured_commandline_id(CanonicalCommandLineEvent::LABEL),
        )));

        self.analysis_metrics(analysis_result);

        // Java: com/google/devtools/build/lib/analysis/BuildView.java:350 BuildView.update
        self.post(Arc::new(BuildEventPost(configuration_event(
            analysis_result.get_configuration(),
        ))));

        self.post_targets_configured(skyframe_executor, analysis_result);
    }

    /// `TargetPatternPhaseFunction.compute`: the `TargetParsingCompleteEvent`.
    // Java: com/google/devtools/build/lib/skyframe/TargetPatternPhaseFunction.java:76 TargetPatternPhaseFunction.compute
    fn post_target_parsing_complete(
        &self,
        request: &BuildRequest,
        skyframe_executor: &SkyframeExecutor,
        loading_result: &TargetPatternPhaseValue,
    ) {
        if loading_result.has_error() {
            // The failed patterns (keep-going pattern errors) are not part of the value.
            blaze_util::unported!("TargetParsingCompleteEvent.failedTargetPatterns");
        }
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
            Vec::new(),
            Vec::new(),
            test_suite_expansions,
        )));
    }

    /// `AnalysisPhaseStartedEvent`, `AnalysisPhaseCompleteEvent` and `AnalysisGraphStatsEvent`
    /// as the `MetricsCollector` receives them.
    // Java: com/google/devtools/build/lib/analysis/BuildView.java:253 BuildView.update
    // Java: com/google/devtools/build/lib/buildtool/AnalysisPhaseRunner.java:431 AnalysisPhaseRunner.execute
    fn analysis_metrics(&self, analysis_result: &AnalysisResult) {
        let metrics = &self.shared.metrics;
        metrics.log_analysis_starting_event();
        let stats = analysis_result.get_analysis_phase_complete_stats();
        if self.publish_package_metrics_in_bep {
            blaze_util::unported!("PackageMetricsPackageLoadingListener.getPackageMetricsRecorder");
        }
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
            package_metrics_recorder: None,
        });
        // AnalysisGraphStatsEvent: posted only when the analysis checked for action conflicts.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeBuildView.java:454 SkyframeBuildView.configureTargets
        if let Some(build_graph_metrics) = analysis_result.get_build_graph_metrics() {
            metrics.log_analysis_graph_stats(build_graph_metrics);
        }
    }

    /// `SkyframeExecutor.configureTargets`: a `TargetConfiguredEvent` per configured top-level
    /// target; also records what the target's `TargetCompleteEvent` reads of it.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:2317 SkyframeExecutor.configureTargets
    fn post_targets_configured(
        &self,
        skyframe_executor: &SkyframeExecutor,
        analysis_result: &AnalysisResult,
    ) {
        let mut by_label: IndexMap<Label, (Target, Option<Arc<BuildConfigurationValue>>)> =
            IndexMap::new();
        for target_and_configuration in analysis_result.get_top_level_targets_with_configs() {
            let target = target_and_configuration.get_target();
            let configuration = target_and_configuration.get_configuration().cloned();
            let checksum = configuration.as_ref().map(|c| c.checksum().to_string());
            self.post(Arc::new(TargetConfiguredEvent::new(
                configured_target_info(target),
                configuration.as_deref().map(configuration_event),
                checksum,
            )));
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
                    configuration_event: configuration.as_deref().map(configuration_event),
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
    ) {
        // Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:154 BuildSummaryStatsModule.buildComplete
        let elapsed_seconds = build_result.get_elapsed_seconds();
        build_result
            .get_build_tool_log_collection()
            .add_direct_value("elapsed time", format!("{elapsed_seconds:.6}").as_bytes());
        // criticalPath.toString(), when executorInit registered a CriticalPathComputer.
        if let Some(critical_path) = collector.critical_path_tool_log() {
            build_result
                .get_build_tool_log_collection()
                .add_direct_value("critical path", critical_path.as_bytes());
        }
        // PORT: the profiler does not run, so there is no profile file ("command.profile.gz",
        // ProfilerStartedEvent.getProfile().publish) to list.
        let spawn_summary_string =
            SpawnStats::convert_summary_to_string(&collector.spawn_stats.get_summary());
        build_result
            .get_build_tool_log_collection()
            .add_direct_value("process stats", spawn_summary_string.as_bytes());

        // UiEventHandler.buildComplete: the progress bar stays while there are activities (the
        // open BEP transports).
        self.write_build_complete_progress_bar(build_result.get_success());

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

    /// `UiEventHandler.buildComplete` → `refresh()` → `addProgressBar()` with
    /// `SkymeldUiStateTracker.writeProgressBar` in the `BUILD_COMPLETED` state: while the BEP
    /// transports announced by `AnnounceBuildEventTransportsEvent` are open
    /// (`UiStateTracker.hasActivities()`), the build-complete progress bar (`INFO: ` or
    /// `FAILED: ` and the empty additional message) is written to the terminal.
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:715 UiEventHandler.buildComplete
    // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:1139 UiEventHandler.addProgressBar
    // Java: com/google/devtools/build/lib/runtime/SkymeldUiStateTracker.java:72 SkymeldUiStateTracker.writeProgressBar
    fn write_build_complete_progress_bar(&self, ok: bool) {
        if !self.ui.show_progress || self.ui.quiet {
            return;
        }
        if self.ui.cursor_control {
            blaze_util::unported!(
                "UiEventHandler.addProgressBar (cursor control: LineWrappingAnsiTerminalWriter, the full progress bar)"
            );
        }
        // PORT: the UI's terminal is the command's stderr; the reporter's stderr reaches it (and
        // its --color=no stripping) as STDERR events.
        let terminal =
            blaze_events::AnsiTerminal::new(self.reporter.get_out_err().get_error_stream().clone());
        // PORT: write errors are logged and ignored in Java ("IO Error writing to output
        // stream"); logging is not ported.
        let _ = self.write_build_complete_progress(&terminal, ok);
    }

    /// `addProgressBar`'s writes: the timestamp, `writeBaseProgress(ok ? "INFO" : "FAILED",
    /// additionalMessage)` and the newline.
    // Java: com/google/devtools/build/lib/runtime/SkymeldUiStateTracker.java:123 SkymeldUiStateTracker.writeBaseProgress
    fn write_build_complete_progress(
        &self,
        terminal: &blaze_events::AnsiTerminal,
        ok: bool,
    ) -> std::io::Result<()> {
        if self.ui.show_timestamp {
            terminal.write_string(&blaze_events::ui::format_timestamp(
                self.shared.clock.current_time_millis(),
            ))?;
        }
        if ok {
            // LineCountingAnsiTerminalWriter.okStatus
            terminal.text_green()?;
        } else {
            // LineCountingAnsiTerminalWriter.failStatus
            terminal.text_red()?;
            terminal.text_bold()?;
        }
        terminal.write_string(if ok { "INFO" } else { "FAILED" })?;
        terminal.write_string(":")?;
        // normal(); UiStateTracker.buildComplete reset the additional message to "".
        terminal.reset_terminal()?;
        terminal.write_string(" ")?;
        // terminalWriter.newline(): System.lineSeparator().
        terminal.write_string("\n")?;
        terminal.flush()
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
        let event = self
            .shared
            .metrics
            .on_command_precomplete_event(BuildMetricsFinishInputs {
                // PORT: Blaze has no persistent workers.
                worker_process_metrics: Vec::new(),
                has_pending_exception: false,
                // PORT: getSkyframeStats is not ported; it is only asked for with
                // --experimental_record_skyframe_metrics.
                skyframe_stats: &|| -> Option<SkyframeStats> {
                    blaze_util::unported!("SkyframeExecutor.getSkyframeStats")
                },
                // PORT: remote analysis caching is not ported.
                remote_analysis_caching: RemoteAnalysisCachingFacts::default(),
                // PORT: Blaze is not a JVM (no heap or GC measurements).
                memory: JvmMemoryFacts::default(),
                // PORT: the profiler and the network metrics collector are not ported.
                profile_elapsed_time_ms: None,
                server_process_cpu_time_ms: None,
                network_metrics: None,
            });
        self.post(Arc::new(event));
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
            // forceShutdownBuildEventStreamer(AbortReason.INTERNAL)
            streamer.close_on_abort(AbortReason::Internal);
        }

        // closeBepTransports(): the file transports close synchronously (PORT: the close is
        // blocking in Blaze, so the futures are the close results).
        for (_name, result) in streamer.streamer().get_close_futures_map() {
            result.map_err(|e| transport_close_exception(&e))?;
        }

        // PORT: the Build Event Service upload is not ported (--bes_backend fails loudly before
        // the command runs, crate::unported_options); only the local files are produced.
        self.reporter.handle(&Event::info(
            "Build Event Protocol files produced successfully.",
        ));
        Ok(())
    }
}

/// `BuildEventServiceModule.blazeShutdownOnCrash` (`forceShutdownBuildEventStreamer`): a crash
/// that unwinds out of the command closes the streamer with `AbortReason.INTERNAL`.
///
/// PORT: Java closes the streamer from `BugReport.handleCrash` (via
/// `BlazeRuntime.cleanUpForCrash`) after printing the crash; Blaze's streamer belongs to the
/// command, so it is closed while the panic unwinds out of the command, before the dispatcher
/// prints the crash. The transports close synchronously (see `after_command`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:500 BuildEventServiceModule.blazeShutdownOnCrash
impl Drop for BepCommandSupport {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.shared.streamer.is_closed() {
            // Nothing may panic here (a panic while unwinding aborts the process).
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // forceShutdownBuildEventStreamer(AbortReason.INTERNAL)
                self.shared.streamer.close_on_abort(AbortReason::Internal);
            }));
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
    let result = exec_internal(env);
    // PORT: skipped, profiling: Profiler.markPhase(ProfilePhase.FINISH).
    if let Some(bep) = env.get_bep() {
        bep.post_no_build_request_finished_event(
            blaze_util::ExitCode::for_code(result.get_exit_code().get_numeric_exit_code())
                .unwrap_or_else(|| panic!("unknown exit code")),
            JavaClock.current_time_millis(),
        );
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
