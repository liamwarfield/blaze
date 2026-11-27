//! Port of `lib/remote/RemoteLeaseExtension.java`.
//!
//! PORT: the `MemoizingEvaluator` (whose `ACTION_EXECUTION` values and action lookup values live
//! in crates above this one) is read through [`LeaseExtensionGraph`], which the build command
//! implements over its evaluator (`memoizingEvaluator.getValues()` and `getActionFromSkyKey`).

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use blaze_actions::Artifact;
use blaze_actions::actions::action::Action;
use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::TreeArtifactValue;
use blaze_exec::actions::action_adapters::as_blaze_artifact;
use blaze_exec::actions::cache::{ActionCache, ActionCacheEntry};
use blaze_shell::thread_interrupt::ThreadInterrupt;
use blaze_util::profiler::{Profiler, ProfilerTask};
use indexmap::IndexMap;
use parking_lot::{Condvar, Mutex};

use crate::combined_cache::CombinedCache;
use crate::common::errors;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::lease_service::LeaseExtension;
use crate::reapi::Digest;
use crate::runtime::get_from_future;
use crate::util::digest_util::DigestUtil;
use crate::util::tracing_metadata_utils;

/// The `MemoizingEvaluator` of `RemoteLeaseExtension`: its values as the lease extension reads
/// them.
pub trait LeaseExtensionGraph: Send + Sync {
    /// `memoizingEvaluator.getValues()`, in the map's order: for each `ACTION_EXECUTION` entry
    /// with a value, `visit(action, getAllFileValues(), getAllTreeArtifactValues())`, the action
    /// being `getActionFromSkyKey(key)` (`ActionLookupValue.getAction(actionIndex)` of the
    /// existing value of the key's `ActionLookupKey`; an error when there is none, Java's
    /// `checkNotNull`). Stops at the first error, of the lookup or of `visit`.
    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:230 RemoteLeaseExtension.getActionFromSkyKey
    fn for_each_action_execution_value(
        &self,
        visit: &mut dyn FnMut(
            &dyn Action,
            &IndexMap<Artifact, FileArtifactValue>,
            &IndexMap<Artifact, Arc<TreeArtifactValue>>,
        ) -> io::Result<()>,
    ) -> io::Result<()>;
}

/// A remote output of an action: `Map.Entry<? extends Artifact, FileArtifactValue>`.
type RemoteFile = (Artifact, FileArtifactValue);

/// A [`LeaseExtension`] implementation that uses REAPI.
// Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:60 RemoteLeaseExtension
pub struct RemoteLeaseExtension {
    inner: Arc<Inner>,
}

struct Inner {
    /// `scheduledExecutor`: `newSingleThreadScheduledExecutor` named `lease-extension-%d`.
    scheduled_executor: ScheduledExecutor,
    /// `lock`: one `doExtendLeases` at a time.
    lock: Mutex<()>,
    memoizing_evaluator: Arc<dyn LeaseExtensionGraph>,
    action_cache: Option<Arc<dyn ActionCache>>,
    combined_cache: Arc<CombinedCache>,
    remote_cache_ttl: Duration,
    context: RemoteActionExecutionContext<'static>,
}

impl RemoteLeaseExtension {
    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:77 RemoteLeaseExtension.<init>
    pub fn new(
        memoizing_evaluator: Arc<dyn LeaseExtensionGraph>,
        action_cache: Option<Arc<dyn ActionCache>>,
        build_request_id: &str,
        command_id: &str,
        combined_cache: Arc<CombinedCache>,
        remote_cache_ttl: Duration,
    ) -> RemoteLeaseExtension {
        let request_metadata = tracing_metadata_utils::build_metadata(
            build_request_id,
            command_id,
            "lease-extension",
            None,
        );
        RemoteLeaseExtension {
            inner: Arc::new(Inner {
                scheduled_executor: ScheduledExecutor::new("lease-extension-%d"),
                lock: Mutex::new(()),
                memoizing_evaluator,
                action_cache,
                combined_cache,
                remote_cache_ttl,
                context: RemoteActionExecutionContext::create(request_metadata),
            }),
        }
    }
}

