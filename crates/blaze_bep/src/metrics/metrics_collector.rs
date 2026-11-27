//! Port of `lib/metrics/BuildMetricsEvent.java` and `lib/metrics/MetricsCollector.java`: the
//! collector that listens to the build's events and, when the command is about to complete,
//! builds the `BuildMetrics` proto and posts it as a [`BuildMetricsEvent`].
//!
//! # How the runtime uses it
//!
//! Java: `MetricsModule.beforeCommand` installs one collector per command
//! (`MetricsCollector.installInEnv(env, numAnalyses, numBuilds)`); `numAnalyses` and `numBuilds`
//! are owned by the module and live as long as the server. The collector subscribes to the
//! command's event bus. In Blaze the runtime calls the handler methods itself, at the point it
//! posts (or, for events Blaze does not post yet, would post) the Java event:
//!
//! | call | Java event / point |
//! |---|---|
//! | [`MetricsCollector::install_in_env`] | `MetricsModule.beforeCommand` (before the command runs) |
//! | [`MetricsCollector::log_analysis_starting_event`] | `AnalysisPhaseStartedEvent` (start of analysis) |
//! | [`MetricsCollector::on_analysis_phase_complete`] | `AnalysisPhaseCompleteEvent` (end of analysis) |
//! | [`MetricsCollector::log_analysis_graph_stats`] | `AnalysisGraphStatsEvent` (after analysis, once per build) |
//! | [`MetricsCollector::log_execution_starting_event`] | `ExecutionStartingEvent` (non-Skymeld execution start) |
//! | [`MetricsCollector::handle_execution_phase_start`] | `TopLevelTargetPendingExecutionEvent` (Skymeld: each top-level target pending execution) |
//! | [`MetricsCollector::on_some_execution_started`] | `SomeExecutionStartedEvent` (first action / workspace status execution) |
//! | [`MetricsCollector::on_action_complete`] | `ActionCompletionEvent` (each executed action) |
//! | [`MetricsCollector::action_result_received`] | `ActionResultReceivedEvent` (each action that returned an `ActionResult`) |
//! | [`MetricsCollector::on_dynamic_execution_finished_event`] | `DynamicExecutionFinishedEvent` |
//! | [`MetricsCollector::log_action_cache_statistics`] | `PostableActionCacheStats` (`ExecutionTool`, after execution) |
//! | [`MetricsCollector::on_execution_complete`] | `ExecutionFinishedEvent` (end of execution) |
//! | [`MetricsCollector::handle_execution_phase_complete`] | `ExecutionPhaseCompleteEvent` (end of execution) |
//! | [`MetricsCollector::on_critical_path`] | `CriticalPathEvent` (after execution) |
//! | [`MetricsCollector::on_skyframe_graph_stats`] | `SkyframeGraphStatsEvent` (after the evaluation) |
//! | [`MetricsCollector::on_command_precomplete_event`] | `CommandPrecompleteEvent` (`BlazeCommandDispatcher`, before `afterCommand`); post the returned [`BuildMetricsEvent`] on the command's event bus |

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering as AtomicOrdering};

use blaze_events::Clock;
use blaze_exec::actions::ActionResult;
use blaze_exec::actions::cache::PostableActionCacheStats;
use blaze_exec::runtime::spawn_stats::SpawnStats;
use blaze_exec::worker::worker_process_metrics::{
    MAX_PUBLISHED_WORKER_METRICS, WorkerProcessMetrics, WorkerProcessMetricsCollector,
};
use blaze_optdefs::misc::metrics_module::MetricsModuleOptions;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_metrics::action_summary::{ActionData, RunnerCount};
use blaze_protos::build_event_stream::build_metrics::artifact_metrics::FilesMetric;
use blaze_protos::build_event_stream::build_metrics::build_graph_metrics::{
    AspectCount, RuleClassCount,
};
use blaze_protos::build_event_stream::build_metrics::dynamic_execution_metrics::RaceStatistics;
use blaze_protos::build_event_stream::build_metrics::memory_metrics::GarbageMetrics;
use blaze_protos::build_event_stream::build_metrics::worker_pool_metrics::WorkerPoolStats as WorkerPoolStatsProto;
use blaze_protos::build_event_stream::build_metrics::{
    ActionSummary, ArtifactMetrics, BuildGraphMetrics, CumulativeMetrics, DynamicExecutionMetrics,
    EvaluationStat, MemoryMetrics, NetworkMetrics, PackageMetrics, RemoteAnalysisCacheStatistics,
    TargetMetrics, TimingMetrics, WorkerMetrics, WorkerPoolMetrics,
};
use blaze_protos::build_event_stream::{self as bes, BuildEventId, BuildMetrics};
use blaze_protos::devtools::build::lib::packages::metrics::PackageLoadMetrics;
use parking_lot::Mutex;

use crate::buildeventstream::build_event::{BuildEvent, Interrupted};
use crate::buildeventstream::build_event_context::BuildEventContext;
use crate::buildeventstream::build_event_id_util;
use crate::buildeventstream::generic_build_event::proto_chaining_with;
use crate::util::{duration_from_millis, duration_from_nanos};

// ---------------------------------------------------------------------------------------------
// BuildMetricsEvent
// ---------------------------------------------------------------------------------------------

/// An event encapsulating build metrics collected during a build.
// Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:27 BuildMetricsEvent
#[derive(Clone, Debug)]
pub struct BuildMetricsEvent {
    build_metrics: BuildMetrics,
}

impl BuildMetricsEvent {
    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:30 BuildMetricsEvent.<init>
    pub fn new(build_metrics: BuildMetrics) -> Self {
        BuildMetricsEvent { build_metrics }
    }

    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:34 BuildMetricsEvent.create
    pub fn create(build_metrics: BuildMetrics) -> Self {
        Self::new(build_metrics)
    }

    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:53 BuildMetricsEvent.getBuildMetrics
    pub fn get_build_metrics(&self) -> &BuildMetrics {
        &self.build_metrics
    }
}

impl BuildEvent for BuildMetricsEvent {
    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:39 BuildMetricsEvent.getEventId
    fn get_event_id(&self) -> BuildEventId {
        build_event_id_util::build_metrics()
    }

    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:44 BuildMetricsEvent.getChildrenEvents
    fn get_children_events(&self) -> Vec<BuildEventId> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:49 BuildMetricsEvent.asStreamProto
    fn as_stream_proto(
        &self,
        _converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, Interrupted> {
        Ok(proto_chaining_with(
            self,
            Payload::BuildMetrics(self.build_metrics.clone()),
        ))
    }

    // Java: com/google/devtools/build/lib/metrics/BuildMetricsEvent.java:58 BuildMetricsEvent.postedAfter
    fn posted_after(&self) -> Option<Vec<BuildEventId>> {
        Some(vec![build_event_id_util::build_finished()])
    }
}

impl blaze_events::Postable for BuildMetricsEvent {}

// ---------------------------------------------------------------------------------------------
// Inputs: what each handler reads from its Java event
// ---------------------------------------------------------------------------------------------

/// `actions.TotalAndConfiguredTargetOnlyMetric`.
// Java: com/google/devtools/build/lib/actions/TotalAndConfiguredTargetOnlyMetric.java:23 TotalAndConfiguredTargetOnlyMetric
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TotalAndConfiguredTargetOnlyMetric {
    pub total: i32,
    pub configured_targets_only: i32,
}

impl TotalAndConfiguredTargetOnlyMetric {
    // Java: com/google/devtools/build/lib/actions/TotalAndConfiguredTargetOnlyMetric.java:25 TotalAndConfiguredTargetOnlyMetric.create
    pub fn create(total: i32, configured_targets_only: i32) -> Self {
        TotalAndConfiguredTargetOnlyMetric {
            total,
            configured_targets_only,
        }
    }
}

/// The state of the `PackageMetricsPackageLoadingListener` singleton that
/// `onAnalysisPhaseComplete` reads when `getPublishPackageMetricsInBep()` is true and the
/// listener has a recorder.
#[derive(Clone, Debug, Default)]
pub struct PackageMetricsRecorderSnapshot {
    /// `recorder.getPackageLoadMetrics()`, in the recorder's order.
    pub package_load_metrics: Vec<PackageLoadMetrics>,
    /// `((ExtremaPackageMetricsRecorder) recorder).getNumPackagesToTrack()` when
    /// `recorder.getRecorderType() == ONLY_EXTREMES`; `None` for any other recorder type.
    pub only_extremes_num_packages_to_track: Option<i32>,
}

/// What `onAnalysisPhaseComplete` reads from its `AnalysisPhaseCompleteEvent` (and from the
/// package metrics listener singleton).
#[derive(Clone, Debug, Default)]
pub struct AnalysisPhaseCompleteInfo {
    /// `event.getActionsConstructed()`.
    pub actions_constructed: TotalAndConfiguredTargetOnlyMetric,
    /// `event.getTargetsConfigured()`.
    pub targets_configured: TotalAndConfiguredTargetOnlyMetric,
    /// `event.getTimeInMs()`.
    pub time_in_ms: i64,
    /// `event.getPkgManagerStats().getPackagesSuccessfullyLoaded()`.
    pub packages_successfully_loaded: i32,
    /// `event.getActionsConstructedByMnemonic()`, in the `ImmutableMap`'s order.
    pub actions_constructed_by_mnemonic: Vec<(String, i32)>,
    /// `PackageMetricsPackageLoadingListener.getInstance()`: `Some` iff
    /// `getPublishPackageMetricsInBep()` is true and `getPackageMetricsRecorder() != null`.
    pub package_metrics_recorder: Option<PackageMetricsRecorderSnapshot>,
}

/// What `onExecutionComplete` reads from its `ExecutionFinishedEvent` (the four
/// `ArtifactMetrics.FilesMetric` record components, all non-null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecutionFinishedInfo {
    /// `event.sourceArtifactsRead()`.
    pub source_artifacts_read: FilesMetric,
    /// `event.outputArtifactsSeen()`.
    pub output_artifacts_seen: FilesMetric,
    /// `event.outputArtifactsFromActionCache()`.
    pub output_artifacts_from_action_cache: FilesMetric,
    /// `event.topLevelArtifacts()`.
    pub top_level_artifacts: FilesMetric,
}

