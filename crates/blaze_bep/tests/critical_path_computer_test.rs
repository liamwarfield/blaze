//! Port of `metrics/criticalpath/CriticalPathComputerTest.java`.
//!
//! The events of the Java test (`ActionStartedEvent`, `ActionCompletionEvent`,
//! `CachedActionEvent`, `SpawnExecutedEvent`, ...) are the arguments of the computer's subscriber
//! methods here (see `critical_path_computer.rs`); the clock is a manual clock as in the Java
//! test.

use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use blaze_actions::actions::action::{AbstractAction, Action, ActionRef, ComputeKeyError};
use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::actions::platform_info::empty_platform_info;
use blaze_actions::{
    ActionKeyContext, ActionLookupKeyRef, ActionOwner, Artifact, ArtifactOwner, ArtifactRoot,
    RootType,
};
use blaze_bep::metrics::aggregated_spawn_metrics::AggregatedSpawnMetrics;
use blaze_bep::metrics::criticalpath::critical_path_computer::SLOWEST_COMPONENTS_SIZE;
use blaze_bep::metrics::criticalpath::{CriticalPathComponent, CriticalPathComputer};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::location::Location;
use blaze_exec::actions::spawn_metrics::{ExecKind, SpawnMetrics};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, Root, UnixFileSystem};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------------------------
// Test utilities (ManualClock, MockAction, NullAction, ActionsTestUtil equivalents)
// ---------------------------------------------------------------------------------------------

/// `ManualClock`: `nanoTime()` is the manual time in milliseconds times 1,000,000.
struct ManualClock {
    millis: Cell<i64>,
}

impl ManualClock {
    fn new() -> ManualClock {
        ManualClock {
            millis: Cell::new(0),
        }
    }
    fn nano_time(&self) -> i64 {
        self.millis.get() * 1_000_000
    }
    fn current_time_millis(&self) -> i64 {
        self.millis.get()
    }
    fn advance_millis(&self, millis: i64) {
        self.millis.set(self.millis.get() + millis);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn shared_fs() -> blaze_vfs::FileSystemRef {
    static FS: std::sync::LazyLock<blaze_vfs::FileSystemRef> =
        std::sync::LazyLock::new(|| UnixFileSystem::create(DigestHashFunction::Sha256));
    FS.clone()
}

/// `ActionsTestUtil.NULL_ACTION_OWNER`.
fn null_action_owner() -> ActionOwner {
    ActionOwner::create_dummy(
        Some(Label::parse_canonical("//null/action:owner").unwrap()),
        Location::new("dummy-file", 0, 0),
        "dummy-kind",
        "dummy-configuration-mnemonic",
        "dummy-configuration",
        None,
        false,
        Some(empty_platform_info()),
        Vec::new(),
        IndexMap::new(),
    )
}

/// `artifact(path)`: `ActionsTestUtil.createArtifactWithExecPath(artifactRoot, path)` on the
/// source root `/workspace`.
fn artifact(path: &str) -> Artifact {
    let root = ArtifactRoot::as_source_root(Root::from_path(Path::create(
        PathFragment::create("/workspace"),
        &shared_fs(),
    )));
    Artifact::new_source_artifact(root, PathFragment::create(path), ArtifactOwner::NullOwner)
}

/// `derivedArtifact(path)`: a derived artifact under `/workspace/test`.
fn derived_artifact(path: &str) -> Artifact {
    let exec_root = Path::create(PathFragment::create("/workspace"), &shared_fs());
    let root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["test"]);
    Artifact::create_derived_artifact(
        root,
        PathFragment::create(path),
        ActionLookupKeyRef::new(NullArtifactOwner),
    )
}

/// `MockAction(inputs, outputs, isShareable)`.
#[derive(Debug)]
struct MockAction {
    base: AbstractAction,
    is_shareable: bool,
}

impl Action for MockAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn get_mnemonic(&self) -> String {
        "MockAction".into()
    }
    fn compute_key(
        &self,
        _: &ActionKeyContext,
        _: Option<&dyn blaze_actions::actions::action::InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_string("MockAction");
        Ok(())
    }
    fn is_shareable(&self) -> bool {
        self.is_shareable
    }
    fn describe(&self) -> String {
        let outputs: Vec<String> = self
            .get_outputs()
            .iter()
            .map(|o| o.get_exec_path_string().to_owned())
            .collect();
        format!("MockAction {}", outputs.join(" "))
    }
}

fn mock_action(inputs: Vec<Artifact>, outputs: Vec<Artifact>) -> ActionRef {
    mock_action_shareable(inputs, outputs, true)
}

fn mock_action_shareable(
    inputs: Vec<Artifact>,
    outputs: Vec<Artifact>,
    is_shareable: bool,
) -> ActionRef {
    let mut builder = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    for input in inputs {
        builder.add(input);
    }
    Arc::new(MockAction {
        base: AbstractAction::new(null_action_owner(), builder.build(), outputs),
        is_shareable,
    })
}

/// `new NullAction()`: one output, `ActionsTestUtil.DUMMY_ARTIFACT`.
fn null_action() -> ActionRef {
    mock_action(vec![], vec![artifact("dummy")])
}

/// `Actions.canBeShared(new ActionKeyContext(), a, b)`.
fn can_be_shared(a: &ActionRef, b: &ActionRef) -> bool {
    let ctx = ActionKeyContext::new();
    if !(a.get_mnemonic() == b.get_mnemonic()
        && a.get_key(&ctx, None).unwrap() == b.get_key(&ctx, None).unwrap())
    {
        return false;
    }
    let equal_without_owner = |x: &[Artifact], y: &[Artifact]| {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.equals_without_owner(q))
    };
    if a.is_shareable() && b.is_shareable() {
        return equal_without_owner(
            &a.get_mandatory_inputs().to_list(),
            &b.get_mandatory_inputs().to_list(),
        ) && equal_without_owner(&a.get_outputs(), &b.get_outputs());
    }
    if a.is_shareable() || b.is_shareable() {
        return false;
    }
    *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
        && a.get_outputs() == b.get_outputs()
}

