//! Port of `lib/remote/RemoteRewoundActionSynchronizer.java`: a `RewoundActionSynchronizer` for
//! Bazel's remote filesystem, which is backed by actual files on disk and requires
//! synchronization to ensure that action outputs aren't deleted while they are being read.
//!
//! PORT (fail-safe, ruling of root 2026-11-27): only the coarse lock is ported. Java starts with
//! a single coarse read-write lock and, when the first rewound action prepares for its
//! execution, inflates it to fine per-action locks so that rewound and non-rewound actions keep
//! running concurrently. Here every action holds the coarse lock for reading while it executes
//! and a rewound action holds it for writing from its preparation to the end of its execution:
//! a rewound action runs alone, the equivalent of `--jobs=1` for its duration. This is correct
//! (the same mutual exclusion, coarser) but slower than Java. The fine locks
//! (`ReaderPreferringReadWriteLock`, `inputKeysFor`, `lockKeysFor`) are not ported yet.
//!
//! PORT: Java requests the cancellation of the action's output uploads that are still in flight
//! (`Cancellable.requestCancellation`, which interrupts the upload) and then awaits them; here
//! the uploads are awaited without cancellation (the background upload has no interrupt to
//! deliver to), which is slower but leaves no upload reading outputs that are deleted.

use std::collections::HashMap;
use std::sync::Arc;

use blaze_actions::ActionLookupData;
use blaze_actions::actions::action::Action;
use blaze_exec::actions::InputMetadataProvider;
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::{ActionExecutionMetadata, ActionInputRef};
use blaze_exec::exec::rewound_action_synchronizer::{RewoundActionSynchronizer, SilentCloseable};
use blaze_shell::InterruptedException;
use blaze_util::profiler::{Profiler, ProfilerTask};
use parking_lot::{Condvar, Mutex};

use crate::remote_action_input_fetcher::RemoteActionInputFetcher;

/// A task whose cancellation can be requested separately from awaiting its completion.
///
/// PORT: only `awaitCompletion` is ported (see the module's PORT note).
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:52 RemoteRewoundActionSynchronizer.Cancellable
pub trait Cancellable: Send + Sync {
    /// Waits until the task no longer accesses the outputs of the action it belongs to.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:57 Cancellable.awaitCompletion
    fn await_completion(&self);
}

/// The coarse lock: Java's `ReentrantReadWriteLock` (its interruptible acquisitions).
#[derive(Default)]
struct CoarseLock {
    state: Mutex<CoarseState>,
    changed: Condvar,
}

#[derive(Default)]
struct CoarseState {
    readers: usize,
    writer: bool,
}

impl CoarseLock {
    /// Waits until `ready` holds for the state, then applies `acquire`; an interrupt of the
    /// waiting thread fails with `InterruptedException` (`lockInterruptibly`).
    fn acquire(
        self: &Arc<Self>,
        ready: impl Fn(&CoarseState) -> bool,
        acquire: impl FnOnce(&mut CoarseState),
    ) -> Result<(), InterruptedException> {
        let me = self.clone();
        let _registration = blaze_shell::ThreadInterrupt::current().add_interrupt_listener(
            Arc::new(move || {
                let _state = me.state.lock();
                me.changed.notify_all();
            }),
        );
        let mut state = self.state.lock();
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
            if ready(&state) {
                acquire(&mut state);
                return Ok(());
            }
            self.changed.wait(&mut state);
        }
    }

    fn lock_read_interruptibly(self: &Arc<Self>) -> Result<(), InterruptedException> {
        self.acquire(|s| !s.writer, |s| s.readers += 1)
    }

    fn unlock_read(&self) {
        let mut state = self.state.lock();
        state.readers -= 1;
        drop(state);
        self.changed.notify_all();
    }

    fn lock_write_interruptibly(self: &Arc<Self>) -> Result<(), InterruptedException> {
        self.acquire(|s| !s.writer && s.readers == 0, |s| s.writer = true)
    }

    fn unlock_write(&self) {
        let mut state = self.state.lock();
        state.writer = false;
        drop(state);
        self.changed.notify_all();
    }
}