/// `DynamicStrategyRegistry.DynamicMode`.
// Java: com/google/devtools/build/lib/actions/DynamicStrategyRegistry.java:23 DynamicStrategyRegistry.DynamicMode
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DynamicMode {
    Remote,
    Local,
}

/// What `onSkyframeGraphStats` reads from its `SkyframeGraphStatsEvent`: the
/// `EvaluationStats` maps (`ImmutableMap<SkyFunctionName, Integer>`, as
/// `(skyFunctionName.getName(), count)` in the map's order) and `getGraphSize()`.
// Java: com/google/devtools/build/skyframe/SkyframeGraphStatsEvent.java:25 SkyframeGraphStatsEvent.EvaluationStats
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkyframeGraphStatsInfo {
    pub dirtied: Vec<(String, i32)>,
    pub changed: Vec<(String, i32)>,
    pub built: Vec<(String, i32)>,
    pub cleaned: Vec<(String, i32)>,
    pub evaluated: Vec<(String, i32)>,
    /// `event.getGraphSize()`.
    pub graph_size: i32,
}

/// `skyframe.SkyKeyStats`: what `addSkyframeStats` reads of one rule class or aspect.
// Java: com/google/devtools/build/lib/skyframe/SkyKeyStats.java:19 SkyKeyStats
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkyKeyStats {
    /// `getKey()`.
    pub key: String,
    /// `getName()`.
    pub name: String,
    /// `getCount()`.
    pub count: i64,
    /// `getActionCount()`.
    pub action_count: i64,
}

/// `skyframe.SkyframeStats`: `ruleStats()` and `aspectStats()`, in their order.
// Java: com/google/devtools/build/lib/skyframe/SkyframeStats.java SkyframeStats
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkyframeStats {
    pub rule_stats: Vec<SkyKeyStats>,
    pub aspect_stats: Vec<SkyKeyStats>,
}

/// `worker.WorkerProcessStatus.Status`.
// Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:30 WorkerProcessStatus.Status
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerProcessStatusKind {
    NotStarted,
    Alive,
    PendingKillDueToUnknown,
    PendingKillDueToInterruptedException,
    PendingKillDueToUserExecException,
    PendingKillDueToIoException,
    PendingKillDueToMemoryPressure,
    KilledUnknown,
    KilledDueToInterruptedException,
    KilledDueToUserExecException,
    KilledDueToIoException,
    KilledDueToMemoryPressure,
}

impl WorkerProcessStatusKind {
    // Java: com/google/devtools/build/lib/worker/WorkerProcessStatus.java:155 WorkerProcessStatus.isKilled
    pub fn is_killed(self) -> bool {
        // Java: WorkerProcessStatus.KILLED_STATUSES
        matches!(
            self,
            WorkerProcessStatusKind::KilledUnknown
                | WorkerProcessStatusKind::KilledDueToIoException
                | WorkerProcessStatusKind::KilledDueToInterruptedException
                | WorkerProcessStatusKind::KilledDueToUserExecException
                | WorkerProcessStatusKind::KilledDueToMemoryPressure
        )
    }
}

/// What `createWorkerPoolMetrics` reads of one `worker.WorkerProcessMetrics`.
// Java: com/google/devtools/build/lib/worker/WorkerProcessMetrics.java WorkerProcessMetrics
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerProcessMetricsInfo {
    /// `getWorkerIds()`.
    pub worker_ids: Vec<i32>,
    /// `getWorkerKeyHash()`.
    pub worker_key_hash: i32,
    /// `getMnemonic()`.
    pub mnemonic: String,
    /// `isNewlyCreated()`.
    pub newly_created: bool,
    /// `getStatus().get()`.
    pub status: WorkerProcessStatusKind,
}

/// `FingerprintValueStore.Stats` as read by `collectRemoteAnalysisCacheStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FingerprintValueStoreStats {
    pub value_bytes_received: i64,
    pub value_bytes_sent: i64,
    pub key_bytes_sent: i64,
    pub entries_written: i64,
    pub entries_found: i64,
    pub entries_not_found: i64,
    pub get_batches: i64,
    pub set_batches: i64,
}

/// `RemoteAnalysisCacheClient.Stats` as read by `collectRemoteAnalysisCacheStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteAnalysisCacheClientStats {
    pub bytes_received: i64,
    pub bytes_sent: i64,
    pub requests_sent: i64,
    pub batches: i64,
}

/// What `collectRemoteAnalysisCacheStats` reads from
/// `env.getRemoteAnalysisCachingEventListener()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteAnalysisCachingFacts {
    /// `getCacheHits().size()`.
    pub cache_hits: i64,
    /// `getCacheMisses().size()`.
    pub cache_misses: i64,
    /// `getFingerprintValueStoreStats()` (nullable).
    pub fingerprint_value_store_stats: Option<FingerprintValueStoreStats>,
    /// `getRemoteAnalysisCacheStats()` (nullable).
    pub remote_analysis_cache_stats: Option<RemoteAnalysisCacheClientStats>,
}

/// The JVM memory facts `createMemoryMetrics` reads. PORT: Blaze is not a JVM; the runtime
/// passes `Default` (no measurement), which yields an empty `MemoryMetrics` as Java does when no
/// GC has been observed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JvmMemoryFacts {
    /// `MemoryProfiler.instance().getHeapUsedMemoryAtFinish()`.
    pub heap_used_memory_at_finish: i64,
    /// `PostGCMemoryUseRecorder.get().getPeakPostGcHeap().map(PeakHeap::bytes)`.
    pub peak_post_gc_heap: Option<i64>,
    /// `PostGCMemoryUseRecorder.get().getPeakPostGcHeapTenuredSpace().map(PeakHeap::bytes)`.
    pub peak_post_gc_heap_tenured_space: Option<i64>,
    /// `PostGCMemoryUseRecorder.get().getGarbageStats()` (an `ImmutableSortedMap`).
    pub garbage_stats: BTreeMap<String, i64>,
}

