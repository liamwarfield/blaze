//! Port of src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java.
//!
//! PORT: the strategies are registered on a real `SpawnStrategyRegistry` together with the
//! dynamic module's registration (`dynamic_execution_module::register_spawn_strategies`), as in
//! Java; the `ActionExecutionContext` is the `dynamic_common` fixture over that registry with the
//! test's `FileOutErr`. Java's `CountDownLatch`es are [`Latch`]; a `RuntimeException` of a
//! strategy is a panic. The mock strategy does not write the spawn's output files (no test reads
//! them).

mod dynamic_common;

use std::fmt;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use blaze_events::{EventHandler, StoredEventHandler};
use blaze_exec::actions::spawn_strategy::{
    SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns,
};
use blaze_exec::actions::{ExecException, Spawn, SpawnResult, UserExecException};
use blaze_exec::dynamic::dynamic_execution_module::{
    ModuleIgnoreFailureCheck, register_spawn_strategies,
};
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::spawn_strategy_registry::{DynamicStrategyRegistry, SpawnStrategyRegistry};
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_options::Options;
use blaze_protos::failure_details::{self, FailureDetail, dynamic_execution::Code, failure_detail};
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};
use dynamic_common::*;
use parking_lot::{Condvar, Mutex};

/// `TestUtils.WAIT_TIMEOUT_MILLISECONDS`.
const WAIT_TIMEOUT_MILLISECONDS: u64 = 60_000;

/// A `java.util.concurrent.CountDownLatch` whose `await` is interruptible.
struct Latch {
    count: Mutex<usize>,
    cv: Condvar,
}

impl Latch {
    fn new(count: usize) -> Arc<Latch> {
        Arc::new(Latch {
            count: Mutex::new(count),
            cv: Condvar::new(),
        })
    }
    fn count_down(&self) {
        let mut c = self.count.lock();
        if *c > 0 {
            *c -= 1;
        }
        self.cv.notify_all();
    }
    fn wait(&self) -> Result<(), ExecException> {
        let mut c = self.count.lock();
        while *c > 0 {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(ExecException::Interrupted);
            }
            self.cv.wait_for(&mut c, Duration::from_millis(10));
        }
        Ok(())
    }
}

/// Syntactic sugar to decrease and await for a latch in a single line.
fn count_down_and_wait(latch: &Latch) -> Result<(), ExecException> {
    latch.count_down();
    latch.wait()
}

/// Hook to implement per-test custom logic in the [`MockSpawnStrategy`].
type DoExec = Arc<
    dyn Fn(&MockSpawnStrategy, &dyn Spawn, &dyn ActionExecutionContext) -> Result<(), ExecException>
        + Send
        + Sync,
>;

fn nothing() -> DoExec {
    Arc::new(|_, _, _| Ok(()))
}

fn append_iso_latin1(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.get_path_string())
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1273 DynamicSpawnStrategyTest.createFailureDetail
fn create_failure_detail(message: &str) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(failure_detail::Category::DynamicExecution(
            failure_details::DynamicExecution {
                code: Code::RunFailure as i32,
            },
        )),
    }
}

/// Minimal implementation of a strategy for testing purposes.
struct MockSpawnStrategy {
    name: String,
    /// The address of the spawn passed to `exec` (0: none yet).
    executed_spawn: AtomicUsize,
    succeeded: AtomicBool,
    do_exec_before_stop: DoExec,
    do_exec_after_stop: DoExec,
    can_exec: bool,
    out_err: FileOutErr,
}

fn address(spawn: &dyn Spawn) -> usize {
    spawn as *const dyn Spawn as *const () as usize
}

impl MockSpawnStrategy {
    fn new(
        name: &str,
        before: DoExec,
        after: DoExec,
        can_exec: bool,
        out_err: &FileOutErr,
    ) -> Arc<MockSpawnStrategy> {
        Arc::new(MockSpawnStrategy {
            name: name.to_string(),
            executed_spawn: AtomicUsize::new(0),
            succeeded: AtomicBool::new(false),
            do_exec_before_stop: before,
            do_exec_after_stop: after,
            can_exec,
            out_err: out_err.clone(),
        })
    }