fn remote(metrics: SpawnMetrics) -> SpawnMetrics {
    SpawnMetrics {
        exec_kind: ExecKind::Remote,
        ..metrics
    }
}

fn worker(metrics: SpawnMetrics) -> SpawnMetrics {
    SpawnMetrics {
        exec_kind: ExecKind::Worker,
        ..metrics
    }
}

/// `createSpawnResult()...setSpawnMetrics(m)`: the parts the computer reads (runner name
/// "test", empty subtype, not remote).
struct SpawnResult {
    metrics: SpawnMetrics,
}

fn spawn_result(metrics: SpawnMetrics) -> SpawnResult {
    SpawnResult { metrics }
}

/// `createSpawnResult(processTimeInMs)`: no spawn metrics set, so `SpawnResult.getMetrics()` is
/// `SpawnMetrics.forLocalExecution(wallTime)`.
fn create_spawn_result(process_time_in_ms: i32) -> SpawnResult {
    SpawnResult {
        metrics: SpawnMetrics::for_local_execution(process_time_in_ms),
    }
}

struct Fixture {
    clock: ManualClock,
    computer: CriticalPathComputer,
}

fn duration_ofs(seconds: i64) -> i64 {
    seconds * 1_000_000_000
}

fn duration_ofms(millis: i64) -> i64 {
    millis * 1_000_000
}

impl Fixture {
    /// `createComputer()`.
    fn new() -> Fixture {
        Fixture {
            clock: ManualClock::new(),
            computer: CriticalPathComputer::new(Arc::new(can_be_shared)),
        }
    }

    fn action_complete(&self, start: i64, action: &ActionRef) {
        self.computer
            .action_complete(action, start, self.clock.nano_time());
    }

    fn simulate_action_exec_time(&self, action: &ActionRef, total_time: i64) {
        let nano_time_start = self.clock.nano_time();
        self.computer.action_started(action, nano_time_start);
        self.clock.advance_millis(total_time);
        self.computer
            .action_complete(action, nano_time_start, self.clock.nano_time());
    }

    fn simulate_action_exec_discover(
        &self,
        action: &ActionRef,
        total_time_in_ms: i64,
        process_time_in_ms: i32,
        complete_action: bool,
        discover_inputs_duration_in_ms: i32,
    ) {
        self.computer.discover_inputs(
            action,
            /* start_time_nanos= */ 0,
            &remote(SpawnMetrics {
                parse_time_in_ms: discover_inputs_duration_in_ms,
                total_time_in_ms: discover_inputs_duration_in_ms,
                ..SpawnMetrics::default()
            }),
        );
        self.simulate_action_exec_process(
            action,
            total_time_in_ms,
            process_time_in_ms,
            complete_action,
        );
    }

    fn simulate_action_exec_process(
        &self,
        action: &ActionRef,
        total_time_in_ms: i64,
        process_time_in_ms: i32,
        complete_action: bool,
    ) {
        let spawn_result = spawn_result(remote(SpawnMetrics {
            total_time_in_ms: process_time_in_ms,
            execution_wall_time_in_ms: process_time_in_ms,
            ..SpawnMetrics::default()
        }));
        self.simulate_action_exec_full(action, total_time_in_ms, &spawn_result, complete_action);
    }

    fn simulate_action_exec_result(&self, action: &ActionRef, spawn_result: &SpawnResult) {
        self.simulate_action_exec_full(
            action,
            spawn_result.metrics.total_time_in_ms as i64,
            spawn_result,
            true,
        );
    }

    fn simulate_action_exec_full(
        &self,
        action: &ActionRef,
        total_time_in_ms: i64,
        spawn_result: &SpawnResult,
        complete_action: bool,
    ) {
        let start_time = self.clock.nano_time();
        self.computer.action_started(action, start_time);
        self.clock.advance_millis(total_time_in_ms);
        self.computer.spawn_executed(
            Some(&action.get_primary_output()),
            &spawn_result.metrics,
            "test",
            "",
            false,
        );
        if complete_action {
            self.computer
                .action_complete(action, start_time, self.clock.nano_time());
        }
    }

    fn simulate_sequential_and_parallel_action_exec(
        &self,
        action: &ActionRef,
        total_times_in_ms: &[&[i32]],
    ) {
        let start_time = self.clock.nano_time();
        for parallel_duration in total_times_in_ms {
            for phase_duration in *parallel_duration {
                self.simulate_action_exec_process(
                    action,
                    *phase_duration as i64,
                    *phase_duration,
                    false,
                );
            }
            self.computer
                .next_critical_path_phase(&action.get_primary_output());
        }
        self.computer
            .action_complete(action, start_time, self.clock.nano_time());
    }

    fn check_critical_path_simple(
        &self,
        total_wall_time_in_millis: i64,
        total_wall_time_str: &str,
    ) {
        let critical_path = self.computer.aggregate();
        assert_eq!(
            critical_path.get_aggregated_elapsed_time(),
            duration_ofms(total_wall_time_in_millis)
        );
        let summary = critical_path.to_string_summary();
        assert!(summary.contains(&format!("Critical Path: {total_wall_time_str}s")));
    }

