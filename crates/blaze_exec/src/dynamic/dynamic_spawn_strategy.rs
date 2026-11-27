//! Port of `lib/dynamic/DynamicSpawnStrategy.java` (and the small classes it uses:
//! `DynamicExecutionFinishedEvent`, `DynamicInterruptedException`).
//!
//! PORT: Java runs the two branches of a spawn on the module's cached thread pool and returns
//! from `exec` when the winning branch's future is done. Here each branch has a thread in the
//! scope of `exec` (it borrows the spawn and the action's context), see
//! [`branch`](super::branch); `exec` returns once both threads have ended, i.e. after the
//! cancelled branch noticed its interrupt. The local branch's thread waits until
//! `tryScheduleLocalJob` (run by any spawn's thread) starts it, as the submission to the pool
//! does in Java.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use blaze_events::Postable;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use parking_lot::Mutex;

use super::branch::{self, Branch, BranchOutcome, FutureState, GetError, mode_to_string};
use super::shrinkable_semaphore::ShrinkableSemaphore;
use crate::actions::spawn_strategy::{SandboxedSpawnStrategy, SpawnStrategy};
use crate::actions::{ExecException, Spawn, SpawnResult, UserExecException};
use crate::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use crate::exec::execution_policy::ExecutionPolicy;
use crate::exec::spawn_strategy_registry::DynamicMode;
use crate::util::FileOutErr;

/// A callback that allows checking if a given failure can be ignored on one branch.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:276 DynamicExecutionModule.IgnoreFailureCheck
pub trait IgnoreFailureCheck: Send + Sync {
    fn can_ignore_failure(
        &self,
        spawn: &dyn Spawn,
        context: &dyn ActionExecutionContext,
        exit_code: i32,
        error_message: &str,
        out_err: Option<&FileOutErr>,
        is_local: bool,
    ) -> bool;
}

/// `Function<Spawn, Optional<Spawn>>`: the post-processing spawn to run after a spawn that ran
/// locally.
pub trait PostProcessingSpawn: Send + Sync {
    fn get(&self, spawn: &dyn Spawn) -> Option<Box<dyn Spawn>>;
}

/// `Function<Spawn, ExecutionPolicy>`.
pub type GetExecutionPolicy = Arc<dyn Fn(&dyn Spawn) -> ExecutionPolicy + Send + Sync>;

/// Posted when a spawn was executed dynamically: which strategies ran it and which branch won.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionFinishedEvent.java:20 DynamicExecutionFinishedEvent
#[derive(Debug, Clone)]
pub struct DynamicExecutionFinishedEvent {
    pub mnemonic: String,
    pub local_branch_name: String,
    /// `remoteBranchName` (the constructor names it `winnerBranchName`).
    pub remote_branch_name: String,
    pub winner_branch_type: DynamicMode,
}

impl Postable for DynamicExecutionFinishedEvent {}

/// The dynamic strategies are `SandboxedSpawnStrategy`s (checked when the registry is built).
pub(crate) fn as_sandboxed(strategy: &dyn SpawnStrategy) -> &dyn SandboxedSpawnStrategy {
    strategy
        .as_sandboxed_spawn_strategy()
        .expect("a dynamic strategy is a SandboxedSpawnStrategy")
}

/// `"... for action " + spawn` of the `AssertionError`s.
///
/// PORT: `Spawn.toString()` is `Object.toString` for Bazel's spawns (class name and identity
/// hash); the crash message uses the spawn's debug form instead.
pub(crate) fn spawn_to_string(spawn: &dyn Spawn) -> String {
    format!("{spawn:?}")
}

/// A strategy that runs spawns locally and remotely at the same time and keeps the result of the
/// branch that finishes first.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:78 DynamicSpawnStrategy
pub struct DynamicSpawnStrategy {
    options: Arc<DynamicExecutionOptions>,
    get_execution_policy: GetExecutionPolicy,
    /// Set to true by the first action that completes remotely. Until that happens, all local
    /// actions are delayed by `--dynamic_local_execution_delay`.
    delay_local_execution: AtomicBool,
    get_extra_spawn_for_local_execution: Arc<dyn PostProcessingSpawn>,
    /// A callback that allows checking if a given failure can be ignored on one branch.
    ignore_failure_check: Option<Arc<dyn IgnoreFailureCheck>>,
    /// Limit on how many threads we should use for dynamic execution.
    thread_limiter: ShrinkableSemaphore,
    /// Set of jobs that are waiting for local execution.
    waiting_local_jobs: Mutex<VecDeque<Arc<Branch>>>,
}

