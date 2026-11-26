//! Port of src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java.
//!
//! PORT: Java's mocks are the fixtures of `dynamic_common`; `verify(..., never())` and
//! `verifyNoInteractions` read their call records. Java's strategy runs on an executor service
//! that the test shuts down; here the branch threads end inside `exec`.

mod dynamic_common;

use std::sync::Arc;

use blaze_exec::actions::spawn_strategy::{SpawnStrategy, StopConcurrentSpawns};
use blaze_exec::actions::{ExecException, Spawn, SpawnResult, SpawnResultBuilder, Status};
use blaze_exec::dynamic::branch::Branch;
use blaze_exec::dynamic::dynamic_spawn_strategy::{
    DynamicExecutionFinishedEvent, DynamicSpawnStrategy, IgnoreFailureCheck, PostProcessingSpawn,
    wait_branches,
};
use blaze_exec::exec::action_execution_context::ActionExecutionContext;
use blaze_exec::exec::execution_policy::ExecutionPolicy;
use blaze_exec::exec::spawn_strategy_registry::DynamicMode;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_options::Options;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use dynamic_common::*;

const OUTPUT1: &str = "root/dir/output1.txt";
const OUTPUT2: &str = "root/dir/output2.txt";

fn result(runner_name: &str) -> SpawnResult {
    SpawnResultBuilder::new()
        .set_runner_name(runner_name)
        .set_status(Status::Success)
        .build()
}

/// `SUCCESSFUL_SPAWN_RESULT`, `SUCCESSFUL_LOCAL_SPAWN_RESULT`, `SUCCESSFUL_REMOTE_SPAWN_RESULT`.
fn successful() -> SpawnResult {
    result("test")
}

/// What the tests compare of a result (Java compares the objects).
fn key(r: &SpawnResult) -> (String, Status, i32) {
    (r.get_runner_name().to_string(), r.status(), r.exit_code())
}

fn keys(results: &[SpawnResult]) -> Vec<(String, Status, i32)> {
    results.iter().map(key).collect()
}

/// `FAILURE_DETAIL`.
fn failure_detail() -> FailureDetail {
    FailureDetail {
        message: String::new(),
        category: Some(failure_detail::Category::Execution(
            failure_details::Execution::default(),
        )),
    }
}

fn options() -> DynamicExecutionOptions {
    Options::<DynamicExecutionOptions>::get_defaults()
}

/// `mockGetPostProcessingSpawn`: the post-processing spawn for any spawn (`Optional.empty()`
/// when `None`), with its calls recorded.
struct MockPostProcessing {
    extra: Option<Arc<dyn Spawn>>,
}

impl PostProcessingSpawn for MockPostProcessing {
    fn get(&self, _spawn: &dyn Spawn) -> Option<Box<dyn Spawn>> {
        self.extra
            .clone()
            .map(|s| Box::new(SpawnRef(s)) as Box<dyn Spawn>)
    }
}

/// A `Spawn` that forwards to a shared one.
#[derive(Debug)]
struct SpawnRef(Arc<dyn Spawn>);