    fn check_critical_path(
        &self,
        total_wall_time: i64,
        total_time: i64,
        total_process_time: i64,
        total_wall_time_str: &str,
        total_time_percent: &str,
        total_process_percent: &str,
    ) -> blaze_bep::metrics::criticalpath::AggregatedCriticalPath {
        let critical_path = self.computer.aggregate();
        assert_eq!(
            critical_path.get_aggregated_elapsed_time(),
            duration_ofms(total_wall_time / 1_000_000)
        );
        assert_eq!(
            critical_path
                .get_spawn_metrics()
                .get_remote_metrics()
                .total_time_in_ms as i64,
            total_time / 1_000_000
        );
        assert_eq!(
            critical_path
                .get_spawn_metrics()
                .get_remote_metrics()
                .execution_wall_time_in_ms as i64,
            total_process_time / 1_000_000
        );
        let summary = critical_path.to_string_summary();
        assert!(
            summary.contains(&format!("Critical Path: {total_wall_time_str}s")),
            "{summary}"
        );
        assert!(
            summary.contains(&format!("Remote ({total_time_percent}% of the time)")),
            "{summary}"
        );
        assert!(
            summary.contains(&format!("process: {total_process_percent}%")),
            "{summary}"
        );
        critical_path
    }
}

fn check_top_components_times(computer: &CriticalPathComputer, times: &[i64]) {
    let top_components = computer.get_slowest_components();
    assert_eq!(top_components.len(), times.len());
    for (i, time) in times.iter().enumerate() {
        assert_eq!(top_components[i].get_elapsed_time(), duration_ofms(*time));
    }
}

fn action_matches(action: &ActionRef, component: &CriticalPathComponent) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(component.get_action()), Arc::as_ptr(action))
}

