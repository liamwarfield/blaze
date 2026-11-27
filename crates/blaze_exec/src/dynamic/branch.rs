//! Port of `lib/dynamic/Branch.java`, `LocalBranch.java`, `RemoteBranch.java` and
//! `DynamicInterruptedException.java` (a branch's `ExecException::Interrupted` marked as such).
//!
//! PORT: Java runs each branch as a task of the module's cached thread pool, behind a Guava
//! `SettableFuture` that `cancel(true)` completes at once and whose running task it interrupts.
//! Here each branch of a spawn has its own thread inside the scope of
//! `DynamicSpawnStrategy::exec` (the branch borrows the spawn and the action's context), and
//! [`BranchFuture`] is the settable future: cancelling completes it, wakes its waiters and
//! interrupts the branch's thread through Blaze's Java interrupt status. A local branch's thread
//! waits until `DynamicSpawnStrategy.tryScheduleLocalJob` (on any spawn's thread) starts it, as
//! Java's submission to the pool does. The only difference: `exec` returns once the threads have
//! ended, after the cancelled branch's `call()` has noticed its interrupt; Java's returns when
//! the future is done (the winner has already waited for the loser's `done` semaphore then).

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use blaze_events::Event;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_shell::ThreadInterrupt;
use parking_lot::{Condvar, Mutex};

use crate::actions::spawn_result::Status;
use crate::actions::spawn_strategy::StopConcurrentSpawns;
use crate::actions::{ExecException, Spawn, SpawnResult};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::spawn_strategy_registry::DynamicMode;
use crate::exec::standalone_test_strategy::DerivedActionExecutionContext;
use crate::util::FileOutErr;

use super::dynamic_spawn_strategy::{self, IgnoreFailureCheck, PostProcessingSpawn};

/// How often an interruptible wait looks at the thread's interrupt status.
///
/// PORT: Java's `Semaphore.acquire` and `Future.get` are woken by `Thread.interrupt()`; these
/// waits poll the status (as the multiplexer's semaphore does).
const INTERRUPT_POLL: Duration = Duration::from_millis(10);

/// `DynamicMode.name()`: the enum constant's name (`toString()` is the lower-case name).
pub(crate) fn mode_name(mode: DynamicMode) -> &'static str {
    match mode {
        DynamicMode::Local => "LOCAL",
        DynamicMode::Remote => "REMOTE",
    }
}

/// `DynamicMode.toString()`.
// Java: com/google/devtools/build/lib/actions/DynamicStrategyRegistry.java:36 DynamicStrategyRegistry.DynamicMode.toString
pub(crate) fn mode_to_string(mode: DynamicMode) -> &'static str {
    match mode {
        DynamicMode::Local => "local",
        DynamicMode::Remote => "remote",
    }
}

/// `DynamicMode.other()`.
pub(crate) fn mode_other(mode: DynamicMode) -> DynamicMode {
    match mode {
        DynamicMode::Local => DynamicMode::Remote,
        DynamicMode::Remote => DynamicMode::Local,
    }
}

/// A `java.util.concurrent.Semaphore` with the interruptible `acquire()`.
#[derive(Debug, Default)]
pub(crate) struct InterruptibleSemaphore {
    permits: Mutex<i32>,
    available: Condvar,
}

impl InterruptibleSemaphore {
    /// `release()`.
    pub(crate) fn release(&self) {
        *self.permits.lock() += 1;
        self.available.notify_all();
    }

    /// `acquire()`: `Err` (Java's `InterruptedException`, the status cleared) when the thread is
    /// interrupted before a permit is available.
    pub(crate) fn acquire(&self) -> Result<(), ExecException> {
        let mut permits = self.permits.lock();
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(ExecException::Interrupted);
            }
            if *permits > 0 {
                *permits -= 1;
                return Ok(());
            }
            self.available.wait_for(&mut permits, INTERRUPT_POLL);
        }
    }
}

