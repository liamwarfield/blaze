//! Port of `lib/remote/RemoteRewoundActionSynchronizer.java`: a `RewoundActionSynchronizer` for
//! Bazel's remote filesystem, which is backed by actual files on disk and requires
//! synchronization to ensure that action outputs aren't deleted while they are being read.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use blaze_actions::actions::action::Action;
use blaze_actions::{ActionLookupData, Artifact};
use blaze_exec::actions::InputMetadataProvider;
use blaze_exec::actions::action_adapters::{artifact_input, as_blaze_artifact};
use blaze_exec::actions::{ActionExecutionMetadata, ActionInputRef};
use blaze_exec::exec::rewound_action_synchronizer::{RewoundActionSynchronizer, SilentCloseable};
use blaze_shell::InterruptedException;
use blaze_util::profiler::{Profiler, ProfilerTask};
use parking_lot::{Condvar, Mutex, RwLock};

use crate::reader_preferring_read_write_lock::ReaderPreferringReadWriteLock;
use crate::remote_action_input_fetcher::RemoteActionInputFetcher;

/// A task whose cancellation can be requested separately from awaiting its completion.
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:52 RemoteRewoundActionSynchronizer.Cancellable
pub trait Cancellable: Send + Sync {
    /// Requests cancellation without awaiting the task's completion.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:54 Cancellable.requestCancellation
    fn request_cancellation(&self);

    /// Waits until the task no longer accesses the outputs of the action it belongs to.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:57 Cancellable.awaitCompletion
    fn await_completion(&self) -> Result<(), InterruptedException>;
}

/// `actionInputFetcher.handleRewoundActionOutputs`, the one call the synchronizer makes on the
/// fetcher.
///
/// PORT: a trait so that the tests can observe the call (Java mocks the fetcher).
pub trait RewoundActionOutputsHandler: Send + Sync {
    fn handle_rewound_action_outputs(&self, outputs: &[Artifact]);
}

impl RewoundActionOutputsHandler for RemoteActionInputFetcher {
    fn handle_rewound_action_outputs(&self, outputs: &[Artifact]) {
        RemoteActionInputFetcher::handle_rewound_action_outputs(self, outputs)
    }
}

/// The reads of the Skyframe graph that `lockKeysFor` makes (`WalkableGraph.getValue`).
///
/// PORT: Java hands the synchronizer a `WalkableGraph`
/// (`SkyframeExecutorWrappingWalkableGraph.of(env.getSkyframeExecutor())`); the synchronizer is
/// created here before the build has its Skyframe executor, so the build supplies these reads
/// with [`RemoteRewoundActionSynchronizer::set_graph`].
pub trait RewindingActionGraph: Send + Sync {
    /// `owner.getActions().get(key.getActionIndex()) instanceof ActionTemplate`, where `owner`
    /// is the (done) `ActionLookupValue` of `key.getActionLookupKey()`.
    fn is_action_template(&self, key: &ActionLookupData) -> bool;

    /// `getGeneratingActionKeys(tree)` of the (done) `ActionTemplateExpansionValue` of
    /// `ActionTemplateExpansionValue.key(key.getActionLookupKey(), key.getActionIndex())`.
    fn get_expansion_generating_action_keys(
        &self,
        key: &ActionLookupData,
        tree: &Artifact,
    ) -> Vec<ActionLookupData>;
}

/// The coarse lock: Java's `ReentrantReadWriteLock` (its interruptible acquisitions).
///
/// PORT: new readers are not queued behind a waiting writer, where the JDK's lock (non-fair)
/// queues them behind a writer at the head of its queue. The coarse lock has one writer at most
/// once (the first rewound action, which inflates the fine locks), so this changes only who goes
/// first, never mutual exclusion.
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
        let _registration =
            blaze_shell::ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
                let _state = me.state.lock();
                me.changed.notify_all();
            }));
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

/// `LoadingCache<ActionLookupData, ReaderPreferringReadWriteLock>` with weak values: a lock
/// lives as long as someone holds it or waits for it.
///
/// PORT: Caffeine drops collected values itself; here dead entries are swept when the map has
/// doubled since the last sweep.
#[derive(Default)]
struct FineLocks {
    locks: Mutex<FineLocksMap>,
}

#[derive(Default)]
struct FineLocksMap {
    map: HashMap<ActionLookupData, Weak<ReaderPreferringReadWriteLock>>,
    sweep_at: usize,
}

impl FineLocks {
    // Java: Caffeine LoadingCache.get
    fn get(&self, key: &ActionLookupData) -> Arc<ReaderPreferringReadWriteLock> {
        let mut locks = self.locks.lock();
        if let Some(lock) = locks.map.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        if locks.map.len() >= locks.sweep_at {
            locks.map.retain(|_, lock| lock.strong_count() > 0);
            locks.sweep_at = (locks.map.len() * 2).max(64);
        }
        let lock = ReaderPreferringReadWriteLock::new();
        locks.map.insert(key.clone(), Arc::downgrade(&lock));
        lock
    }

    /// `getAll(keys).values()`: one lock per distinct key, in the order of first appearance.
    // Java: Caffeine LoadingCache.getAll
    fn get_all(&self, keys: Vec<ActionLookupData>) -> Vec<Arc<ReaderPreferringReadWriteLock>> {
        let mut seen = std::collections::HashSet::with_capacity(keys.len());
        keys.into_iter()
            .filter(|key| seen.insert(key.clone()))
            .map(|key| self.get(&key))
            .collect()
    }
}