impl Spawn for SpawnRef {
    fn get_execution_info(&self) -> &indexmap::IndexMap<String, String> {
        self.0.get_execution_info()
    }
    fn get_arguments(&self) -> &[String] {
        self.0.get_arguments()
    }
    fn get_environment(&self) -> &indexmap::IndexMap<String, String> {
        self.0.get_environment()
    }
    fn get_tool_files(&self) -> &blaze_collect::NestedSet<blaze_exec::actions::ActionInputRef> {
        self.0.get_tool_files()
    }
    fn get_input_files(&self) -> &blaze_collect::NestedSet<blaze_exec::actions::ActionInputRef> {
        self.0.get_input_files()
    }
    fn get_output_files(&self) -> &[blaze_exec::actions::ActionInputRef] {
        self.0.get_output_files()
    }
    fn get_resource_owner(&self) -> &dyn blaze_exec::actions::ActionExecutionMetadata {
        self.0.get_resource_owner()
    }
    fn get_local_resources(&self) -> Result<blaze_exec::actions::ResourceSet, ExecException> {
        self.0.get_local_resources()
    }
    fn get_mnemonic(&self) -> String {
        self.0.get_mnemonic()
    }
    fn get_execution_platform(&self) -> Option<&blaze_exec::actions::PlatformInfo> {
        self.0.get_execution_platform()
    }
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:732 DynamicSpawnStrategyUnitTest.createDynamicSpawnStrategy
fn create_dynamic_spawn_strategy(
    execution_policy: ExecutionPolicy,
    post_processing: Arc<dyn PostProcessingSpawn>,
    options: DynamicExecutionOptions,
    ignore_failure_check: Option<Arc<dyn IgnoreFailureCheck>>,
) -> Arc<DynamicSpawnStrategy> {
    DynamicSpawnStrategy::new(
        Arc::new(options),
        Arc::new(move |_| execution_policy),
        post_processing,
        10,
        10,
        ignore_failure_check,
    )
}

fn no_post_processing() -> Arc<dyn PostProcessingSpawn> {
    Arc::new(MockPostProcessing { extra: None })
}

fn post_processing(extra: Arc<dyn Spawn>) -> Arc<dyn PostProcessingSpawn> {
    Arc::new(MockPostProcessing { extra: Some(extra) })
}

fn answer(
    f: impl Fn(&dyn Spawn, Option<&dyn StopConcurrentSpawns>) -> Result<Vec<SpawnResult>, ExecException>
    + Send
    + Sync
    + 'static,
) -> ExecAnswer {
    Arc::new(f)
}

fn user_exec_message(e: ExecException) -> String {
    match e {
        ExecException::User(e) => e.message,
        other => panic!("expected a UserExecException, got {other:?}"),
    }
}

fn finished_events(context: &TestContext) -> Vec<DynamicExecutionFinishedEvent> {
    context
        .event_handler
        .get_posts()
        .iter()
        .filter_map(|p| {
            let any: &dyn std::any::Any = &**p;
            any.downcast_ref::<DynamicExecutionFinishedEvent>().cloned()
        })
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:141 DynamicSpawnStrategyUnitTest.exec_remoteOnlySpawn_doesNotExecLocalPostProcessingSpawn
#[test]
fn exec_remote_only_spawn_does_not_exec_local_post_processing_spawn() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::REMOTE_EXECUTION_ONLY,
        no_post_processing(),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", true);
    remote.when_exec(answer(|_, _| Ok(vec![successful()])));
    let context = TestContext::with_strategies(local.clone(), remote.clone());

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful()]));
    assert_eq!(local.exec_count(), 0);
    assert_eq!(remote.exec_count(), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:161 DynamicSpawnStrategyUnitTest.exec_remoteOnlySpawn_noneCanExec_fails
#[test]
fn exec_remote_only_spawn_none_can_exec_fails() {
    let spawn = spawn(&[], "TheThing", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::REMOTE_EXECUTION_ONLY,
        no_post_processing(),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", false);
    let context = TestContext::with_strategies(local.clone(), remote);

    let thrown = user_exec_message(strategy.exec(&*spawn, &context).unwrap_err());
    assert_eq!(
        thrown,
        "Spawn is not executable in local: No usable dynamic_remote_strategy found (and local execution disabled) for action TheThing. "
    );
    assert!(!thrown.contains("dynamic_local_strategy"));
    assert!(local.no_interactions());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:185 DynamicSpawnStrategyUnitTest.exec_localOnlySpawn_runsLocalPostProcessingSpawn
#[test]
fn exec_local_only_spawn_runs_local_post_processing_spawn() {
    let spawn = spawn(&["command"], "Mnemonic", OUTPUT1, false);
    let extra: Arc<dyn Spawn> =
        dynamic_common::spawn(&["extra_command"], "Mnemonic", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::LOCAL_EXECUTION_ONLY,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    local.when_exec(answer(|_, _| Ok(vec![successful()])));
    let remote = MockStrategy::new("remote", true);
    let context = TestContext::with_strategies(local.clone(), remote.clone());

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful(), successful()]));
    assert!(remote.no_interactions());
    let calls: Vec<Vec<String>> = local
        .exec_calls
        .lock()
        .iter()
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(
        calls,
        vec![
            vec!["command".to_string()],
            vec!["extra_command".to_string()]
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:208 DynamicSpawnStrategyUnitTest.exec_localOnlySpawn_noneCanExec_fails
#[test]
fn exec_local_only_spawn_none_can_exec_fails() {
    let spawn = spawn(&[], "ThisMnemonic1", OUTPUT1, false);
    let extra: Arc<dyn Spawn> = dynamic_common::spawn(&[], "ThatMnemonic2", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::LOCAL_EXECUTION_ONLY,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", false);
    let remote = MockStrategy::new("remote", true);
    let context = TestContext::with_strategies(local, remote.clone());

    let thrown = user_exec_message(strategy.exec(&*spawn, &context).unwrap_err());
    assert_eq!(
        thrown,
        "Spawn is not executable in local: No usable dynamic_local_strategy found (and remote execution disabled) for action ThisMnemonic1. Post-Processing Spawn is not executable in local: No usable dynamic_local_strategy found (and remote execution disabled) for action ThatMnemonic2. "
    );
    assert!(!thrown.contains("dynamic_remote_strategy"));
    assert!(remote.no_interactions());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:235 DynamicSpawnStrategyUnitTest.exec_localOnlySpawnWithNonExecutablePostProcessingSpawn_doesNotExecLocalSpawn
#[test]
fn exec_local_only_spawn_with_non_executable_post_processing_spawn_does_not_exec_local_spawn() {
    let spawn = spawn(&[], "ThisMnemonic1", OUTPUT1, false);
    let extra: Arc<dyn Spawn> = dynamic_common::spawn(&[], "ThatMnemonic2", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::LOCAL_EXECUTION_ONLY,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let local_for_registry = local.clone();
    let context = TestContext::new(Arc::new(TestDynamicStrategyRegistry {
        local: Arc::new(move |spawn: &dyn Spawn| {
            if spawn.get_mnemonic() == "ThisMnemonic1" {
                vec![local_for_registry.clone() as Arc<dyn SpawnStrategy>]
            } else {
                vec![]
            }
        }),
        remote: Arc::new(|_| vec![]),
    }));

    let thrown = user_exec_message(strategy.exec(&*spawn, &context).unwrap_err());
    assert_eq!(
        thrown,
        "Post-Processing Spawn is not executable in local: No usable dynamic_local_strategy found (and remote execution disabled) for action ThatMnemonic2. "
    );
    assert!(!thrown.contains("dynamic_remote_strategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:315 DynamicSpawnStrategyUnitTest.testExecFailedLocalSpawnDoesNotExecLocalPostProcessingSpawn
fn test_exec_failed_local_spawn_does_not_exec_local_post_processing_spawn(failed: SpawnResult) {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let extra: Arc<dyn Spawn> = dynamic_common::spawn(&["post"], "Post", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::LOCAL_EXECUTION_ONLY,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let failed_for_answer = failed.clone();
    local.when_exec(answer(move |_, _| Ok(vec![failed_for_answer.clone()])));
    let remote = MockStrategy::new("remote", true);
    let context = TestContext::with_strategies(local.clone(), remote.clone());

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[failed]));
    // Only the spawn ran: verifyNoInteractions(postProcessingSpawn).
    assert_eq!(local.exec_count(), 1);
    assert_eq!(remote.exec_count(), 0);
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:292 DynamicSpawnStrategyUnitTest.exec_failedLocalSpawn_doesNotExecLocalPostProcessingSpawn
#[test]
fn exec_failed_local_spawn_does_not_exec_local_post_processing_spawn() {
    test_exec_failed_local_spawn_does_not_exec_local_post_processing_spawn(
        SpawnResultBuilder::new()
            .set_runner_name("test")
            .set_status(Status::Timeout)
            .set_exit_code(blaze_exec::actions::spawn_result::POSIX_TIMEOUT_EXIT_CODE)
            .set_failure_detail(failure_detail())
            .build(),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:303 DynamicSpawnStrategyUnitTest.exec_nonZeroExitCodeLocalSpawn_doesNotExecLocalPostProcessingSpawn
#[test]
fn exec_non_zero_exit_code_local_spawn_does_not_exec_local_post_processing_spawn() {
    test_exec_failed_local_spawn_does_not_exec_local_post_processing_spawn(
        SpawnResultBuilder::new()
            .set_runner_name("test")
            .set_status(Status::ExecutionFailed)
            .set_exit_code(123)
            .set_failure_detail(failure_detail())
            .build(),
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:338 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_runsLocalPostProcessingSpawn
#[test]
fn exec_run_anywhere_spawn_runs_local_post_processing_spawn() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let extra: Arc<dyn Spawn> =
        dynamic_common::spawn(&["extra_command"], "Mnemonic", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    // Make sure that local execution does not win the race before remote starts.
    let remote_started = TestSemaphore::new(0);
    let rs = remote_started.clone();
    // Only the first spawn should be able to stop the concurrent remote execution.
    local.when_exec(answer(move |spawn, stop| {
        if spawn.get_arguments().is_empty() {
            let stop = stop.expect("isNotNull");
            rs.acquire()?;
            stop.stop(0, "", None)?;
        } else {
            assert!(stop.is_none(), "isNull");
        }
        Ok(vec![successful()])
    }));
    let remote = MockStrategy::new("remote", true);
    let rs = remote_started.clone();
    remote.when_exec(answer(move |_, _| {
        rs.release();
        sleep(60_000)?;
        panic!("java.lang.AssertionError: Timed out waiting for interruption");
    }));
    let context = TestContext::with_strategies(local, remote);

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful(), successful()]));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:375 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_localWins
#[test]
fn exec_run_anywhere_spawn_local_wins() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        no_post_processing(),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", true);
    let remote_started = TestSemaphore::new(0);
    let remote_done = TestSemaphore::new(0);
    let (rs, rd) = (remote_started.clone(), remote_done.clone());
    remote.when_exec(answer(move |_, stop| {
        rs.release();
        rd.acquire()?;
        stop.expect("isNotNull").stop(0, "", None)?;
        Ok(vec![result("remote")])
    }));
    let rs = remote_started.clone();
    local.when_exec(answer(move |_, stop| {
        rs.acquire()?;
        stop.expect("isNotNull").stop(0, "", None)?;
        Ok(vec![result("local")])
    }));
    let context = TestContext::with_strategies(local, remote);

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[result("local")]));
    let events = finished_events(&context);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].winner_branch_type, DynamicMode::Local);
    assert_eq!(events[0].remote_branch_name, "remote");
    assert_eq!(events[0].local_branch_name, "local");
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:415 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_remoteWins
#[test]
fn exec_run_anywhere_spawn_remote_wins() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        no_post_processing(),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", true);
    let remote_started = TestSemaphore::new(0);
    let local_done = TestSemaphore::new(0);
    let rs = remote_started.clone();
    remote.when_exec(answer(move |_, stop| {
        rs.release();
        stop.expect("isNotNull").stop(0, "", None)?;
        Ok(vec![result("remote")])
    }));
    let (rs, ld) = (remote_started.clone(), local_done.clone());
    local.when_exec(answer(move |_, stop| {
        rs.acquire()?;
        ld.acquire()?;
        stop.expect("isNotNull").stop(0, "", None)?;
        Ok(vec![result("local")])
    }));
    let context = TestContext::with_strategies(local, remote);

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[result("remote")]));
    let events = finished_events(&context);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].winner_branch_type, DynamicMode::Remote);
    assert_eq!(events[0].remote_branch_name, "remote");
    assert_eq!(events[0].local_branch_name, "local");
}

/// `(s, context, exitCode, errorMsg, outErr, isLocal) -> isLocal && errorMsg.contains("Ignorable")`.
struct IgnorableCheck;

impl IgnoreFailureCheck for IgnorableCheck {
    fn can_ignore_failure(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn ActionExecutionContext,
        _exit_code: i32,
        error_message: &str,
        _out_err: Option<&FileOutErr>,
        is_local: bool,
    ) -> bool {
        is_local && error_message.contains("Ignorable")
    }
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:455 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_allowsIgnoringFailure
#[test]
fn exec_run_anywhere_spawn_allows_ignoring_failure() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        no_post_processing(),
        options(),
        Some(Arc::new(IgnorableCheck)),
    );
    let local = MockStrategy::new("local", true);
    let remote_started = TestSemaphore::new(0);
    let rs = remote_started.clone();
    local.when_exec(answer(move |_, stop| {
        rs.acquire()?;
        stop.expect("isNotNull")
            .stop(1, "Ignorable failure", None)?;
        // We should never get here, so return a different result from remote.
        Ok(vec![successful(), successful()])
    }));
    let remote = MockStrategy::new("remote", true);
    let rs = remote_started.clone();
    remote.when_exec(answer(move |_, _| {
        rs.release();
        sleep(10)?;
        Ok(vec![successful()])
    }));
    let context = TestContext::with_strategies(local, remote);

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful()]));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:500 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_notAlwaysIgnoringFailure
#[test]
fn exec_run_anywhere_spawn_not_always_ignoring_failure() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        no_post_processing(),
        options(),
        Some(Arc::new(IgnorableCheck)),
    );
    let local = MockStrategy::new("local", true);
    let remote_started = TestSemaphore::new(0);
    let local_done = TestSemaphore::new(0);
    let rs = remote_started.clone();
    local.when_exec(answer(move |_, stop| {
        rs.acquire()?;
        stop.expect("isNotNull")
            .stop(1, "Not an ignorable failure", None)?;
        Ok(vec![successful(), successful()])
    }));
    let remote = MockStrategy::new("remote", true);
    let (rs, ld) = (remote_started.clone(), local_done.clone());
    remote.when_exec(answer(move |_, _| {
        rs.release();
        ld.acquire()?;
        Ok(vec![successful()])
    }));
    let context = TestContext::with_strategies(local, remote);

    let results = strategy.exec(&*spawn, &context).unwrap();
    local_done.release();
    assert_eq!(keys(&results), keys(&[successful(), successful()]));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:545 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_excludeTools_onlyRemote