/// The result a branch's future holds.
pub(crate) enum BranchOutcome {
    /// `set(results)`.
    Success(Vec<SpawnResult>),
    /// `setException(e)`: an `ExecException` or (`ExecException::Interrupted`) an
    /// `InterruptedException`.
    Failed(ExecException),
    /// A `RuntimeException` or `Error` of the branch (a panic), thrown again by the waiter.
    Panicked(Box<dyn Any + Send>),
}

/// `Future.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FutureState {
    Running,
    Success,
    Failed,
    Cancelled,
}

struct FutureInner {
    state: FutureState,
    /// Whether the failure is an `InterruptedException`.
    failed_with_interrupt: bool,
    outcome: Option<BranchOutcome>,
    /// The thread running the branch's task (`setFuture(executor.submit(this))` started it).
    running_thread: Option<ThreadInterrupt>,
}

/// The branch's `SettableFuture<ImmutableList<SpawnResult>>`.
pub(crate) struct BranchFuture {
    inner: Mutex<FutureInner>,
    changed: Condvar,
}

impl BranchFuture {
    fn new() -> BranchFuture {
        BranchFuture {
            inner: Mutex::new(FutureInner {
                state: FutureState::Running,
                failed_with_interrupt: false,
                outcome: None,
                running_thread: None,
            }),
            changed: Condvar::new(),
        }
    }

    /// `state()`.
    pub(crate) fn state(&self) -> FutureState {
        self.inner.lock().state
    }

    /// `isDone()`.
    pub(crate) fn is_done(&self) -> bool {
        self.state() != FutureState::Running
    }

    /// `isCancelled()`.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state() == FutureState::Cancelled
    }

    /// The submitted task starts on the current thread; false if the future is already done (a
    /// cancelled task does not run).
    fn start_running(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.state != FutureState::Running {
            return false;
        }
        inner.running_thread = Some(ThreadInterrupt::current());
        true
    }

    /// `cancel(true)`: completes the future and interrupts the thread running its task. Returns
    /// false if the future was already done.
    fn cancel(&self) -> bool {
        let thread = {
            let mut inner = self.inner.lock();
            if inner.state != FutureState::Running {
                return false;
            }
            inner.state = FutureState::Cancelled;
            inner.running_thread.take()
        };
        self.changed.notify_all();
        if let Some(thread) = thread {
            thread.interrupt();
        }
        true
    }

    /// Completes the future with the task's outcome, unless it is done already (cancelled).
    fn complete(&self, outcome: BranchOutcome) -> bool {
        {
            let mut inner = self.inner.lock();
            inner.running_thread = None;
            if inner.state != FutureState::Running {
                return false;
            }
            match &outcome {
                BranchOutcome::Success(_) => inner.state = FutureState::Success,
                BranchOutcome::Failed(e) => {
                    inner.state = FutureState::Failed;
                    inner.failed_with_interrupt = matches!(e, ExecException::Interrupted);
                }
                BranchOutcome::Panicked(_) => inner.state = FutureState::Failed,
            }
            inner.outcome = Some(outcome);
        }
        self.changed.notify_all();
        true
    }

    /// `get()`: waits until done. `Err(None)` is a `CancellationException`, `Err(Some(Err(())))`
    /// an `InterruptedException` of the waiting thread.
    fn get(&self) -> Result<BranchOutcome, GetError> {
        let mut inner = self.inner.lock();
        loop {
            // Guava's AbstractFuture.get() tests Thread.interrupted() before it reads the value,
            // on entry and after every wake-up: a pending interrupt wins over a finished result.
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(GetError::Interrupted);
            }
            match inner.state {
                FutureState::Running => {}
                FutureState::Cancelled => return Err(GetError::Cancelled),
                FutureState::Success | FutureState::Failed => {
                    return Ok(inner.outcome.take().expect("the outcome is read once"));
                }
            }
            self.changed.wait_for(&mut inner, INTERRUPT_POLL);
        }
    }
}

/// Why `Future.get()` did not return a value.
pub(crate) enum GetError {
    /// `CancellationException`.
    Cancelled,
    /// `InterruptedException` of the waiting thread.
    Interrupted,
}