/// A `RewoundActionSynchronizer` implementation for Bazel's remote filesystem, which is backed by
/// actual files on disk and requires synchronization to ensure that action outputs aren't
/// deleted while they are being read.
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:50 RemoteRewoundActionSynchronizer
pub struct RemoteRewoundActionSynchronizer {
    action_input_fetcher: Arc<dyn RewoundActionOutputsHandler>,
    graph: RwLock<Option<Arc<dyn RewindingActionGraph>>>,
    // An action generally has at most one such task in flight, but nothing prevents an action
    // from executing multiple spawns whose outputs are uploaded concurrently.
    output_upload_tasks: Mutex<HashMap<ActionLookupData, Vec<Arc<dyn Cancellable>>>>,
    // A single coarse lock is used to synchronize rewound actions (writers) and both rewound and
    // non-rewound actions (readers) as long as no rewound action has attempted to prepare for its
    // execution. This ensures high throughput and low memory footprint for the common case of no
    // rewound actions. Note that it wouldn't be correct to only start using this lock once an
    // action is rewound, because a non-rewound action consuming its non-lost outputs could have
    // already started executing.
    coarse_lock: RwLock<Option<Arc<CoarseLock>>>,
    // A fine-grained lock structure that is switched to when the first rewound action attempts
    // to prepare for its execution. This structure is used to ensure that rewound actions do not
    // delete their outputs while they are being read by other actions, while still allowing
    // rewound actions and non-rewound actions to run concurrently. A rewound action will acquire
    // the write lock on its own key before it prepares for execution, while any action will
    // acquire a read lock on the key of each action generating one of its inputs (see
    // inputKeysFor) before it starts executing.
    //
    // ReaderPreferringReadWriteLock is used as locks that queue readers behind waiting writers
    // even while other readers hold the lock would cause deadlocks (see the proof in the Java
    // source).
    fine_locks: RwLock<Option<Arc<FineLocks>>>,
}

impl RemoteRewoundActionSynchronizer {
    /// PORT: the `WalkableGraph` comes later, through [`Self::set_graph`].
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:98 RemoteRewoundActionSynchronizer.<init>
    pub fn new(
        action_input_fetcher: Arc<dyn RewoundActionOutputsHandler>,
    ) -> RemoteRewoundActionSynchronizer {
        RemoteRewoundActionSynchronizer {
            action_input_fetcher,
            graph: RwLock::new(None),
            output_upload_tasks: Mutex::new(HashMap::new()),
            coarse_lock: RwLock::new(Some(Arc::new(CoarseLock::default()))),
            fine_locks: RwLock::new(None),
        }
    }