impl DynamicSpawnStrategy {
    /// Constructs a `DynamicSpawnStrategy` (`num_cpus` is `--local_resources=cpu=`, `jobs`
    /// `--jobs`).
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:128 DynamicSpawnStrategy.<init>
    pub fn new(
        options: Arc<DynamicExecutionOptions>,
        get_execution_policy: GetExecutionPolicy,
        get_post_processing_spawn_for_local_execution: Arc<dyn PostProcessingSpawn>,
        num_cpus: i32,
        jobs: i32,
        ignore_failure_check: Option<Arc<dyn IgnoreFailureCheck>>,
    ) -> Arc<DynamicSpawnStrategy> {
        let thread_limiter = ShrinkableSemaphore::new(
            if options.local_load_factor > 0.0 {
                num_cpus
            } else {
                jobs
            },
            jobs,
            options.local_load_factor,
        );
        Arc::new(DynamicSpawnStrategy {
            options,
            get_execution_policy,
            delay_local_execution: AtomicBool::new(false),
            get_extra_spawn_for_local_execution: get_post_processing_spawn_for_local_execution,
            ignore_failure_check,
            thread_limiter,
            waiting_local_jobs: Mutex::new(VecDeque::new()),
        })
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:153 DynamicSpawnStrategy.canExecLocal
    fn can_exec_local(
        spawn: &dyn Spawn,
        execution_policy: ExecutionPolicy,
        acr: &dyn ActionContextRegistry,
    ) -> bool {
        DynamicSpawnStrategy::get_local_strategy(spawn, execution_policy, acr).is_some()
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:162 DynamicSpawnStrategy.getLocalStrategy
    fn get_local_strategy(
        spawn: &dyn Spawn,
        execution_policy: ExecutionPolicy,
        acr: &dyn ActionContextRegistry,
    ) -> Option<Arc<dyn SpawnStrategy>> {
        if !execution_policy.can_run_locally() {
            return None;
        }
        let dsr = acr
            .get_dynamic_strategy_registry()
            .expect("getContext(DynamicStrategyRegistry.class)");
        dsr.get_dynamic_spawn_action_contexts(spawn, DynamicMode::Local)
            .into_iter()
            .find(|s| s.can_exec(spawn, acr))
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:178 DynamicSpawnStrategy.canExecRemote
    fn can_exec_remote(
        spawn: &dyn Spawn,
        execution_policy: ExecutionPolicy,
        acr: &dyn ActionContextRegistry,
    ) -> bool {
        DynamicSpawnStrategy::get_remote_strategy(spawn, execution_policy, acr).is_some()
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:187 DynamicSpawnStrategy.getRemoteStrategy
    fn get_remote_strategy(
        spawn: &dyn Spawn,
        execution_policy: ExecutionPolicy,
        acr: &dyn ActionContextRegistry,
    ) -> Option<Arc<dyn SpawnStrategy>> {
        if !execution_policy.can_run_remotely() {
            return None;
        }
        let dsr = acr
            .get_dynamic_strategy_registry()
            .expect("getContext(DynamicStrategyRegistry.class)");
        dsr.get_dynamic_spawn_action_contexts(spawn, DynamicMode::Remote)
            .into_iter()
            .find(|s| s.can_exec(spawn, acr))
    }

    /// The local branch's `callImpl`: the start check, the local delay, `runLocally` with the
    /// branch's `StopConcurrentSpawns`.
    // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:141 LocalBranch.callImpl
    fn local_call_impl(
        &self,
        branch: &Arc<Branch>,
        context: &dyn ActionExecutionContext,
        spawn: &dyn Spawn,
        strategy_that_cancelled: &Mutex<Option<DynamicMode>>,
        owning_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        branch.call_impl_common(spawn, || {
            if self
                .delay_local_execution
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                // PORT: skipped, profiling ("delay local branch").
                // Thread.sleep(options.localExecutionDelay): a negative value is an
                // IllegalArgumentException, unchecked.
                // Java: com/google/devtools/build/lib/dynamic/LocalBranch.java:154 LocalBranch.callImpl
                let delay = self.options.local_execution_delay;
                if delay < 0 {
                    panic!("java.lang.IllegalArgumentException: timeout value is negative");
                }
                if branch::interruptible_sleep(Duration::from_millis(delay as u64)).is_err() {
                    return Err(ExecException::Interrupted);
                }
            }
            let stop = branch.stop_concurrent_spawns(
                spawn,
                strategy_that_cancelled,
                self.ignore_failure_check.as_deref(),
                owning_context,
            );
            branch::run_locally(
                spawn,
                context,
                Some(&stop),
                &*self.get_extra_spawn_for_local_execution,
            )
        })
    }

    /// The remote branch's `callImpl`.
    // Java: com/google/devtools/build/lib/dynamic/RemoteBranch.java:109 RemoteBranch.callImpl
    fn remote_call_impl(
        &self,
        branch: &Arc<Branch>,
        context: &dyn ActionExecutionContext,
        spawn: &dyn Spawn,
        strategy_that_cancelled: &Mutex<Option<DynamicMode>>,
        owning_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        branch.call_impl_common(spawn, || {
            let stop = branch.stop_concurrent_spawns(
                spawn,
                strategy_that_cancelled,
                self.ignore_failure_check.as_deref(),
                owning_context,
            );
            branch::run_remotely(spawn, context, Some(&stop), &self.delay_local_execution)
        })
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:263 DynamicSpawnStrategy.updateStrategyWinner
    fn update_strategy_winner(
        &self,
        context: &dyn ActionExecutionContext,
        spawn: &dyn Spawn,
        result: &SpawnResult,
        strategy_that_cancelled: &Mutex<Option<DynamicMode>>,
    ) {
        let execution_policy = (self.get_execution_policy)(spawn);

        // In case of remote runner, we could have "runner-name-cached" instead of
        // "runner-name", in this case we want more precise name of branch.
        let winner = result.get_runner_name().to_string();
        let local_strategy =
            DynamicSpawnStrategy::get_local_strategy(spawn, execution_policy, context);
        let remote_strategy =
            DynamicSpawnStrategy::get_remote_strategy(spawn, execution_policy, context);

        let (Some(local_strategy), Some(remote_strategy)) = (local_strategy, remote_strategy)
        else {
            return;
        };

        let mut local_name = local_strategy.to_string();
        let mut remote_name = remote_strategy.to_string();

        let Some(cancelled) = *strategy_that_cancelled.lock() else {
            return;
        };
        let winner_branch_type = match cancelled {
            DynamicMode::Local => {
                local_name = winner;
                DynamicMode::Local
            }
            DynamicMode::Remote => {
                remote_name = winner;
                DynamicMode::Remote
            }
        };

        context
            .get_event_handler()
            .post(Arc::new(DynamicExecutionFinishedEvent {
                mnemonic: spawn.get_mnemonic(),
                local_branch_name: local_name,
                remote_branch_name: remote_name,
                winner_branch_type,
            }));
    }

    /// Tries to schedule as many local jobs as are permitted by `threadLimiter`. "Scheduling"
    /// here means putting it on a thread and making it start the normal strategy execution, but
    /// it will still have to wait for resources, so it may not execute for a while.
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:317 DynamicSpawnStrategy.tryScheduleLocalJob
    fn try_schedule_local_job(&self, waiting_local_jobs: &mut VecDeque<Arc<Branch>>) {
        self.thread_limiter
            .update_load(waiting_local_jobs.len() as i32);
        while !waiting_local_jobs.is_empty() && self.thread_limiter.try_acquire() {
            // TODO(b/120910324): Prioritize jobs where the remote branch has already failed.
            let slow_remote_time = self.options.slow_remote_time;
            let job = if slow_remote_time > Duration::ZERO
                && waiting_local_jobs.front().expect("not empty").get_age() > slow_remote_time
            {
                waiting_local_jobs.pop_front()
            } else {
                waiting_local_jobs.pop_back()
            }
            .expect("not empty");
            job.execute();
        }
    }

    /// Checks if this action should be executed dynamically, and if not executes it locally or
    /// remotely as applicable, or fails if it cannot be executed at all. `Ok(None)` if this
    /// action can be executed dynamically.
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:351 DynamicSpawnStrategy.maybeExecuteNonDynamically
    pub fn maybe_execute_non_dynamically(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<Vec<SpawnResult>>, ExecException> {
        let post_processing_spawn = self.get_extra_spawn_for_local_execution.get(spawn);
        let execution_policy = (self.get_execution_policy)(spawn);
        let post_processing_spawn_execution_policy = post_processing_spawn
            .as_ref()
            .map(|s| (self.get_execution_policy)(&**s));

        let spawn_local_can_exec =
            DynamicSpawnStrategy::can_exec_local(spawn, execution_policy, action_execution_context);
        let post_processing_spawn_local_can_exec = match &post_processing_spawn {
            None => true,
            Some(s) => DynamicSpawnStrategy::can_exec_local(
                &**s,
                post_processing_spawn_execution_policy.expect("set with the spawn"),
                action_execution_context,
            ),
        };
        // To declare a spawn being executable in local, we need to make sure that the
        // post-processing spawn is also executable in local.
        let local_can_exec = spawn_local_can_exec && post_processing_spawn_local_can_exec;
        let remote_can_exec = DynamicSpawnStrategy::can_exec_remote(
            spawn,
            execution_policy,
            action_execution_context,
        );

        if !local_can_exec && !remote_can_exec {
            // Java sets DynamicExecution, then Spawn: they are a oneof, so Spawn is kept.
            let failure = FailureDetail {
                message: get_no_executable_user_exec_exception_message(
                    spawn,
                    spawn_local_can_exec,
                    execution_policy,
                    post_processing_spawn.as_deref(),
                    post_processing_spawn_local_can_exec,
                    post_processing_spawn_execution_policy,
                ),
                category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                    code: failure_details::spawn::Code::NoUsableStrategyFound as i32,
                    ..Default::default()
                })),
            };
            // PORT: skipped, logging (debugLog: "can be done neither locally nor remotely").
            return Err(UserExecException::new(failure).into());
        } else if !local_can_exec && remote_can_exec {
            // PORT: skipped, logging (debugLog: "can only be done remotely", with the local
            // strategies of the spawn and of the post-processing spawn).
            return Ok(Some(branch::run_remotely(
                spawn,
                action_execution_context,
                None,
                &self.delay_local_execution,
            )?));
        } else if local_can_exec && !remote_can_exec {
            // PORT: skipped, logging (debugLog: "can only be done locally").
            return Ok(Some(branch::run_locally(
                spawn,
                action_execution_context,
                None,
                &*self.get_extra_spawn_for_local_execution,
            )?));
        } else if self.options.exclude_tools
            && spawn
                .get_resource_owner()
                .get_owner()
                .is_build_configuration_for_tool()
        {
            return Ok(Some(branch::run_remotely(
                spawn,
                action_execution_context,
                None,
                &self.delay_local_execution,
            )?));
        }
        Ok(None)
    }
}

/// Returns an error string for being unable to execute locally and/or remotely the given
/// execution state.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:451 DynamicSpawnStrategy.getNoExecutableUserExecExceptionMessage
fn get_no_executable_user_exec_exception_message(
    spawn: &dyn Spawn,
    spawn_local_can_exec: bool,
    spawn_execution_policy: ExecutionPolicy,
    post_processing_spawn: Option<&dyn Spawn>,
    post_processing_spawn_local_can_exec: bool,
    post_processing_spawn_execution_policy: Option<ExecutionPolicy>,
) -> String {
    let mut msg = String::new();
    if !spawn_local_can_exec {
        msg.push_str("Spawn is not executable in local: ");
        msg.push_str(&get_spawn_not_executable_reason(
            spawn,
            spawn_execution_policy,
        ));
    }
    if !post_processing_spawn_local_can_exec {
        msg.push_str("Post-Processing Spawn is not executable in local: ");
        msg.push_str(&get_spawn_not_executable_reason(
            post_processing_spawn.expect("NullPointerException: postProcessingSpawn"),
            post_processing_spawn_execution_policy.expect("NullPointerException"),
        ));
    }
    msg
}

// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:474 DynamicSpawnStrategy.getSpawnNotExecutableReason
fn get_spawn_not_executable_reason(
    spawn: &dyn Spawn,
    spawn_execution_policy: ExecutionPolicy,
) -> String {
    let mut msg = String::new();
    if !spawn_execution_policy.can_run_locally() && !spawn_execution_policy.can_run_remotely() {
        msg.push_str("Neither local nor remote execution allowed for action ");
    } else if !spawn_execution_policy.can_run_remotely() {
        msg.push_str(
            "No usable dynamic_local_strategy found (and remote execution disabled) for action ",
        );
    } else if !spawn_execution_policy.can_run_locally() {
        msg.push_str(
            "No usable dynamic_remote_strategy found (and local execution disabled) for action ",
        );
    } else {
        msg.push_str(
            "No usable dynamic_local_strategy or dynamic_remote_strategy found for action ",
        );
    }
    msg.push_str(&spawn.get_mnemonic());
    msg.push_str(". ");
    msg
}

/// Waits for the two branches of a spawn's execution to complete; the result of the branch that
/// terminates first.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:510 DynamicSpawnStrategy.waitBranches
pub fn wait_branches(
    local_branch: &Arc<Branch>,
    remote_branch: &Arc<Branch>,
    spawn: &dyn Spawn,
    options: &DynamicExecutionOptions,
    context: &dyn ActionExecutionContext,
) -> Result<Vec<SpawnResult>, ExecException> {
    let local_result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_branch(local_branch, spawn, options, context)
    })) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            if options.debug_spawn_scheduler {
                branch::debug_info(
                    context,
                    format!(
                        "Cancelling remote branch of {} after local exception {}",
                        get_spawn_readable_id(spawn),
                        exception_message(&e)
                    ),
                );
            }
            remote_branch.cancel();
            return Err(e);
        }
        Err(payload) => {
            remote_branch.cancel();
            std::panic::resume_unwind(payload);
        }
    };

    let remote_result = wait_branch(remote_branch, spawn, options, context)?;

    match (local_result, remote_result) {
        (Some(_), Some(_)) => panic!(
            "java.lang.AssertionError: Neither branch of {} cancelled the other one. Local was {} and remote was {}.",
            get_spawn_readable_id(spawn),
            local_branch.branch_state(),
            remote_branch.branch_state()
        ),
        (Some(local), None) => Ok(local),
        (None, Some(remote)) => Ok(remote),
        // TODO(b/173153395): Sometimes gets thrown for currently unknown reasons.
        (None, None) => panic!(
            "java.lang.AssertionError: Neither branch of {} completed. Local was {} and remote was {}.",
            get_spawn_readable_id(spawn),
            local_branch.branch_state(),
            remote_branch.branch_state()
        ),
    }
}