/// The facts `createBuildMetrics` reads from singletons and the environment when the command
/// precompletes.
pub struct BuildMetricsFinishInputs<'a> {
    /// `WorkerProcessMetricsCollector.instance().collectMetrics()`, as `createWorkerPoolMetrics`
    /// reads it.
    pub worker_process_metrics: Vec<WorkerProcessMetricsInfo>,
    /// `WorkerProcessMetrics::toProto` of the same collected metrics, in the same order.
    pub worker_metrics: Vec<WorkerMetrics>,
    /// `env.getPendingException() != null`.
    pub has_pending_exception: bool,
    /// `env.getSkyframeExecutor().getSkyframeStats(env.getReporter())`; called only when
    /// `--experimental_record_skyframe_metrics` is set and there is no pending exception.
    pub skyframe_stats: &'a dyn Fn() -> Option<SkyframeStats>,
    /// `env.getRemoteAnalysisCachingEventListener()`.
    pub remote_analysis_caching: RemoteAnalysisCachingFacts,
    /// `MemoryProfiler` / `PostGCMemoryUseRecorder`.
    pub memory: JvmMemoryFacts,
    /// `Profiler.getProfileElapsedTime()` in milliseconds (nullable).
    pub profile_elapsed_time_ms: Option<i64>,
    /// `Profiler.getServerProcessCpuTime()` in milliseconds (nullable).
    pub server_process_cpu_time_ms: Option<i64>,
    /// `NetworkMetricsCollector.instance().collectMetrics()` (nullable).
    pub network_metrics: Option<NetworkMetrics>,
}

