//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java.
//!
//! PORT: Java mocks `PsInfoCollector` and `CgroupsInfoCollector` and stubs the spy's
//! `collectResourceUsage()`; here the collector is built with fake collectors
//! (`WorkerProcessMetricsCollector::with_collectors`) and the stub is
//! `set_resource_usage_for_testing`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use blaze_events::Clock;
use blaze_exec::sandbox::cgroups::Cgroup;
use blaze_exec::worker::worker_process_metrics::{
    CgroupsInfoCollectorApi, PsInfoCollectorApi, ResourceSnapshot, WorkerProcessMetrics,
    WorkerProcessMetricsCollector, worker_metrics_publish_comparator,
};
use blaze_exec::worker::worker_process_status::{Status, WorkerProcessStatus};
use blaze_protos::build_event_stream::build_metrics::WorkerMetrics;
use blaze_protos::build_event_stream::build_metrics::worker_metrics::{WorkerStats, WorkerStatus};
use blaze_shell::util::os::OS;

const WORKER_ID_1: i32 = 1;
const WORKER_ID_2: i32 = 2;
const WORKER_ID_3: i32 = 3;
const WORKER_ID_4: i32 = 4;
const WORKER_ID_5: i32 = 5;
const PROCESS_ID_1: i64 = 100;
const PROCESS_ID_2: i64 = 200;
const PROCESS_ID_3: i64 = 300;
const PROCESS_ID_4: i64 = 400;
const PROCESS_ID_5: i64 = 500;
const WORKER_KEY_HASH_1: i32 = 1;
const WORKER_KEY_HASH_2: i32 = 2;
const WORKER_KEY_HASH_3: i32 = 3;
const WORKER_KEY_HASH_4: i32 = 4;
const WORKER_KEY_HASH_5: i32 = 5;
const JAVAC_MNEMONIC: &str = "Javac";
const CPP_COMPILE_MNEMONIC: &str = "CppCompile";
const PROTO_MNEMONIC: &str = "Proto";

const DEFAULT_CLOCK_START_TIME: i64 = 0;

/// `ManualClock` of the Java test.
struct ManualClock {
    current_time: AtomicI64,
}