fn assert_action_matches(action: &ActionRef, component: &CriticalPathComponent) {
    assert!(
        action_matches(action, component),
        "Action {action:?} did not match one in {component}"
    );
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_no_spawn_metrics() {
    let cp = CriticalPathComponent::new(1, null_action(), 0);
    assert_eq!(cp.get_spawn_metrics(), AggregatedSpawnMetrics::empty());
    assert_eq!(cp.get_longest_phase_spawn_runner_name(), None);
}

#[test]
fn test_multiple_spawn_metrics() {
    let cp = CriticalPathComponent::new(1, null_action(), 0);
    for (total, name) in [(10, "first"), (30, "second"), (20, "third")] {
        cp.add_spawn_result(
            &remote(SpawnMetrics {
                total_time_in_ms: total * 1000,
                ..SpawnMetrics::default()
            }),
            Some(name),
            "",
            false,
        );
    }
    cp.finish_action_execution(0, 40, "test");
    // The current implementation keeps the maximum spawn metrics because we do not
    // differentiate between sequential or parallel spawn invocations within a single Bazel
    // action. So while it is still 'incorrect', it is more fair than keeping the latest
    // invocation data.
    assert_eq!(
        cp.get_spawn_metrics().get_remote_metrics().total_time_in_ms,
        30 * 1000
    );
    assert_eq!(
        cp.get_longest_phase_spawn_runner_name().as_deref(),
        Some("second")
    );
}

/// Test that 'other' time is correctly computed as any time not measured by the rest of the
/// stats.
#[test]
fn test_spawn_metrics_other_time_computed() {
    let spawn_metrics = remote(SpawnMetrics {
        total_time_in_ms: 100 * 1000,
        parse_time_in_ms: 1000,
        network_time_in_ms: 2 * 1000,
        fetch_time_in_ms: 3 * 1000,
        queue_time_in_ms: 4 * 1000,
        setup_time_in_ms: 5 * 1000,
        upload_time_in_ms: 6 * 1000,
        execution_wall_time_in_ms: 7 * 1000,
        retry_time_in_ms: [(1, 8 * 1000)].into_iter().collect(),
        process_outputs_time_in_ms: 9 * 1000,
        ..SpawnMetrics::default()
    });
    assert_eq!(spawn_metrics.other_time_in_ms(), 55 * 1000);
}

#[test]
fn test_critical_path_one_action() {
    let f = Fixture::new();
    f.simulate_action_exec_process(&null_action(), 2 * 1000, 1000, true);
    f.check_critical_path(
        duration_ofs(2),
        duration_ofs(1),
        duration_ofs(1),
        "2.00",
        "50.00",
        "50.00",
    );
    check_top_components_times(&f.computer, &[2000]);
}

#[test]
fn test_critical_path_queue_time_without_retries() {
    let f = Fixture::new();
    let spawn_result = spawn_result(remote(SpawnMetrics {
        total_time_in_ms: 4 * 1000,
        execution_wall_time_in_ms: 1000,
        queue_time_in_ms: 1000,
        ..SpawnMetrics::default()
    }));
    f.simulate_action_exec_full(&null_action(), 8 * 1000, &spawn_result, true);
    let stats = f.check_critical_path(
        duration_ofs(8),
        duration_ofs(4),
        duration_ofs(1),
        "8.00",
        "50.00",
        "12.50",
    );
    assert_eq!(
        stats
            .get_spawn_metrics()
            .get_remote_metrics()
            .queue_time_in_ms,
        1000
    );
}

/// Check that the timing stats are printed correctly, that the printed values correctly match
/// their label.
#[test]
fn test_critical_path_to_string() {
    let f = Fixture::new();
    let action_a = mock_action(vec![], vec![artifact("a.out")]);
    let spawn_result = spawn_result(remote(SpawnMetrics {
        parse_time_in_ms: 5 * 1000,
        network_time_in_ms: 6 * 1000,
        fetch_time_in_ms: 7 * 1000,
        queue_time_in_ms: 8 * 1000,
        setup_time_in_ms: 9 * 1000,
        upload_time_in_ms: 10 * 1000,
        process_outputs_time_in_ms: 4 * 1000,
        execution_wall_time_in_ms: 40 * 1000,
        total_time_in_ms: 100 * 1000,
        ..SpawnMetrics::default()
    }));
    f.simulate_action_exec_result(&action_a, &spawn_result);
    let stats = f.computer.aggregate();

    let to_string = stats.to_string();
    for expected in [
        "parse: 5.00%",
        "network: 6.00%",
        "fetch: 7.00%",
        "queue: 8.00%",
        "setup: 9.00%",
        "upload: 10.00%",
        "processOutputs: 4.00%",
        "process: 40.00%",
        "other: 11.00%",
    ] {
        assert!(to_string.contains(expected), "{to_string}");
    }
}

/// Check that we only print certain critical parts of the timing stats when they are below a
/// certain threshold, to avoid spamming the user.
#[test]
fn test_critical_path_to_string_summary() {
    let f = Fixture::new();
    let action_a = mock_action(vec![], vec![artifact("a.out")]);
    let action_b = mock_action(vec![artifact("a.out")], vec![artifact("b.out")]);

    let spawn = spawn_result(remote(SpawnMetrics {
        network_time_in_ms: 10 * 1000,
        parse_time_in_ms: 10 * 1000,
        fetch_time_in_ms: 10 * 1000,
        queue_time_in_ms: 10 * 1000,
        setup_time_in_ms: 10 * 1000,
        process_outputs_time_in_ms: 10 * 1000,
        execution_wall_time_in_ms: 20 * 1000,
        upload_time_in_ms: 10 * 1000,
        total_time_in_ms: 100 * 1000,
        ..SpawnMetrics::default()
    }));
    f.simulate_action_exec_result(&action_a, &spawn);
    let stats = f.computer.aggregate();
    let summary = stats.to_string();
    for expected in [
        "network: 10.00%",
        "parse: 10.00%",
        "queue: 10.00%",
        "upload: 10.00%",
        "setup: 10.00%",
        "processOutputs: 10.00%",
        "process: 20.00%",
        "fetch: 10.00%",
        "other: 10.00%",
    ] {
        assert!(summary.contains(expected), "{summary}");
    }

    // Add another action execution so that now the critical path is A + B, and the 10 second
    // stats each are bumped below 10%, bringing them below the "summary" threshold.
    let spawn = create_spawn_result(10 * 1000);
    f.simulate_action_exec_result(&action_b, &spawn);
    let stats = f.computer.aggregate();
    let summary = stats.to_string_summary();
    assert!(!summary.contains("network:"), "{summary}");
    assert!(!summary.contains("parse:"), "{summary}");
    assert!(summary.contains("queue:"), "{summary}");
    assert!(!summary.contains("upload:"), "{summary}");
    assert!(summary.contains("setup:"), "{summary}");
    assert!(summary.contains("process:"), "{summary}");
    assert!(!summary.contains("fetch:"), "{summary}");
    assert!(!summary.contains("processOutputs:"), "{summary}");
    assert!(!summary.contains("other:"), "{summary}");
}

#[test]
fn test_aggregate_metrics() {
    let f = Fixture::new();
    let action_a = mock_action(vec![], vec![artifact("a.out")]);
    let action_b = mock_action(vec![artifact("a.out")], vec![artifact("b.out")]);
    let action_c = mock_action(vec![artifact("b.out")], vec![artifact("c.out")]);
    let action_d = mock_action(vec![artifact("c.out")], vec![artifact("d.out")]);

    let metrics = |scale: i32, total: i32| SpawnMetrics {
        network_time_in_ms: scale * 1000,
        parse_time_in_ms: 2 * scale * 1000,
        fetch_time_in_ms: 3 * scale * 1000,
        queue_time_in_ms: 4 * scale * 1000,
        setup_time_in_ms: 5 * scale * 1000,
        process_outputs_time_in_ms: 6 * scale * 1000,
        execution_wall_time_in_ms: 7 * scale * 1000,
        upload_time_in_ms: 8 * scale * 1000,
        total_time_in_ms: total * 1000,
        ..SpawnMetrics::default()
    };
    f.simulate_action_exec_result(&action_a, &spawn_result(remote(metrics(1, 100))));
    f.simulate_action_exec_result(
        &action_b,
        &spawn_result(remote(SpawnMetrics {
            network_time_in_ms: 20 * 1000,
            parse_time_in_ms: 30 * 1000,
            fetch_time_in_ms: 40 * 1000,
            queue_time_in_ms: 50 * 1000,
            setup_time_in_ms: 60 * 1000,
            process_outputs_time_in_ms: 70 * 1000,
            execution_wall_time_in_ms: 80 * 1000,
            upload_time_in_ms: 90 * 1000,
            total_time_in_ms: 1000 * 1000,
            ..SpawnMetrics::default()
        })),
    );
    f.simulate_action_exec_result(
        &action_c,
        &spawn_result(worker(SpawnMetrics {
            network_time_in_ms: 10 * 1000,
            parse_time_in_ms: 20 * 1000,
            fetch_time_in_ms: 30 * 1000,
            queue_time_in_ms: 40 * 1000,
            setup_time_in_ms: 50 * 1000,
            process_outputs_time_in_ms: 60 * 1000,
            execution_wall_time_in_ms: 70 * 1000,
            upload_time_in_ms: 80 * 1000,
            total_time_in_ms: 1000 * 1000,
            ..SpawnMetrics::default()
        })),
    );
    f.simulate_action_exec_result(
        &action_d,
        &spawn_result(worker(SpawnMetrics {
            network_time_in_ms: 200 * 1000,
            parse_time_in_ms: 300 * 1000,
            fetch_time_in_ms: 400 * 1000,
            queue_time_in_ms: 500 * 1000,
            setup_time_in_ms: 600 * 1000,
            process_outputs_time_in_ms: 700 * 1000,
            execution_wall_time_in_ms: 800 * 1000,
            upload_time_in_ms: 900 * 1000,
            total_time_in_ms: 10000 * 1000,
            ..SpawnMetrics::default()
        })),
    );

    let aggregated = f.computer.aggregate().get_spawn_metrics().clone();
    let remote_metrics = aggregated.get_metrics(ExecKind::Remote);
    assert_eq!(remote_metrics.network_time_in_ms, 21 * 1000);
    assert_eq!(remote_metrics.parse_time_in_ms, 32 * 1000);
    assert_eq!(remote_metrics.fetch_time_in_ms, 43 * 1000);
    assert_eq!(remote_metrics.queue_time_in_ms, 54 * 1000);
    assert_eq!(remote_metrics.setup_time_in_ms, 65 * 1000);
    assert_eq!(remote_metrics.process_outputs_time_in_ms, 76 * 1000);
    assert_eq!(remote_metrics.execution_wall_time_in_ms, 87 * 1000);
    assert_eq!(remote_metrics.upload_time_in_ms, 98 * 1000);
    assert_eq!(remote_metrics.total_time_in_ms, 1100 * 1000);

    let worker_metrics = aggregated.get_metrics(ExecKind::Worker);
    assert_eq!(worker_metrics.network_time_in_ms, 210 * 1000);
    assert_eq!(worker_metrics.parse_time_in_ms, 320 * 1000);
    assert_eq!(worker_metrics.fetch_time_in_ms, 430 * 1000);
    assert_eq!(worker_metrics.queue_time_in_ms, 540 * 1000);
    assert_eq!(worker_metrics.setup_time_in_ms, 650 * 1000);
    assert_eq!(worker_metrics.process_outputs_time_in_ms, 760 * 1000);
    assert_eq!(worker_metrics.execution_wall_time_in_ms, 870 * 1000);
    assert_eq!(worker_metrics.upload_time_in_ms, 980 * 1000);
    assert_eq!(worker_metrics.total_time_in_ms, 11000 * 1000);
}

#[test]
fn test_empty_critical_path() {
    let f = Fixture::new();
    let empty = f.computer.aggregate();
    assert!(empty.components().is_empty());
    assert_eq!(empty.get_aggregated_elapsed_time() / 1_000_000, 0);
    check_top_components_times(&f.computer, &[]);
}

/// Tests that we only record the top slowest components and that we drop the rest.
#[test]
fn test_top_components_overflow() {
    let f = Fixture::new();
    for i in 0..=1000 {
        let action = mock_action(vec![], vec![artifact(&format!("{i}.out"))]);
        f.simulate_action_exec_time(&action, i);
    }
    let top_times: Vec<i64> = (0..SLOWEST_COMPONENTS_SIZE as i64)
        .map(|i| 1000 - i)
        .collect();
    check_top_components_times(&f.computer, &top_times);
}

fn overflow_test(
    set: impl Fn(i64) -> SpawnMetrics,
    largest: impl Fn(&CriticalPathComputer) -> Vec<Arc<CriticalPathComponent>>,
    read: impl Fn(&SpawnMetrics) -> i64,
) {
    let f = Fixture::new();
    for i in 0..1000 {
        let action = mock_action(vec![], vec![artifact(&format!("{i}.out"))]);
        // the largest actions are in the middle
        let value = if 500 < i && i < 600 { i } else { 0 };
        f.simulate_action_exec_result(
            &action,
            &spawn_result(remote(SpawnMetrics {
                execution_wall_time_in_ms: 1000,
                total_time_in_ms: (i * 1000) as i32,
                ..set(value)
            })),
        );
    }
    let result = largest(&f.computer);
    assert_eq!(result.len(), 20);
    assert_eq!(
        read(&result[0].get_spawn_metrics().get_remote_metrics()),
        599
    );
}

#[test]
fn test_largest_memory_components_overflow() {
    overflow_test(
        |v| SpawnMetrics {
            memory_estimate_bytes: v,
            ..SpawnMetrics::default()
        },
        |c| c.get_largest_memory_components(),
        |m| m.memory_estimate_bytes,
    );
}

#[test]
fn test_largest_input_size_components_overflow() {
    overflow_test(
        |v| SpawnMetrics {
            input_bytes: v,
            ..SpawnMetrics::default()
        },
        |c| c.get_largest_input_size_components(),
        |m| m.input_bytes,
    );
}

#[test]
fn test_largest_input_count_components_overflow() {
    overflow_test(
        |v| SpawnMetrics {
            input_files: v,
            ..SpawnMetrics::default()
        },
        |c| c.get_largest_input_count_components(),
        |m| m.input_files,
    );
}

#[test]
fn test_action_cached() {
    let f = Fixture::new();
    let cached_action = mock_action(vec![], vec![artifact("cached.out")]);
    let top_level_action = mock_action(vec![artifact("cached.out")], vec![artifact("top.out")]);

    f.computer
        .action_cached(&cached_action, f.clock.nano_time(), f.clock.nano_time());
    f.simulate_action_exec_time(&top_level_action, 1000);

    let aggregated = f.computer.aggregate();

    assert_eq!(aggregated.components().len(), 2);
    assert_action_matches(&top_level_action, &aggregated.components()[0]);
    assert_action_matches(&cached_action, &aggregated.components()[1]);
    assert_eq!(
        aggregated.components()[0].get_elapsed_time(),
        duration_ofs(1)
    );
    assert_eq!(aggregated.components()[1].get_elapsed_time(), 0);

    check_top_components_times(&f.computer, &[1000, 0]);
}

/// Test that wall time is not computed using nanotime.
#[test]
fn test_wall_time() {
    let f = Fixture::new();
    f.simulate_action_exec_time(&null_action(), 2000);
    f.check_critical_path_simple(2000, "2.00");
    check_top_components_times(&f.computer, &[2000]);
    // BlazeClock.createNanosToMillisSinceEpochConverter(clock).
    let converter =
        |nanos: i64| f.clock.current_time_millis() - (f.clock.nano_time() - nanos) / 1_000_000;
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_start_time_millis_since_epoch(&converter),
        0
    );
}