/// `e.getMessage()` of an exception that stopped a branch.
fn exception_message(e: &ExecException) -> String {
    match e {
        ExecException::Interrupted => "null".to_string(),
        other => other.get_message(),
    }
}

/// Waits for a branch (a spawn execution) to complete: the spawn results, or `None` if the
/// branch was cancelled.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:563 DynamicSpawnStrategy.waitBranch
fn wait_branch(
    branch: &Arc<Branch>,
    spawn: &dyn Spawn,
    options: &DynamicExecutionOptions,
    context: &dyn ActionExecutionContext,
) -> Result<Option<Vec<SpawnResult>>, ExecException> {
    let mode = branch.get_mode();
    match branch.get_results() {
        Ok(BranchOutcome::Success(results)) => Ok(Some(results)),
        Err(GetError::Cancelled) => {
            if options.debug_spawn_scheduler {
                branch::debug_info(
                    context,
                    format!(
                        "CancellationException of {} branch of {}, returning null",
                        branch::mode_name(mode),
                        get_spawn_readable_id(spawn)
                    ),
                );
            }
            Ok(None)
        }
        Ok(BranchOutcome::Failed(ExecException::Interrupted)) => {
            // If the branch was interrupted, it might be due to a user interrupt or due to our
            // request for cancellation. Assume the latter here because if this was actually a
            // user interrupt, our own get() would have been interrupted as well. It makes no
            // sense to propagate the interrupt status across threads.
            if options.debug_spawn_scheduler {
                // PORT: the stack trace of the InterruptedException is not available.
                branch::debug_info(
                    context,
                    format!(
                        "Caught InterruptedException from ExecutionException for {} branch of {}, which may cause a crash:\n",
                        branch::mode_name(mode),
                        get_spawn_readable_id(spawn)
                    ),
                );
            }
            Ok(None)
        }
        Ok(BranchOutcome::Failed(e)) => Err(e),
        // Unchecked exceptions bubble up.
        Ok(BranchOutcome::Panicked(payload)) => std::panic::resume_unwind(payload),
        Err(GetError::Interrupted) => {
            branch.cancel();
            Err(ExecException::Interrupted)
        }
    }
}