impl ManualClock {
    fn set_time(&self, current_time: i64) {
        self.current_time.store(current_time, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.current_time.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        panic!("unexpected method call")
    }
}

/// `when(collector.collectResourceUsage(any(), any())).thenReturn(snapshot)`.
struct FakePs(Option<ResourceSnapshot>);

impl PsInfoCollectorApi for FakePs {
    fn collect_resource_usage(&self, _ids: &[i64], _clock: &dyn Clock) -> ResourceSnapshot {
        self.0.clone().expect("unstubbed mock returns null")
    }
}

struct FakeCgroups(Option<ResourceSnapshot>);

impl CgroupsInfoCollectorApi for FakeCgroups {
    fn collect_resource_usage(
        &self,
        _pid_to_cgroups: &BTreeMap<i64, Arc<dyn Cgroup>>,
        _clock: &dyn Clock,
    ) -> ResourceSnapshot {
        self.0.clone().expect("unstubbed mock returns null")
    }
}

struct Fixture {
    clock: Arc<ManualClock>,
    spy_collector: WorkerProcessMetricsCollector,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:57 WorkerProcessMetricsCollectorTest.setUp
fn set_up_with(ps: Option<ResourceSnapshot>, cgroups: Option<ResourceSnapshot>) -> Fixture {
    let clock = Arc::new(ManualClock {
        current_time: AtomicI64::new(DEFAULT_CLOCK_START_TIME),
    });
    let spy_collector = WorkerProcessMetricsCollector::with_collectors(
        Arc::new(FakePs(ps)),
        Arc::new(FakeCgroups(cgroups)),
    );
    spy_collector.clear();
    spy_collector.set_clock(clock.clone());
    Fixture {
        clock,
        spy_collector,
    }
}

fn set_up() -> Fixture {
    set_up_with(None, None)
}

#[allow(clippy::too_many_arguments)]
fn assert_worker_metric_contains(
    worker_metric: Option<&Arc<WorkerProcessMetrics>>,
    expected_worker_ids: &[i32],
    expected_process_id: i64,
    expected_mnemonic: &str,
    expected_is_multiplex: bool,
    expected_is_sandboxed: bool,
    expected_worker_key_hash: i32,
    expected_actions_executed: i32,
    expected_is_measurable: bool,
    expected_last_call_time: i64,
    expected_collected_time: Option<i64>,
) {
    let worker_metric = worker_metric.expect("isNotNull");
    let mut ids = worker_metric.get_worker_ids();
    ids.sort();
    let mut expected_ids = expected_worker_ids.to_vec();
    expected_ids.sort();
    assert_eq!(ids, expected_ids);
    assert_eq!(worker_metric.get_process_id(), expected_process_id);
    assert_eq!(worker_metric.get_mnemonic(), expected_mnemonic);
    assert_eq!(worker_metric.is_multiplex(), expected_is_multiplex);
    assert_eq!(worker_metric.is_sandboxed(), expected_is_sandboxed);
    assert_eq!(
        worker_metric.get_worker_key_hash(),
        expected_worker_key_hash
    );
    assert_eq!(
        worker_metric.get_actions_executed(),
        expected_actions_executed
    );
    assert_eq!(worker_metric.is_measurable(), expected_is_measurable);
    assert_eq!(
        worker_metric.get_last_call_time(),
        Some(expected_last_call_time)
    );
    assert_eq!(
        worker_metric.get_last_collected_time(),
        expected_collected_time
    );
}

#[allow(clippy::too_many_arguments)]
fn register(
    c: &WorkerProcessMetricsCollector,
    worker_id: i32,
    process_id: i64,
    status: &Arc<WorkerProcessStatus>,
    mnemonic: &str,
    is_multiplex: bool,
    is_sandboxed: bool,
    hash: i32,
) {
    c.register_worker(
        worker_id,
        process_id,
        status,
        mnemonic,
        is_multiplex,
        is_sandboxed,
        hash,
        None,
    );
}

fn new_status() -> Arc<WorkerProcessStatus> {
    Arc::new(WorkerProcessStatus::new())
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:118 WorkerProcessMetricsCollectorTest.testRegisterWorker_insertDifferent
#[test]
fn test_register_worker_insert_different() {
    let f = set_up();
    let c = &f.spy_collector;
    register(
        c,
        WORKER_ID_1,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        false,
        WORKER_KEY_HASH_1,
    );
    assert_eq!(
        c.get_pid_to_worker_process_metrics()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![PROCESS_ID_1]
    );
    register(
        c,
        WORKER_ID_2,
        PROCESS_ID_2,
        &new_status(),
        CPP_COMPILE_MNEMONIC,
        false,
        true,
        WORKER_KEY_HASH_2,
    );
    assert_eq!(
        c.get_pid_to_worker_process_metrics()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![PROCESS_ID_1, PROCESS_ID_2]
    );
    let m = c.get_pid_to_worker_process_metrics();
    assert_worker_metric_contains(
        m.get(&PROCESS_ID_1),
        &[WORKER_ID_1],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        false,
        WORKER_KEY_HASH_1,
        0,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );
    assert_worker_metric_contains(
        m.get(&PROCESS_ID_2),
        &[WORKER_ID_2],
        PROCESS_ID_2,
        CPP_COMPILE_MNEMONIC,
        false,
        true,
        WORKER_KEY_HASH_2,
        0,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:168 WorkerProcessMetricsCollectorTest.testRegisterWorker_insertMultiplex
#[test]
fn test_register_worker_insert_multiplex() {
    let f = set_up();
    let c = &f.spy_collector;
    register(
        c,
        WORKER_ID_1,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
    );
    assert_eq!(c.get_pid_to_worker_process_metrics().len(), 1);
    assert_worker_metric_contains(
        c.get_pid_to_worker_process_metrics().get(&PROCESS_ID_1),
        &[WORKER_ID_1],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
        0,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );

    let second_time = DEFAULT_CLOCK_START_TIME + 10_000;
    f.clock.set_time(second_time);
    register(
        c,
        WORKER_ID_2,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
    );
    assert_eq!(
        c.get_pid_to_worker_process_metrics()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![PROCESS_ID_1]
    );
    assert_worker_metric_contains(
        c.get_pid_to_worker_process_metrics().get(&PROCESS_ID_1),
        &[WORKER_ID_1, WORKER_ID_2],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
        0,
        false,
        second_time,
        None,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:221 WorkerProcessMetricsCollectorTest.testRegisterWorker_insertSame
#[test]
fn test_register_worker_insert_same() {
    let f = set_up();
    let c = &f.spy_collector;
    register(
        c,
        WORKER_ID_1,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
    );
    assert_worker_metric_contains(
        c.get_pid_to_worker_process_metrics().get(&PROCESS_ID_1),
        &[WORKER_ID_1],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
        0,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );

    let second_time = DEFAULT_CLOCK_START_TIME + 10_000;
    f.clock.set_time(second_time);
    // When it is the same worker, it should only update the last call time.
    register(
        c,
        WORKER_ID_1,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
    );
    assert_eq!(c.get_pid_to_worker_process_metrics().len(), 1);
    assert_worker_metric_contains(
        c.get_pid_to_worker_process_metrics().get(&PROCESS_ID_1),
        &[WORKER_ID_1],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_1,
        0,
        false,
        second_time,
        None,
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:274 WorkerProcessMetricsCollectorTest.testCollectMetrics
#[test]
fn test_collect_metrics() {
    let f = set_up();
    let c = &f.spy_collector;
    // Worker 1 simulates a measurable worker processes has executed some actions.
    register(
        c,
        WORKER_ID_1,
        PROCESS_ID_1,
        &new_status(),
        JAVAC_MNEMONIC,
        true,
        false,
        WORKER_KEY_HASH_1,
    );
    c.on_worker_finish_execution(PROCESS_ID_1);
    // Worker 2: measurable, no action executed yet.
    register(
        c,
        WORKER_ID_2,
        PROCESS_ID_2,
        &new_status(),
        CPP_COMPILE_MNEMONIC,
        false,
        true,
        WORKER_KEY_HASH_2,
    );
    // Worker 3: non-measurable, no action executed.
    register(
        c,
        WORKER_ID_3,
        PROCESS_ID_3,
        &new_status(),
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_3,
    );
    // Worker 4: non-measurable, executed an action and was killed.
    let s4 = new_status();
    register(
        c,
        WORKER_ID_4,
        PROCESS_ID_4,
        &s4,
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_4,
    );
    c.on_worker_finish_execution(PROCESS_ID_4);
    s4.maybe_update_status(Status::KilledDueToMemoryPressure);
    // Worker 5: non-measurable, executed an action, not killed.
    let s5 = new_status();
    register(
        c,
        WORKER_ID_5,
        PROCESS_ID_5,
        &s5,
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_5,
    );
    c.on_worker_finish_execution(PROCESS_ID_5);

    let memory_usage_map: HashMap<i64, i32> = [(PROCESS_ID_1, 1234), (PROCESS_ID_2, 2345)].into();
    let collection_time = DEFAULT_CLOCK_START_TIME + 10_000;
    c.set_resource_usage_for_testing(Some(ResourceSnapshot::create(
        memory_usage_map,
        collection_time,
    )));
    f.clock.set_time(collection_time);

    let metrics = c.collect_metrics();

    // All workers measurable or non-measurable should be reported.
    let mut all_ids: Vec<i32> = metrics.iter().flat_map(|m| m.get_worker_ids()).collect();
    all_ids.sort();
    assert_eq!(
        all_ids,
        vec![
            WORKER_ID_1,
            WORKER_ID_2,
            WORKER_ID_3,
            WORKER_ID_4,
            WORKER_ID_5
        ]
    );
    let find = |id: i32| metrics.iter().find(|m| m.get_worker_ids().contains(&id));
    assert_worker_metric_contains(
        find(WORKER_ID_1),
        &[WORKER_ID_1],
        PROCESS_ID_1,
        JAVAC_MNEMONIC,
        true,
        false,
        WORKER_KEY_HASH_1,
        1,
        true,
        DEFAULT_CLOCK_START_TIME,
        Some(collection_time),
    );
    assert_worker_metric_contains(
        find(WORKER_ID_2),
        &[WORKER_ID_2],
        PROCESS_ID_2,
        CPP_COMPILE_MNEMONIC,
        false,
        true,
        WORKER_KEY_HASH_2,
        0,
        true,
        DEFAULT_CLOCK_START_TIME,
        Some(collection_time),
    );
    assert_worker_metric_contains(
        find(WORKER_ID_3),
        &[WORKER_ID_3],
        PROCESS_ID_3,
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_3,
        0,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );
    assert_worker_metric_contains(
        find(WORKER_ID_4),
        &[WORKER_ID_4],
        PROCESS_ID_4,
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_4,
        1,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );
    assert_worker_metric_contains(
        find(WORKER_ID_5),
        &[WORKER_ID_5],
        PROCESS_ID_5,
        PROTO_MNEMONIC,
        true,
        true,
        WORKER_KEY_HASH_5,
        1,
        false,
        DEFAULT_CLOCK_START_TIME,
        None,
    );
    // Worker 5's status should have been updated to killed_unknown, because it had executed
    // actions but is now non-measurable.
    assert_eq!(s5.get(), Status::KilledUnknown);
}

/// The ps snapshot (1000 KB) and the cgroups snapshot (2000 KB) of the resource usage tests.
fn resource_usage_fixture(use_cgroups: Option<bool>) -> (Fixture, i64) {
    let collection_time = DEFAULT_CLOCK_START_TIME + 10_000;
    let f = set_up_with(
        Some(ResourceSnapshot::create(
            [(PROCESS_ID_1, 1000)].into(),
            collection_time,
        )),
        Some(ResourceSnapshot::create(
            [(PROCESS_ID_1, 2000)].into(),
            collection_time,
        )),
    );
    if let Some(u) = use_cgroups {
        f.spy_collector.set_use_cgroups_on_linux(u);
    }
    f.clock.set_time(collection_time);
    (f, collection_time)
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:423 WorkerProcessMetricsCollectorTest.testCollectResourceUsage_windows
#[test]
fn test_collect_resource_usage_windows() {
    let (f, collection_time) = resource_usage_fixture(None);
    let snapshot = f
        .spy_collector
        .collect_resource_usage_for(OS::Windows, &[PROCESS_ID_1]);
    // On non-linux and non-darwin, it should always return an empty snapshot.
    assert_eq!(
        snapshot,
        ResourceSnapshot::create(HashMap::new(), collection_time)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:438 WorkerProcessMetricsCollectorTest.testCollectResourceUsage_darwin_usingPs
#[test]
fn test_collect_resource_usage_darwin_using_ps() {
    let (f, collection_time) = resource_usage_fixture(None);
    let snapshot = f
        .spy_collector
        .collect_resource_usage_for(OS::Darwin, &[PROCESS_ID_1]);
    assert_eq!(
        snapshot,
        ResourceSnapshot::create([(PROCESS_ID_1, 1000)].into(), collection_time)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:454 WorkerProcessMetricsCollectorTest.testCollectResourceUsage_linux_usingPs
#[test]
fn test_collect_resource_usage_linux_using_ps() {
    let (f, collection_time) = resource_usage_fixture(Some(false));
    let snapshot = f
        .spy_collector
        .collect_resource_usage_for(OS::Linux, &[PROCESS_ID_1]);
    // Should return the ps information rather than the cgroup information.
    assert_eq!(
        snapshot,
        ResourceSnapshot::create([(PROCESS_ID_1, 1000)].into(), collection_time)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:471 WorkerProcessMetricsCollectorTest.testCollectResourceUsage_linux_usingCgroups
#[test]
fn test_collect_resource_usage_linux_using_cgroups() {
    let (f, collection_time) = resource_usage_fixture(Some(true));
    let snapshot = f
        .spy_collector
        .collect_resource_usage_for(OS::Linux, &[PROCESS_ID_1]);
    // Should return the cgroup information rather than the ps information.
    assert_eq!(
        snapshot,
        ResourceSnapshot::create([(PROCESS_ID_1, 2000)].into(), collection_time)
    );
}

fn new_worker_metrics(id: i32, status: WorkerStatus, memory_in_kb: i32) -> WorkerMetrics {
    WorkerMetrics {
        worker_ids: vec![id as u32 as _],
        worker_status: status as i32,
        worker_stats: vec![WorkerStats {
            worker_memory_in_kb: memory_in_kb,
            ..Default::default()
        }],
        ..Default::default()
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:488 WorkerProcessMetricsCollectorTest.testWorkerMetricsPublishComparator_compare
#[test]
fn test_worker_metrics_publish_comparator_compare() {
    use std::cmp::Ordering::{Greater, Less};
    let alive1 = new_worker_metrics(1, WorkerStatus::Alive, 100);
    let alive2 = new_worker_metrics(2, WorkerStatus::Alive, 200);
    let evicted1 = new_worker_metrics(3, WorkerStatus::KilledDueToMemoryPressure, 100);
    let evicted2 = new_worker_metrics(4, WorkerStatus::KilledDueToMemoryPressure, 200);
    let others1 = new_worker_metrics(5, WorkerStatus::KilledUnknown, 100);
    let others2 = new_worker_metrics(6, WorkerStatus::KilledDueToUserExecException, 200);

    // Same status priority: compared by memory usage (higher gets prioritized).
    assert_eq!(worker_metrics_publish_comparator(&alive1, &alive2), Greater);
    assert_eq!(
        worker_metrics_publish_comparator(&evicted1, &evicted2),
        Greater
    );
    assert_eq!(
        worker_metrics_publish_comparator(&others1, &others2),
        Greater
    );
    // First compared by status priorities rather than memory usage.
    assert_eq!(worker_metrics_publish_comparator(&alive1, &evicted2), Less);
    assert_eq!(worker_metrics_publish_comparator(&evicted1, &others2), Less);
    assert_eq!(
        worker_metrics_publish_comparator(&others2, &alive1),
        Greater
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerProcessMetricsCollectorTest.java:513 WorkerProcessMetricsCollectorTest.testLimitWorkerMetricsToPublish
#[test]
fn test_limit_worker_metrics_to_publish() {
    let alive1 = new_worker_metrics(1, WorkerStatus::Alive, 200);
    let alive2 = new_worker_metrics(2, WorkerStatus::Alive, 100);
    let evicted3 = new_worker_metrics(3, WorkerStatus::KilledDueToMemoryPressure, 100);
    let evicted4 = new_worker_metrics(4, WorkerStatus::KilledDueToMemoryPressure, 200);
    let others5 = new_worker_metrics(5, WorkerStatus::KilledUnknown, 200);
    let others6 = new_worker_metrics(6, WorkerStatus::KilledDueToUserExecException, 100);

    // Based on prioritization and then sorted by worker id (containsExactly: any order).
    let ids = |v: Vec<WorkerMetrics>| {
        let mut ids: Vec<_> = v.iter().map(|m| m.worker_ids[0]).collect();
        ids.sort();
        ids
    };
    assert_eq!(
        ids(
            WorkerProcessMetricsCollector::limit_worker_metrics_to_publish(
                vec![
                    alive1.clone(),
                    alive2.clone(),
                    evicted3,
                    evicted4.clone(),
                    others5.clone(),
                    others6.clone()
                ],
                3
            )
        ),
        ids(vec![alive1.clone(), alive2.clone(), evicted4.clone()])
    );
    assert_eq!(
        ids(
            WorkerProcessMetricsCollector::limit_worker_metrics_to_publish(
                vec![
                    alive1.clone(),
                    evicted4.clone(),
                    others5.clone(),
                    others6.clone()
                ],
                3
            )
        ),
        ids(vec![alive1.clone(), evicted4.clone(), others5])
    );
    // If under the limit, it should just report everything.
    assert_eq!(
        ids(
            WorkerProcessMetricsCollector::limit_worker_metrics_to_publish(
                vec![
                    alive1.clone(),
                    alive2.clone(),
                    evicted4.clone(),
                    others6.clone()
                ],
                10
            )
        ),
        ids(vec![alive1, alive2, evicted4, others6])
    );
}