/// When running shared actions concurrently we might end up receiving multiple events, one per
/// shared action. In that case we record a single component and we update the time of the
/// maximum elapsed time.
#[test]
fn test_concurrent_shared_actions() {
    let f = Fixture::new();
    let shared1 = mock_action(vec![], vec![artifact("shared.out")]);
    let shared2 = mock_action(vec![], vec![artifact("shared.out")]);
    let action1 = mock_action(vec![artifact("shared.out")], vec![artifact("action1.out")]);
    let action2 = mock_action(vec![artifact("shared.out")], vec![artifact("action2.out")]);

    let shared1_start = f.clock.nano_time();
    f.computer.action_started(&shared1, shared1_start);
    f.clock.advance_millis(1000);
    let shared2_start = f.clock.nano_time();
    // We concurrently execute shared2 before shared1 could finish. But we record it as a cache
    // hit.
    f.computer
        .action_cached(&shared2, f.clock.nano_time(), shared2_start);
    f.clock.advance_millis(1);
    // Action2 depends on shared2, so it can start executing without waiting to shared1. This
    // will prevent us from identifying the critical path in some circumstance, but we are OK with
    // that.
    f.simulate_action_exec_time(&action2, 11);

    f.action_complete(shared1_start, &shared1);
    f.simulate_action_exec_time(&action1, 10);
    let critical_path = f.computer.aggregate();

    // Yes, this is not correct but expected. While action2.time > action1.time, because
    // action2 executed before shared1 finishes it incorrectly gets the time set by shared2.
    assert_action_matches(&action1, &critical_path.components()[0]);
    // We expect that the component used for any critical path is shared1, as it is the first
    // that was started.
    assert_action_matches(&shared1, &critical_path.components()[1]);
    assert_eq!(
        critical_path.components()[1].get_elapsed_time(),
        duration_ofms(1012)
    );

    let slowest = f.computer.get_slowest_components();
    assert_eq!(slowest.len(), 3);
    for cpath in &slowest {
        if action_matches(&shared1, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(1012));
        }
        // While shared2 was a cache hit, because it was executed concurrently with shared1 we
        // keep one component with the maximum time.
        if action_matches(&shared2, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(1012));
        }
        if action_matches(&action1, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(10));
        }
        if action_matches(&action2, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(11));
            assert_eq!(
                cpath.get_child().unwrap().get_elapsed_time(),
                duration_ofms(1012)
            );
        }
    }
}