    /// Helper to record an execution failure from within `doExecBeforeStop`.
    fn fail_execution(&self, context: &dyn ActionExecutionContext) -> Result<(), ExecException> {
        append_iso_latin1(
            context.get_file_out_err().get_output_path(),
            &format!("action failed with {}", self.name),
        );
        Err(UserExecException::new(create_failure_detail(&format!(
            "{} failed to execute the Spawn",
            self.name
        )))
        .into())
    }

    fn executed(&self, spawn: &dyn Spawn) -> bool {
        self.executed_spawn.load(Ordering::SeqCst) == address(spawn)
    }

    fn executed_nothing(&self) -> bool {
        self.executed_spawn.load(Ordering::SeqCst) == 0
    }

    fn succeeded(&self) -> bool {
        self.succeeded.load(Ordering::SeqCst)
    }
}

impl fmt::Display for MockSpawnStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl SpawnStrategy for MockSpawnStrategy {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        panic!("java.lang.IllegalStateException: Not expected to be called")
    }
    fn can_exec(&self, _spawn: &dyn Spawn, _registry: &dyn ActionContextRegistry) -> bool {
        self.can_exec
    }
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        Some(self)
    }
    fn get_simple_class_name(&self) -> String {
        "MockSpawnStrategy".to_string()
    }
}