/// Cancels and waits for a branch (a spawn execution) to terminate. This is the body of the
/// `StopConcurrentSpawns` each branch passes to its strategy; each strategy may call it at most
/// once. `Err(ExecException::Interrupted)` if this branch lost the race (Java's
/// `DynamicInterruptedException`) or was interrupted.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:651 DynamicSpawnStrategy.stopBranch
pub(crate) fn stop_branch(
    other_branch: &Arc<Branch>,
    cancelling_branch: &Arc<Branch>,
    strategy_that_cancelled: &Mutex<Option<DynamicMode>>,
    options: &DynamicExecutionOptions,
    context: &dyn ActionExecutionContext,
    spawn: &dyn Spawn,
) -> Result<(), ExecException> {
    let cancelling_strategy = cancelling_branch.get_mode();
    if cancelling_branch.is_cancelled() {
        return Err(cancelling_branch.dynamic_interrupted_exception(format!(
            "Execution of {} strategy was cancelled just before it could get the lock.",
            mode_to_string(cancelling_strategy)
        )));
    }
    // This multi-step, unlocked access to "strategyThatCancelled" is valid because, for a given
    // value of "cancellingStrategy", we do not expect concurrent calls to this method.
    let current = *strategy_that_cancelled.lock();
    if current != Some(cancelling_strategy) {
        // Protect against the two branches from cancelling each other. The first branch to set
        // the reference to its own identifier wins and is allowed to issue the cancellation; the
        // other branch just has to give up execution.
        let won = {
            let mut guard = strategy_that_cancelled.lock();
            if guard.is_none() {
                *guard = Some(cancelling_strategy);
                true
            } else {
                false
            }
        };
        if won {
            if options.debug_spawn_scheduler {
                branch::debug_info(
                    context,
                    format!(
                        "{} branch of {} finished and was {}",
                        mode_to_string(cancelling_strategy),
                        get_spawn_readable_id(spawn),
                        if cancelling_branch.is_cancelled() {
                            "cancelled"
                        } else {
                            "not cancelled"
                        }
                    ),
                );
            }
            // PORT: skipped, profiling (DYNAMIC_LOCK "Cancelling %s branch of %s").
            if !other_branch.cancel() {
                // This can happen if the other branch is local under local_lockfree and has
                // returned its result but not yet cancelled this branch, or if the other branch
                // was already cancelled for other reasons. In the latter case, we are good to
                // continue.
                if other_branch.future_state() == FutureState::Success {
                    return Err(cancelling_branch.dynamic_interrupted_exception(format!(
                        "Execution of {} strategy stopped because {} strategy could not be cancelled",
                        mode_to_string(cancelling_strategy),
                        mode_to_string(branch::mode_other(cancelling_strategy))
                    )));
                }
            }
            other_branch.get_done_semaphore().acquire()?;
        } else {
            let winner = strategy_that_cancelled.lock().expect("set by the winner");
            return Err(cancelling_branch.dynamic_interrupted_exception(format!(
                "Execution of {} strategy stopped because {} strategy finished first",
                mode_to_string(cancelling_strategy),
                mode_to_string(winner)
            )));
        }
    }
    Ok(())
}