impl WorkerProcessMetricsInfo {
    /// What `createWorkerPoolMetrics` reads of a collected `WorkerProcessMetrics`.
    pub fn of(wpm: &WorkerProcessMetrics) -> WorkerProcessMetricsInfo {
        use blaze_exec::worker::worker_process_status::Status;
        WorkerProcessMetricsInfo {
            worker_ids: wpm.get_worker_ids(),
            worker_key_hash: wpm.get_worker_key_hash(),
            mnemonic: wpm.get_mnemonic().to_string(),
            newly_created: wpm.is_newly_created(),
            status: match wpm.get_status().get() {
                Status::NotStarted => WorkerProcessStatusKind::NotStarted,
                Status::Alive => WorkerProcessStatusKind::Alive,
                Status::PendingKillDueToUnknown => WorkerProcessStatusKind::PendingKillDueToUnknown,
                Status::PendingKillDueToInterruptedException => {
                    WorkerProcessStatusKind::PendingKillDueToInterruptedException
                }
                Status::PendingKillDueToUserExecException => {
                    WorkerProcessStatusKind::PendingKillDueToUserExecException
                }
                Status::PendingKillDueToIoException => {
                    WorkerProcessStatusKind::PendingKillDueToIoException
                }
                Status::PendingKillDueToMemoryPressure => {
                    WorkerProcessStatusKind::PendingKillDueToMemoryPressure
                }
                Status::KilledUnknown => WorkerProcessStatusKind::KilledUnknown,
                Status::KilledDueToInterruptedException => {
                    WorkerProcessStatusKind::KilledDueToInterruptedException
                }
                Status::KilledDueToUserExecException => {
                    WorkerProcessStatusKind::KilledDueToUserExecException
                }
                Status::KilledDueToIoException => WorkerProcessStatusKind::KilledDueToIoException,
                Status::KilledDueToMemoryPressure => {
                    WorkerProcessStatusKind::KilledDueToMemoryPressure
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// MetricsCollector
// ---------------------------------------------------------------------------------------------

// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:431 MetricsCollector.MAX_ACTION_DATA
const MAX_ACTION_DATA: usize = 20;

/// The per-mnemonic counters of the `ActionSummary`.
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:633 MetricsCollector.ActionStats
#[derive(Debug)]
struct ActionStats {
    /// `LongAccumulator(Math::min, Long.MAX_VALUE)`.
    first_started: i64,
    /// `LongAccumulator(Math::max, 0)`.
    last_ended: i64,
    num_actions_executed: i64,
    num_actions_registered: i64,
    mnemonic: String,
    system_time: i64,
    user_time: i64,
}

impl ActionStats {
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:643 ActionStats.<init>
    fn new(mnemonic: &str) -> Self {
        ActionStats {
            mnemonic: mnemonic.to_string(),
            first_started: i64::MAX,
            last_ended: 0,
            num_actions_executed: 0,
            num_actions_registered: 0,
            system_time: 0,
            user_time: 0,
        }
    }
}

/// Collects stats about dynamic execution races of remote vs local branches.
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:655 MetricsCollector.DynamicExecutionStats
#[derive(Debug, Default)]
struct DynamicExecutionStats {
    /// `branchWinners`: (mnemonic, local name, remote name) -> `RaceWinners` (local wins, remote
    /// wins), in insertion order; the map's iteration order is `branch_winners_order`.
    #[allow(clippy::type_complexity)]
    branch_winners: Vec<((String, String, String), (i32, i32))>,
    branch_winners_order: JavaConcurrentHashMapOrder,
}

/// `RaceIdentifier.hashCode()`: a record's hash code, `31 * h + c.hashCode()` over its
/// components in order, starting from 0.
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:710 MetricsCollector.DynamicExecutionStats.RaceIdentifier
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:711 MetricsCollector.DynamicExecutionStats.RaceIdentifier.RaceIdentifier
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:717 MetricsCollector.DynamicExecutionStats.RaceIdentifier.create
// Java: java/lang/runtime/ObjectMethods.java ObjectMethods.makeHashCode
fn race_identifier_hash_code(mnemonic: &str, local_name: &str, remote_name: &str) -> i32 {
    let mut result: i32 = 0;
    for component in [mnemonic, local_name, remote_name] {
        result = result
            .wrapping_mul(31)
            .wrapping_add(java_string_hash(component));
    }
    result
}

impl DynamicExecutionStats {
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:664 DynamicExecutionStats.update
    fn update(
        &mut self,
        menemonic: &str,
        local_name: &str,
        remote_name: &str,
        winner: DynamicMode,
    ) {
        let key = (
            menemonic.to_string(),
            local_name.to_string(),
            remote_name.to_string(),
        );
        // branchWinners.compute(RaceIdentifier.create(...), ...): equality of the record is
        // the equality of the three strings (joined with a NUL, which no name contains).
        let hash_code = race_identifier_hash_code(menemonic, local_name, remote_name);
        let joined = format!("{menemonic}\0{local_name}\0{remote_name}");
        let next = self.branch_winners.len();
        let pos = self
            .branch_winners_order
            .compute_if_absent_with_hash(hash_code, &joined, next);
        if pos == next {
            // new RaceWinners(/* localWins= */ 0, /* remoteWins= */ 0)
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:688 MetricsCollector.DynamicExecutionStats.RaceWinners.RaceWinners
            self.branch_winners.push((key, (0, 0)));
        }
        let value = &mut self.branch_winners[pos].1;
        match winner {
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:701 MetricsCollector.DynamicExecutionStats.RaceWinners.incrementLocalWins
            DynamicMode::Local => value.0 += 1,
            // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:705 MetricsCollector.DynamicExecutionStats.RaceWinners.incrementRemoteWins
            DynamicMode::Remote => value.1 += 1,
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:722 DynamicExecutionStats.toMetrics
    ///
    /// PORT: java-hash-order. The races are listed in the iteration order of the
    /// `ConcurrentHashMap` keyed by the record `RaceIdentifier` (see
    /// [`race_identifier_hash_code`]).
    fn to_metrics(&self) -> DynamicExecutionMetrics {
        let mut builder = DynamicExecutionMetrics::default();
        for index in self.branch_winners_order.iteration_order() {
            let ((mnemonic, local_name, remote_name), (local_wins, remote_wins)) =
                &self.branch_winners[index];
            builder.race_statistics.push(RaceStatistics {
                mnemonic: mnemonic.clone(),
                local_runner: local_name.clone(),
                remote_runner: remote_name.clone(),
                // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:693 MetricsCollector.DynamicExecutionStats.RaceWinners.getLocalWins
                local_wins: *local_wins,
                // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:697 MetricsCollector.DynamicExecutionStats.RaceWinners.getRemoteWins
                remote_wins: *remote_wins,
            });
        }
        builder
    }
}

/// The builders and maps Java keeps in the collector's fields.
#[derive(Debug, Default)]
struct State {
    /// `actionStatsMap`: a `ConcurrentHashMap<String, ActionStats>`; the values are kept in
    /// `action_stats`, the map's iteration order in `action_stats_order`.
    action_stats: Vec<ActionStats>,
    action_stats_order: JavaConcurrentHashMapOrder,
    action_summary: ActionSummary,
    target_metrics: TargetMetrics,
    package_metrics: PackageMetrics,
    timing_metrics: TimingMetrics,
    artifact_metrics: ArtifactMetrics,
    build_graph_metrics: BuildGraphMetrics,
    dynamic_execution_stats: DynamicExecutionStats,
}

impl State {
    /// `actionStatsMap.computeIfAbsent(mnemonic, ActionStats::new)`.
    fn action_stats_for(&mut self, mnemonic: &str) -> &mut ActionStats {
        let next = self.action_stats.len();
        let index = self.action_stats_order.compute_if_absent(mnemonic, next);
        if index == next {
            self.action_stats.push(ActionStats::new(mnemonic));
        }
        &mut self.action_stats[index]
    }
}

/// Collects the build's metrics from its events and builds the `BuildMetrics` proto.
// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:95 MetricsCollector
pub struct MetricsCollector {
    record_metrics_for_all_mnemonics: bool,
    record_skyframe_metrics: bool,
    /// `BlazeClock.instance()`.
    clock: Arc<dyn Clock>,
    // For CumulativeMetrics.
    num_analyses: Arc<AtomicI32>,
    num_builds: Arc<AtomicI32>,
    state: Mutex<State>,
    spawn_stats: SpawnStats,
    // Skymeld-specific: we don't have an ExecutionStartingEvent for skymeld, so we have to use
    // TopLevelTargetExecutionStartedEvent. This AtomicBoolean is so that we only account for the
    // build once.
    build_accounted_for: AtomicBool,
    // Identify when the actual actions execution starts (excluding workspace status actions).
    execution_started: AtomicBool,
}

impl std::fmt::Debug for MetricsCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsCollector")
            .field(
                "record_metrics_for_all_mnemonics",
                &self.record_metrics_for_all_mnemonics,
            )
            .field("record_skyframe_metrics", &self.record_skyframe_metrics)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}

impl MetricsCollector {
    /// `options` is `env.getOptions().getOptions(MetricsModule.Options.class)` (nullable);
    /// `clock` is `BlazeClock.instance()`; `num_analyses` / `num_builds` are the module's
    /// counters, shared by every command of the server.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:124 MetricsCollector.<init>
    fn new(
        options: Option<&MetricsModuleOptions>,
        clock: Arc<dyn Clock>,
        num_analyses: Arc<AtomicI32>,
        num_builds: Arc<AtomicI32>,
    ) -> Self {
        // PORT: env.getEventBus().register(this) is the runtime calling the handlers (see the
        // module docs). WorkerProcessMetricsCollector.instance().setClock(env.getClock()) is
        // skipped: Blaze has no persistent workers.
        MetricsCollector {
            record_metrics_for_all_mnemonics: options
                .is_some_and(|o| o.record_metrics_for_all_mnemonics),
            record_skyframe_metrics: options.is_some_and(|o| o.record_skyframe_metrics),
            clock,
            num_analyses,
            num_builds,
            state: Mutex::new(State::default()),
            spawn_stats: SpawnStats::new(),
            build_accounted_for: AtomicBool::new(false),
            execution_started: AtomicBool::new(false),
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:138 MetricsCollector.installInEnv
    pub fn install_in_env(
        options: Option<&MetricsModuleOptions>,
        clock: Arc<dyn Clock>,
        num_analyses: Arc<AtomicI32>,
        num_builds: Arc<AtomicI32>,
    ) -> MetricsCollector {
        MetricsCollector::new(options, clock, num_analyses, num_builds)
    }

    /// Whether `--experimental_record_skyframe_metrics` is set (then `createBuildMetrics` asks
    /// for the Skyframe stats).
    pub fn records_skyframe_metrics(&self) -> bool {
        self.record_skyframe_metrics
    }

    /// `AnalysisPhaseStartedEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:145 MetricsCollector.logAnalysisStartingEvent
    pub fn log_analysis_starting_event(&self) {
        let _guard = self.state.lock(); // synchronized
        self.num_analyses.fetch_add(1, AtomicOrdering::SeqCst);
    }

    /// `AnalysisPhaseCompleteEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:151 MetricsCollector.onAnalysisPhaseComplete
    pub fn on_analysis_phase_complete(&self, event: &AnalysisPhaseCompleteInfo) {
        let mut state = self.state.lock();
        let actions_constructed = event.actions_constructed;
        state.action_summary.actions_created = actions_constructed.total as i64;
        state.action_summary.actions_created_not_including_aspects =
            actions_constructed.configured_targets_only as i64;
        let targets_configured = event.targets_configured;
        state.target_metrics.targets_configured = targets_configured.total as i64;
        state
            .target_metrics
            .targets_configured_not_including_aspects =
            targets_configured.configured_targets_only as i64;
        state.timing_metrics.analysis_phase_time_in_ms = event.time_in_ms;

        state.package_metrics.packages_loaded = event.packages_successfully_loaded as i64;

        if let Some(recorder) = &event.package_metrics_recorder {
            let mut metrics: Vec<PackageLoadMetrics> = recorder.package_load_metrics.clone();

            if let Some(num_packages_to_track) = recorder.only_extremes_num_packages_to_track {
                // Safeguard: we have 5 metrics, so print at most 5 times the number of packages as
                // being tracked per metric.
                let limit = 5i64 * num_packages_to_track as i64;
                // Stream.limit throws IllegalArgumentException for a negative size.
                blaze_util::check_argument!(limit >= 0, "{}", limit);
                metrics.truncate(limit as usize);
            }
            state.package_metrics.package_load_metrics.extend(metrics);
        }

        for (mnemonic, count) in &event.actions_constructed_by_mnemonic {
            let action_stats = state.action_stats_for(mnemonic);
            action_stats.num_actions_registered += *count as i64;
        }
    }

    /// `AnalysisGraphStatsEvent`: `event.getBuildGraphMetrics()`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:191 MetricsCollector.logAnalysisGraphStats
    pub fn log_analysis_graph_stats(&self, build_graph_metrics: &BuildGraphMetrics) {
        let mut state = self.state.lock();
        // Check only one event per build. No proto3 check for presence, so check for not-default
        // value.
        if state.build_graph_metrics.action_lookup_value_count > 0 {
            // PORT: the two messages are printed with Debug where Java uses the protos'
            // `toString` (text format).
            blaze_util::bug_report::send_bug_report(&format!(
                "Already initialized build graph metrics builder: {:?}, {:?}",
                state.build_graph_metrics, build_graph_metrics
            ));
        }
        merge_build_graph_metrics(&mut state.build_graph_metrics, build_graph_metrics);
    }

    /// `ExecutionStartingEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:206 MetricsCollector.logExecutionStartingEvent
    pub fn log_execution_starting_event(&self) {
        let _guard = self.state.lock(); // synchronized
        self.num_builds.fetch_add(1, AtomicOrdering::SeqCst);
    }

    /// `TopLevelTargetPendingExecutionEvent` (Skymeld-specific: there is no
    /// `ExecutionStartingEvent` for Skymeld).
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:213 MetricsCollector.handleExecutionPhaseStart
    pub fn handle_execution_phase_start(&self) {
        let _guard = self.state.lock(); // synchronized
        if self
            .build_accounted_for
            .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
            .is_ok()
        {
            self.num_builds.fetch_add(1, AtomicOrdering::SeqCst);
        }
    }

    /// `SomeExecutionStartedEvent`: `event.countedInExecutionTime()`;
    /// `profile_elapsed_time_ms` is `Profiler.getProfileElapsedTime()` at this point (nullable).
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:221 MetricsCollector.onSomeExecutionStarted
    pub fn on_some_execution_started(
        &self,
        counted_in_execution_time: bool,
        profile_elapsed_time_ms: Option<i64>,
    ) {
        if counted_in_execution_time
            && self
                .execution_started
                .compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst)
                .is_ok()
            && let Some(elapsed_wall_time) = profile_elapsed_time_ms
        {
            self.state
                .lock()
                .timing_metrics
                .actions_execution_start_in_ms = elapsed_wall_time;
        }
    }

    /// `ExecutionPhaseCompleteEvent`: `event.getTimeInMs()`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:233 MetricsCollector.handleExecutionPhaseComplete
    pub fn handle_execution_phase_complete(&self, time_in_ms: i64) {
        self.state.lock().timing_metrics.execution_phase_time_in_ms = time_in_ms;
    }

    /// `ActionCompletionEvent`: `event.getAction().getMnemonic()` and
    /// `event.getRelativeActionStartTimeNanos()` (a `BlazeClock.nanoTime()` reading).
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:240 MetricsCollector.onActionComplete
    pub fn on_action_complete(&self, mnemonic: &str, relative_action_start_time_nanos: i64) {
        {
            let mut state = self.state.lock();
            let now = self.clock.nano_time();
            let action_stats = state.action_stats_for(mnemonic);
            action_stats.num_actions_executed += 1;
            action_stats.first_started = action_stats
                .first_started
                .min(relative_action_start_time_nanos);
            action_stats.last_ended = action_stats.last_ended.max(now);
        }
        self.spawn_stats.increment_action_count();
    }

    /// `ActionResultReceivedEvent`: `event.getAction().getMnemonic()` and
    /// `event.getActionResult()`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:251 MetricsCollector.actionResultReceived
    pub fn action_result_received(&self, mnemonic: &str, action_result: &ActionResult) {
        self.spawn_stats
            .count_action_result(action_result.spawn_results());
        let mut state = self.state.lock();
        let action_stats = state.action_stats_for(mnemonic);
        let system_time = cumulative_command_execution_system_time_in_ms(action_result);
        if system_time > 0 {
            action_stats.system_time += system_time as i64;
        }
        let user_time = cumulative_command_execution_user_time_in_ms(action_result);
        if user_time > 0 {
            action_stats.user_time += user_time as i64;
        }
    }

    /// `ExecutionFinishedEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:267 MetricsCollector.onExecutionComplete
    pub fn on_execution_complete(&self, event: &ExecutionFinishedInfo) {
        let mut state = self.state.lock();
        let artifact_metrics = &mut state.artifact_metrics;
        artifact_metrics.source_artifacts_read = Some(event.source_artifacts_read);
        artifact_metrics.output_artifacts_seen = Some(event.output_artifacts_seen);
        artifact_metrics.output_artifacts_from_action_cache =
            Some(event.output_artifacts_from_action_cache);
        artifact_metrics.top_level_artifacts = Some(event.top_level_artifacts);
    }

    /// `DynamicExecutionFinishedEvent`: `getMnemonic()`, `getLocalBranchName()`,
    /// `getRemoteBranchName()`, `getWinnerBranchType()`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:277 MetricsCollector.onDynamicExecutionFinishedEvent
    pub fn on_dynamic_execution_finished_event(
        &self,
        mnemonic: &str,
        local_branch_name: &str,
        remote_branch_name: &str,
        winner_branch_type: DynamicMode,
    ) {
        self.state.lock().dynamic_execution_stats.update(
            mnemonic,
            local_branch_name,
            remote_branch_name,
            winner_branch_type,
        );
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:285 MetricsCollector.toEvaluationStats
    fn to_evaluation_stats(map: &[(String, i32)]) -> Vec<EvaluationStat> {
        map.iter()
            .map(|(name, count)| EvaluationStat {
                skyfunction_name: name.clone(),
                count: *count as i64,
            })
            .collect()
    }

    /// `SkyframeGraphStatsEvent`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:299 MetricsCollector.onSkyframeGraphStats
    pub fn on_skyframe_graph_stats(&self, event: &SkyframeGraphStatsInfo) {
        let mut state = self.state.lock();
        let build_graph_metrics = &mut state.build_graph_metrics;
        build_graph_metrics
            .dirtied_values
            .extend(Self::to_evaluation_stats(&event.dirtied));
        build_graph_metrics
            .changed_values
            .extend(Self::to_evaluation_stats(&event.changed));
        build_graph_metrics
            .built_values
            .extend(Self::to_evaluation_stats(&event.built));
        build_graph_metrics
            .cleaned_values
            .extend(Self::to_evaluation_stats(&event.cleaned));
        build_graph_metrics
            .evaluated_values
            .extend(Self::to_evaluation_stats(&event.evaluated));
        build_graph_metrics.post_invocation_skyframe_node_count = event.graph_size;
    }

    /// `CommandPrecompleteEvent`. This needs to be done in `CommandPrecompleteEvent` because the
    /// metrics are reported on the BEP, which is closed in `BlazeModule.afterCommand()`. Java
    /// posts the returned event on the command's event bus; the runtime must post it.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:313 MetricsCollector.onCommandPrecompleteEvent
    pub fn on_command_precomplete_event(
        &self,
        inputs: BuildMetricsFinishInputs<'_>,
    ) -> BuildMetricsEvent {
        BuildMetricsEvent::new(self.create_build_metrics(inputs))
    }

    /// `CriticalPathEvent`: `event.getCriticalPath().getAggregatedElapsedTime().toNanos()`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:318 MetricsCollector.onCriticalPath
    pub fn on_critical_path(&self, aggregated_elapsed_time_nanos: i64) {
        let proto_duration = duration_from_nanos(aggregated_elapsed_time_nanos);
        self.state.lock().timing_metrics.critical_path_time = Some(proto_duration);
    }

    /// `PostableActionCacheStats`.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:326 MetricsCollector.logActionCacheStatistics
    pub fn log_action_cache_statistics(&self, stats: &PostableActionCacheStats) {
        self.state.lock().action_summary.action_cache_statistics = Some(stats.as_proto().clone());
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:330 MetricsCollector.createBuildMetrics
    fn create_build_metrics(&self, inputs: BuildMetricsFinishInputs<'_>) -> BuildMetrics {
        let worker_process_metrics = inputs.worker_process_metrics;
        // Restrict the number of WorkerMetrics that we report based on a predefined prioritization
        // so that we don't spam the BEP in the event that something like a kill-create cycle
        // happens.
        let worker_metrics: Vec<WorkerMetrics> =
            WorkerProcessMetricsCollector::limit_worker_metrics_to_publish(
                inputs.worker_metrics,
                MAX_PUBLISHED_WORKER_METRICS,
            );

        let mut state = self.state.lock();
        self.add_skyframe_stats(
            &mut state.build_graph_metrics,
            inputs.has_pending_exception,
            inputs.skyframe_stats,
        );

        let remote_analysis_cache_statistics =
            Self::collect_remote_analysis_cache_stats(&inputs.remote_analysis_caching);

        let action_summary = self.finish_action_summary(&mut state);
        let memory_metrics = Self::create_memory_metrics(&inputs.memory);
        let target_metrics = state.target_metrics;
        let package_metrics = state.package_metrics.clone();
        let timing_metrics = Self::finish_timing_metrics(
            &mut state,
            inputs.profile_elapsed_time_ms,
            inputs.server_process_cpu_time_ms,
        );
        let cumulative_metrics = self.create_cumulative_metrics();
        let artifact_metrics = state.artifact_metrics;
        let build_graph_metrics = state.build_graph_metrics.clone();
        let worker_pool_metrics = Self::create_worker_pool_metrics(&worker_process_metrics);
        let dynamic_execution_metrics = state.dynamic_execution_stats.to_metrics();

        let mut build_metrics = BuildMetrics {
            action_summary: Some(action_summary),
            memory_metrics: Some(memory_metrics),
            target_metrics: Some(target_metrics),
            package_metrics: Some(package_metrics),
            timing_metrics: Some(timing_metrics),
            cumulative_metrics: Some(cumulative_metrics),
            artifact_metrics: Some(artifact_metrics),
            build_graph_metrics: Some(build_graph_metrics),
            worker_metrics,
            network_metrics: None,
            worker_pool_metrics: Some(worker_pool_metrics),
            dynamic_execution_metrics: Some(dynamic_execution_metrics),
            remote_analysis_cache_statistics: Some(remote_analysis_cache_statistics),
        };

        if let Some(network_metrics) = inputs.network_metrics {
            build_metrics.network_metrics = Some(network_metrics);
        }

        build_metrics
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:369 MetricsCollector.collectRemoteAnalysisCacheStats
    fn collect_remote_analysis_cache_stats(
        facts: &RemoteAnalysisCachingFacts,
    ) -> RemoteAnalysisCacheStatistics {
        let mut result = RemoteAnalysisCacheStatistics {
            cache_hits: facts.cache_hits,
            cache_misses: facts.cache_misses,
            ..Default::default()
        };

        if let Some(fvs_stats) = &facts.fingerprint_value_store_stats {
            result.value_store_value_bytes_received = fvs_stats.value_bytes_received;
            result.value_store_value_bytes_sent = fvs_stats.value_bytes_sent;
            result.value_store_key_bytes_sent = fvs_stats.key_bytes_sent;
            result.value_store_write_ops = fvs_stats.entries_written;
            result.value_store_read_ops_successful = fvs_stats.entries_found;
            result.value_store_read_ops_not_found = fvs_stats.entries_not_found;
            result.value_store_read_batches = fvs_stats.get_batches;
            result.value_store_write_batches = fvs_stats.set_batches;
        }

        if let Some(racc_stats) = &facts.remote_analysis_cache_stats {
            result.analysis_cache_bytes_received = racc_stats.bytes_received;
            result.analysis_cache_key_bytes_sent = racc_stats.bytes_sent;
            result.analysis_cache_ops = racc_stats.requests_sent;
            result.analysis_cache_batches = racc_stats.batches;
        }

        result
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:402 MetricsCollector.buildActionData
    fn build_action_data(&self, action_stats: &ActionStats) -> ActionData {
        // BlazeClock.createNanosToMillisSinceEpochConverter()
        // Java: com/google/devtools/build/lib/clock/BlazeClock.java:63 BlazeClock.createNanosToMillisSinceEpochConverter
        let now_in_millis = self.clock.current_time_millis();
        let now_in_nanos = self.clock.nano_time();
        let to_epoch_millis = |time_nanos: i64| -> i64 {
            // TimeUnit.NANOSECONDS.toMillis truncates toward zero.
            now_in_millis - (now_in_nanos.wrapping_sub(time_nanos) / 1_000_000)
        };
        let num_actions_executed = action_stats.num_actions_executed;
        let mut builder = ActionData {
            mnemonic: action_stats.mnemonic.clone(),
            actions_executed: num_actions_executed,
            actions_created: action_stats.num_actions_registered,
            ..Default::default()
        };

        if num_actions_executed > 0 {
            builder.first_started_ms = to_epoch_millis(action_stats.first_started);
            builder.last_ended_ms = to_epoch_millis(action_stats.last_ended);
        }

        let system_time = action_stats.system_time;
        if system_time > 0 {
            builder.system_time = Some(duration_from_millis(system_time));
        }
        let user_time = action_stats.user_time;
        if user_time > 0 {
            builder.user_time = Some(duration_from_millis(user_time));
        }
        builder
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:433 MetricsCollector.finishActionSummary
    fn finish_action_summary(&self, state: &mut State) -> ActionSummary {
        // actionStatsMap.values().stream(): the ConcurrentHashMap's iteration order.
        let mut action_stats_stream: Vec<usize> = state.action_stats_order.iteration_order();

        if !self.record_metrics_for_all_mnemonics {
            // Stream.sorted is stable for ordered streams.
            action_stats_stream.sort_by_key(|&i| -state.action_stats[i].num_actions_executed);
            action_stats_stream.truncate(MAX_ACTION_DATA);
        }

        for i in action_stats_stream {
            let action_data = self.build_action_data(&state.action_stats[i]);
            state.action_summary.action_data.push(action_data);
        }

        let spawn_summary = self.spawn_stats.get_summary();
        state.action_summary.actions_executed =
            spawn_summary.get("total").copied().unwrap_or(0) as i64;
        for (name, count) in &spawn_summary {
            let mut builder = RunnerCount {
                name: name.clone(),
                count: *count,
                ..Default::default()
            };
            if let Some(exec_kind) = self.spawn_stats.get_exec_kind_for(name) {
                builder.exec_kind = exec_kind;
            }
            state.action_summary.runner_count.push(builder);
        }
        state.action_summary.clone()
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:342 MetricsCollector.addSkyframeStats
    fn add_skyframe_stats(
        &self,
        builder: &mut BuildGraphMetrics,
        has_pending_exception: bool,
        get_skyframe_stats: &dyn Fn() -> Option<SkyframeStats>,
    ) {
        // short-circuit if not requested
        if !self.record_skyframe_metrics {
            return;
        }

        // NOTE: This can potentially unintentionally consume a pending Exception by
        // calling getSkyframeStats, with our Reporter which ends up consuming the
        // analysis failure unintentionally.  So if our CommandEnvironment has a
        // pending exception, don't touch the Skyframe executor.
        if has_pending_exception {
            return;
        }

        // getSkyframeStats return Nullable for unsupported implementations, so
        // ensure we get stats before proceeding.
        let Some(skyframe_stats) = get_skyframe_stats() else {
            return;
        };

        for a in &skyframe_stats.rule_stats {
            builder.rule_class.push(RuleClassCount {
                key: a.key.clone(),
                rule_class: a.name.clone(),
                count: a.count as u64,
                action_count: a.action_count as u64,
            });
        }
        for a in &skyframe_stats.aspect_stats {
            builder.aspect.push(AspectCount {
                key: a.key.clone(),
                aspect_name: a.name.clone(),
                count: a.count as u64,
                action_count: a.action_count as u64,
            });
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:506 MetricsCollector.createMemoryMetrics
    fn create_memory_metrics(memory: &JvmMemoryFacts) -> MemoryMetrics {
        let mut memory_metrics = MemoryMetrics::default();
        if memory.heap_used_memory_at_finish > 0 {
            memory_metrics.used_heap_size_post_build = memory.heap_used_memory_at_finish;
        }
        if let Some(bytes) = memory.peak_post_gc_heap {
            memory_metrics.peak_post_gc_heap_size = bytes;
        }

        if memory_metrics.peak_post_gc_heap_size < memory_metrics.used_heap_size_post_build {
            // If we just did a GC and computed the heap size, update the one we got from the GC
            // notification (which may arrive too late for this specific GC).
            memory_metrics.peak_post_gc_heap_size = memory_metrics.used_heap_size_post_build;
        }

        if let Some(bytes) = memory.peak_post_gc_heap_tenured_space {
            memory_metrics.peak_post_gc_tenured_space_heap_size = bytes;
        }

        for (type_, garbage_collected) in &memory.garbage_stats {
            memory_metrics.garbage_metrics.push(GarbageMetrics {
                r#type: type_.clone(),
                garbage_collected: *garbage_collected,
            });
        }

        memory_metrics
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:537 MetricsCollector.createCumulativeMetrics
    fn create_cumulative_metrics(&self) -> CumulativeMetrics {
        CumulativeMetrics {
            num_analyses: self.num_analyses.load(AtomicOrdering::SeqCst),
            num_builds: self.num_builds.load(AtomicOrdering::SeqCst),
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:544 MetricsCollector.finishTimingMetrics
    fn finish_timing_metrics(
        state: &mut State,
        profile_elapsed_time_ms: Option<i64>,
        server_process_cpu_time_ms: Option<i64>,
    ) -> TimingMetrics {
        if let Some(elapsed_wall_time) = profile_elapsed_time_ms {
            state.timing_metrics.wall_time_in_ms = elapsed_wall_time;
        }
        if let Some(cpu_time) = server_process_cpu_time_ms {
            state.timing_metrics.cpu_time_in_ms = cpu_time;
        }
        state.timing_metrics
    }

    /// Creates the WorkerPoolMetrics by aggregating the collected WorkerProcessMetrics.
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:557 MetricsCollector.createWorkerPoolMetrics
    pub fn create_worker_pool_metrics(
        collected_worker_process_metrics: &[WorkerProcessMetricsInfo],
    ) -> WorkerPoolMetrics {
        // HashMap<Integer, WorkerPoolStats>: values kept in insertion order here, iterated in
        // java.util.HashMap order below.
        let mut keys: Vec<i32> = Vec::new();
        let mut aggregated_pool_stats: Vec<WorkerPoolStats> = Vec::new();
        for wpm in collected_worker_process_metrics {
            let pos = match keys.iter().position(|&k| k == wpm.worker_key_hash) {
                Some(pos) => pos,
                None => {
                    keys.push(wpm.worker_key_hash);
                    aggregated_pool_stats
                        .push(WorkerPoolStats::new(&wpm.mnemonic, wpm.worker_key_hash));
                    keys.len() - 1
                }
            };
            aggregated_pool_stats[pos].update(wpm);
        }
        WorkerPoolMetrics {
            worker_pool_stats: java_integer_hash_map_order(&keys)
                .into_iter()
                .map(|i| aggregated_pool_stats[i].build())
                .collect(),
        }
    }
}

/// `ActionResult.cumulativeCommandExecutionSystemTimeInMs()`.
// Java: com/google/devtools/build/lib/actions/ActionResult.java ActionResult.cumulativeCommandExecutionSystemTimeInMs
fn cumulative_command_execution_system_time_in_ms(action_result: &ActionResult) -> i32 {
    // getCumulativeInt(SpawnResult::getSystemTimeInMs)
    action_result.spawn_results().iter().fold(0i32, |total, r| {
        total.wrapping_add(r.get_system_time_in_ms())
    })
}

/// `ActionResult.cumulativeCommandExecutionUserTimeInMs()`.
// Java: com/google/devtools/build/lib/actions/ActionResult.java ActionResult.cumulativeCommandExecutionUserTimeInMs
fn cumulative_command_execution_user_time_in_ms(action_result: &ActionResult) -> i32 {
    // getCumulativeInt(SpawnResult::getUserTimeInMs)
    action_result
        .spawn_results()
        .iter()
        .fold(0i32, |total, r| total.wrapping_add(r.get_user_time_in_ms()))
}

/// `buildGraphMetrics.mergeFrom(other)`: proto3 `mergeFrom` sets each scalar field of `other`
/// that is not the default and appends each repeated field.
fn merge_build_graph_metrics(builder: &mut BuildGraphMetrics, other: &BuildGraphMetrics) {
    macro_rules! merge_scalar {
        ($($field:ident),*) => {
            $(
                if other.$field != 0 {
                    builder.$field = other.$field;
                }
            )*
        };
    }
    merge_scalar!(
        action_lookup_value_count,
        action_lookup_value_count_not_including_aspects,
        action_count,
        action_count_not_including_aspects,
        input_file_configured_target_count,
        output_file_configured_target_count,
        other_configured_target_count,
        output_artifact_count,
        post_invocation_skyframe_node_count
    );
    builder
        .dirtied_values
        .extend(other.dirtied_values.iter().cloned());
    builder
        .changed_values
        .extend(other.changed_values.iter().cloned());
    builder
        .built_values
        .extend(other.built_values.iter().cloned());
    builder
        .cleaned_values
        .extend(other.cleaned_values.iter().cloned());
    builder
        .evaluated_values
        .extend(other.evaluated_values.iter().cloned());
    builder.rule_class.extend(other.rule_class.iter().cloned());
    builder.aspect.extend(other.aspect.iter().cloned());
}

// Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:574 MetricsCollector.WorkerPoolStats
#[derive(Debug)]
struct WorkerPoolStats {
    created_count: i32,
    destroyed_count: i32,
    evicted_count: i32,
    user_exec_exception_destroyed_count: i32,
    io_exception_destroyed_count: i32,
    interrupted_exception_destroyed_count: i32,
    unknown_destroyed_count: i32,
    alive_count: i32,
    mnemonic: String,
    hash: i32,
}

impl WorkerPoolStats {
    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:587 WorkerPoolStats.<init>
    fn new(mnemonic: &str, hash: i32) -> Self {
        WorkerPoolStats {
            created_count: 0,
            destroyed_count: 0,
            evicted_count: 0,
            user_exec_exception_destroyed_count: 0,
            io_exception_destroyed_count: 0,
            interrupted_exception_destroyed_count: 0,
            unknown_destroyed_count: 0,
            alive_count: 0,
            mnemonic: mnemonic.to_string(),
            hash,
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:592 WorkerPoolStats.update
    fn update(&mut self, wpm: &WorkerProcessMetricsInfo) {
        let num_workers = wpm.worker_ids.len() as i32;
        if wpm.newly_created {
            self.created_count += num_workers;
        }
        let status = wpm.status;
        if status.is_killed() {
            match status {
                // If the process is killed due to a specific reason, we attribute the cause to all
                // workers of that process (plural in the case of multiplex workers).
                WorkerProcessStatusKind::KilledUnknown => {
                    self.unknown_destroyed_count += num_workers
                }
                WorkerProcessStatusKind::KilledDueToInterruptedException => {
                    self.interrupted_exception_destroyed_count += num_workers
                }
                WorkerProcessStatusKind::KilledDueToIoException => {
                    self.io_exception_destroyed_count += num_workers
                }
                WorkerProcessStatusKind::KilledDueToMemoryPressure => {
                    self.evicted_count += num_workers
                }
                WorkerProcessStatusKind::KilledDueToUserExecException => {
                    self.user_exec_exception_destroyed_count += num_workers
                }
                _ => {}
            }
            self.destroyed_count += num_workers;
        } else {
            self.alive_count += num_workers;
        }
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsCollector.java:617 WorkerPoolStats.build
    fn build(&self) -> WorkerPoolStatsProto {
        WorkerPoolStatsProto {
            mnemonic: self.mnemonic.clone(),
            hash: self.hash,
            created_count: self.created_count as i64,
            destroyed_count: self.destroyed_count as i64,
            evicted_count: self.evicted_count as i64,
            user_exec_exception_destroyed_count: self.user_exec_exception_destroyed_count as i64,
            io_exception_destroyed_count: self.io_exception_destroyed_count as i64,
            interrupted_exception_destroyed_count: self.interrupted_exception_destroyed_count
                as i64,
            unknown_destroyed_count: self.unknown_destroyed_count as i64,
            alive_count: self.alive_count as i64,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Java hash-map iteration orders
// ---------------------------------------------------------------------------------------------

/// `String.hashCode()`: over UTF-16 code units, wrapping.
fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

// java.util.concurrent.ConcurrentHashMap constants.
const CHM_DEFAULT_CAPACITY: usize = 16;
const CHM_HASH_BITS: i32 = 0x7fff_ffff;
const CHM_TREEIFY_THRESHOLD: usize = 8;

/// The iteration order of a `java.util.concurrent.ConcurrentHashMap<String, V>` filled only by
/// `computeIfAbsent` without contention: the table, its bins (linked lists) and the
/// `transfer` split on resize, which keeps the run of nodes after `lastRun` and prepends the
/// others (reversing them).
///
/// PORT: java-hash-order. `ActionSummary.actionData` lists `actionStatsMap.values()` in this
/// order (stably sorted by executed count), so it reaches the BEP.
#[derive(Debug, Default)]
struct JavaConcurrentHashMapOrder {
    /// Bins of `(spread hash, key, value index)`; empty until `initTable`.
    table: Vec<Vec<(i32, String, usize)>>,
    count: i64,
    size_ctl: i64,
}

impl JavaConcurrentHashMapOrder {
    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.spread
    fn spread(h: i32) -> i32 {
        (h ^ ((h as u32) >> 16) as i32) & CHM_HASH_BITS
    }

    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.initTable
    fn init_table(&mut self) {
        let n = if self.size_ctl > 0 {
            self.size_ctl as usize
        } else {
            CHM_DEFAULT_CAPACITY
        };
        self.table = (0..n).map(|_| Vec::new()).collect();
        self.size_ctl = (n - (n >> 2)) as i64;
    }

    /// `computeIfAbsent(key, k -> new value)`: returns the index of the existing value, or
    /// `new_index` after inserting the key.
    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.computeIfAbsent
    fn compute_if_absent(&mut self, key: &str, new_index: usize) -> usize {
        self.compute_if_absent_with_hash(java_string_hash(key), key, new_index)
    }

    /// [`Self::compute_if_absent`] for a key whose `hashCode()` is `hash_code` and whose
    /// `equals` is the equality of `key`.
    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.computeIfAbsent
    fn compute_if_absent_with_hash(
        &mut self,
        hash_code: i32,
        key: &str,
        new_index: usize,
    ) -> usize {
        let h = Self::spread(hash_code);
        if self.table.is_empty() {
            self.init_table();
        }
        let n = self.table.len();
        let i = (n - 1) & (h as usize);
        let bin = &mut self.table[i];
        let bin_count;
        if bin.is_empty() {
            bin.push((h, key.to_string(), new_index));
            bin_count = 1;
        } else if bin[0].0 == h && bin[0].1 == key {
            // check first node without acquiring lock
            return bin[0].2;
        } else {
            if let Some(pos) = bin.iter().position(|(eh, ek, _)| *eh == h && ek == key) {
                if pos + 1 >= CHM_TREEIFY_THRESHOLD {
                    blaze_util::unported!("ConcurrentHashMap.treeifyBin");
                }
                return bin[pos].2;
            }
            bin_count = bin.len();
            bin.push((h, key.to_string(), new_index));
            if bin_count >= CHM_TREEIFY_THRESHOLD {
                blaze_util::unported!("ConcurrentHashMap.treeifyBin");
            }
        }
        self.add_count(1, bin_count);
        new_index
    }

    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.addCount
    fn add_count(&mut self, x: i64, _check: usize) {
        self.count += x;
        // check >= 0
        while self.count >= self.size_ctl && !self.table.is_empty() && self.table.len() < (1 << 30)
        {
            self.transfer();
        }
    }

    // Java: java/util/concurrent/ConcurrentHashMap.java ConcurrentHashMap.transfer
    fn transfer(&mut self) {
        let n = self.table.len();
        let mut next_tab: Vec<Vec<(i32, String, usize)>> =
            (0..(n << 1)).map(|_| Vec::new()).collect();
        let old = std::mem::take(&mut self.table);
        for (i, bin) in old.into_iter().enumerate() {
            if bin.is_empty() {
                continue;
            }
            let bit = |node: &(i32, String, usize)| (node.0 as usize) & n;
            let mut run_bit = bit(&bin[0]);
            let mut last_run = 0;
            for (p, node) in bin.iter().enumerate().skip(1) {
                let b = bit(node);
                if b != run_bit {
                    run_bit = b;
                    last_run = p;
                }
            }
            let mut nodes = bin;
            let tail: Vec<_> = nodes.split_off(last_run);
            let (mut ln, mut hn) = if run_bit == 0 {
                (tail, Vec::new())
            } else {
                (Vec::new(), tail)
            };
            for node in nodes {
                // new Node<K,V>(ph, pk, pv, ln): prepend.
                if bit(&node) == 0 {
                    ln.insert(0, node);
                } else {
                    hn.insert(0, node);
                }
            }
            next_tab[i] = ln;
            next_tab[i + n] = hn;
        }
        self.table = next_tab;
        self.size_ctl = ((n << 1) - (n >> 1)) as i64;
    }

    /// The value indices in iteration order (`Traverser`: bins by increasing index, each bin in
    /// list order).
    fn iteration_order(&self) -> Vec<usize> {
        self.table
            .iter()
            .flat_map(|bin| bin.iter().map(|(_, _, index)| *index))
            .collect()
    }
}

/// The iteration order of a `java.util.HashMap<Integer, V>` (default capacity and load factor)
/// whose keys were inserted in the order of `keys` (distinct): bins by increasing index under
/// the final capacity, each bin in insertion order (`resize` keeps the relative order).
/// Returns indices into `keys`.
///
/// PORT: java-hash-order.
fn java_integer_hash_map_order(keys: &[i32]) -> Vec<usize> {
    // Java: java/util/HashMap.java HashMap.hash, HashMap.putVal, HashMap.resize
    let hash = |k: i32| (k ^ ((k as u32) >> 16) as i32) as u32 as usize;
    let mut capacity: usize = 16;
    let mut threshold: usize = 12;
    for (i, &key) in keys.iter().enumerate() {
        let index = hash(key) & (capacity - 1);
        let bin_len = keys[..i]
            .iter()
            .filter(|&&k| hash(k) & (capacity - 1) == index)
            .count();
        if bin_len >= 8 {
            blaze_util::unported!("HashMap.treeifyBin");
        }
        // if (++size > threshold) resize();
        let size = i + 1;
        if size > threshold {
            capacity <<= 1;
            threshold <<= 1;
        }
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by_key(|&i| hash(keys[i]) & (capacity - 1));
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_hash_map_order_matches_brotli_oracle() {
        // The mnemonics of the brotli build; at 13 entries the table has grown to 32.
        let mnemonics = [
            "CppCompile",
            "Symlink",
            "CppLink",
            "CppArchive",
            "RepoMappingManifest",
            "SymlinkTree",
            "SourceSymlinkManifest",
            "BazelWorkspaceStatusAction",
            "RunfilesTree",
            "TranslateBuildInfo",
            "CcStrip",
            "FileWrite",
            "SolibSymlink",
        ];
        let mut map = JavaConcurrentHashMapOrder::default();
        for (i, m) in mnemonics.iter().enumerate() {
            assert_eq!(map.compute_if_absent(m, i), i);
        }
        assert_eq!(map.compute_if_absent("CppLink", 99), 2);
        assert_eq!(map.table.len(), 32);
        let order: Vec<&str> = map
            .iteration_order()
            .into_iter()
            .map(|i| mnemonics[i])
            .collect();
        assert_eq!(
            order,
            vec![
                "TranslateBuildInfo",
                "CcStrip",
                "RepoMappingManifest",
                "FileWrite",
                "SolibSymlink",
                "CppArchive",
                "SymlinkTree",
                "Symlink",
                "CppLink",
                "SourceSymlinkManifest",
                "BazelWorkspaceStatusAction",
                "RunfilesTree",
                "CppCompile",
            ]
        );
    }

    #[test]
    fn integer_hash_map_order() {
        assert_eq!(java_integer_hash_map_order(&[200, 100]), vec![1, 0]);
        assert_eq!(java_integer_hash_map_order(&[17, 1, 16]), vec![2, 0, 1]);
    }
}

#[cfg(test)]
mod dynamic_execution_stats_tests {
    use super::*;

    /// Expected values from a JVM (openjdk 25): `ConcurrentHashMap.compute` of the record
    /// `RaceIdentifier(mnemonic, localName, remoteName)` with the same calls, then
    /// `keySet()` in iteration order (program `RaceOrder.java` of task ea-4).
    #[test]
    fn races_are_listed_in_the_concurrent_hash_map_order() {
        let races = [
            ("CppCompile", "sandboxed", "remote"),
            ("Javac", "worker", "remote"),
            ("Genrule", "local", "remote"),
            ("CppLink", "sandboxed", "remote"),
            ("CppCompile", "local", "remote"),
            ("GoCompile", "sandboxed", "grpc"),
        ];
        let winners = [0, 1, 0, 1, 1, 0];
        let mut stats = DynamicExecutionStats::default();
        for r in 0..3 {
            for (i, (mnemonic, local, remote)) in races.iter().enumerate() {
                let winner = if winners[(i + r) % winners.len()] == 0 {
                    DynamicMode::Local
                } else {
                    DynamicMode::Remote
                };
                stats.update(mnemonic, local, remote, winner);
            }
        }
        let got: Vec<(String, String, String, i32, i32)> = stats
            .to_metrics()
            .race_statistics
            .into_iter()
            .map(|r| {
                (
                    r.mnemonic,
                    r.local_runner,
                    r.remote_runner,
                    r.local_wins,
                    r.remote_wins,
                )
            })
            .collect();
        let expected = [
            ("CppCompile", "sandboxed", "remote", 2, 1),
            ("CppCompile", "local", "remote", 2, 1),
            ("CppLink", "sandboxed", "remote", 1, 2),
            ("Javac", "worker", "remote", 1, 2),
            ("Genrule", "local", "remote", 1, 2),
            ("GoCompile", "sandboxed", "grpc", 2, 1),
        ];
        let expected: Vec<(String, String, String, i32, i32)> = expected
            .iter()
            .map(|(m, l, r, lw, rw)| (m.to_string(), l.to_string(), r.to_string(), *lw, *rw))
            .collect();
        assert_eq!(got, expected);
    }
}
