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
use std::sync::atomic::AtomicI32;
use std::time::Instant;

use blaze_bep::bep_module::{
    BepCommandContext, BepStreamer, BuildEventPost, BuildEventStreamerBuilder,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::{
    BuildEvent, BuildToolLogs, LocalFile, LocalFileCompression, LocalFileType, LogFileEntry,
    TransportCloseError,
};
use blaze_bep::events::actions_events::{
    ActionExecutedEvent, ActionFailure, ErrorTiming, ExecutedActionInfo, ExecutedActionOwner,
};
use blaze_bep::events::analysis_events::{
    BuildInfoEvent, ConfiguredTargetInfo, PackagesTestSize, TargetConfiguredEvent,
};
use blaze_bep::events::buildtool_events::{
    BuildResultData, BuildStartingData, BuildStartingEvent, ConvenienceSymlinksIdentifiedEvent,
    build_complete_event, testing_complete_event,
};
use blaze_bep::events::completion_context::{CompletionContext, ImportantInputMap};
use blaze_bep::events::pkgcache_events::{TargetParsingCompleteEvent, ThinTarget};
use blaze_bep::events::target_complete_event::{
    ArtifactsInOutputGroup, Cause, CompletedTargetData, TargetCompleteEvent,
};
use blaze_bep::metrics::metrics_collector::{
    AnalysisPhaseCompleteInfo, BuildMetricsFinishInputs, JvmMemoryFacts, MetricsCollector,
    RemoteAnalysisCachingFacts, SkyframeStats, TotalAndConfiguredTargetOnlyMetric,
};
use blaze_bep::runtime::ReleaseReplaceableBuildEvent;
use blaze_bep::runtime::command_line_event::{
    CanonicalCommandLineEvent, OriginalCommandLineEvent, OriginalUnstructuredCommandLineEvent,
    ToolCommandLineEvent,
};
use blaze_bep::runtime::got_options_event::GotOptionsEvent;
use blaze_bep::runtime::runtime_events::{BuildMetadataEvent, ExecRootEvent};
use blaze_bep::runtime::test_summary_event::TestSummaryEvent;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_buildtool::buildtool::build_result::{self as tool_build_result, BuildResult};
use blaze_buildtool::buildtool::execution_tool::ExecutionEventCollector;
use blaze_buildtool::buildtool::output_directory_links_utils::SymlinkCreationResult;
use blaze_buildtool::runtime::test_command_support::{
    NoTestsFound, TestingCompleteEvent as ToolTestingCompleteEvent,
};
use blaze_buildtool::runtime::test_summary::{TestSummary, get_test_params};
use blaze_cmdline::Label;
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
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_optdefs::misc::metrics_module::MetricsModuleOptions;
use blaze_optdefs::misc::package_metrics_module::PackageMetricsModuleOptions;
use blaze_packages::attribute::{AttributeMap, RawAttributeMapper};
use blaze_packages::target::Target;
use blaze_packages::target::target_utils;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::build_event_stream::aborted::AbortReason;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_shell::util::shell_escaper::ShellEscaper;
use blaze_sky_exec::events::{
    ActionCompletionEvent, ActionExecutedEvent as SkyActionExecutedEvent,
    ActionResultReceivedEvent, CompletionRootCause, ErrorTiming as SkyErrorTiming,
    TargetCompleteEvent as SkyTargetCompleteEvent, TopLevelTargetPendingExecutionEvent,
    downcast_event,
};
use blaze_sky_top::build_tool::BuildRequest;
use blaze_sky_top::build_view::{AnalysisResult, construct_label_to_target_map};
use blaze_sky_top::execution_seam::configured_target_views;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
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
        if downcast_event::<TopLevelTargetPendingExecutionEvent>(&event).is_some() {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:213 MetricsCollector.handleExecutionPhaseStart
            shared.metrics.handle_execution_phase_start();
            return;
        }
        // The build events whose Rust type is not the blaze_bep one.
        if let Some(e) = downcast_event::<SkyTargetCompleteEvent>(&event) {
            shared
                .streamer
                .build_event(Arc::new(convert_target_complete(shared, e)));
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
        let root_causes: Vec<Cause> = e
            .root_cause_details
            .iter()
            .map(|cause| match cause {
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
            })
            .collect();
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
    /// `BuildTool.shouldStopOnFailure(request)`.
    stop_on_first_failure: bool,
    publish_package_metrics_in_bep: bool,
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
        std::ffi::CStr::from_ptr(pwd.pw_name)
            .to_string_lossy()
            .into_owned()
    }
}

