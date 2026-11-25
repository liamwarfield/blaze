//! Port of `src/test/java/com/google/devtools/build/lib/metrics/MetricsCollectorTest.java`.
//!
//! Most of the Java tests are `BuildIntegrationTestCase`s that run whole builds and read the
//! posted `BuildMetricsEvent`; they are kept as ignored tests. The collector itself is tested by
//! feeding it the events of the brotli oracle build and comparing the produced event with the
//! oracle's `buildMetrics` event.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use blaze_bep::buildeventstream::build_event::BuildEvent;
use blaze_bep::buildeventstream::build_event_context::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEventContext, NoConversion, PathConverter,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::generic_build_event::LastBuildEvent;
use blaze_bep::metrics::metrics_collector::{
    AnalysisPhaseCompleteInfo, BuildMetricsEvent, BuildMetricsFinishInputs, ExecutionFinishedInfo,
    JvmMemoryFacts, MetricsCollector, SkyframeGraphStatsInfo, TotalAndConfiguredTargetOnlyMetric,
    WorkerProcessMetricsInfo, WorkerProcessStatusKind,
};
use blaze_events::Clock;
use blaze_exec::actions::cache::PostableActionCacheStats;
use blaze_exec::actions::{ActionResult, SpawnResultBuilder, Status};
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_protos::blaze::ActionCacheStatistics;
use blaze_protos::blaze::action_cache_statistics::{MissDetail, MissReason};
use blaze_protos::build_event_stream::BuildMetrics;
use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use blaze_protos::build_event_stream::build_metrics::artifact_metrics::FilesMetric;
use blaze_protos::build_event_stream::build_metrics::network_metrics::SystemNetworkStats;
use blaze_protos::build_event_stream::build_metrics::worker_pool_metrics::WorkerPoolStats;
use blaze_protos::build_event_stream::build_metrics::{BuildGraphMetrics, NetworkMetrics};
use blaze_protos::json::JsonFormat;

// ---------------------------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------------------------

/// A clock the test sets (`ManualClock`).
#[derive(Default)]
struct ManualClock {
    millis: AtomicI64,
    nanos: AtomicI64,
}

impl ManualClock {
    fn set(&self, millis: i64, nanos: i64) {
        self.millis.store(millis, Ordering::SeqCst);
        self.nanos.store(nanos, Ordering::SeqCst);
    }

    fn set_nanos(&self, nanos: i64) {
        self.nanos.store(nanos, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.millis.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        self.nanos.load(Ordering::SeqCst)
    }
}

struct NoNamer;

impl ArtifactGroupNamer for NoNamer {
    fn apply(&self, _node: &ArtifactGroupNode) -> Option<NamedSetOfFilesId> {
        None
    }
}

struct TestContext {
    path_converter: Box<dyn PathConverter>,
}

impl BuildEventContext for TestContext {
    fn path_converter(&self) -> &dyn PathConverter {
        self.path_converter.as_ref()
    }

    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        &NoNamer
    }

    fn get_options(&self) -> &BuildEventProtocolOptions {
        unreachable!("the build metrics event does not read the BEP options")
    }
}

fn no_skyframe_stats() -> Option<blaze_bep::metrics::metrics_collector::SkyframeStats> {
    None
}

fn finish_inputs<'a>(
    skyframe_stats: &'a dyn Fn() -> Option<blaze_bep::metrics::metrics_collector::SkyframeStats>,
) -> BuildMetricsFinishInputs<'a> {
    BuildMetricsFinishInputs {
        worker_process_metrics: Vec::new(),
        worker_metrics: Vec::new(),
        has_pending_exception: false,
        skyframe_stats,
        remote_analysis_caching: Default::default(),
        memory: JvmMemoryFacts::default(),
        profile_elapsed_time_ms: None,
        server_process_cpu_time_ms: None,
        network_metrics: None,
    }
}

fn new_collector(clock: Arc<ManualClock>) -> MetricsCollector {
    MetricsCollector::install_in_env(
        None,
        clock,
        Arc::new(AtomicI32::new(0)),
        Arc::new(AtomicI32::new(0)),
    )
}

fn spawn_action_result(system_time_in_ms: i32, user_time_in_ms: i32) -> ActionResult {
    let spawn_result = SpawnResultBuilder::new()
        .set_runner_name("linux-sandbox")
        .set_status(Status::Success)
        .set_exit_code(0)
        .set_system_time_in_ms(system_time_in_ms)
        .set_user_time_in_ms(user_time_in_ms)
        .build();
    ActionResult::create(vec![spawn_result])
}