#[test]
fn exec_run_anywhere_spawn_exclude_tools_only_remote() {
    let spawn = spawn(&[], "TheThing", OUTPUT1, true);
    let mut options = options();
    options.exclude_tools = true;
    options.local_execution_delay = 0;
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        no_post_processing(),
        options,
        None,
    );
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", true);
    remote.when_exec(answer(|_, _| Ok(vec![successful()])));
    let context = TestContext::with_strategies(local.clone(), remote);

    let spawn_results = strategy
        .maybe_execute_non_dynamically(&*spawn, &context)
        .unwrap();

    let spawn_results = spawn_results.expect("Should have been executed remote-only");
    assert_eq!(keys(&spawn_results), keys(&[successful()]));
    assert_eq!(local.exec_count(), 0);
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:575 DynamicSpawnStrategyUnitTest.waitBranches_givesDebugOutputIfBothCancelled
#[test]
fn wait_branches_gives_debug_output_if_both_cancelled() {
    let spawn = spawn(&[], "Mnemonic", "/foo", false);
    let local = MockStrategy::new("local", true);
    let remote = MockStrategy::new("remote", true);
    let context = TestContext::with_strategies(local, remote);
    let options = Arc::new(options());
    let local_branch = Branch::new(DynamicMode::Local, options.clone());
    let remote_branch = Branch::new(DynamicMode::Remote, options.clone());
    local_branch.prepare_future(&remote_branch);
    remote_branch.prepare_future(&local_branch);
    local_branch.cancel();
    remote_branch.cancel();
    let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_branches(&local_branch, &remote_branch, &*spawn, &options, &context)
    }))
    .expect_err("AssertionError");
    let message = blaze_util::uncaught_exception::panic_message(&*error);
    assert!(
        message.contains("Neither branch of /foo completed."),
        "{message}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:605 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_localCantExec_runsRemote
#[test]
fn exec_run_anywhere_spawn_local_cant_exec_runs_remote() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let extra: Arc<dyn Spawn> = dynamic_common::spawn(&["post"], "Post", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", false);
    let remote = MockStrategy::new("remote", true);
    remote.when_exec(answer(|_, stop| {
        if let Some(stop) = stop {
            stop.stop(0, "", None)?;
        }
        Ok(vec![successful()])
    }));
    let context = TestContext::with_strategies(local.clone(), remote);

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful()]));
    // Never runs anything as it says it can't execute anything at all.
    assert_eq!(local.exec_count(), 0);
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:632 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_remoteCantExec_runsLocal
#[test]
fn exec_run_anywhere_spawn_remote_cant_exec_runs_local() {
    let spawn = spawn(&[], "Mnemonic", OUTPUT1, false);
    let extra: Arc<dyn Spawn> =
        dynamic_common::spawn(&["extra_command"], "Mnemonic", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", true);
    local.when_exec(answer(|_, stop| {
        if let Some(stop) = stop {
            stop.stop(0, "", None)?;
        }
        Ok(vec![successful()])
    }));
    let remote = MockStrategy::new("remote", false);
    let context = TestContext::with_strategies(local.clone(), remote.clone());

    let results = strategy.exec(&*spawn, &context).unwrap();

    assert_eq!(keys(&results), keys(&[successful(), successful()]));
    let calls: Vec<Vec<String>> = local
        .exec_calls
        .lock()
        .iter()
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(
        calls,
        vec![Vec::<String>::new(), vec!["extra_command".to_string()]]
    );
    assert_eq!(remote.exec_count(), 0);
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyUnitTest.java:660 DynamicSpawnStrategyUnitTest.exec_runAnywhereSpawn_noneCanExec_fails
#[test]
fn exec_run_anywhere_spawn_none_can_exec_fails() {
    let spawn = spawn(&[], "ThisMnemonic1", OUTPUT1, false);
    let extra: Arc<dyn Spawn> = dynamic_common::spawn(&[], "ThatMnemonic2", OUTPUT2, false);
    let strategy = create_dynamic_spawn_strategy(
        ExecutionPolicy::ANYWHERE,
        post_processing(extra),
        options(),
        None,
    );
    let local = MockStrategy::new("local", false);
    let remote = MockStrategy::new("remote", false);
    let context = TestContext::with_strategies(local, remote);

    let thrown = user_exec_message(strategy.exec(&*spawn, &context).unwrap_err());
    assert_eq!(
        thrown,
        "Spawn is not executable in local: No usable dynamic_local_strategy or dynamic_remote_strategy found for action ThisMnemonic1. Post-Processing Spawn is not executable in local: No usable dynamic_local_strategy or dynamic_remote_strategy found for action ThatMnemonic2. "
    );
}