/// `OptionsUtils.asShellEscapedString(options)`.
// Java: com/google/devtools/build/lib/util/OptionsUtils.java:54 OptionsUtils.asShellEscapedString
fn as_shell_escaped_string(options: &OptionsParsingResult) -> String {
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
    /// registration on the event bus) and `MetricsModule.beforeCommand`; then the events Java
    /// posts before the command runs: `CommandEnvironment.beforeCommand`'s `BuildMetadataEvent`
    /// and `GotOptionsEvent`, and `BlazeCommandDispatcher.execExclusively`'s command-line
    /// events. Returns `None` when no `--build_event_*_file` is given.
    ///
    /// `options` is the command's final options (`execExclusively`'s `options`, after the
    /// "module resolution" `--flag_alias` options); `env.get_options()` is the first parse, which
    /// the modules and `GotOptionsEvent` see.
    ///
    /// PORT: Java runs these in the dispatcher before `command.exec`; the build command calls
    /// this first thing in `exec`. The stream's order does not depend on it: every event posted
    /// here waits for `BuildStartingEvent` (`postedAfter`).
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:337 BuildEventServiceModule.beforeCommand
    pub fn before_command(
        env: &CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> Option<Arc<BepCommandSupport>> {
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
            Some(workspace) => std::path::PathBuf::from(workspace.get_path_string()),
            None => blaze_util::unported!(
                "InstrumentationOutputFactory.createInstrumentationOutput (no workspace: home directory)"
            ),
        };
        let (streamer, errors) = BuildEventStreamerBuilder::from_options(
            &bep_options,
            &bes_stream_options,
            &BepCommandContext {
                workspace_or_home,
                oom_message: common_options.oom_message.clone(),
            },
        );
        if let Some(error) = errors.first() {
            // Java reports the IOException's message and exits the command through the module
            // environment (BES_LOCAL_WRITE_ERROR).
            blaze_util::unported!("BuildEventServiceModule.reportError", "{}", error);
        }
        let streamer = streamer?;

        if bep_options.publish_target_summary {
            blaze_util::unported!("TargetSummaryPublisher");
        }

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
            expand_filesets: bep_options.expand_filesets,
            publish_all_actions: bes_stream_options.publish_all_actions,
            clock,
        });
        // cmdEnv.getEventBus().register(streamer) (and the MetricsCollector's registration).
        env.get_event_bus().register(Arc::new(BepSubscriber {
            shared: shared.clone(),
        }));
        // PORT: registerOutAndErrOutputStreams (the module's OutErr, teed by the dispatcher into
        // the command's output) is not ported: progress events carry no stdout/stderr.

        let keep_going = options
            .get_options::<KeepGoingOption>()
            .map(|o| o.keep_going)
            .unwrap_or(false);
        let test_keep_going = options
            .get_options::<ExecutionOptions>()
            .map(|o| o.test_keep_going)
            .unwrap_or_else(|| panic!("NullPointerException: ExecutionOptions"));
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
            stop_on_first_failure: !(keep_going && test_keep_going),
            publish_package_metrics_in_bep,
        });

        support.post_before_command_events(env);
        support.post_command_line_events(env, options);
        Some(support)
    }

    fn post(&self, event: Arc<dyn Postable>) {
        self.event_bus.post(event);
    }

    /// `CommandEnvironment.beforeCommand`: the build metadata and the parsed options.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:919 CommandEnvironment.beforeCommand
    fn post_before_command_events(&self, env: &CommandEnvironment) {
        let common_options: CommonCommandOptions = env
            .get_options()
            .get_options()
            .unwrap_or_else(|| panic!("NullPointerException: CommonCommandOptions"));
        self.post(Arc::new(BuildMetadataEvent::new(
            make_map_from_map_entries(&common_options.build_metadata),
        )));
        // PORT: invocation policies are not ported (a non-empty one fails loudly before the
        // command runs), so the policy is the empty one.
        self.post(Arc::new(GotOptionsEvent::new(
            env.get_runtime().get_startup_options_provider_arc(),
            env.get_options().clone(),
            InvocationPolicy::default(),
        )));
    }

    /// The command-line events of `BlazeCommandDispatcher.execExclusively`.
    // Java: com/google/devtools/build/lib/runtime/BlazeCommandDispatcher.java:779 BlazeCommandDispatcher.execExclusively
    fn post_command_line_events(&self, env: &CommandEnvironment, options: &OptionsParsingResult) {
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
        // PORT: CriticalPathComputer is not ported, so the "critical path" value
        // (AggregatedCriticalPath.toString()) is not added; the profiler is not ported, so the
        // profile file ("command.profile.gz") is not published either.
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
                        || self.stop_on_first_failure,
                },
                vec![
                    build_event_id_util::build_tool_logs(),
                    build_event_id_util::build_metrics(),
                ],
            ),
        ))));
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

        // PORT: the Build Event Service (--bes_backend) is not ported; only the local files.
        self.reporter.handle(&Event::info(
            "Build Event Protocol files produced successfully.",
        ));
        Ok(())
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