impl SandboxedSpawnStrategy for MockSpawnStrategy {
    fn exec_with_stop_concurrent_spawns(
        &self,
        spawn: &dyn Spawn,
        context: &dyn ActionExecutionContext,
        stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.executed_spawn.store(address(spawn), Ordering::SeqCst);
        (self.do_exec_before_stop)(self, spawn, context)?;
        if let Some(stop) = stop_concurrent_spawns {
            stop.stop(0, "", Some(&self.out_err))?;
            (self.do_exec_after_stop)(self, spawn, context)?;
        }
        append_iso_latin1(
            context.get_file_out_err().get_output_path(),
            &format!("output files written with {}", self.name),
        );
        self.succeeded.store(true, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    out_err: FileOutErr,
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:233 DynamicSpawnStrategyTest.setUp
fn set_up() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let test_root = fs.get_path(tmp.path().to_str().unwrap());
    Fixture {
        out_err: FileOutErr::new(test_root.get_child("stdout"), test_root.get_child("stderr")),
        _tmp: tmp,
    }
}

impl Fixture {
    fn strategy(&self, name: &str, before: DoExec, after: DoExec) -> Arc<MockSpawnStrategy> {
        MockSpawnStrategy::new(name, before, after, true, &self.out_err)
    }

    fn simple(&self, name: &str) -> Arc<MockSpawnStrategy> {
        self.strategy(name, nothing(), nothing())
    }

    fn cannot_exec(&self, name: &str) -> Arc<MockSpawnStrategy> {
        MockSpawnStrategy::new(name, nothing(), nothing(), false, &self.out_err)
    }

    /// `outErr.outAsLatin1()`.
    fn out(&self) -> String {
        std::fs::read_to_string(self.out_err.get_output_path().get_path_string())
            .unwrap_or_default()
    }

    // Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:301 DynamicSpawnStrategyTest.createSpawnStrategyWithExecutor
    fn create_spawn_strategy(
        &self,
        local: &Arc<MockSpawnStrategy>,
        remote: &Arc<MockSpawnStrategy>,
        sandboxed: Option<&Arc<MockSpawnStrategy>>,
    ) -> StrategyAndContext {
        let mut dynamic_local_strategies = vec![(String::new(), vec!["mock-local".to_string()])];
        let mut dynamic_remote_strategies = vec![(String::new(), vec!["mock-remote".to_string()])];
        if sandboxed.is_some() {
            dynamic_local_strategies.push((
                "testMnemonic".to_string(),
                vec!["mock-sandboxed".to_string()],
            ));
            dynamic_remote_strategies.push((
                "testMnemonic".to_string(),
                vec!["mock-sandboxed".to_string()],
            ));
        }
        let mut options = Options::<DynamicExecutionOptions>::get_defaults();
        options.dynamic_local_strategy = dynamic_local_strategies;
        options.dynamic_remote_strategy = dynamic_remote_strategies;
        options.internal_spawn_scheduler = true;
        options.local_execution_delay = 0;

        let mut builder = SpawnStrategyRegistry::builder();
        builder.register_strategy(local.clone(), &["mock-local"]);
        builder.register_strategy(remote.clone(), &["mock-remote"]);
        if let Some(sandboxed) = sandboxed {
            builder.register_strategy(sandboxed.clone(), &["mock-sandboxed"]);
        }
        // new DynamicExecutionModule(executorService) without beforeCommand: no local options,
        // no ignored signals.
        register_spawn_strategies(
            &mut builder,
            &Arc::new(options),
            None,
            10,
            10,
            Arc::new(ModuleIgnoreFailureCheck {
                ignore_local_signals: Vec::new(),
                verbose_failures: false,
                reporter: Arc::new(StoredEventHandler::new()) as Arc<dyn EventHandler>,
            }),
        )
        .ok()
        .unwrap();
        let registry = Arc::new(builder.build().ok().unwrap());
        let strategy = registry
            .get_strategies(&*custom_spawn("RunDynamic", &[]), None)
            .into_iter()
            .find(|s| s.get_simple_class_name() == "DynamicSpawnStrategy")
            .expect("Expected module to register a dynamic strategy");
        StrategyAndContext {
            strategy,
            context: Arc::new(TestContext::with_out_err(
                registry as Arc<dyn DynamicStrategyRegistry>,
                self.out_err.clone(),
            )),
        }
    }
}

struct StrategyAndContext {
    strategy: Arc<dyn SpawnStrategy>,
    context: Arc<TestContext>,
}

impl StrategyAndContext {
    fn exec(&self, spawn: &dyn Spawn) -> Result<Vec<SpawnResult>, ExecException> {
        self.strategy.exec(spawn, &*self.context)
    }
}

/// `newDynamicSpawn()`.
fn new_dynamic_spawn() -> Arc<blaze_exec::actions::SimpleSpawn> {
    custom_spawn("Null", &[])
}

fn exec_message(e: &ExecException) -> String {
    match e {
        ExecException::User(e) => e.message.clone(),
        other => other.get_message(),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:416 DynamicSpawnStrategyTest.nonRemotableSpawnRunsLocally
#[test]
fn non_remotable_spawn_runs_locally() {
    let f = set_up();
    let local = f.simple("MockLocalSpawnStrategy");
    let remote = f.simple("MockRemoteSpawnStrategy");
    let sc = f.create_spawn_strategy(&local, &remote, None);

    let spawn = custom_spawn("Null", &[("local", "1")]);
    sc.exec(&*spawn).unwrap();

    assert!(local.executed(&*spawn));
    assert!(local.succeeded());
    assert!(remote.executed_nothing());
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
    assert!(!f.out().contains("MockRemoteSpawnStrategy"));
}

fn uses_strategy_by_mnemonic(execution_info: (&str, &str)) {
    let f = set_up();
    let local = f.simple("MockLocalSpawnStrategy");
    let remote = f.simple("MockRemoteSpawnStrategy");
    let sandboxed = f.simple("MockSandboxedSpawnStrategy");
    let sc = f.create_spawn_strategy(&local, &remote, Some(&sandboxed));

    let spawn = custom_spawn("testMnemonic", &[execution_info]);
    sc.exec(&*spawn).unwrap();

    assert!(local.executed_nothing());
    assert!(!local.succeeded());
    assert!(remote.executed_nothing());
    assert!(!remote.succeeded());
    assert!(sandboxed.executed(&*spawn));
    assert!(sandboxed.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockSandboxedSpawnStrategy")
    );
    assert!(!f.out().contains("MockLocalSpawnStrategy"));
    assert!(!f.out().contains("MockRemoteSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:433 DynamicSpawnStrategyTest.localSpawnUsesStrategyByMnemonicWithWorkerFlagDisabled
#[test]
fn local_spawn_uses_strategy_by_mnemonic_with_worker_flag_disabled() {
    uses_strategy_by_mnemonic(("local", "1"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:455 DynamicSpawnStrategyTest.remoteSpawnUsesStrategyByMnemonic
#[test]
fn remote_spawn_uses_strategy_by_mnemonic() {
    uses_strategy_by_mnemonic(("remote", "1"));
}

/// The four "X succeeds even if Y fails later" and "X fails immediately" tests: `fast` and
/// `slow` are the hooks of the two branches.
fn race(
    local_hook: impl Fn(&Arc<Latch>) -> DoExec,
    remote_hook: impl Fn(&Arc<Latch>) -> DoExec,
) -> (
    Fixture,
    Arc<MockSpawnStrategy>,
    Arc<MockSpawnStrategy>,
    Result<(), ExecException>,
    usize,
) {
    let f = set_up();
    let latch = Latch::new(2);
    let local = f.strategy("MockLocalSpawnStrategy", local_hook(&latch), nothing());
    let remote = f.strategy("MockRemoteSpawnStrategy", remote_hook(&latch), nothing());
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    let result = sc.exec(&*spawn).map(|_| ());
    let addr = address(&*spawn);
    (f, local, remote, result, addr)
}

fn waits(latch: &Arc<Latch>) -> DoExec {
    let latch = latch.clone();
    Arc::new(move |_, _, _| count_down_and_wait(&latch))
}

fn waits_then_sleeps_then_fails(latch: &Arc<Latch>) -> DoExec {
    let latch = latch.clone();
    Arc::new(move |s, _, c| {
        count_down_and_wait(&latch)?;
        sleep(2000)?;
        s.fail_execution(c)
    })
}

fn waits_then_fails(latch: &Arc<Latch>) -> DoExec {
    let latch = latch.clone();
    Arc::new(move |s, _, c| {
        count_down_and_wait(&latch)?;
        s.fail_execution(c)
    })
}

fn waits_then_sleeps(latch: &Arc<Latch>) -> DoExec {
    let latch = latch.clone();
    Arc::new(move |_, _, _| {
        count_down_and_wait(&latch)?;
        sleep(2000)
    })
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:477 DynamicSpawnStrategyTest.actionSucceedsIfLocalExecutionSucceedsEvenIfRemoteFailsLater
#[test]
fn action_succeeds_if_local_execution_succeeds_even_if_remote_fails_later() {
    let (f, local, remote, result, spawn) = race(waits, waits_then_sleeps_then_fails);
    result.unwrap();
    assert_eq!(local.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(local.succeeded());
    assert_eq!(remote.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
    assert!(!f.out().contains("MockRemoteSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:510 DynamicSpawnStrategyTest.actionSucceedsIfRemoteExecutionSucceedsEvenIfLocalFailsLater
#[test]
fn action_succeeds_if_remote_execution_succeeds_even_if_local_fails_later() {
    let (f, local, remote, result, spawn) = race(waits_then_sleeps_then_fails, waits);
    result.unwrap();
    assert_eq!(local.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!local.succeeded());
    assert_eq!(remote.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockRemoteSpawnStrategy")
    );
    assert!(!f.out().contains("MockLocalSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:543 DynamicSpawnStrategyTest.actionSucceedsIfLocalExecutionSucceedsEvenIfRemoteRunsNothing
#[test]
fn action_succeeds_if_local_execution_succeeds_even_if_remote_runs_nothing() {
    let f = set_up();
    let local = f.simple("MockLocalSpawnStrategy");
    let remote = f.cannot_exec("MockRemoteSpawnStrategy");
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.executed(&*spawn));
    assert!(local.succeeded());
    assert!(remote.executed_nothing());
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
    assert!(!f.out().contains("MockRemoteSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:561 DynamicSpawnStrategyTest.actionSucceedsIfRemoteExecutionSucceedsEvenIfLocalRunsNothing
#[test]
fn action_succeeds_if_remote_execution_succeeds_even_if_local_runs_nothing() {
    let f = set_up();
    let local = f.cannot_exec("MockLocalSpawnStrategy");
    let remote = f.simple("MockRemoteSpawnStrategy");
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.executed_nothing());
    assert!(!local.succeeded());
    assert!(remote.executed(&*spawn));
    assert!(remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockRemoteSpawnStrategy")
    );
    assert!(!f.out().contains("MockLocalSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:579 DynamicSpawnStrategyTest.actionFailsIfLocalFailsImmediatelyEvenIfRemoteSucceedsLater
#[test]
fn action_fails_if_local_fails_immediately_even_if_remote_succeeds_later() {
    let (f, local, remote, result, spawn) = race(waits_then_fails, waits_then_sleeps);
    let e = result.unwrap_err();
    assert_eq!(
        exec_message(&e),
        "MockLocalSpawnStrategy failed to execute the Spawn"
    );
    assert_eq!(local.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!local.succeeded());
    assert_eq!(remote.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("action failed with MockLocalSpawnStrategy")
    );
    assert!(!f.out().contains("MockRemoteSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:613 DynamicSpawnStrategyTest.actionFailsIfRemoteFailsImmediatelyEvenIfLocalSucceedsLater
#[test]
fn action_fails_if_remote_fails_immediately_even_if_local_succeeds_later() {
    let (f, local, remote, result, spawn) = race(waits_then_sleeps, waits_then_fails);
    let e = result.unwrap_err();
    assert_eq!(
        exec_message(&e),
        "MockRemoteSpawnStrategy failed to execute the Spawn"
    );
    assert_eq!(local.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!local.succeeded());
    assert_eq!(remote.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("action failed with MockRemoteSpawnStrategy")
    );
    assert!(!f.out().contains("MockLocalSpawnStrategy"));
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:647 DynamicSpawnStrategyTest.actionFailsIfLocalAndRemoteFail
#[test]
fn action_fails_if_local_and_remote_fail() {
    let (_f, local, remote, result, spawn) = race(waits_then_fails, waits_then_fails);
    let e = result.unwrap_err();
    let message = exec_message(&e);
    assert!(
        message == "MockLocalSpawnStrategy failed to execute the Spawn"
            || message == "MockRemoteSpawnStrategy failed to execute the Spawn",
        "{message}"
    );
    assert_eq!(local.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!local.succeeded());
    assert_eq!(remote.executed_spawn.load(Ordering::SeqCst), spawn);
    assert!(!remote.succeeded());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:681 DynamicSpawnStrategyTest.actionFailsIfLocalAndRemoteRunNothing
#[test]
fn action_fails_if_local_and_remote_run_nothing() {
    let f = set_up();
    let local = f.cannot_exec("MockLocalSpawnStrategy");
    let remote = f.cannot_exec("MockRemoteSpawnStrategy");
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    let e = sc.exec(&*spawn).unwrap_err();
    assert!(matches!(e, ExecException::User(_)), "{e:?}");
    // Has "No usable", followed by both dynamic_local_strategy and dynamic_remote_strategy in,
    // followed by the action's mnemonic.
    let re = regex::Regex::new(&format!(
        r"[nN]o usable\b.*\bdynamic_local_strategy\b.*\bdynamic_remote_strategy\b.*\b{}\b",
        spawn.get_mnemonic()
    ))
    .unwrap();
    assert!(re.is_match(&exec_message(&e)), "{}", exec_message(&e));
    assert!(local.executed_nothing());
    assert!(!local.succeeded());
    assert!(remote.executed_nothing());
    assert!(!remote.succeeded());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:705 DynamicSpawnStrategyTest.stopConcurrentSpawnsWaitForCompletion
#[test]
fn stop_concurrent_spawns_wait_for_completion() {
    let f = set_up();
    let latch = Latch::new(2);
    let slow_cleanup_finished = Arc::new(AtomicBool::new(false));
    let (l, s) = (latch.clone(), slow_cleanup_finished.clone());
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        Arc::new(move |_, _, _| {
            count_down_and_wait(&l)?;
            // Block indefinitely waiting for the remote branch to interrupt us.
            if sleep(WAIT_TIMEOUT_MILLISECONDS).is_ok() {
                panic!("Should have been interrupted");
            }
            // Wait for "long enough" hoping that the remoteStrategy will have enough time to
            // check the value of slowCleanupFinished before we finish this sleep, in case we
            // have a bug.
            std::thread::sleep(Duration::from_secs(5));
            s.store(true, Ordering::SeqCst);
            Ok(())
        }),
        nothing(),
    );
    let (l, s) = (latch.clone(), slow_cleanup_finished.clone());
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        Arc::new(move |_, _, _| count_down_and_wait(&l)),
        Arc::new(move |_, _, _| {
            // This runs after we have asked the local spawn to complete and, in theory, awaited
            // for InterruptedException to propagate.
            if !s.load(Ordering::SeqCst) {
                panic!("Did not await for the local branch to do its cleanup");
            }
            Ok(())
        }),
    );
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.executed(&*spawn));
    assert!(!local.succeeded());
    assert!(remote.executed(&*spawn));
    assert!(remote.succeeded());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:748 DynamicSpawnStrategyTest.noDeadlockWithSingleThreadedExecutor
#[test]
#[ignore = "declared PORT (dynamic_spawn_strategy.rs): no executor service; each branch has its \
            own thread, so a single-threaded executor (local first, then remote) cannot be set"]
fn no_deadlock_with_single_threaded_executor() {}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:776 DynamicSpawnStrategyTest.interruptDuringExecutionDoesActuallyInterruptTheExecution
#[test]
fn interrupt_during_execution_does_actually_interrupt_the_execution() {
    let f = set_up();
    let latch = Latch::new(2);
    let hook = |latch: &Arc<Latch>| -> DoExec {
        let l = latch.clone();
        Arc::new(move |_, _, _| {
            count_down_and_wait(&l)?;
            sleep(60_000)
        })
    };
    let local = f.strategy("MockLocalSpawnStrategy", hook(&latch), nothing());
    let remote = f.strategy("MockRemoteSpawnStrategy", hook(&latch), nothing());
    let sc = Arc::new(f.create_spawn_strategy(&local, &remote, None));
    let (tx, rx) = std::sync::mpsc::channel();
    let sc2 = sc.clone();
    let test_thread = std::thread::spawn(move || {
        tx.send(blaze_shell::ThreadInterrupt::current()).unwrap();
        let spawn = new_dynamic_spawn();
        match sc2.exec(&*spawn) {
            Err(ExecException::Interrupted) => {} // This is expected.
            other => panic!("expected an interrupt, got {other:?}"),
        }
    });
    let interrupt = rx.recv().unwrap();
    // countDownLatch.await(5, TimeUnit.SECONDS)
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while *latch.count.lock() > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    interrupt.interrupt();
    test_thread.join().unwrap();
    assert!(!f.out_err.get_output_path().exists());
    assert!(!f.out_err.get_error_path().exists());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:881 DynamicSpawnStrategyTest.assertThatStrategyWaitsForBothSpawnsToFinish
// Java skips these four tests (`if (true) { logger.atInfo().log("Skipping test"); return; }`,
// TODO(b/177406907)); so do the ports.
#[test]
fn strategy_waits_for_both_spawns_to_finish() {}

#[test]
fn strategy_waits_for_both_spawns_to_finish_even_if_interrupted() {}

#[test]
fn strategy_waits_for_both_spawns_to_finish_on_failure() {}

#[test]
fn strategy_waits_for_both_spawns_to_finish_on_failure_even_if_interrupted() {}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1025 DynamicSpawnStrategyTest.assertThatStrategyPropagatesException
fn assert_that_strategy_propagates_exception(
    local_exec: DoExec,
    remote_exec: DoExec,
    message: &str,
) {
    let f = set_up();
    let local = f.strategy("MockLocalSpawnStrategy", local_exec, nothing());
    let remote = f.strategy("MockRemoteSpawnStrategy", remote_exec, nothing());
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sc.exec(&*spawn)))
        .expect_err("assertThrows(IllegalArgumentException.class, ...)");
    assert!(
        blaze_util::uncaught_exception::panic_message(&*payload).contains(message),
        "{}",
        blaze_util::uncaught_exception::panic_message(&*payload)
    );
    assert!(local.executed(&*spawn) || remote.executed(&*spawn));
    assert!(!local.succeeded());
    assert!(!remote.succeeded());
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1047 DynamicSpawnStrategyTest.strategyPropagatesFasterLocalException
#[test]
fn strategy_propagates_faster_local_exception() {
    assert_that_strategy_propagates_exception(
        Arc::new(|_, _, _| {
            panic!("java.lang.IllegalArgumentException: Local spawn execution exception")
        }),
        Arc::new(|_, _, _| {
            sleep(WAIT_TIMEOUT_MILLISECONDS)?;
            panic!("java.lang.AssertionError: Not reachable");
        }),
        "Local spawn execution exception",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1062 DynamicSpawnStrategyTest.strategyPropagatesFasterRemoteException
#[test]
fn strategy_propagates_faster_remote_exception() {
    assert_that_strategy_propagates_exception(
        Arc::new(|_, _, _| {
            sleep(WAIT_TIMEOUT_MILLISECONDS)?;
            panic!("java.lang.AssertionError: Not reachable");
        }),
        Arc::new(|_, _, _| {
            panic!("java.lang.IllegalArgumentException: Remote spawn execution exception")
        }),
        "Remote spawn execution exception",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1077 DynamicSpawnStrategyTest.remoteBranchThrowsInterruptedExceptionAfterCancellation
#[test]
fn remote_branch_throws_interrupted_exception_after_cancellation() {
    let f = set_up();
    let local_can_finish = Latch::new(1);
    let remote_started = Latch::new(1);
    let (rs, lcf) = (remote_started.clone(), local_can_finish.clone());
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        Arc::new(move |_, _, _| {
            rs.wait()?;
            lcf.wait()
        }),
        nothing(),
    );
    let rs = remote_started.clone();
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        Arc::new(move |_, _, _| {
            rs.count_down();
            sleep(WAIT_TIMEOUT_MILLISECONDS)
        }),
        nothing(),
    );
    let sc = Arc::new(f.create_spawn_strategy(&local, &remote, None));
    let sc2 = sc.clone();
    let test_thread = std::thread::spawn(move || {
        let spawn = new_dynamic_spawn();
        sc2.exec(&*spawn).unwrap();
    });
    remote_started.wait().unwrap();
    local_can_finish.count_down();
    test_thread.join().unwrap();
    assert!(local.succeeded());
    assert!(!remote.succeeded());
}

fn both_started_then(latch: &Arc<Latch>, then_sleep: bool) -> DoExec {
    let l = latch.clone();
    Arc::new(move |_, _, _| {
        count_down_and_wait(&l)?;
        if then_sleep {
            sleep(WAIT_TIMEOUT_MILLISECONDS)?;
        }
        Ok(())
    })
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1116 DynamicSpawnStrategyTest.localBranchCancelsRemoteWhichThrowsInterruptedException
#[test]
fn local_branch_cancels_remote_which_throws_interrupted_exception() {
    let f = set_up();
    let both_started = Latch::new(2);
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        both_started_then(&both_started, false),
        nothing(),
    );
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        both_started_then(&both_started, true),
        nothing(),
    );
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.succeeded());
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1146 DynamicSpawnStrategyTest.remoteFinishesFirstAndThrowsInterruptedExceptionDuringLocalCancellation
#[test]
fn remote_finishes_first_and_throws_interrupted_exception_during_local_cancellation() {
    let f = set_up();
    let both_started = Latch::new(2);
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        both_started_then(&both_started, true),
        nothing(),
    );
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        both_started_then(&both_started, false),
        nothing(),
    );
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    // This should complete successfully with remote results, not throw AssertionError
    sc.exec(&*spawn).unwrap();
    assert!(!local.succeeded());
    assert!(remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockRemoteSpawnStrategy")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1178 DynamicSpawnStrategyTest.remoteBranchThrowsInterruptedExceptionBeforeStopConcurrentSpawns
#[test]
fn remote_branch_throws_interrupted_exception_before_stop_concurrent_spawns() {
    let f = set_up();
    let remote_started = Latch::new(1);
    let rs = remote_started.clone();
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        Arc::new(move |_, _, _| {
            rs.wait()?;
            sleep(100) // Give remote time to fail
        }),
        nothing(),
    );
    let rs = remote_started.clone();
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        Arc::new(move |_, _, _| {
            rs.count_down();
            // throw new InterruptedException("Remote operation interrupted")
            Err(ExecException::Interrupted)
        }),
        nothing(),
    );
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.succeeded());
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/dynamic/DynamicSpawnStrategyTest.java:1209 DynamicSpawnStrategyTest.remoteBranchInterruptedAfterLocalStartsStopConcurrentSpawns
#[test]
fn remote_branch_interrupted_after_local_starts_stop_concurrent_spawns() {
    let f = set_up();
    let both_started = Latch::new(2);
    let local_ready_to_stop = Latch::new(1);
    let lr = local_ready_to_stop.clone();
    let local = f.strategy(
        "MockLocalSpawnStrategy",
        both_started_then(&both_started, false),
        Arc::new(move |_, _, _| {
            lr.count_down();
            Ok(())
        }),
    );
    let remote = f.strategy(
        "MockRemoteSpawnStrategy",
        both_started_then(&both_started, true),
        nothing(),
    );
    let sc = f.create_spawn_strategy(&local, &remote, None);
    let spawn = new_dynamic_spawn();
    sc.exec(&*spawn).unwrap();
    assert!(local.succeeded());
    assert!(!remote.succeeded());
    assert!(
        f.out()
            .contains("output files written with MockLocalSpawnStrategy")
    );
}