#[test]
fn test_total_aggregate_run_time_with_gaps() {
    let f = Fixture::new();
    let action1 = mock_action(vec![], vec![artifact("action1.out")]);
    let action2 = mock_action(vec![artifact("action1.out")], vec![artifact("action2.out")]);
    let action3 = mock_action(vec![artifact("action2.out")], vec![artifact("action3.out")]);

    let action1_start = f.clock.nano_time();
    f.computer.action_started(&action1, action1_start);
    f.clock.advance_millis(1000);
    f.action_complete(action1_start, &action1);

    f.clock.advance_millis(2000);
    let action2_start = f.clock.nano_time();
    f.computer.action_started(&action2, action2_start);
    f.clock.advance_millis(3000);
    f.action_complete(action2_start, &action2);

    f.clock.advance_millis(2000);
    let action3_start = f.clock.nano_time();
    f.computer.action_started(&action3, action3_start);
    f.clock.advance_millis(4000);
    f.action_complete(action3_start, &action3);

    // The runtime of the critical path ignoring gaps is 8 seconds.
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_aggregated_elapsed_time(),
        duration_ofs(8)
    );
    assert_eq!(f.clock.nano_time() - action1_start, duration_ofs(12));
}

#[test]
fn test_total_aggregate_run_time_with_overlapping_times() {
    let f = Fixture::new();
    let action1 = mock_action(vec![], vec![artifact("action1.out")]);
    let action2 = mock_action(vec![artifact("action1.out")], vec![artifact("action2.out")]);

    let action1_start = f.clock.nano_time();
    f.computer.action_started(&action1, action1_start);
    f.clock.advance_millis(1000);
    let action2_start = f.clock.nano_time();
    f.computer.action_started(&action2, action2_start);
    f.clock.advance_millis(2000);
    f.action_complete(action1_start, &action1);
    f.clock.advance_millis(2000);
    f.action_complete(action2_start, &action2);

    // The total run time of all actions in the critical path is 5 seconds.
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_aggregated_elapsed_time(),
        duration_ofs(5)
    );
    let critical_path = f.computer.aggregate();
    assert_eq!(critical_path.components().len(), 2);
    // Action 2  has a run time of 4 seconds
    assert_eq!(
        critical_path.components()[0].get_elapsed_time(),
        duration_ofs(4)
    );
    // Action 1 has a run time of 3 seconds
    assert_eq!(
        critical_path.components()[1].get_elapsed_time(),
        duration_ofs(3)
    );
}

#[test]
fn test_total_aggregate_run_time_with_parallel_runs() {
    let f = Fixture::new();
    let action1 = mock_action(vec![], vec![artifact("action1.out")]);
    let action2 = mock_action(vec![artifact("action1.out")], vec![artifact("action2.out")]);

    let action2_start = f.clock.nano_time();
    f.computer.action_started(&action2, action2_start);
    f.clock.advance_millis(1000);
    let action1_start = f.clock.nano_time();
    f.computer.action_started(&action1, action1_start);
    f.clock.advance_millis(2000);
    f.action_complete(action1_start, &action1);
    f.clock.advance_millis(2000);
    f.action_complete(action2_start, &action2);

    // The total run time of all actions in the critical path is 5 seconds.
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_aggregated_elapsed_time(),
        duration_ofs(5)
    );
    let critical_path = f.computer.aggregate();
    assert_eq!(critical_path.components().len(), 2);
    // Action 2 has a run time of 5 seconds
    assert_eq!(
        critical_path.components()[0].get_elapsed_time(),
        duration_ofs(5)
    );
    // Action 1 has a run time of 2 seconds
    assert_eq!(
        critical_path.components()[1].get_elapsed_time(),
        duration_ofs(2)
    );
}