/// What a local branch's thread waits for before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartState {
    /// Waiting in `waitingLocalJobs`.
    Waiting,
    /// `execute(executorService)` was called.
    Started,
    /// The owning `exec` is over; a branch that was never started does not run.
    Abandoned,
}

/// Wraps the execution of a function that is supposed to execute a spawn via a strategy and
/// only updates the stdout/stderr files if this spawn succeeds.
// Java: com/google/devtools/build/lib/dynamic/Branch.java:52 Branch
pub struct Branch {
    mode: DynamicMode,
    /// True if this branch is still starting up, i.e. didn't get to the inner part of
    /// `callImpl` yet.
    starting: AtomicBool,
    future: BranchFuture,
    /// Semaphore that indicates whether this branch is done, i.e. either completed or cancelled.
    /// This is needed to wait for the branch to finish its own cleanup (e.g. terminating
    /// subprocesses) once it has been cancelled.
    done: InterruptibleSemaphore,
    options: Arc<DynamicExecutionOptions>,
    other_branch: OnceLock<Weak<Branch>>,
    /// `LocalBranch.creationTime`.
    creation_time: Instant,
    /// The branch threw a `DynamicInterruptedException` (from its `StopConcurrentSpawns` or
    /// `maybeIgnoreFailure`); `ExecException::Interrupted` carries no class.
    dynamic_interrupted: AtomicBool,
    start: Mutex<StartState>,
    start_changed: Condvar,
}

impl Branch {
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:79 Branch.<init>
    pub fn new(mode: DynamicMode, options: Arc<DynamicExecutionOptions>) -> Arc<Branch> {
        Arc::new(Branch {
            mode,
            starting: AtomicBool::new(true),
            future: BranchFuture::new(),
            done: InterruptibleSemaphore::default(),
            options,
            other_branch: OnceLock::new(),
            creation_time: Instant::now(),
            dynamic_interrupted: AtomicBool::new(false),
            start: Mutex::new(StartState::Waiting),
            start_changed: Condvar::new(),
        })
    }

    /// Returns true if this branch has already been cancelled.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:95 Branch.isDone
    pub(crate) fn is_done(&self) -> bool {
        self.future.is_done()
    }

    // Java: com/google/devtools/build/lib/dynamic/Branch.java:100 Branch.getDoneSemaphore
    pub(crate) fn get_done_semaphore(&self) -> &InterruptibleSemaphore {
        &self.done
    }

    // Java: com/google/devtools/build/lib/dynamic/Branch.java:105 Branch.isCancelled
    pub(crate) fn is_cancelled(&self) -> bool {
        self.future.is_cancelled()
    }

    /// The state of the branch's future.
    pub(crate) fn future_state(&self) -> FutureState {
        self.future.state()
    }

    /// Cancels the branch. Returns true if the cancellation was successful.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:114 Branch.cancel
    pub fn cancel(self: &Arc<Self>) -> bool {
        let cancelled = self.future.cancel();
        if cancelled {
            self.start_changed.notify_all();
            self.on_future_done();
        }
        cancelled
    }

    /// `getResults()`: `future.get()`.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:123 Branch.getResults
    pub(crate) fn get_results(&self) -> Result<BranchOutcome, GetError> {
        self.future.get()
    }

    // Java: com/google/devtools/build/lib/dynamic/Branch.java:133 Branch.getMode
    pub(crate) fn get_mode(&self) -> DynamicMode {
        self.mode
    }

    /// Returns a human-readable description of what we can tell about the state of this
    /// branch.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:136 Branch.branchState
    pub(crate) fn branch_state(&self) -> String {
        format!(
            "{} and {}",
            if self.is_cancelled() {
                "cancelled"
            } else {
                "not cancelled"
            },
            if self.is_done() { "done" } else { "not done" }
        )
    }

    /// `LocalBranch.getAge()`.
    // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:85 LocalBranch.getAge
    pub(crate) fn get_age(&self) -> Duration {
        self.creation_time.elapsed()
    }

    /// Executes this branch using the provided executor: lets its thread run `call()`.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:146 Branch.execute
    pub(crate) fn execute(&self) {
        let mut start = self.start.lock();
        if *start == StartState::Waiting {
            *start = StartState::Started;
        }
        drop(start);
        self.start_changed.notify_all();
    }