/// A `RewoundActionSynchronizer` implementation for Bazel's remote filesystem, which is backed by
/// actual files on disk and requires synchronization to ensure that action outputs aren't
/// deleted while they are being read.
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:50 RemoteRewoundActionSynchronizer
pub struct RemoteRewoundActionSynchronizer {
    action_input_fetcher: Arc<RemoteActionInputFetcher>,
    // An action generally has at most one such task in flight, but nothing prevents an action
    // from executing multiple spawns whose outputs are uploaded concurrently.
    output_upload_tasks: Mutex<HashMap<ActionLookupData, Vec<Arc<dyn Cancellable>>>>,
    // A single coarse lock is used to synchronize rewound actions (writers) and both rewound and
    // non-rewound actions (readers).
    coarse_lock: Arc<CoarseLock>,
}

impl RemoteRewoundActionSynchronizer {
    /// PORT: Java also takes the `WalkableGraph`, used only by the fine locks (not ported).
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:98 RemoteRewoundActionSynchronizer.<init>
    pub fn new(action_input_fetcher: Arc<RemoteActionInputFetcher>) -> RemoteRewoundActionSynchronizer {
        RemoteRewoundActionSynchronizer {
            action_input_fetcher,
            output_upload_tasks: Mutex::new(HashMap::new()),
            coarse_lock: Arc::new(CoarseLock::default()),
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:182 RemoteRewoundActionSynchronizer.enterActionPreparationForRewinding
    fn enter_action_preparation_for_rewinding(
        &self,
        action: &dyn Action,
    ) -> Result<SilentCloseable, InterruptedException> {
        // PORT: the coarse write lock is held for the whole preparation and execution of the
        // rewound action (see the module's PORT note), where Java switches to the fine locks and
        // takes the write lock of the action's own key.
        {
            let _c = Profiler::instance()
                .profile(ProfilerTask::ACTION_LOCK, "action.awaitRewoundActionConsumers");
            self.coarse_lock.lock_write_interruptibly()?;
        }
        let prepared = {
            let _c = Profiler::instance()
                .profile(ProfilerTask::INFO, "action.prepareOutputsForRewinding");
            self.prepare_outputs_for_rewinding(action)
        };
        let coarse_lock = self.coarse_lock.clone();
        match prepared {
            Ok(()) => Ok(SilentCloseable::new(move || coarse_lock.unlock_write())),
            Err(e) => {
                coarse_lock.unlock_write();
                Err(e)
            }
        }
    }

    /// Cancels all async tasks that operate on the action's outputs and resets any cached data
    /// about their prefetching state.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:227 RemoteRewoundActionSynchronizer.prepareOutputsForRewinding
    fn prepare_outputs_for_rewinding(&self, action: &dyn Action) -> Result<(), InterruptedException> {
        let tasks = self
            .output_upload_tasks
            .lock()
            .remove(action.get_primary_output().get_generating_action_key());
        if let Some(tasks) = tasks {
            // PORT: no cancellation request (see the module's PORT note); the uploads are awaited.
            for task in &tasks {
                task.await_completion();
            }
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
        }
        self.action_input_fetcher
            .handle_rewound_action_outputs(&action.get_outputs());
        Ok(())
    }

    /// Guards a call to `RemoteImportantOutputHandler.processOutputsAndGetLostArtifacts`.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:277 RemoteRewoundActionSynchronizer.enterProcessOutputsAndGetLostArtifacts
    pub fn enter_process_outputs_and_get_lost_artifacts(
        &self,
        _important_outputs: &[ActionInputRef],
        _full_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        let _c = Profiler::instance().profile(
            ProfilerTask::ACTION_LOCK,
            "action.enterProcessOutputsAndGetLostArtifacts",
        );
        self.lock_artifacts_for_consumption()
    }

    /// Registers a cancellation callback for an upload of action outputs that may still be
    /// running after the action has completed.
    ///
    /// The returned callback must be run once the upload has completed so that the task doesn't
    /// remain registered (and thus retained) for the rest of the build.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:298 RemoteRewoundActionSynchronizer.registerOutputUploadTask
    pub fn register_output_upload_task(
        self: &Arc<Self>,
        action: &dyn ActionExecutionMetadata,
        task: Arc<dyn Cancellable>,
    ) -> Box<dyn FnOnce() + Send> {
        let key = action_key_for(&action.get_primary_output());
        self.output_upload_tasks
            .lock()
            .entry(key.clone())
            .or_default()
            .push(task.clone());
        let this = self.clone();
        Box::new(move || this.unregister_output_upload_task(&key, &task))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:315 RemoteRewoundActionSynchronizer.unregisterOutputUploadTask
    fn unregister_output_upload_task(&self, key: &ActionLookupData, task: &Arc<dyn Cancellable>) {
        let mut tasks = self.output_upload_tasks.lock();
        if let Some(remaining) = tasks.get_mut(key) {
            // Identity comparison: a task is only ever registered once, and a task registered by
            // a re-execution of the action must not be unregistered by its predecessor.
            remaining.retain(|t| !Arc::ptr_eq(t, task));
            if remaining.is_empty() {
                tasks.remove(key);
            }
        }
    }

    /// PORT: the coarse read lock only (see the module's PORT note).
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:328 RemoteRewoundActionSynchronizer.lockArtifactsForConsumption
    fn lock_artifacts_for_consumption(&self) -> Result<SilentCloseable, InterruptedException> {
        self.coarse_lock.lock_read_interruptibly()?;
        let coarse_lock = self.coarse_lock.clone();
        Ok(SilentCloseable::new(move || coarse_lock.unlock_read()))
    }
}

impl RewoundActionSynchronizer for RemoteRewoundActionSynchronizer {
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:170 RemoteRewoundActionSynchronizer.enterActionPreparation
    fn enter_action_preparation(
        &self,
        action: &dyn Action,
        was_rewound: bool,
    ) -> Result<SilentCloseable, InterruptedException> {
        // Skyframe schedules non-rewound actions such that they never run concurrently with
        // actions that consume their outputs.
        if !was_rewound {
            return Ok(SilentCloseable::noop());
        }
        let _c = Profiler::instance().profile(ProfilerTask::ACTION_LOCK, "action.enterActionPreparation");
        self.enter_action_preparation_for_rewinding(action)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:268 RemoteRewoundActionSynchronizer.enterActionExecution
    fn enter_action_execution(
        &self,
        _action: &dyn Action,
        was_rewound: bool,
        _metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        // PORT: a rewound action holds the coarse write lock since its preparation (see the
        // module's PORT note); taking the read lock as well would wait for itself. With Java's
        // fine locks it takes the read locks of its inputs' producers here.
        if was_rewound {
            return Ok(SilentCloseable::noop());
        }
        let _c = Profiler::instance().profile(ProfilerTask::ACTION_LOCK, "action.enterActionExecution");
        self.lock_artifacts_for_consumption()
    }
}

/// Returns the key that uniquely identifies the given action: the generating action key of its
/// outputs. A rewound action holds the write lock of this key while re-executing and consumers
/// of its outputs hold its read lock while executing.
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:421 RemoteRewoundActionSynchronizer.actionKeyFor
fn action_key_for(primary_output: &ActionInputRef) -> ActionLookupData {
    as_blaze_artifact(primary_output)
        .unwrap_or_else(|| panic!("ClassCastException: {primary_output:?} is not a DerivedArtifact"))
        .get_generating_action_key()
        .clone()
}