#[test]
fn test_longest_total_time() {
    let f = Fixture::new();
    let action1 = mock_action(vec![], vec![artifact("action1.out")]);
    let action2 = mock_action(vec![], vec![artifact("action2.out")]);
    let action3 = mock_action(
        vec![artifact("action1.out"), artifact("action2.out")],
        vec![artifact("action3.out")],
    );

    // Action 1 - 0s - 3s
    let action1_start = f.clock.nano_time();
    f.computer.action_started(&action1, action1_start);
    f.clock.advance_millis(3000);
    f.action_complete(action1_start, &action1);
    // Action 2 - 3s - 7s
    let action2_start = f.clock.nano_time();
    f.computer.action_started(&action2, action2_start);
    f.clock.advance_millis(1000);
    // Action 3 - 4s - 7s
    let action3_start = f.clock.nano_time();
    f.computer.action_started(&action3, action3_start);
    f.clock.advance_millis(3000);
    f.action_complete(action2_start, &action2);
    f.action_complete(action3_start, &action3);

    // The total run time should be 6s (Action 1 + Action 3) since Action 2 overlaps with action
    // 3, they will not be aggregated.
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_aggregated_elapsed_time(),
        duration_ofs(6)
    );
    let critical_path = f.computer.aggregate();
    assert_eq!(critical_path.components().len(), 2);
    // Action 3 has a run time of 3 seconds
    assert_eq!(
        critical_path.components()[0].get_elapsed_time(),
        duration_ofs(3)
    );
    // Action 1 has a run time of 3 seconds
    assert_eq!(
        critical_path.components()[1].get_elapsed_time(),
        duration_ofs(3)
    );
}

#[test]
fn rewound_action_may_start_twice() {
    // This test demonstrates that a rewound action can cause two ActionStartedEvents to be
    // emitted, one paired with an ActionRewoundEvent and the other with an
    // ActionCompletedEvent, and the CriticalPathComputer handles it.
    let f = Fixture::new();
    let producer = mock_action(vec![], vec![artifact("shared.out")]);
    let consumer = mock_action(vec![artifact("shared.out")], vec![artifact("consumer.out")]);

    f.simulate_action_exec_time(&producer, 10);
    let consumer_first_start = f.clock.nano_time();
    f.computer.action_started(&consumer, consumer_first_start);
    f.clock.advance_millis(5);
    f.computer
        .action_rewound(&consumer, consumer_first_start, f.clock.nano_time());

    // In a real rewinding case, "producer" would be re-evaluated, and the events for that
    // re-evaluation would be suppressed. This statement simulates that process by advancing the
    // clock without any associated events.
    f.clock.advance_millis(10);
    f.simulate_action_exec_time(&consumer, 20);

    let critical_path = f.computer.aggregate();

    assert_action_matches(&consumer, &critical_path.components()[0]);
    assert_action_matches(&producer, &critical_path.components()[1]);

    assert_eq!(
        critical_path.components()[0].get_elapsed_time(),
        duration_ofms(20)
    );
    assert_eq!(
        critical_path.components()[1].get_elapsed_time(),
        duration_ofms(10)
    );

    let slowest = f.computer.get_slowest_components();
    assert_eq!(slowest.len(), 2);
    for cpath in &slowest {
        if action_matches(&producer, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(10));
        }
        if action_matches(&consumer, cpath) {
            assert_eq!(cpath.get_elapsed_time(), duration_ofms(20));
        }
    }
}

/// Check that the slowest components list does not duplicate entries when an action has
/// multiple outputs.
#[test]
fn test_slowest_components_no_duplicates() {
    let f = Fixture::new();
    let action = mock_action(vec![], vec![artifact("a.out"), artifact("b.out")]);
    f.simulate_action_exec_time(&action, 123);

    let slowest = f.computer.get_slowest_components();
    assert_eq!(slowest.len(), 1);
}

#[test]
fn test_sequential_action_exec() {
    let f = Fixture::new();
    f.simulate_sequential_and_parallel_action_exec(
        &mock_action(vec![], vec![artifact("a.out")]),
        &[&[2 * 1000], &[3 * 1000], &[4 * 1000]],
    );
    let metrics = f
        .computer
        .get_max_critical_path()
        .unwrap()
        .get_spawn_metrics()
        .get_remote_metrics();
    assert_eq!(metrics.total_time_in_ms, 9 * 1000);
}

#[test]
fn test_maximum_sequential_and_parallel_action_metrics() {
    let f = Fixture::new();
    let action = mock_action(vec![], vec![artifact("a.out")]);
    f.simulate_sequential_and_parallel_action_exec(
        &action,
        &[
            &[5 * 1000],       // +5
            &[1000, 3 * 1000], // +3
            &[7 * 1000],       // +7
        ],
    );
    let metrics = f
        .computer
        .get_max_critical_path()
        .unwrap()
        .get_spawn_metrics()
        .get_remote_metrics();
    assert_eq!(metrics.total_time_in_ms, 15 * 1000);
}

#[test]
fn test_input_discovery_and_action() {
    let f = Fixture::new();
    let action = mock_action(vec![], vec![artifact("a.out")]);
    f.simulate_action_exec_discover(&action, 2 * 1000, 2 * 1000, true, 5 * 1000);
    let metrics = f
        .computer
        .get_max_critical_path()
        .unwrap()
        .get_spawn_metrics()
        .get_remote_metrics();
    assert_eq!(metrics.parse_time_in_ms, 5 * 1000);
    assert_eq!(metrics.execution_wall_time_in_ms, 2 * 1000);
    assert_eq!(metrics.total_time_in_ms, 7 * 1000);
}