/// The primary output's pretty form, or the mnemonic and the owner's label.
// Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:750 DynamicSpawnStrategy.getSpawnReadableId
pub(crate) fn get_spawn_readable_id(spawn: &dyn Spawn) -> String {
    let action = spawn.get_resource_owner();
    let primary_output = action.get_primary_output();
    // Artifact.prettyPrint(): the root-relative path.
    match primary_output.0.as_artifact() {
        Some(artifact) => artifact
            .get_root_relative_path()
            .get_path_string()
            .to_string(),
        None => primary_output.0.get_exec_path_string(),
    }
}

impl fmt::Display for DynamicSpawnStrategy {
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:745 DynamicSpawnStrategy.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("dynamic")
    }
}

impl SpawnStrategy for DynamicSpawnStrategy {
    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:143 DynamicSpawnStrategy.canExec
    fn can_exec(
        &self,
        spawn: &dyn Spawn,
        action_context_registry: &dyn ActionContextRegistry,
    ) -> bool {
        let execution_policy = (self.get_execution_policy)(spawn);
        DynamicSpawnStrategy::can_exec_local(spawn, execution_policy, action_context_registry)
            || DynamicSpawnStrategy::can_exec_remote(
                spawn,
                execution_policy,
                action_context_registry,
            )
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:200 DynamicSpawnStrategy.exec
    fn exec(
        &self,
        spawn: &dyn Spawn,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        if let Some(non_dynamic_results) =
            self.maybe_execute_non_dynamically(spawn, action_execution_context)?
        {
            return Ok(non_dynamic_results);
        }
        // PORT: skipped, logging (debugLog "Dynamic execution of %s beginning").
        // else both can exec. Fallthrough to below.

        let strategy_that_cancelled: Mutex<Option<DynamicMode>> = Mutex::new(None);
        let local_branch = Branch::new(DynamicMode::Local, self.options.clone());
        let remote_branch = Branch::new(DynamicMode::Remote, self.options.clone());
        local_branch.prepare_future(&remote_branch);
        remote_branch.prepare_future(&local_branch);

        std::thread::scope(|scope| {
            let strategy_that_cancelled = &strategy_that_cancelled;
            let (lb, rb) = (local_branch.clone(), remote_branch.clone());
            let _ = std::thread::Builder::new()
                .name("dynamic-execution-thread".to_string())
                .spawn_scoped(scope, move || {
                    lb.run(action_execution_context, &|b, c| {
                        self.local_call_impl(
                            b,
                            c,
                            spawn,
                            strategy_that_cancelled,
                            action_execution_context,
                        )
                    });
                })
                .unwrap_or_else(|e| {
                    panic!("OutOfMemoryError: unable to create native thread: {e}")
                });
            let _ = std::thread::Builder::new()
                .name("dynamic-execution-thread".to_string())
                .spawn_scoped(scope, move || {
                    rb.run(action_execution_context, &|b, c| {
                        self.remote_call_impl(
                            b,
                            c,
                            spawn,
                            strategy_that_cancelled,
                            action_execution_context,
                        )
                    });
                })
                .unwrap_or_else(|e| {
                    panic!("OutOfMemoryError: unable to create native thread: {e}")
                });

            {
                let mut waiting_local_jobs = self.waiting_local_jobs.lock();
                waiting_local_jobs.push_back(local_branch.clone());
                self.try_schedule_local_job(&mut waiting_local_jobs);
            }
            remote_branch.execute();

            let results = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                wait_branches(
                    &local_branch,
                    &remote_branch,
                    spawn,
                    &self.options,
                    action_execution_context,
                )
            }));

            // finally
            blaze_util::check_state!(local_branch.is_done());
            blaze_util::check_state!(remote_branch.is_done());
            if let Ok(Ok(results)) = &results
                && let Some(first) = results.first()
            {
                self.update_strategy_winner(
                    action_execution_context,
                    spawn,
                    first,
                    strategy_that_cancelled,
                );
            }
            {
                let mut waiting_local_jobs = self.waiting_local_jobs.lock();
                let position = waiting_local_jobs
                    .iter()
                    .position(|b| Arc::ptr_eq(b, &local_branch));
                match position {
                    Some(i) => {
                        waiting_local_jobs.remove(i);
                        // A branch that was never started does not run.
                        local_branch.abandon();
                    }
                    None => {
                        self.thread_limiter.release();
                        self.try_schedule_local_job(&mut waiting_local_jobs);
                    }
                }
            }
            // PORT: skipped, logging (debugLog "Dynamic execution of %s ended with ...").
            match results {
                Ok(r) => r,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        })
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicSpawnStrategy.java:737 DynamicSpawnStrategy.usedContext
    fn used_context(&self, action_context_registry: Option<&dyn ActionContextRegistry>) {
        action_context_registry
            .and_then(|r| r.get_dynamic_strategy_registry())
            .expect("getContext(DynamicStrategyRegistry.class)")
            .notify_used_dynamic(action_context_registry);
    }

    fn get_simple_class_name(&self) -> String {
        "DynamicSpawnStrategy".to_string()
    }
}