fn to_json(event: &dyn BuildEvent) -> serde_json::Value {
    let context = TestContext {
        path_converter: Box::new(NoConversion),
    };
    let proto = event.as_stream_proto(&context).unwrap();
    let json = JsonFormat::printer()
        .omitting_insignificant_whitespace()
        .print(&proto)
        .unwrap();
    serde_json::from_str(&json).unwrap()
}

// ---------------------------------------------------------------------------------------------
// Integration tests of the Java class (whole builds)
// ---------------------------------------------------------------------------------------------

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_actions_created() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_actions_created_is_zero_on_second_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_actions_executed() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_actions_executed_is_zero_on_second_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_actions_created_for_individual_mnemonics() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_aspect_actions_created_not_over_counted_for_alias() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn build_graph_and_artifact_metrics() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn tree_artifact_and_top_level_metrics() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_target_counts() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_targets_counts_are_zero_on_second_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn aspect_loaded_metric() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_packages_loaded() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_packages_loaded_is_zero_on_second_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_analysis_time_in_ms() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_execution_time_in_ms() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime) and the Profiler"]
fn test_actions_execution_start_time_in_ms() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase and a JVM heap (MemoryProfiler, MemoryPressureModule)"]
fn test_used_heap_size_post_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase and a JVM heap (MemoryProfiler, MemoryPressureModule)"]
fn test_used_heap_size_post_build_collection_off_by_default() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime) and the Profiler"]
fn test_wall_time_post_build() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (several builds and analyses through the runtime)"]
fn cumulative_metrics() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase (a whole build through the runtime)"]
fn test_action_data() {}

#[test]
#[ignore = "needs BuildIntegrationTestCase with --experimental_merged_skyframe_analysis_execution"]
fn skymeld_null_incremental_build_build_graph_metrics_not_collected() {}

// ---------------------------------------------------------------------------------------------
// Unit tests of the Java class
// ---------------------------------------------------------------------------------------------

const DUMMY_MNEMONIC: &str = "DUMMY_MNEMONIC";

fn create_worker_process_metrics(
    worker_ids: Vec<i32>,
    hash: i32,
    status: WorkerProcessStatusKind,
    newly_created: bool,
) -> WorkerProcessMetricsInfo {
    // Java: new WorkerProcessStatus().maybeUpdateStatus(status); onBeforeCommand() when the
    // process has been alive since before the command (not newly created).
    WorkerProcessMetricsInfo {
        worker_ids,
        worker_key_hash: hash,
        mnemonic: "DUMMY_MNEMONIC".to_string(),
        newly_created,
        status,
    }
}