    /// The owning `exec` is over: a branch whose thread still waits to be started never runs.
    pub(crate) fn abandon(&self) {
        let mut start = self.start.lock();
        if *start == StartState::Waiting {
            *start = StartState::Abandoned;
        }
        drop(start);
        self.start_changed.notify_all();
    }

    /// Blocks the branch's thread until it is started; false if it never will be.
    fn wait_for_start(&self) -> bool {
        let mut start = self.start.lock();
        loop {
            match *start {
                StartState::Started => return true,
                StartState::Abandoned => return false,
                StartState::Waiting => {
                    self.start_changed.wait(&mut start);
                }
            }
        }
    }

    /// Links this branch to its `other_branch` (the listener of the future is
    /// [`Branch::on_future_done`]).
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:157 Branch.prepareFuture
    pub fn prepare_future(&self, other_branch: &Arc<Branch>) {
        let _ = self.other_branch.set(Arc::downgrade(other_branch));
    }

    fn other_branch(&self) -> Option<Arc<Branch>> {
        self.other_branch.get().and_then(Weak::upgrade)
    }

    /// The future's listener, run (directExecutor) by the thread that completed the future.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:160 Branch.prepareFuture
    fn on_future_done(&self) {
        // When this branch has already been cancelled, we know the other branch (if any) has
        // been started and doesn't need cancelling. But because cancel() is called before the
        // other branch's callImpl() is called, we need to release the done semaphore here.
        if self
            .starting
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.done.release();
        }
        let (state, failed_with_interrupt) = {
            let inner = self.future.inner.lock();
            (inner.state, inner.failed_with_interrupt)
        };
        // If the branch is done (successfully or failed) but not cancelled, cancel the other
        // branch.
        if (state == FutureState::Success
            || (state == FutureState::Failed && !failed_with_interrupt))
            && let Some(other) = self.other_branch()
        {
            other.cancel();
        }
        // PORT: skipped, logging (the listener's debug log of the future's state).
    }

    /// Runs the branch on the current thread (the thread `DynamicSpawnStrategy::exec` started
    /// for it): waits to be started, then `call()`, then completes the future.
    pub(crate) fn run(
        self: &Arc<Self>,
        context: &dyn ActionExecutionContext,
        call_impl: &dyn Fn(
            &Arc<Branch>,
            &dyn ActionExecutionContext,
        ) -> Result<Vec<SpawnResult>, ExecException>,
    ) {
        if !self.wait_for_start() || !self.future.start_running() {
            return;
        }
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.call(context, call_impl)
        })) {
            Ok(Ok(results)) => BranchOutcome::Success(results),
            Ok(Err(e)) => BranchOutcome::Failed(e),
            Err(payload) => BranchOutcome::Panicked(payload),
        };
        if self.future.complete(outcome) {
            self.on_future_done();
        }
    }

    /// Moves the stdout and stderr files of `from` to `to`.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:196 Branch.moveFileOutErr
    fn move_file_out_err(from: &FileOutErr, to: &FileOutErr) {
        // FileSystemUtils.moveFile: renameTo (the files are in one directory).
        // Could not move action logs from execution (logged).
        if from.get_output_path().exists() {
            let _ = from.get_output_path().rename_to(to.get_output_path());
        }
        if from.get_error_path().exists() {
            let _ = from.get_error_path().rename_to(to.get_error_path());
        }
    }

    /// Returns a [`FileOutErr`] whose files are `fileOutErr`'s with `suffix` appended.
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:214 Branch.getSuffixedFileOutErr
    fn get_suffixed_file_out_err(file_out_err: &FileOutErr, suffix: &str) -> FileOutErr {
        let out_dir = file_out_err
            .get_output_path()
            .get_parent_directory()
            .expect("checkNotNull");
        let out_base_name = file_out_err.get_output_path().get_base_name();
        let err_dir = file_out_err
            .get_error_path()
            .get_parent_directory()
            .expect("checkNotNull");
        let err_base_name = file_out_err.get_error_path().get_base_name();
        FileOutErr::new(
            out_dir.get_child(&format!("{out_base_name}{suffix}")),
            err_dir.get_child(&format!("{err_base_name}{suffix}")),
        )
    }

    /// Executes the strategy with a branch-specific stdout/stderr, moving them to the action's
    /// files only if the strategy completed (successfully or with an `ExecException`).
    // Java: com/google/devtools/build/lib/dynamic/Branch.java:247 Branch.call
    fn call(
        self: &Arc<Self>,
        context: &dyn ActionExecutionContext,
        call_impl: &dyn Fn(
            &Arc<Branch>,
            &dyn ActionExecutionContext,
        ) -> Result<Vec<SpawnResult>, ExecException>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        let suffix = format!(".{}", mode_name(self.mode));
        let file_out_err = Branch::get_suffixed_file_out_err(context.get_file_out_err(), &suffix);
        let branch_context = DerivedActionExecutionContext::with_file_out_err(
            context,
            Branch::get_suffixed_file_out_err(context.get_file_out_err(), &suffix),
        );
        let result = call_impl(self, &branch_context);
        // PORT: fileOutErr.close() (Blaze's FileOutErr holds no open stream).
        if matches!(result, Err(ExecException::Interrupted)) {
            // Java's call() catches only ExecException: an InterruptedException leaves before the
            // logs are moved.
            return result;
        }
        Branch::move_file_out_err(&file_out_err, context.get_file_out_err());
        result
    }

    /// The common part of `LocalBranch.callImpl` and `RemoteBranch.callImpl`.
    // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:141 LocalBranch.callImpl
    // Java: com/google/devtools/build/lib/dynamic/RemoteBranch.java:107 RemoteBranch.callImpl
    pub(crate) fn call_impl_common(
        self: &Arc<Self>,
        spawn: &dyn Spawn,
        run: impl FnOnce() -> Result<Vec<SpawnResult>, ExecException>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        if self.other_branch.get().is_none() {
            panic!("java.lang.IllegalStateException: prepareFuture not called");
        }
        let result = (|| {
            if self
                .starting
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                // If we got cancelled, we just abort.
                blaze_util::check_state!(blaze_shell::thread_interrupt::interrupted());
                return Err(ExecException::Interrupted);
            }
            run()
        })();
        if matches!(result, Err(ExecException::Interrupted))
            && self.dynamic_interrupted.load(Ordering::SeqCst)
        {
            // A DynamicInterruptedException: the branch cancels itself.
            // PORT: skipped, logging ("%s branch of %s self-cancelling with %s: '%s'").
            self.cancel();
        }
        // PORT: skipped, logging ("%s branch of %s failed with %s: '%s'").
        let _ = spawn;
        self.done.release();
        result
    }

    /// `throw new DynamicInterruptedException(message)`.
    pub(crate) fn dynamic_interrupted_exception(&self, _message: String) -> ExecException {
        // The message is only logged (debug); ExecException::Interrupted carries none.
        self.dynamic_interrupted.store(true, Ordering::SeqCst);
        ExecException::Interrupted
    }

    /// The `StopConcurrentSpawns` of this branch: `maybeIgnoreFailure`, then
    /// `DynamicSpawnStrategy.stopBranch`.
    // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:160 LocalBranch.callImpl
    // Java: com/google/devtools/build/lib/dynamic/RemoteBranch.java:126 RemoteBranch.callImpl
    pub(crate) fn stop_concurrent_spawns<'a>(
        self: &'a Arc<Self>,
        spawn: &'a dyn Spawn,
        strategy_that_cancelled: &'a Mutex<Option<DynamicMode>>,
        ignore_failure_check: Option<&'a dyn IgnoreFailureCheck>,
        owning_context: &'a dyn ActionExecutionContext,
    ) -> BranchStopConcurrentSpawns<'a> {
        BranchStopConcurrentSpawns {
            branch: self,
            spawn,
            strategy_that_cancelled,
            ignore_failure_check,
            owning_context,
        }
    }

    /// `maybeIgnoreFailure(exitCode, errorMessage, outErr)`: when the failure can be ignored, the
    /// branch cancels itself in favor of the other one.
    // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:200 LocalBranch.maybeIgnoreFailure
    // Java: com/google/devtools/build/lib/dynamic/RemoteBranch.java:166 RemoteBranch.maybeIgnoreFailure
    fn maybe_ignore_failure(
        &self,
        spawn: &dyn Spawn,
        owning_context: &dyn ActionExecutionContext,
        ignore_failure_check: Option<&dyn IgnoreFailureCheck>,
        exit_code: i32,
        error_message: &str,
        out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException> {
        let Some(ignore_failure_check) = ignore_failure_check else {
            return Ok(());
        };
        if exit_code == 0 {
            return Ok(());
        }
        let is_local = self.mode == DynamicMode::Local;
        // synchronized (spawn)
        let _guard = SPAWN_MONITOR.lock();
        if ignore_failure_check.can_ignore_failure(
            spawn,
            owning_context,
            exit_code,
            error_message,
            out_err,
            is_local,
        ) {
            let message = if is_local {
                format!(
                    "Local branch of {} cancelling self in favor of remote.",
                    spawn.get_resource_owner().pretty_print()
                )
            } else {
                format!(
                    "Remote branch of {} cancelling self in favor of local.",
                    spawn.get_resource_owner().pretty_print()
                )
            };
            return Err(self.dynamic_interrupted_exception(message));
        }
        Ok(())
    }
}