impl LeaseExtension for RemoteLeaseExtension {
    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:94 RemoteLeaseExtension.start
    fn start(&self) {
        // Immediately extend leases for outputs that are already known to skyframe. For clean
        // build, the set of outputs is empty. For incremental build, it contains outputs that
        // were not invalidated after skyframe's dirtiness check.
        Inner::schedule_extend_leases(&self.inner, Duration::ZERO);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:220 RemoteLeaseExtension.stop
    fn stop(&self) {
        // ExecutorUtil.uninterruptibleShutdownNow(scheduledExecutor): the running task is
        // interrupted, the scheduled ones are dropped, and the thread is waited for.
        self.inner.scheduled_executor.shutdown_now();
    }
}

impl Inner {
    fn schedule_extend_leases(this: &Arc<Inner>, delay: Duration) {
        let weak = Arc::downgrade(this);
        this.scheduled_executor.schedule(
            delay,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    Inner::extend_leases(&inner);
                }
            }),
        );
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:101 RemoteLeaseExtension.extendLeases
    fn extend_leases(this: &Arc<Inner>) {
        // Acquire the lock to prevent multiple doExtendLeases() running.
        let _lock = this.lock.lock();
        let _profile = Profiler::instance().profile(ProfilerTask::INFO, "doExtendLeases");
        Inner::do_extend_leases(this);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:111 RemoteLeaseExtension.doExtendLeases
    fn do_extend_leases(this: &Arc<Inner>) {
        // We will extend leases for all known outputs so the earliest time when one output could
        // be expired is (now + ttl).
        let earliest_expiration = SystemTime::now() + this.remote_cache_ttl;

        let result = this.memoizing_evaluator.for_each_action_execution_value(
            &mut |action, file_values, tree_artifact_values| {
                let remote_files = collect_remote_files(file_values, tree_artifact_values);
                if !remote_files.is_empty() {
                    // Lease extensions are performed on action basis, not by collecting all
                    // outputs and issue one giant `FindMissingBlobs` call to avoid increasing
                    // memory footprint. Since this happens in the background, increased network
                    // calls are acceptable.
                    let _profile =
                        Profiler::instance().profile(ProfilerTask::INFO, &action.describe());
                    this.extend_lease_for_action(action, &remote_files, earliest_expiration)?;
                }
                Ok(())
            },
        );
        match result {
            Ok(()) => {}
            Err(e) if errors::is_interrupted(&e) => {
                // Thread.currentThread().interrupt(); return
                blaze_shell::thread_interrupt::interrupt_current();
                return;
            }
            // logger.atWarning().withCause(e).log("Failed to extend the lease")
            // PORT: skipped, the server log (java.log) is not ported.
            Err(_e) => {}
        }

        // Only extend the leases again when one of the outputs is about to expire.
        let now = SystemTime::now();
        let delay = earliest_expiration
            .duration_since(now)
            .unwrap_or(Duration::ZERO);
        // Duration.toMillis(): whole milliseconds.
        let delay = Duration::from_millis(delay.as_millis() as u64);
        Inner::schedule_extend_leases(this, delay);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:178 RemoteLeaseExtension.extendLeaseForAction
    fn extend_lease_for_action(
        &self,
        action: &dyn Action,
        remote_files: &[RemoteFile],
        expiration_time: SystemTime,
    ) -> io::Result<()> {
        let missing_digests = {
            let _profile = Profiler::instance().profile(ProfilerTask::INFO, "findMissingDigests");
            // We assume remote server will extend the leases for all referenced blobs by a
            // FindMissingBlobs call.
            let digests: Vec<Digest> = remote_files
                .iter()
                .map(|(_, metadata)| build_digest(metadata))
                .collect();
            get_from_future(
                self.combined_cache
                    .find_missing_digests(&self.context, &digests),
            )?
        };

        let expiration_millis = epoch_millis(expiration_time);
        let mut token = self.get_action_cache_token(action);
        for (artifact, metadata) in remote_files {
            // Only extend the lease for the remote output if it is still alive remotely.
            if !missing_digests.contains(&build_digest(metadata)) {
                metadata.set_expiration_time(Some(expiration_millis));
                if let Some(token) = &mut token {
                    if artifact.get_parent().is_some() {
                        token.extend_output_tree_file(artifact, expiration_millis);
                    } else {
                        token.extend_output_file(artifact, expiration_millis);
                    }
                }
            }
        }

        if let Some(action_cache) = &self.action_cache
            && let Some(token) = token
            && token.dirty
        {
            // Only update the action cache entry if the token was updated because it usually
            // involves serialization.
            action_cache.put(&token.key, token.entry);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:240 RemoteLeaseExtension.getActionCacheToken
    fn get_action_cache_token(&self, action: &dyn Action) -> Option<ActionCacheToken> {
        let action_cache = self.action_cache.as_ref()?;
        // ActionCacheUtils.getCacheEntryWithKey(actionCache, action): the first output whose
        // exec path has an entry.
        // Java: com/google/devtools/build/lib/actions/ActionCacheUtils.java:26 ActionCacheUtils.getCacheEntryWithKey
        for output in action.get_outputs() {
            let key = output.get_exec_path_string();
            if let Some(entry) = action_cache.get(&key) {
                return Some(ActionCacheToken {
                    key: key.to_string(),
                    entry,
                    dirty: false,
                });
            }
        }
        None
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:153 RemoteLeaseExtension.isRemoteMetadataWithTtl
fn is_remote_metadata_with_ttl(metadata: &FileArtifactValue) -> bool {
    metadata.is_remote() && metadata.get_expiration_time().is_some()
}

// Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:157 RemoteLeaseExtension.collectRemoteFiles
fn collect_remote_files(
    file_values: &IndexMap<Artifact, FileArtifactValue>,
    tree_artifact_values: &IndexMap<Artifact, Arc<TreeArtifactValue>>,
) -> Vec<RemoteFile> {
    let mut result = Vec::new();
    for (artifact, metadata) in file_values {
        if is_remote_metadata_with_ttl(metadata) {
            // The clone shares the expiration time with the Skyframe value.
            result.push((artifact.clone(), metadata.clone()));
        }
    }

    for tree_metadata in tree_artifact_values.values() {
        for (child, metadata) in &tree_metadata.child_values {
            if is_remote_metadata_with_ttl(metadata) {
                let child = as_blaze_artifact(child)
                    .expect("ClassCastException: a tree child is a TreeFileArtifact")
                    .clone();
                result.push((child, metadata.clone()));
            }
        }
    }

    result
}

// Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:226 RemoteLeaseExtension.buildDigest
fn build_digest(metadata: &FileArtifactValue) -> Digest {
    DigestUtil::build_digest_from_bytes(
        metadata.get_digest().unwrap_or_default(),
        metadata.get_size(),
    )
}

/// `Instant.toEpochMilli()`.
fn epoch_millis(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// The action cache entry of an action, with whether the lease extension changed it.
// Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:252 RemoteLeaseExtension.ActionCacheToken
struct ActionCacheToken {
    key: String,
    entry: ActionCacheEntry,
    dirty: bool,
}

impl ActionCacheToken {
    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:262 RemoteLeaseExtension.ActionCacheToken.extendOutputFile
    fn extend_output_file(&mut self, artifact: &Artifact, expiration_millis: i64) {
        if let Some(metadata) = self.entry.get_output_file(artifact) {
            metadata.set_expiration_time(Some(expiration_millis));
            self.dirty = true;
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteLeaseExtension.java:270 RemoteLeaseExtension.ActionCacheToken.extendOutputTreeFile
    fn extend_output_tree_file(&mut self, tree_file: &Artifact, expiration_millis: i64) {
        let parent = tree_file
            .get_parent()
            .expect("a TreeFileArtifact has a parent");
        if let Some(tree_metadata) = self.entry.get_output_tree(parent) {
            let relative_path = tree_file
                .get_tree_relative_path_string()
                .expect("a TreeFileArtifact has a tree-relative path");
            if let Some(metadata) = tree_metadata.child_values.get(&relative_path) {
                metadata.set_expiration_time(Some(expiration_millis));
                self.dirty = true;
            }
        }
    }
}

/// A task of the [`ScheduledExecutor`].
type Task = Box<dyn FnOnce() + Send>;

/// `Executors.newSingleThreadScheduledExecutor(threadFactory)`: one thread, started on the first
/// `schedule`, that runs the tasks when they are due (in due order; tasks due at the same time in
/// the order they were scheduled), and `shutdownNow`.
struct ScheduledExecutor {
    name_format: &'static str,
    state: Arc<(Mutex<SchedulerState>, Condvar)>,
    thread: Mutex<Option<(JoinHandle<()>, ThreadInterrupt)>>,
}

struct SchedulerState {
    /// The pending tasks by due time, then by sequence.
    queue: BTreeMap<(Instant, u64), Task>,
    next_sequence: u64,
    shutdown: bool,
}

impl ScheduledExecutor {
    fn new(name_format: &'static str) -> ScheduledExecutor {
        ScheduledExecutor {
            name_format,
            state: Arc::new((
                Mutex::new(SchedulerState {
                    queue: BTreeMap::new(),
                    next_sequence: 0,
                    shutdown: false,
                }),
                Condvar::new(),
            )),
            thread: Mutex::new(None),
        }
    }

    /// `schedule(task, delay, MILLISECONDS)`; a task scheduled after `shutdownNow` is rejected
    /// (dropped: Java's `RejectedExecutionException` reaches no one, the result is unused).
    fn schedule(&self, delay: Duration, task: Task) {
        {
            let (lock, condvar) = &*self.state;
            let mut state = lock.lock();
            if state.shutdown {
                return;
            }
            let sequence = state.next_sequence;
            state.next_sequence += 1;
            state.queue.insert((Instant::now() + delay, sequence), task);
            condvar.notify_all();
        }
        let mut thread = self.thread.lock();
        if thread.is_none() {
            let state = self.state.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            // ThreadFactoryBuilder().setNameFormat("lease-extension-%d"): the first thread.
            let name = self.name_format.replace("%d", "0");
            let handle = std::thread::Builder::new()
                .name(name)
                .spawn(move || {
                    let _ = sender.send(ThreadInterrupt::current());
                    run_scheduled(&state);
                })
                .expect("cannot start the lease extension thread");
            let interrupt = receiver
                .recv()
                .expect("the lease extension thread reports its interrupt status");
            *thread = Some((handle, interrupt));
        }
    }

    /// `ExecutorUtil.uninterruptibleShutdownNow`: drops the pending tasks, interrupts the
    /// running one and waits for the thread.
    fn shutdown_now(&self) {
        {
            let (lock, condvar) = &*self.state;
            let mut state = lock.lock();
            state.shutdown = true;
            state.queue.clear();
            condvar.notify_all();
        }
        if let Some((handle, interrupt)) = self.thread.lock().take() {
            interrupt.interrupt();
            let _ = handle.join();
        }
    }
}

impl Drop for ScheduledExecutor {
    /// The thread ends with its owner (not waited for: the owner may be dropped on it).
    fn drop(&mut self) {
        let (lock, condvar) = &*self.state;
        let mut state = lock.lock();
        state.shutdown = true;
        state.queue.clear();
        condvar.notify_all();
    }
}

fn run_scheduled(state: &(Mutex<SchedulerState>, Condvar)) {
    let (lock, condvar) = state;
    loop {
        let task = {
            let mut state = lock.lock();
            loop {
                if state.shutdown {
                    return;
                }
                let now = Instant::now();
                let due = state.queue.keys().next().copied();
                match due {
                    Some(key) if key.0 <= now => break state.queue.remove(&key),
                    Some(key) => {
                        condvar.wait_until(&mut state, key.0);
                    }
                    None => condvar.wait(&mut state),
                }
            }
        };
        if let Some(task) = task {
            task();
        }
    }
}

#[cfg(test)]
mod scheduled_executor_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runs_tasks_when_due_in_order() {
        let executor = ScheduledExecutor::new("test-%d");
        let order = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = std::sync::mpsc::channel();
        for (delay, n) in [(30u64, 3), (0, 1), (10, 2)] {
            let order = order.clone();
            let sender = sender.clone();
            executor.schedule(
                Duration::from_millis(delay),
                Box::new(move || {
                    order.lock().push(n);
                    let _ = sender.send(());
                }),
            );
        }
        for _ in 0..3 {
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert_eq!(*order.lock(), vec![1, 2, 3]);
        executor.shutdown_now();
    }

    #[test]
    fn shutdown_now_drops_pending_tasks_and_rejects_new_ones() {
        let executor = ScheduledExecutor::new("test-%d");
        let ran = Arc::new(AtomicUsize::new(0));
        let r = ran.clone();
        executor.schedule(
            Duration::from_secs(60),
            Box::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        );
        executor.shutdown_now();
        let r = ran.clone();
        executor.schedule(
            Duration::ZERO,
            Box::new(move || {
                r.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn shutdown_now_interrupts_the_running_task() {
        let executor = ScheduledExecutor::new("test-%d");
        let (started, wait_started) = std::sync::mpsc::channel();
        let (finished, wait_finished) = std::sync::mpsc::channel();
        executor.schedule(
            Duration::ZERO,
            Box::new(move || {
                let _ = started.send(());
                // getFromFuture of a call that does not end: the interrupt ends it.
                let result = get_from_future(async {
                    futures::future::pending::<()>().await;
                    Ok(())
                });
                let _ = finished.send(result.is_err_and(|e| errors::is_interrupted(&e)));
            }),
        );
        wait_started.recv_timeout(Duration::from_secs(5)).unwrap();
        executor.shutdown_now();
        assert!(wait_finished.recv_timeout(Duration::from_secs(5)).unwrap());
    }
}