    /// The graph the synchronizer reads (see [`RewindingActionGraph`]).
    pub fn set_graph(&self, graph: Arc<dyn RewindingActionGraph>) {
        *self.graph.write() = Some(graph);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:183 RemoteRewoundActionSynchronizer.enterActionPreparationForRewinding
    fn enter_action_preparation_for_rewinding(
        &self,
        action: &dyn Action,
    ) -> Result<SilentCloseable, InterruptedException> {
        let local_coarse_lock = self.coarse_lock.read().clone();
        if let Some(local_coarse_lock) = local_coarse_lock {
            // This is the first time a rewound action has attempted to prepare for its
            // execution. Switch to using the fine locks under the protection of the coarse write
            // lock.
            {
                let _c = Profiler::instance()
                    .profile(ProfilerTask::ACTION_LOCK, "action.prepareFirstRewinding");
                local_coarse_lock.lock_write_interruptibly()?;
            }
            // Check again under the lock to avoid a race between multiple rewound actions
            // attempting to prepare for execution at the same time.
            if self.fine_locks.read().is_none() {
                *self.fine_locks.write() = Some(Arc::new(FineLocks::default()));
                // Must be assigned after fineLocks as lockArtifactsForConsumption relies on a
                // null coarseLock implying a non-null fineLocks.
                *self.coarse_lock.write() = None;
            }
            local_coarse_lock.unlock_write();
        }

        let fine_locks = self
            .fine_locks
            .read()
            .clone()
            .expect("a null coarseLock implies a non-null fineLocks");
        let write_lock = fine_locks.get(&action_key_for(&action.get_primary_output()));
        {
            let _c = Profiler::instance().profile(
                ProfilerTask::ACTION_LOCK,
                "action.awaitRewoundActionConsumers",
            );
            write_lock.lock_write_interruptibly()?;
        }
        let prepared = {
            let _c = Profiler::instance()
                .profile(ProfilerTask::INFO, "action.prepareOutputsForRewinding");
            self.prepare_outputs_for_rewinding(action)
        };
        match prepared {
            Ok(()) => Ok(SilentCloseable::new(move || write_lock.unlock_write())),
            Err(e) => {
                write_lock.unlock_write();
                Err(e)
            }
        }
    }

    /// Cancels all async tasks that operate on the action's outputs and resets any cached data
    /// about their prefetching state.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:230 RemoteRewoundActionSynchronizer.prepareOutputsForRewinding
    fn prepare_outputs_for_rewinding(
        &self,
        action: &dyn Action,
    ) -> Result<(), InterruptedException> {
        let tasks = self
            .output_upload_tasks
            .lock()
            .remove(&action_key_for(&action.get_primary_output()));
        if let Some(tasks) = tasks {
            // Request cancellation from every task before awaiting any one of them so that an
            // interruption while awaiting cannot leave later tasks running without cancellation.
            for task in &tasks {
                task.request_cancellation();
            }

            let mut interruption = None;
            for task in &tasks {
                loop {
                    match task.await_completion() {
                        Ok(()) => break,
                        Err(e) => {
                            // The tasks have already been removed from the registry, so
                            // abandoning one here would let a retry delete outputs it still
                            // accesses. Finish awaiting every task and only then propagate the
                            // interruption.
                            if interruption.is_none() {
                                interruption = Some(e);
                            }
                        }
                    }
                }
            }
            if blaze_shell::thread_interrupt::interrupted() && interruption.is_none() {
                interruption = Some(InterruptedException);
            }
            if let Some(interruption) = interruption {
                return Err(interruption);
            }
        }
        self.action_input_fetcher
            .handle_rewound_action_outputs(&action.get_outputs());
        Ok(())
    }

    /// Guards a call to `RemoteImportantOutputHandler.processOutputsAndGetLostArtifacts`.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:281 RemoteRewoundActionSynchronizer.enterProcessOutputsAndGetLostArtifacts
    pub fn enter_process_outputs_and_get_lost_artifacts(
        &self,
        important_outputs: &[ActionInputRef],
        full_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        let _c = Profiler::instance().profile(
            ProfilerTask::ACTION_LOCK,
            "action.enterProcessOutputsAndGetLostArtifacts",
        );
        let artifacts: Vec<Artifact> = important_outputs
            .iter()
            .map(|output| {
                as_blaze_artifact(output)
                    .unwrap_or_else(|| panic!("ClassCastException: {output:?} is not an Artifact"))
                    .clone()
            })
            .collect();
        self.lock_artifacts_for_consumption(&artifacts, full_metadata_provider)
    }

    /// Registers a cancellation callback for an upload of action outputs that may still be
    /// running after the action has completed.
    ///
    /// The returned callback must be run once the upload has completed so that the task doesn't
    /// remain registered (and thus retained) for the rest of the build.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:301 RemoteRewoundActionSynchronizer.registerOutputUploadTask
    pub fn register_output_upload_task(
        self: &Arc<Self>,
        action: &dyn ActionExecutionMetadata,
        task: Arc<dyn Cancellable>,
    ) -> Box<dyn FnOnce() + Send> {
        let key = action_key_for_input(&action.get_primary_output());
        // merge is atomic with respect to the removal of the entry in prepareOutputsForRewinding.
        self.output_upload_tasks
            .lock()
            .entry(key.clone())
            .or_default()
            .push(task.clone());
        let this = self.clone();
        Box::new(move || this.unregister_output_upload_task(&key, &task))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:313 RemoteRewoundActionSynchronizer.hasRegisteredOutputUploadTasks
    #[cfg(test)]
    fn has_registered_output_upload_tasks(&self, action: &dyn Action) -> bool {
        self.output_upload_tasks
            .lock()
            .contains_key(&action_key_for(&action.get_primary_output()))
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:317 RemoteRewoundActionSynchronizer.unregisterOutputUploadTask
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

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:328 RemoteRewoundActionSynchronizer.lockArtifactsForConsumption
    fn lock_artifacts_for_consumption(
        &self,
        artifacts: &[Artifact],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        let local_coarse_lock = self.coarse_lock.read().clone();
        if let Some(local_coarse_lock) = &local_coarse_lock {
            // Common case for builds without any rewound actions: acquire the single lock that
            // is never acquired by a writer.
            local_coarse_lock.lock_read_interruptibly()?;
        }
        // Read the fine locks after acquiring the coarse lock to allow the fine locks to be
        // inflated lazily.
        let local_fine_locks = self.fine_locks.read().clone();
        let Some(local_fine_locks) = local_fine_locks else {
            // Continuation of the common case for builds without any rewound actions: the fine
            // locks have not been inflated.
            let coarse = local_coarse_lock.expect("a null fineLocks implies a non-null coarseLock");
            return Ok(SilentCloseable::new(move || coarse.unlock_read()));
        };

        // At this point, there has been at least one rewound action that has inflated the fine
        // locks. We need to switch to them.
        if let Some(local_coarse_lock) = &local_coarse_lock {
            local_coarse_lock.unlock_read();
        }
        let locks = local_fine_locks.get_all(self.input_keys_for(artifacts, metadata_provider));
        let mut locks_to_unlock: Vec<Arc<ReaderPreferringReadWriteLock>> =
            Vec::with_capacity(locks.len());
        for lock in locks {
            if let Err(e) = lock.lock_read_interruptibly() {
                for lock in &locks_to_unlock {
                    lock.unlock_read();
                }
                return Err(e);
            }
            locks_to_unlock.push(lock);
        }
        Ok(SilentCloseable::new(move || {
            for lock in &locks_to_unlock {
                lock.unlock_read();
            }
        }))
    }

    /// Returns the keys of the locks that guard the given artifacts (see [`Self::lock_keys_for`]).
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:372 RemoteRewoundActionSynchronizer.inputKeysFor
    fn input_keys_for(
        &self,
        artifacts: &[Artifact],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Vec<ActionLookupData> {
        let mut keys = Vec::new();
        for artifact in artifacts {
            // Iterables.filter(artifacts, DerivedArtifact.class)
            if artifact.is_source_artifact() {
                continue;
            }
            self.lock_keys_for(artifact, metadata_provider, &mut keys);
        }
        keys
    }

    /// Returns the keys of the locks that guard the given artifact. Runfiles trees are expanded
    /// into the artifacts they contain. Tree artifacts declared by an `ActionTemplate` use the
    /// keys of the expanded actions that populate them. Other artifacts use the key of their
    /// generating action.
    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:389 RemoteRewoundActionSynchronizer.lockKeysFor
    fn lock_keys_for(
        &self,
        artifact: &Artifact,
        metadata_provider: &dyn InputMetadataProvider,
        keys: &mut Vec<ActionLookupData>,
    ) {
        if artifact.is_runfiles_tree() {
            let runfiles_metadata = metadata_provider
                .get_runfiles_metadata(&artifact_input(artifact))
                .unwrap_or_else(|| panic!("NullPointerException: {artifact}"));
            let artifacts: Vec<Artifact> = runfiles_metadata
                .get_all_artifacts()
                .map(|input| {
                    as_blaze_artifact(input)
                        .unwrap_or_else(|| {
                            panic!("ClassCastException: {input:?} is not an Artifact")
                        })
                        .clone()
                })
                .collect();
            keys.extend(self.input_keys_for(&artifacts, metadata_provider));
            return;
        }
        let key = artifact.get_generating_action_key().clone();
        if !artifact.is_tree_artifact() {
            keys.push(key);
            return;
        }
        let graph = self
            .graph
            .read()
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: graph"));
        if !graph.is_action_template(&key) {
            // This tree artifact is the output of a regular action and thus always consumed as a
            // whole.
            keys.push(key);
            return;
        }
        // Crucially, action template expansion is never rewound and can thus be queried without
        // locking.
        keys.extend(graph.get_expansion_generating_action_keys(&key, artifact));
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
        let _c = Profiler::instance()
            .profile(ProfilerTask::ACTION_LOCK, "action.enterActionPreparation");
        self.enter_action_preparation_for_rewinding(action)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:268 RemoteRewoundActionSynchronizer.enterActionExecution
    fn enter_action_execution(
        &self,
        action: &dyn Action,
        _was_rewound: bool,
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<SilentCloseable, InterruptedException> {
        let _c =
            Profiler::instance().profile(ProfilerTask::ACTION_LOCK, "action.enterActionExecution");
        let inputs: Vec<Artifact> = action.get_inputs().to_list().iter().cloned().collect();
        self.lock_artifacts_for_consumption(&inputs, metadata_provider)
    }
}

/// Returns the key that uniquely identifies the given action: the generating action key of its
/// outputs. A rewound action holds the write lock of this key while re-executing and consumers
/// of its outputs hold its read lock while executing (see `lockKeysFor`).
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:425 RemoteRewoundActionSynchronizer.actionKeyFor
fn action_key_for(primary_output: &Artifact) -> ActionLookupData {
    primary_output.get_generating_action_key().clone()
}

/// [`action_key_for`] for the primary output of an `ActionExecutionMetadata`.
// Java: com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizer.java:425 RemoteRewoundActionSynchronizer.actionKeyFor
fn action_key_for_input(primary_output: &ActionInputRef) -> ActionLookupData {
    action_key_for(as_blaze_artifact(primary_output).unwrap_or_else(|| {
        panic!("ClassCastException: {primary_output:?} is not a DerivedArtifact")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// Readers share the coarse lock; a writer waits for them and excludes new readers.
    #[test]
    fn coarse_lock_writer_excludes_readers() {
        let lock = Arc::new(CoarseLock::default());
        lock.lock_read_interruptibly().unwrap();
        lock.lock_read_interruptibly().unwrap();
        let writer_in = Arc::new(AtomicBool::new(false));
        let writer = {
            let lock = lock.clone();
            let writer_in = writer_in.clone();
            std::thread::spawn(move || {
                lock.lock_write_interruptibly().unwrap();
                writer_in.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                writer_in.store(false, Ordering::SeqCst);
                lock.unlock_write();
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !writer_in.load(Ordering::SeqCst),
            "the writer waits for the readers"
        );
        lock.unlock_read();
        lock.unlock_read();
        while !writer_in.load(Ordering::SeqCst) && !writer.is_finished() {
            std::thread::yield_now();
        }
        // A reader arriving while the writer holds the lock waits for it.
        lock.lock_read_interruptibly().unwrap();
        assert!(!writer_in.load(Ordering::SeqCst));
        lock.unlock_read();
        writer.join().unwrap();
    }

    /// `lockInterruptibly`: an interrupt of the waiting thread ends the wait with
    /// `InterruptedException`.
    #[test]
    fn coarse_lock_wait_is_interruptible() {
        let lock = Arc::new(CoarseLock::default());
        lock.lock_write_interruptibly().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let waiter = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                tx.send(blaze_shell::ThreadInterrupt::current()).unwrap();
                lock.lock_read_interruptibly()
            })
        };
        let interrupt = rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        interrupt.interrupt();
        assert!(waiter.join().unwrap().is_err());
        lock.unlock_write();
    }

    // ---------------------------------------------------------------------------------------
    // RemoteRewoundActionSynchronizerTest
    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java

    use blaze_actions::actions::action::{AbstractAction, ComputeKeyError};
    use blaze_actions::actions::action_key_context::ActionKeyContext;
    use blaze_actions::actions::action_lookup::{ACTION_TEMPLATE_EXPANSION, ActionLookupKey};
    use blaze_actions::{ActionLookupKeyRef, ArtifactRoot, RootType, SpecialArtifactType};
    use blaze_collect::{NestedSet, NestedSetBuilder, Order};
    use blaze_exec::actions::file_artifact_value::FileArtifactValue;
    use blaze_exec::actions::input_metadata_provider::{
        FilesetOutputTree, RunfilesArtifactValue, RunfilesTree, SymlinkEntry, TreeArtifactValue,
    };
    use blaze_vfs::{DigestHashFunction, FileSystemExt, PathFragment, UnixFileSystem};
    use std::any::Any;
    use std::fmt;
    use std::sync::atomic::AtomicUsize;

    const DEADLOCK_TIMEOUT: Duration = Duration::from_secs(10);

    /// `ActionsTestUtil.NULL_ARTIFACT_OWNER` (and, with an index, the expansion key of its
    /// template: `ActionTemplateExpansionValue.key(owner, index)`).
    #[derive(Debug)]
    struct Owner(Option<i32>);

    impl fmt::Display for Owner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self.0)
        }
    }

    impl ActionLookupKey for Owner {
        fn get_label(&self) -> Option<blaze_cmdline::Label> {
            Some(blaze_cmdline::Label::parse_canonical("//null/action:owner").unwrap())
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            None
        }
        fn function_name(&self) -> &'static str {
            if self.0.is_some() {
                ACTION_TEMPLATE_EXPANSION
            } else {
                "NULL_ARTIFACT_OWNER"
            }
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other
                .as_any()
                .downcast_ref::<Owner>()
                .is_some_and(|o| o.0 == self.0)
        }
        fn key_hash(&self) -> u64 {
            self.0.map_or(1, |i| 100 + i as u64)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn owner() -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(Owner(None))
    }

    fn expansion(index: i32) -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(Owner(Some(index)))
    }

    fn root() -> ArtifactRoot {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        ArtifactRoot::as_derived_root(&fs.get_path("/exec"), RootType::OUTPUT, &["out"])
    }

    /// `ActionsTestUtil.createArtifact(root, name)` with a generating action key.
    fn derived(root: &ArtifactRoot, name: &str, key: ActionLookupData) -> Artifact {
        let a = Artifact::create_derived_artifact(
            root.clone(),
            root.get_exec_path()
                .get_relative(&PathFragment::create(name)),
            owner(),
        );
        a.set_generating_action_key(key);
        a
    }

    fn tree(root: &ArtifactRoot, name: &str, key: ActionLookupData) -> Artifact {
        let t = Artifact::create_special_artifact(
            root.clone(),
            root.get_exec_path()
                .get_relative(&PathFragment::create(name)),
            key.get_action_lookup_key().clone(),
            SpecialArtifactType::TREE,
        );
        t.set_generating_action_key(key);
        t
    }

    fn tree_file(parent: &Artifact, expansion_index: i32, action_index: i32) -> Artifact {
        let f = Artifact::create_template_expansion_output(
            parent,
            PathFragment::create("file"),
            expansion(expansion_index),
        );
        f.set_generating_action_key(ActionLookupData::create(
            expansion(expansion_index),
            action_index,
        ));
        f
    }

    #[derive(Debug)]
    struct TestAction {
        base: AbstractAction,
    }

    impl Action for TestAction {
        fn abstract_action(&self) -> &AbstractAction {
            &self.base
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn get_mnemonic(&self) -> String {
            "Test".to_owned()
        }
        fn compute_key(
            &self,
            _action_key_context: &ActionKeyContext,
            _input_metadata_provider: Option<
                &dyn blaze_actions::actions::action::InputMetadataProvider,
            >,
            _fp: &mut blaze_util::Fingerprint,
        ) -> Result<(), ComputeKeyError> {
            Ok(())
        }
    }

    fn new_action_with(outputs: Vec<Artifact>, inputs: Vec<Artifact>) -> Arc<TestAction> {
        Arc::new(TestAction {
            base: AbstractAction::new(
                blaze_actions::analysis::workspace_status_action::system_action_owner(),
                NestedSetBuilder::wrap(Order::STABLE_ORDER, inputs),
                outputs,
            ),
        })
    }

    fn new_action() -> Arc<TestAction> {
        new_action_with(
            vec![derived(
                &root(),
                "output",
                ActionLookupData::create(owner(), 0),
            )],
            Vec::new(),
        )
    }

    #[derive(Default)]
    struct CountingFetcher {
        handled: AtomicUsize,
    }

    impl RewoundActionOutputsHandler for CountingFetcher {
        fn handle_rewound_action_outputs(&self, _outputs: &[Artifact]) {
            self.handled.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// `graph`: the owner's action 0 is a template whose expansion consists of `expanded`.
    struct TemplateGraph {
        expanded: Vec<Arc<TestAction>>,
    }

    impl RewindingActionGraph for TemplateGraph {
        fn is_action_template(&self, key: &ActionLookupData) -> bool {
            key.get_action_index() == 0
        }
        fn get_expansion_generating_action_keys(
            &self,
            _key: &ActionLookupData,
            tree: &Artifact,
        ) -> Vec<ActionLookupData> {
            // ActionTemplateExpansionValue.getGeneratingActionKeys.
            let mut keys = Vec::new();
            for action in &self.expanded {
                for output in action.get_outputs() {
                    if output.get_parent() == Some(tree) {
                        keys.push(output.get_generating_action_key().clone());
                        break;
                    }
                }
            }
            keys
        }
    }

    struct Fixture {
        fetcher: Arc<CountingFetcher>,
        synchronizer: Arc<RemoteRewoundActionSynchronizer>,
    }

    fn set_up() -> Fixture {
        let fetcher = Arc::new(CountingFetcher::default());
        let synchronizer = Arc::new(RemoteRewoundActionSynchronizer::new(
            fetcher.clone() as Arc<dyn RewoundActionOutputsHandler>
        ));
        Fixture {
            fetcher,
            synchronizer,
        }
    }

    fn rewind(
        synchronizer: &RemoteRewoundActionSynchronizer,
        action: &dyn Action,
    ) -> Result<(), InterruptedException> {
        let _ignored = synchronizer.enter_action_preparation(action, true)?;
        // Cancellation happens while entering preparation.
        Ok(())
    }

    fn metadata(
        action: &TestAction,
    ) -> Arc<blaze_exec::actions::action_adapters::ActionMetadataSnapshot> {
        blaze_exec::actions::action_adapters::ActionMetadataSnapshot::of(action)
    }

    /// A mock `Cancellable` that records its calls.
    #[derive(Default)]
    struct MockCancellable {
        calls: Mutex<Vec<(&'static str, usize)>>,
        id: usize,
        log: Option<Arc<Mutex<Vec<(&'static str, usize)>>>>,
        await_failures: AtomicUsize,
    }

    impl MockCancellable {
        fn new(id: usize, log: &Arc<Mutex<Vec<(&'static str, usize)>>>) -> Arc<MockCancellable> {
            Arc::new(MockCancellable {
                id,
                log: Some(log.clone()),
                ..Default::default()
            })
        }
        fn count(&self, call: &str) -> usize {
            self.calls.lock().iter().filter(|(c, _)| *c == call).count()
        }
        fn record(&self, call: &'static str) {
            self.calls.lock().push((call, self.id));
            if let Some(log) = &self.log {
                log.lock().push((call, self.id));
            }
        }
    }

    impl Cancellable for MockCancellable {
        fn request_cancellation(&self) {
            self.record("requestCancellation");
        }
        fn await_completion(&self) -> Result<(), InterruptedException> {
            self.record("awaitCompletion");
            if self.await_failures.load(Ordering::SeqCst) > 0 {
                self.await_failures.fetch_sub(1, Ordering::SeqCst);
                return Err(InterruptedException);
            }
            Ok(())
        }
    }

    fn register(
        f: &Fixture,
        action: &TestAction,
        task: Arc<dyn Cancellable>,
    ) -> Box<dyn FnOnce() + Send> {
        f.synchronizer
            .register_output_upload_task(&*metadata(action), task)
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:79 RemoteRewoundActionSynchronizerTest.rewind_cancelsAllRegisteredTasksOnce
    #[test]
    fn rewind_cancels_all_registered_tasks_once() {
        let f = set_up();
        let action = new_action();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = MockCancellable::new(1, &log);
        let second = MockCancellable::new(2, &log);
        let _unused_first = register(&f, &action, first.clone());
        let _unused_second = register(&f, &action, second.clone());

        rewind(&f.synchronizer, &*action).unwrap();
        rewind(&f.synchronizer, &*action).unwrap();

        assert_eq!(first.count("requestCancellation"), 1);
        assert_eq!(first.count("awaitCompletion"), 1);
        assert_eq!(second.count("requestCancellation"), 1);
        assert_eq!(second.count("awaitCompletion"), 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:96 RemoteRewoundActionSynchronizerTest.rewind_interruptedWhileAwaiting_cancelsAndAwaitsEveryTask
    #[test]
    fn rewind_interrupted_while_awaiting_cancels_and_awaits_every_task() {
        let f = set_up();
        let action = new_action();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = MockCancellable::new(1, &log);
        first.await_failures.store(1, Ordering::SeqCst);
        let second = MockCancellable::new(2, &log);
        let _unused_first = register(&f, &action, first.clone());
        let _unused_second = register(&f, &action, second.clone());

        assert!(rewind(&f.synchronizer, &*action).is_err());

        assert_eq!(
            *log.lock(),
            vec![
                ("requestCancellation", 1),
                ("requestCancellation", 2),
                ("awaitCompletion", 1),
                ("awaitCompletion", 1),
                ("awaitCompletion", 2),
            ]
        );
        assert_eq!(f.fetcher.handled.load(Ordering::SeqCst), 0);
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:115 RemoteRewoundActionSynchronizerTest.unregisterHandle_preventsCancellation
    #[test]
    fn unregister_handle_prevents_cancellation() {
        let f = set_up();
        let action = new_action();
        let log = Arc::new(Mutex::new(Vec::new()));
        let task = MockCancellable::new(1, &log);
        let unregister = register(&f, &action, task.clone());

        unregister();
        rewind(&f.synchronizer, &*action).unwrap();

        assert_eq!(task.count("requestCancellation"), 0);
        assert_eq!(task.count("awaitCompletion"), 0);
        assert!(!f.synchronizer.has_registered_output_upload_tasks(&*action));
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:129 RemoteRewoundActionSynchronizerTest.predecessorUnregisterHandle_doesNotRemoveReplacementTask
    #[test]
    fn predecessor_unregister_handle_does_not_remove_replacement_task() {
        let f = set_up();
        let action = new_action();
        let log = Arc::new(Mutex::new(Vec::new()));
        let predecessor = MockCancellable::new(1, &log);
        let unregister_predecessor = register(&f, &action, predecessor.clone());
        rewind(&f.synchronizer, &*action).unwrap();
        let replacement = MockCancellable::new(2, &log);
        let _unused = register(&f, &action, replacement.clone());

        unregister_predecessor();
        rewind(&f.synchronizer, &*action).unwrap();

        assert_eq!(predecessor.count("requestCancellation"), 1);
        assert_eq!(predecessor.count("awaitCompletion"), 1);
        assert_eq!(replacement.count("requestCancellation"), 1);
        assert_eq!(replacement.count("awaitCompletion"), 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:147 RemoteRewoundActionSynchronizerTest.unregisterHandle_usesIdentityComparison
    #[test]
    fn unregister_handle_uses_identity_comparison() {
        // PORT: Java's EqualCancellable overrides equals; Rust identity is Arc::ptr_eq, which two
        // distinct (equal-valued) tasks never share.
        let f = set_up();
        let action = new_action();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = MockCancellable::new(0, &log);
        let second = MockCancellable::new(0, &log);
        let unregister_first = register(&f, &action, first.clone());
        let _unused = register(&f, &action, second.clone());

        unregister_first();
        rewind(&f.synchronizer, &*action).unwrap();

        assert_eq!(first.count("requestCancellation"), 0);
        assert_eq!(second.count("requestCancellation"), 1);
    }

    /// An `InputMetadataProvider` that only knows runfiles metadata.
    #[derive(Default)]
    struct RunfilesMetadataProvider {
        runfiles: Vec<(Artifact, Arc<RunfilesArtifactValue>)>,
    }

    impl InputMetadataProvider for RunfilesMetadataProvider {
        fn get_input_metadata(
            &self,
            _input: &ActionInputRef,
        ) -> std::io::Result<Option<FileArtifactValue>> {
            Ok(None)
        }
        fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
            None
        }
        fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
            None
        }
        fn get_runfiles_metadata(
            &self,
            input: &ActionInputRef,
        ) -> Option<Arc<RunfilesArtifactValue>> {
            self.runfiles
                .iter()
                .find(|(a, _)| Some(a) == as_blaze_artifact(input))
                .map(|(_, v)| v.clone())
        }
        fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
            None
        }
    }

    #[derive(Debug)]
    struct TestRunfilesTree {
        artifacts: Vec<ActionInputRef>,
    }

    impl RunfilesTree for TestRunfilesTree {
        fn get_exec_path(&self) -> PathFragment {
            PathFragment::create("out/runfiles")
        }
        fn get_mapping(&self) -> Vec<(PathFragment, Option<ActionInputRef>)> {
            self.artifacts
                .iter()
                .map(|a| (a.get_exec_path().clone(), Some(a.clone())))
                .collect()
        }
        fn get_workspace_name(&self) -> String {
            "workspace".to_owned()
        }
        fn get_artifacts_at_canonical_locations_for_logging(&self) -> NestedSet<ActionInputRef> {
            NestedSetBuilder::wrap(Order::STABLE_ORDER, self.artifacts.clone())
        }
        fn get_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
            NestedSetBuilder::empty_set(Order::STABLE_ORDER)
        }
        fn get_root_symlinks_for_logging(&self) -> NestedSet<SymlinkEntry> {
            NestedSetBuilder::empty_set(Order::STABLE_ORDER)
        }
        fn get_empty_filenames_for_logging(&self) -> Vec<PathFragment> {
            Vec::new()
        }
        fn get_repo_mapping_manifest_for_logging(&self) -> Option<ActionInputRef> {
            None
        }
    }

    /// Waits until `thread` blocks (Java: `Thread.State.WAITING`).
    ///
    /// PORT: Rust threads do not expose their state; the test gives the thread time to block and
    /// checks that it has not finished.
    fn wait_until_blocked<T>(thread: &std::thread::JoinHandle<T>) {
        std::thread::sleep(Duration::from_millis(100));
        assert!(!thread.is_finished());
    }

    fn join_within<T>(thread: std::thread::JoinHandle<T>) -> T {
        let deadline = std::time::Instant::now() + DEADLOCK_TIMEOUT;
        while !thread.is_finished() {
            assert!(std::time::Instant::now() < deadline, "deadlock");
            std::thread::sleep(Duration::from_millis(5));
        }
        thread.join().unwrap()
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:168 RemoteRewoundActionSynchronizerTest.outputProcessing_runfilesTree_locksOnlyItsProducers
    #[test]
    fn output_processing_runfiles_tree_locks_only_its_producers() {
        let f = set_up();
        let root = root();
        let file = derived(&root, "file", ActionLookupData::create(owner(), 0));
        let producer = new_action_with(vec![file.clone()], Vec::new());
        let unrelated_file = derived(&root, "unrelated", ActionLookupData::create(owner(), 1));
        let unrelated_producer = new_action_with(vec![unrelated_file], Vec::new());
        let runfiles = Artifact::create_special_artifact(
            root.clone(),
            PathFragment::create("out/runfiles"),
            owner(),
            SpecialArtifactType::RUNFILES,
        );
        runfiles.set_generating_action_key(ActionLookupData::create(owner(), 2));
        let runfiles_action = new_action_with(vec![runfiles.clone()], vec![file.clone()]);

        let runfiles_value = RunfilesArtifactValue::new(
            Arc::new(TestRunfilesTree {
                artifacts: vec![artifact_input(&file)],
            }),
            vec![artifact_input(&file)],
            vec![FileArtifactValue::create_for_normal_file(
                Some(vec![0; 32]),
                None,
                0,
            )],
            Vec::new(),
            Vec::new(),
        );
        let metadata_provider = RunfilesMetadataProvider {
            runfiles: vec![(runfiles.clone(), Arc::new(runfiles_value))],
        };

        // Switch to the fine locks with an unrelated rewound action first: the coarse lock would
        // exclude every rewound action regardless of which keys guard the runfiles tree.
        rewind(&f.synchronizer, &*new_action()).unwrap();

        let spawn_rewind = |action: Arc<TestAction>| {
            let synchronizer = f.synchronizer.clone();
            std::thread::spawn(move || rewind(&synchronizer, &*action).unwrap())
        };
        let processing = f
            .synchronizer
            .enter_process_outputs_and_get_lost_artifacts(
                &[artifact_input(&runfiles)],
                &metadata_provider,
            )
            .unwrap();
        let preparation = spawn_rewind(producer);
        wait_until_blocked(&preparation);
        join_within(spawn_rewind(unrelated_producer));
        join_within(spawn_rewind(runfiles_action));
        drop(processing);
        join_within(preparation);
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:228 RemoteRewoundActionSynchronizerTest.expandedActionRewound_treeConsumer_excludesOnlyProducersOfItsTree
    #[test]
    fn expanded_action_rewound_tree_consumer_excludes_only_producers_of_its_tree() {
        let f = set_up();
        let root = root();
        let template_key = ActionLookupData::create(owner(), 0);
        let first_tree = tree(&root, "first", template_key.clone());
        let second_tree = tree(&root, "second", template_key);
        let first_file = tree_file(&first_tree, 0, 0);
        let second_file = tree_file(&second_tree, 0, 1);
        let first_producer = new_action_with(vec![first_file], Vec::new());
        let second_producer = new_action_with(vec![second_file], Vec::new());
        let consumer_output = derived(&root, "consumer.out", ActionLookupData::create(owner(), 1));
        let consumer = new_action_with(vec![consumer_output], vec![first_tree]);

        let metadata_provider = Arc::new(RunfilesMetadataProvider::default());
        f.synchronizer.set_graph(Arc::new(TemplateGraph {
            expanded: vec![first_producer.clone(), second_producer.clone()],
        }));

        // Switch to the fine locks with an unrelated rewound action first.
        rewind(&f.synchronizer, &*new_action()).unwrap();

        let second_preparation = f
            .synchronizer
            .enter_action_preparation(&*second_producer, true)
            .unwrap();
        // The consumer of the first tree is admitted while the producer of the second tree holds
        // its write lock.
        let consumer_execution = {
            let (synchronizer, consumer, mp) = (
                f.synchronizer.clone(),
                consumer.clone(),
                metadata_provider.clone(),
            );
            std::thread::spawn(move || {
                let _execution = synchronizer
                    .enter_action_execution(&*consumer, false, &*mp)
                    .unwrap();
            })
        };
        join_within(consumer_execution);

        // The producer of the first tree still waits for the consumer.
        let execution = f
            .synchronizer
            .enter_action_execution(&*consumer, false, &*metadata_provider)
            .unwrap();
        let first_preparation = {
            let synchronizer = f.synchronizer.clone();
            std::thread::spawn(move || rewind(&synchronizer, &*first_producer).unwrap())
        };
        wait_until_blocked(&first_preparation);
        drop(execution);
        join_within(first_preparation);
        drop(second_preparation);
    }

    // Java: src/test/java/com/google/devtools/build/lib/remote/RemoteRewoundActionSynchronizerTest.java:299 RemoteRewoundActionSynchronizerTest.expandedActionRewound_treeConsumerNotQueuedBehindWaitingProducer
    #[test]
    fn expanded_action_rewound_tree_consumer_not_queued_behind_waiting_producer() {
        let f = set_up();
        let root = root();

        // The upstream template (action 0 of the owner) declares a tree artifact that its single
        // expanded action P populates with a file.
        let upstream_tree = tree(&root, "upstream", ActionLookupData::create(owner(), 0));
        let upstream_file = tree_file(&upstream_tree, 0, 0);
        let upstream_action = new_action_with(vec![upstream_file.clone()], Vec::new());

        // The regular action C (action 1 of the owner) consumes the whole upstream tree.
        let tree_consumer_output = derived(
            &root,
            "tree_consumer.out",
            ActionLookupData::create(owner(), 1),
        );
        let tree_consumer =
            new_action_with(vec![tree_consumer_output.clone()], vec![upstream_tree]);

        // The downstream template (action 2 of the owner) is expanded over the upstream tree.
        // Its expanded action D consumes the file generated by P as well as the output of C and
        // acquires their read locks in this order.
        let downstream_tree = tree(&root, "downstream", ActionLookupData::create(owner(), 2));
        let downstream_file = tree_file(&downstream_tree, 2, 0);
        let downstream_action = new_action_with(
            vec![downstream_file],
            vec![upstream_file, tree_consumer_output],
        );

        let metadata_provider = Arc::new(RunfilesMetadataProvider::default());
        f.synchronizer.set_graph(Arc::new(TemplateGraph {
            expanded: vec![upstream_action.clone()],
        }));

        // C is rewound and prepares for its re-execution, which makes it hold the write lock
        // guarding its output until the end of its execution.
        let tree_consumer_preparation = f
            .synchronizer
            .enter_action_preparation(&*tree_consumer, true)
            .unwrap();
        // D enters execution, acquires the read lock guarding the upstream file and then blocks
        // on the read lock guarding the output of C.
        let downstream_execution = {
            let (synchronizer, action, mp) = (
                f.synchronizer.clone(),
                downstream_action.clone(),
                metadata_provider.clone(),
            );
            std::thread::spawn(move || {
                let _execution = synchronizer
                    .enter_action_execution(&*action, false, &*mp)
                    .unwrap();
            })
        };
        wait_until_blocked(&downstream_execution);
        // P is rewound and prepares for its re-execution, which blocks on the read lock held by
        // D.
        let upstream_preparation = {
            let synchronizer = f.synchronizer.clone();
            std::thread::spawn(move || rewind(&synchronizer, &*upstream_action).unwrap())
        };
        wait_until_blocked(&upstream_preparation);
        // C enters execution, which requires the read lock guarding the upstream tree. It must
        // not wait for P, which waits for D, which waits for C.
        let tree_consumer_execution = {
            let (synchronizer, action, mp) = (
                f.synchronizer.clone(),
                tree_consumer.clone(),
                metadata_provider.clone(),
            );
            std::thread::spawn(move || {
                let _preparation = tree_consumer_preparation;
                let _execution = synchronizer
                    .enter_action_execution(&*action, true, &*mp)
                    .unwrap();
            })
        };

        join_within(tree_consumer_execution);
        join_within(downstream_execution);
        join_within(upstream_preparation);
    }
}