/// `synchronized (spawn)` of `maybeIgnoreFailure`: both branches of a spawn take the spawn's
/// monitor.
///
/// PORT: one lock for all spawns (a spawn has no monitor here); the check it guards is short.
static SPAWN_MONITOR: Mutex<()> = Mutex::new(());

/// The lambda each branch passes to its strategy as `StopConcurrentSpawns`.
pub(crate) struct BranchStopConcurrentSpawns<'a> {
    branch: &'a Arc<Branch>,
    spawn: &'a dyn Spawn,
    strategy_that_cancelled: &'a Mutex<Option<DynamicMode>>,
    ignore_failure_check: Option<&'a dyn IgnoreFailureCheck>,
    /// `this.context`: the action's context (not the branch's).
    owning_context: &'a dyn ActionExecutionContext,
}

impl StopConcurrentSpawns for BranchStopConcurrentSpawns<'_> {
    fn stop(
        &self,
        exit_code: i32,
        error_message: &str,
        out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException> {
        if !self.branch.is_cancelled() {
            self.branch.maybe_ignore_failure(
                self.spawn,
                self.owning_context,
                self.ignore_failure_check,
                exit_code,
                error_message,
                out_err,
            )?;
        }
        let other = self
            .branch
            .other_branch()
            .expect("the other branch outlives this one");
        dynamic_spawn_strategy::stop_branch(
            &other,
            self.branch,
            self.strategy_that_cancelled,
            &self.branch.options,
            self.owning_context,
            self.spawn,
        )
    }
}