#[test]
fn test_create_worker_pool_metrics() {
    // Given a list of WorkerProcessMetric(s), it should correctly aggregate the counts and create
    // the WorkerPoolMetrics proto.
    let all_process_metrics = vec![
        // Hash 100: alive_count = 3
        //           created_count = 1
        create_worker_process_metrics(vec![1], 100, WorkerProcessStatusKind::Alive, true),
        create_worker_process_metrics(vec![2, 3], 100, WorkerProcessStatusKind::Alive, false),
        // Hash 200: evicted_count = 3
        //           io_exception_destroyed_count = 2
        //           unknown_destroyed_count = 1
        //           destroyed_count = 6
        //           created_count = 3  (should still be counted even if they are killed).
        create_worker_process_metrics(
            vec![1, 2, 3],
            200,
            WorkerProcessStatusKind::KilledDueToMemoryPressure,
            false,
        ),
        create_worker_process_metrics(
            vec![1, 2],
            200,
            WorkerProcessStatusKind::KilledDueToIoException,
            true,
        ),
        create_worker_process_metrics(vec![1], 200, WorkerProcessStatusKind::KilledUnknown, true),
    ];
    // Java: containsExactly (any order); java.util.HashMap<Integer, _> iterates 100 then 200.
    assert_eq!(
        MetricsCollector::create_worker_pool_metrics(&all_process_metrics).worker_pool_stats,
        vec![
            WorkerPoolStats {
                mnemonic: DUMMY_MNEMONIC.to_string(),
                hash: 100,
                created_count: 1,
                alive_count: 3,
                ..Default::default()
            },
            WorkerPoolStats {
                mnemonic: DUMMY_MNEMONIC.to_string(),
                hash: 200,
                evicted_count: 3,
                io_exception_destroyed_count: 2,
                unknown_destroyed_count: 1,
                destroyed_count: 6,
                created_count: 3,
                ..Default::default()
            },
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Blaze tests
// ---------------------------------------------------------------------------------------------

#[test]
fn build_metrics_event_ids_and_order() {
    let event = BuildMetricsEvent::create(BuildMetrics::default());
    assert_eq!(event.get_event_id(), build_event_id_util::build_metrics());
    assert!(event.get_children_events().is_empty());
    assert_eq!(
        event.posted_after(),
        Some(vec![build_event_id_util::build_finished()])
    );
}

#[test]
fn empty_build_sets_every_sub_message() {
    let clock = Arc::new(ManualClock::default());
    let collector = new_collector(clock);
    let skyframe_stats = no_skyframe_stats;
    let event = collector.on_command_precomplete_event(finish_inputs(&skyframe_stats));
    let metrics = event.get_build_metrics();
    assert!(metrics.action_summary.is_some());
    assert!(metrics.memory_metrics.is_some());
    assert!(metrics.target_metrics.is_some());
    assert!(metrics.package_metrics.is_some());
    assert!(metrics.timing_metrics.is_some());
    assert!(metrics.cumulative_metrics.is_some());
    assert!(metrics.artifact_metrics.is_some());
    assert!(metrics.build_graph_metrics.is_some());
    assert!(metrics.worker_pool_metrics.is_some());
    assert!(metrics.dynamic_execution_metrics.is_some());
    assert!(metrics.remote_analysis_cache_statistics.is_some());
    assert!(metrics.network_metrics.is_none());
    // SpawnStats.getSummary always reports "total".
    let action_summary = metrics.action_summary.as_ref().unwrap();
    assert_eq!(action_summary.runner_count.len(), 1);
    assert_eq!(action_summary.runner_count[0].name, "total");
    assert_eq!(action_summary.runner_count[0].count, 0);
}

#[test]
fn action_data_is_limited_to_the_top_20_by_executed_count() {
    let clock = Arc::new(ManualClock::default());
    let collector = new_collector(clock);
    let by_mnemonic: Vec<(String, i32)> = (0..25).map(|i| (format!("Mnemonic{i}"), 1)).collect();
    collector.on_analysis_phase_complete(&AnalysisPhaseCompleteInfo {
        actions_constructed_by_mnemonic: by_mnemonic,
        ..Default::default()
    });
    for _ in 0..3 {
        collector.on_action_complete("Mnemonic24", 0);
    }
    collector.on_action_complete("Mnemonic7", 0);
    let skyframe_stats = no_skyframe_stats;
    let event = collector.on_command_precomplete_event(finish_inputs(&skyframe_stats));
    let action_data = &event
        .get_build_metrics()
        .action_summary
        .as_ref()
        .unwrap()
        .action_data;
    assert_eq!(action_data.len(), 20);
    assert_eq!(action_data[0].mnemonic, "Mnemonic24");
    assert_eq!(action_data[0].actions_executed, 3);
    assert_eq!(action_data[1].mnemonic, "Mnemonic7");
    assert_eq!(action_data[1].actions_executed, 1);
}

/// The brotli oracle's `buildMetrics` event (`bep.json`, last line), without the
/// dirtied/built/evaluated per-SkyFunction lists, which the test does not feed.
const BROTLI_ORACLE_BUILD_METRICS: &str = r#"{"id":{"buildMetrics":{}},"lastMessage":true,"buildMetrics":{"actionSummary":{"actionsCreated":"61","actionsExecuted":"53","actionsCreatedNotIncludingAspects":"60","actionData":[{"mnemonic":"CppCompile","actionsExecuted":"36","firstStartedMs":"1794967231810","lastEndedMs":"1794967232831","systemTime":"0.825s","userTime":"4.003s","actionsCreated":"36"},{"mnemonic":"Symlink","actionsExecuted":"5","firstStartedMs":"1794967231810","lastEndedMs":"1794967231811","actionsCreated":"6"},{"mnemonic":"CppLink","actionsExecuted":"4","firstStartedMs":"1794967231889","lastEndedMs":"1794967232861","systemTime":"0.039s","userTime":"0.042s","actionsCreated":"4"},{"mnemonic":"CppArchive","actionsExecuted":"3","firstStartedMs":"1794967231932","lastEndedMs":"1794967232853","systemTime":"0.054s","userTime":"0.017s","actionsCreated":"3"},{"mnemonic":"RepoMappingManifest","actionsExecuted":"1","firstStartedMs":"1794967231879","lastEndedMs":"1794967231880","actionsCreated":"1"},{"mnemonic":"SymlinkTree","actionsExecuted":"1","firstStartedMs":"1794967231884","lastEndedMs":"1794967231887","actionsCreated":"1"},{"mnemonic":"SourceSymlinkManifest","actionsExecuted":"1","firstStartedMs":"1794967231879","lastEndedMs":"1794967231880","actionsCreated":"1"},{"mnemonic":"BazelWorkspaceStatusAction","actionsExecuted":"1","firstStartedMs":"1794967231169","lastEndedMs":"1794967231189","actionsCreated":"1"},{"mnemonic":"RunfilesTree","actionsExecuted":"1","firstStartedMs":"1794967232861","lastEndedMs":"1794967232863","actionsCreated":"1"},{"mnemonic":"TranslateBuildInfo","actionsCreated":"2"},{"mnemonic":"CcStrip","actionsCreated":"1"},{"mnemonic":"FileWrite","actionsCreated":"1"},{"mnemonic":"SolibSymlink","actionsCreated":"3"}],"runnerCount":[{"name":"total","count":53},{"name":"internal","count":10},{"name":"linux-sandbox","count":43,"execKind":"Local"}],"actionCacheStatistics":{"sizeInBytes":"9367","misses":53,"missDetails":[{},{"reason":"DIFFERENT_DEPS"},{"reason":"DIFFERENT_ENVIRONMENT"},{"reason":"DIFFERENT_FILES"},{"reason":"CORRUPTED_CACHE_ENTRY"},{"reason":"NOT_CACHED","count":52},{"reason":"UNCONDITIONAL_EXECUTION","count":1},{"reason":"DIGEST_MISMATCH"}],"loadTimeInMs":"1"}},"memoryMetrics":{"garbageMetrics":[{"type":"G1 Eden Space","garbageCollected":"266338304"},{"type":"G1 Survivor Space","garbageCollected":"6498544"}]},"targetMetrics":{"targetsConfigured":"595","targetsConfiguredNotIncludingAspects":"594"},"packageMetrics":{"packagesLoaded":"89"},"timingMetrics":{"cpuTimeInMs":"7620","wallTimeInMs":"2063","analysisPhaseTimeInMs":"679","executionPhaseTimeInMs":"1674","actionsExecutionStartInMs":"382","criticalPathTime":"1.041423539s"},"cumulativeMetrics":{"numAnalyses":1,"numBuilds":1},"artifactMetrics":{"sourceArtifactsRead":{"sizeInBytes":"2328125","count":115},"outputArtifactsSeen":{"sizeInBytes":"7671064","count":89},"outputArtifactsFromActionCache":{},"topLevelArtifacts":{"sizeInBytes":"11766429","count":165}},"buildGraphMetrics":{"actionLookupValueCount":177,"actionCount":61,"outputArtifactCount":98,"postInvocationSkyframeNodeCount":7305,"actionLookupValueCountNotIncludingAspects":176,"actionCountNotIncludingAspects":60,"inputFileConfiguredTargetCount":121,"otherConfiguredTargetCount":9,"changedValues":[{"skyfunctionName":"PRECOMPUTED","count":"1"},{"skyfunctionName":"ERROR_TRANSIENCE","count":"1"},{"skyfunctionName":"FILE_STATE","count":"2"}],"cleanedValues":[{"skyfunctionName":"SINGLE_EXTENSION_EVAL","count":"15"},{"skyfunctionName":"BAZEL_DEP_GRAPH","count":"1"},{"skyfunctionName":"REPO_SPEC","count":"27"}]},"networkMetrics":{"systemNetworkStats":{"bytesSent":"170100","bytesRecv":"658203","packetsSent":"1163","packetsRecv":"1383","peakBytesSentPerSec":"89959","peakBytesRecvPerSec":"553279","peakPacketsSentPerSec":"442","peakPacketsRecvPerSec":"666"}},"workerPoolMetrics":{},"dynamicExecutionMetrics":{},"remoteAnalysisCacheStatistics":{}}}"#;

/// One mnemonic's executed actions in the brotli oracle build.
struct Executed {
    mnemonic: &'static str,
    count: usize,
    first_started_ms: i64,
    last_ended_ms: i64,
    system_time_ms: i32,
    user_time_ms: i32,
    /// Whether the actions ran a spawn (`linux-sandbox`) or are internal.
    spawn: bool,
}

#[test]
fn brotli_oracle_build_metrics() {
    const MS: i64 = 1_000_000;
    let clock = Arc::new(ManualClock::default());
    let num_analyses = Arc::new(AtomicI32::new(0));
    let num_builds = Arc::new(AtomicI32::new(0));
    let collector = MetricsCollector::install_in_env(
        None,
        clock.clone(),
        num_analyses.clone(),
        num_builds.clone(),
    );

    // Analysis.
    collector.log_analysis_starting_event();
    collector.on_analysis_phase_complete(&AnalysisPhaseCompleteInfo {
        actions_constructed: TotalAndConfiguredTargetOnlyMetric::create(61, 60),
        targets_configured: TotalAndConfiguredTargetOnlyMetric::create(595, 594),
        time_in_ms: 679,
        packages_successfully_loaded: 89,
        actions_constructed_by_mnemonic: [
            ("CppCompile", 36),
            ("Symlink", 6),
            ("CppLink", 4),
            ("CppArchive", 3),
            ("RepoMappingManifest", 1),
            ("SymlinkTree", 1),
            ("SourceSymlinkManifest", 1),
            ("BazelWorkspaceStatusAction", 1),
            ("RunfilesTree", 1),
            ("TranslateBuildInfo", 2),
            ("CcStrip", 1),
            ("FileWrite", 1),
            ("SolibSymlink", 3),
        ]
        .into_iter()
        .map(|(m, c)| (m.to_string(), c))
        .collect(),
        package_metrics_recorder: None,
    });
    collector.log_analysis_graph_stats(&BuildGraphMetrics {
        action_lookup_value_count: 177,
        action_count: 61,
        output_artifact_count: 98,
        action_lookup_value_count_not_including_aspects: 176,
        action_count_not_including_aspects: 60,
        input_file_configured_target_count: 121,
        other_configured_target_count: 9,
        ..Default::default()
    });

    // Execution (Skymeld: one TopLevelTargetPendingExecutionEvent per top-level target).
    collector.handle_execution_phase_start();
    collector.handle_execution_phase_start();
    collector.on_some_execution_started(false, Some(1));
    collector.on_some_execution_started(true, Some(382));
    collector.on_some_execution_started(true, Some(999));
    let executed = [
        Executed {
            mnemonic: "CppCompile",
            count: 36,
            first_started_ms: 1794967231810,
            last_ended_ms: 1794967232831,
            system_time_ms: 825,
            user_time_ms: 4003,
            spawn: true,
        },
        Executed {
            mnemonic: "Symlink",
            count: 5,
            first_started_ms: 1794967231810,
            last_ended_ms: 1794967231811,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
        Executed {
            mnemonic: "CppLink",
            count: 4,
            first_started_ms: 1794967231889,
            last_ended_ms: 1794967232861,
            system_time_ms: 39,
            user_time_ms: 42,
            spawn: true,
        },
        Executed {
            mnemonic: "CppArchive",
            count: 3,
            first_started_ms: 1794967231932,
            last_ended_ms: 1794967232853,
            system_time_ms: 54,
            user_time_ms: 17,
            spawn: true,
        },
        Executed {
            mnemonic: "RepoMappingManifest",
            count: 1,
            first_started_ms: 1794967231879,
            last_ended_ms: 1794967231880,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
        Executed {
            mnemonic: "SymlinkTree",
            count: 1,
            first_started_ms: 1794967231884,
            last_ended_ms: 1794967231887,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
        Executed {
            mnemonic: "SourceSymlinkManifest",
            count: 1,
            first_started_ms: 1794967231879,
            last_ended_ms: 1794967231880,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
        Executed {
            mnemonic: "BazelWorkspaceStatusAction",
            count: 1,
            first_started_ms: 1794967231169,
            last_ended_ms: 1794967231189,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
        Executed {
            mnemonic: "RunfilesTree",
            count: 1,
            first_started_ms: 1794967232861,
            last_ended_ms: 1794967232863,
            system_time_ms: 0,
            user_time_ms: 0,
            spawn: false,
        },
    ];
    for e in &executed {
        for k in 0..e.count {
            // The first action starts first, the last one ends last.
            let end_ms = if k + 1 == e.count {
                e.last_ended_ms
            } else {
                e.first_started_ms
            };
            if e.spawn {
                let (system, user) = if k == 0 {
                    (e.system_time_ms, e.user_time_ms)
                } else {
                    (0, 0)
                };
                collector.action_result_received(e.mnemonic, &spawn_action_result(system, user));
            }
            clock.set_nanos(end_ms * MS);
            collector.on_action_complete(e.mnemonic, e.first_started_ms * MS);
        }
    }
    let reasons = [
        (MissReason::DifferentActionKey, 0),
        (MissReason::DifferentDeps, 0),
        (MissReason::DifferentEnvironment, 0),
        (MissReason::DifferentFiles, 0),
        (MissReason::CorruptedCacheEntry, 0),
        (MissReason::NotCached, 52),
        (MissReason::UnconditionalExecution, 1),
        (MissReason::DigestMismatch, 0),
    ];
    collector.log_action_cache_statistics(&PostableActionCacheStats::new(ActionCacheStatistics {
        size_in_bytes: 9367,
        misses: 53,
        miss_details: reasons
            .iter()
            .map(|&(reason, count)| MissDetail {
                reason: reason as i32,
                count,
            })
            .collect(),
        load_time_in_ms: 1,
        ..Default::default()
    }));
    collector.on_execution_complete(&ExecutionFinishedInfo {
        source_artifacts_read: FilesMetric {
            size_in_bytes: 2328125,
            count: 115,
        },
        output_artifacts_seen: FilesMetric {
            size_in_bytes: 7671064,
            count: 89,
        },
        output_artifacts_from_action_cache: FilesMetric::default(),
        top_level_artifacts: FilesMetric {
            size_in_bytes: 11766429,
            count: 165,
        },
    });
    collector.handle_execution_phase_complete(1674);
    collector.on_critical_path(1_041_423_539);
    let stats = |list: &[(&str, i32)]| -> Vec<(String, i32)> {
        list.iter().map(|&(n, c)| (n.to_string(), c)).collect()
    };
    collector.on_skyframe_graph_stats(&SkyframeGraphStatsInfo {
        changed: stats(&[
            ("PRECOMPUTED", 1),
            ("ERROR_TRANSIENCE", 1),
            ("FILE_STATE", 2),
        ]),
        cleaned: stats(&[
            ("SINGLE_EXTENSION_EVAL", 15),
            ("BAZEL_DEP_GRAPH", 1),
            ("REPO_SPEC", 27),
        ]),
        graph_size: 7305,
        ..Default::default()
    });

    // Command precomplete. The epoch-millis converter maps nanos n to n / 1e6.
    let now_ms = 1794967240000;
    clock.set(now_ms, now_ms * MS);
    let skyframe_stats = no_skyframe_stats;
    let mut garbage_stats = BTreeMap::new();
    garbage_stats.insert("G1 Eden Space".to_string(), 266338304);
    garbage_stats.insert("G1 Survivor Space".to_string(), 6498544);
    let event = collector.on_command_precomplete_event(BuildMetricsFinishInputs {
        memory: JvmMemoryFacts {
            garbage_stats,
            ..Default::default()
        },
        profile_elapsed_time_ms: Some(2063),
        server_process_cpu_time_ms: Some(7620),
        network_metrics: Some(NetworkMetrics {
            system_network_stats: Some(SystemNetworkStats {
                bytes_sent: 170100,
                bytes_recv: 658203,
                packets_sent: 1163,
                packets_recv: 1383,
                peak_bytes_sent_per_sec: 89959,
                peak_bytes_recv_per_sec: 553279,
                peak_packets_sent_per_sec: 442,
                peak_packets_recv_per_sec: 666,
            }),
        }),
        ..finish_inputs(&skyframe_stats)
    });
    assert_eq!(num_analyses.load(Ordering::SeqCst), 1);
    assert_eq!(num_builds.load(Ordering::SeqCst), 1);

    let actual = to_json(&LastBuildEvent::new(Arc::new(event)));
    let expected: serde_json::Value = serde_json::from_str(BROTLI_ORACLE_BUILD_METRICS).unwrap();
    assert_eq!(
        actual["buildMetrics"]["actionSummary"],
        expected["buildMetrics"]["actionSummary"]
    );
    assert_eq!(
        actual["buildMetrics"]["targetMetrics"],
        expected["buildMetrics"]["targetMetrics"]
    );
    assert_eq!(
        actual["buildMetrics"]["packageMetrics"],
        expected["buildMetrics"]["packageMetrics"]
    );
    assert_eq!(
        actual["buildMetrics"]["buildGraphMetrics"],
        expected["buildMetrics"]["buildGraphMetrics"]
    );
    assert_eq!(actual, expected);
}