#[test]
fn test_input_discovery_before_action_started() {
    let f = Fixture::new();
    let action = mock_action(vec![], vec![artifact("a.out")]);
    f.computer.discover_inputs(
        &action,
        /* start_time_nanos= */ 0,
        &remote(SpawnMetrics {
            parse_time_in_ms: 5 * 1000,
            total_time_in_ms: 5 * 1000,
            ..SpawnMetrics::default()
        }),
    );

    f.computer.action_complete(&action, 0, f.clock.nano_time());
    let metrics = f
        .computer
        .get_max_critical_path()
        .unwrap()
        .get_spawn_metrics()
        .get_remote_metrics();
    assert_eq!(metrics.parse_time_in_ms, 5 * 1000);
    assert_eq!(metrics.total_time_in_ms, 5 * 1000);
}

#[test]
fn test_try_add_component_should_add_non_shared_actions() {
    let f = Fixture::new();
    let artifact = artifact("a.out");
    let shared_action = mock_action(vec![], vec![artifact.clone()]);
    let non_shared_action =
        mock_action_shareable(vec![], vec![artifact], /* is_shareable= */ false);
    f.computer
        .action_started(&shared_action, f.clock.nano_time());
    let exception = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.computer
            .action_started(&non_shared_action, f.clock.nano_time())
    }))
    .expect_err("IllegalStateException");
    let message = exception
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(
        message.contains("Duplicate output artifact found for unsharable actions."),
        "{message}"
    );
}

#[test]
fn tolerates_critical_path_inconsistency() {
    let f = Fixture::new();
    let dep_artifact = derived_artifact("test/a.out");
    let parent_artifact = derived_artifact("test/b.out");
    let dep_action = mock_action(vec![], vec![dep_artifact.clone()]);
    let parent_action = mock_action(vec![dep_artifact], vec![parent_artifact]);

    f.computer.action_started(&dep_action, f.clock.nano_time());
    f.clock.advance_millis(1000);
    f.computer
        .action_started(&parent_action, f.clock.nano_time());

    // Complete the parent action while the dep action is still running and check that the
    // resulting critical path ignores the still-running dep.
    f.computer
        .action_complete(&parent_action, f.clock.nano_time(), f.clock.nano_time());
    let components = f.computer.aggregate().components().to_vec();
    assert_eq!(components.len(), 1);
    assert_action_matches(&parent_action, &components[0]);
}

/// PORT: Java's computer looks `action3` up in a fake `WalkableGraph`; the port's
/// `action_change_pruned` takes the looked-up action.
#[test]
fn test_change_pruning() {
    let f = Fixture::new();
    let action1 = mock_action(vec![], vec![derived_artifact("test/action1.out")]);
    let action2 = mock_action(
        vec![derived_artifact("test/action1.out")],
        vec![derived_artifact("test/action2.out")],
    );
    let action3 = mock_action(
        vec![derived_artifact("test/action2.out")],
        vec![derived_artifact("test/action3.out")],
    );
    let action4 = mock_action(
        vec![
            derived_artifact("test/action1.out"),
            derived_artifact("test/action3.out"),
        ],
        vec![derived_artifact("test/action4.out")],
    );

    // Action 1 - 0s - 1s
    let action1_start = f.clock.nano_time();
    f.computer.action_started(&action1, action1_start);
    f.clock.advance_millis(1000);
    f.action_complete(action1_start, &action1);
    // Action 2 - 1s - 3s
    let action2_start = f.clock.nano_time();
    f.computer.action_started(&action2, action2_start);
    f.clock.advance_millis(2000);
    f.action_complete(action2_start, &action2);
    // Action 3 - 3s - 3s, change pruned, no events
    f.computer
        .action_change_pruned(Some(&action3), f.clock.nano_time());
    // Action 4 - 3s - 6s
    let action4_start = f.clock.nano_time();
    f.computer.action_started(&action4, action4_start);
    f.clock.advance_millis(3000);
    f.action_complete(action4_start, &action4);

    // The total run time should be 6s (Action 1 + Action 2 + Action 4) since Action 3 is
    // change-pruned.
    assert_eq!(
        f.computer
            .get_max_critical_path()
            .unwrap()
            .get_aggregated_elapsed_time(),
        duration_ofs(6)
    );
    let critical_path = f.computer.aggregate();
    assert_eq!(critical_path.components().len(), 4);
    let c = critical_path.components();
    // Action 4 has a run time of 3 seconds
    assert!(c[0].pretty_print_action().contains("action4.out"));
    assert_eq!(c[0].get_elapsed_time(), duration_ofs(3));
    // Action 3 has a run time of 0 seconds
    assert!(c[1].pretty_print_action().contains("action3.out"));
    assert_eq!(c[1].get_elapsed_time(), 0);
    // Action 2 has a run time of 2 seconds
    assert!(c[2].pretty_print_action().contains("action2.out"));
    assert_eq!(c[2].get_elapsed_time(), duration_ofs(2));
    // Action 1 has a run time of 1 seconds
    assert!(c[3].pretty_print_action().contains("action1.out"));
    assert_eq!(c[3].get_elapsed_time(), duration_ofs(1));
}

/// `AggregatedCriticalPath.toStringSummaryNoRemote()`: the item of Bazel's
/// `INFO: Elapsed time: ..., Critical Path: ...` line, and `EMPTY` prints `0.00s`.
#[test]
fn to_string_summary_no_remote_is_the_elapsed_time_item() {
    let f = Fixture::new();
    assert_eq!(
        f.computer.aggregate().to_string_summary_no_remote(),
        "Critical Path: 0.00s"
    );
    f.simulate_action_exec_time(&null_action(), 1234);
    assert_eq!(
        f.computer.aggregate().to_string_summary_no_remote(),
        "Critical Path: 1.23s"
    );
}