/// `LocalBranch.runLocally`: runs the spawn with the first local strategy that can, then the
/// post-processing spawn if any.
// Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:96 LocalBranch.runLocally
pub(crate) fn run_locally(
    spawn: &dyn Spawn,
    action_execution_context: &dyn ActionExecutionContext,
    stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    get_extra_spawn_for_local_execution: &dyn PostProcessingSpawn,
) -> Result<Vec<SpawnResult>, ExecException> {
    let spawn_result = run_spawn_locally(spawn, action_execution_context, stop_concurrent_spawns)?;
    if spawn_result.iter().any(|r| r.status() != Status::Success) {
        return Ok(spawn_result);
    }
    let Some(extra_spawn) = get_extra_spawn_for_local_execution.get(spawn) else {
        return Ok(spawn_result);
    };
    // Can't have post-processing spawns without a stop function: it's called in the spawn's own
    // stopConcurrentSpawns.
    let extra_spawn_result = run_spawn_locally(&*extra_spawn, action_execution_context, None)?;
    let mut results = spawn_result;
    results.extend(extra_spawn_result);
    Ok(results)
}

// Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:121 LocalBranch.runSpawnLocally
fn run_spawn_locally(
    spawn: &dyn Spawn,
    action_execution_context: &dyn ActionExecutionContext,
    stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
) -> Result<Vec<SpawnResult>, ExecException> {
    let dynamic_strategy_registry = action_execution_context
        .get_dynamic_strategy_registry()
        .expect("getContext(DynamicStrategyRegistry.class)");
    for strategy in
        dynamic_strategy_registry.get_dynamic_spawn_action_contexts(spawn, DynamicMode::Local)
    {
        if strategy.can_exec(spawn, action_execution_context) {
            let sandboxed = dynamic_spawn_strategy::as_sandboxed(&*strategy);
            // PORT: skipped, logging (a strategy returning null).
            return sandboxed.exec_with_stop_concurrent_spawns(
                spawn,
                action_execution_context,
                stop_concurrent_spawns,
            );
        }
    }
    panic!(
        "java.lang.AssertionError: canExec passed but no usable local strategy for action {}",
        dynamic_spawn_strategy::spawn_to_string(spawn)
    );
}

/// `RemoteBranch.runRemotely`: runs the spawn with the first remote strategy that can; a cache
/// hit delays later local executions.
// Java: com/google/devtools/build/lib/dynamic/RemoteBranch.java:69 RemoteBranch.runRemotely
pub(crate) fn run_remotely(
    spawn: &dyn Spawn,
    action_execution_context: &dyn ActionExecutionContext,
    stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    delay_local_execution: &AtomicBool,
) -> Result<Vec<SpawnResult>, ExecException> {
    let dynamic_strategy_registry = action_execution_context
        .get_dynamic_strategy_registry()
        .expect("getContext(DynamicStrategyRegistry.class)");
    for strategy in
        dynamic_strategy_registry.get_dynamic_spawn_action_contexts(spawn, DynamicMode::Remote)
    {
        if strategy.can_exec(spawn, action_execution_context) {
            let sandboxed = dynamic_spawn_strategy::as_sandboxed(&*strategy);
            let results = sandboxed.exec_with_stop_concurrent_spawns(
                spawn,
                action_execution_context,
                stop_concurrent_spawns,
            )?;
            // PORT: Java warns when a strategy returns null; a Vec cannot be null.
            for r in &results {
                if r.is_cache_hit() {
                    delay_local_execution.store(true, Ordering::SeqCst);
                    break;
                }
            }
            return Ok(results);
        }
    }
    panic!(
        "java.lang.AssertionError: canExec passed but no usable remote strategy for action {}",
        dynamic_spawn_strategy::spawn_to_string(spawn)
    );
}

/// `Event.info` through the action's event handler (the `--debug_spawn_scheduler` messages).
pub(crate) fn debug_info(context: &dyn ActionExecutionContext, message: String) {
    context.get_event_handler().handle(&Event::info(message));
}

/// `Thread.sleep(duration)`: `Err` (an `InterruptedException`, the status cleared) when the
/// thread is interrupted.
pub(crate) fn interruptible_sleep(duration: Duration) -> Result<(), ()> {
    let deadline = Instant::now() + duration;
    loop {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        std::thread::sleep((deadline - now).min(INTERRUPT_POLL));
    }
}

#[cfg(test)]
mod future_tests {
    use super::*;

    /// Guava's `AbstractFuture.get()`: a pending interrupt is thrown even when the future is done
    /// (review exec-3, E3-1).
    #[test]
    fn get_on_done_future_with_pending_interrupt_throws_interrupted() {
        let future = BranchFuture::new();
        assert!(future.start_running());
        assert!(future.complete(BranchOutcome::Success(Vec::new())));
        blaze_shell::thread_interrupt::interrupt_current();
        assert!(matches!(future.get(), Err(GetError::Interrupted)));
        // Thread.interrupted() cleared the status; the result is still there.
        assert!(matches!(future.get(), Ok(BranchOutcome::Success(_))));
    }
}
