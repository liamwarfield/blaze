//! Port of `lib/remote/util/AsyncTaskCache.java`.
//!
//! PORT: the Java class is built on RxJava `Single`s with reference-counted subscribers. Here an
//! `Execution` is a [`futures::future::Shared`] future over the task; every call of
//! [`AsyncTaskCache::execute`] that joins the execution is a subscriber that holds a clone of it
//! and drives it. As in Java:
//!
//! - at most one execution of a key runs at a time; finished keys are not executed again unless
//!   `force`; an outcome is delivered to every subscriber and an error is forgotten;
//! - the execution keeps running while any subscriber remains: a subscriber that goes away
//!   (Java: `ExecutionDisposable.dispose()`; here: its `execute` future is dropped) neither
//!   restarts nor cancels it, and the last one to go away cancels it (`upstreamDisposable.dispose()`;
//!   here: the task future is dropped) and removes it from `inProgress`;
//! - `shutdownNow` cancels every running execution (its subscribers get a
//!   `CancellationException`), and the cache terminates when an execution ends, `shutdown` or
//!   `shutdownNow` finds nothing in progress (`maybeNotifyTermination`).
//!
//! The cache keeps only a weak handle of an execution, so the tasks must not borrow from the call
//! that started them: they are `'static` (owned inputs, a detached context, `Arc` clients).

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::io;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::{AbortHandle, Abortable, Aborted, BoxFuture, Shared, WeakShared};
use parking_lot::Mutex;

use crate::common::errors::shared;

/// `new CancellationException(message)`.
///
/// PORT: carried as an error of kind `Interrupted` whose message names the exception.
fn cancellation_exception(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        format!("CancellationException: {message}"),
    )
}

const STATE_ACTIVE: u8 = 0;
const STATE_SHUTDOWN: u8 = 1;
const STATE_TERMINATED: u8 = 2;

/// The outcome every subscriber of an execution gets (Java hands every observer the same value
/// or exception object).
type Outcome<V> = Result<V, Arc<io::Error>>;

/// The shared execution of a task (`AsyncTaskCache.Execution`).
type ExecutionFuture<V> = Shared<BoxFuture<'static, Outcome<V>>>;

struct Execution<V> {
    /// Unique per execution, so that an execution only removes its own entry.
    id: u64,
    /// The cache does not keep the execution running; its subscribers do.
    execution: WeakShared<BoxFuture<'static, Outcome<V>>>,
    /// `upstreamDisposable` (for `cancel()`).
    abort: AbortHandle,
    /// `observers.size()`.
    subscribers: usize,
}

struct Inner<K, V> {
    state: u8,
    next_id: u64,
    finished: HashMap<K, V>,
    in_progress: HashMap<K, Execution<V>>,
}

/// The state shared by the cache and its executions.
struct State<K, V> {
    inner: Mutex<Inner<K, V>>,
    /// Notified whenever an execution ends or the state changes (for `awaitInProgressTasks` /
    /// `awaitTermination`).
    idle: tokio::sync::Notify,
}

/// A cache which de-duplicates the executions and stores the results of asynchronous tasks.
/// Each task is identified by a key of type `K` and has a result of type `V`.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:64 AsyncTaskCache
pub struct AsyncTaskCache<K, V> {
    state: Arc<State<K, V>>,
}

/// One subscriber of an execution (`ExecutionDisposable`): dropping it is `dispose()`, which
/// removes the observer, and the last observer of an execution that has not terminated removes
/// the execution from `inProgress` and disposes the upstream.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:262 AsyncTaskCache.ExecutionDisposable
struct Subscription<K: Eq + Hash, V> {
    state: Arc<State<K, V>>,
    key: K,
    id: u64,
    execution: Option<ExecutionFuture<V>>,
}

impl<K: Eq + Hash, V> Drop for Subscription<K, V> {
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:234 AsyncTaskCache.Execution.remove
    fn drop(&mut self) {
        let removed = {
            let mut inner = self.state.inner.lock();
            match inner.in_progress.get_mut(&self.key) {
                // `!terminated`: the execution is still registered.
                Some(execution) if execution.id == self.id => {
                    execution.subscribers -= 1;
                    if execution.subscribers == 0 {
                        inner.in_progress.remove(&self.key)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        // Dropping the last strong handle drops the task (`upstreamDisposable.dispose()`); done
        // outside the lock, since the task may own subscriptions of its own.
        drop(self.execution.take());
        if let Some(removed) = removed {
            removed.abort.abort();
            drop(removed);
            self.state.idle.notify_waiters();
        }
    }
}

impl<K, V> Default for AsyncTaskCache<K, V>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        AsyncTaskCache::create()
    }
}

impl<K, V> AsyncTaskCache<K, V>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// `AsyncTaskCache.create()`.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:85 AsyncTaskCache.create
    pub fn create() -> AsyncTaskCache<K, V> {
        AsyncTaskCache {
            state: Arc::new(State {
                inner: Mutex::new(Inner {
                    state: STATE_ACTIVE,
                    next_id: 0,
                    finished: HashMap::new(),
                    in_progress: HashMap::new(),
                }),
                idle: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Returns a set of keys for tasks which is finished.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:90 AsyncTaskCache.getFinishedTasks
    pub fn get_finished_tasks(&self) -> Vec<K> {
        self.state.inner.lock().finished.keys().cloned().collect()
    }

    /// Removes a finished task from the cache, so that it is executed again when requested.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:99 AsyncTaskCache.invalidate
    pub fn invalidate(&self, key: &K) {
        self.state.inner.lock().finished.remove(key);
    }

    /// Records a task as finished with the given value.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:108 AsyncTaskCache.put
    pub fn put(&self, key: K, value: V) {
        self.state.inner.lock().finished.insert(key, value);
    }

    /// Returns a set of keys for tasks which is in progress.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:113 AsyncTaskCache.getInProgressTasks
    pub fn get_in_progress_tasks(&self) -> Vec<K> {
        self.state
            .inner
            .lock()
            .in_progress
            .keys()
            .cloned()
            .collect()
    }

    /// Returns count of subscribers for a task.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:131 AsyncTaskCache.getSubscriberCount
    pub fn get_subscriber_count(&self, key: &K) -> usize {
        self.state
            .inner
            .lock()
            .in_progress
            .get(key)
            .map_or(0, |e| e.subscribers)
    }

    /// Executes a task if it hasn't been executed.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:126 AsyncTaskCache.executeIfNot
    pub async fn execute_if_not<F>(&self, key: K, task: F) -> io::Result<V>
    where
        F: Future<Output = io::Result<V>> + Send + 'static,
    {
        self.execute(key, task, false).await
    }

    /// Executes a task. If the task has already finished, this execution of the task is ignored
    /// unless `force` is true. If the task is in progress this execution of the task is always
    /// ignored.
    ///
    /// If the cache is already shutdown, a `CancellationException` is returned.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:307 AsyncTaskCache.execute
    pub async fn execute<F>(&self, key: K, task: F, force: bool) -> io::Result<V>
    where
        F: Future<Output = io::Result<V>> + Send + 'static,
    {
        let subscription = {
            let mut inner = self.state.inner.lock();
            if inner.state != STATE_ACTIVE {
                return Err(cancellation_exception("already shutdown"));
            }

            if !force {
                if let Some(cached) = inner.finished.get(&key) {
                    return Ok(cached.clone());
                }
            } else {
                inner.finished.remove(&key);
            }

            // `inProgress.get(key)`; an execution whose last subscriber went away has been
            // removed by that subscriber, so a registered execution has a live handle.
            let joined = match inner.in_progress.get_mut(&key) {
                Some(execution) => {
                    let handle = execution
                        .execution
                        .upgrade()
                        .expect("a registered execution has a subscriber");
                    execution.subscribers += 1;
                    Some((execution.id, handle))
                }
                None => None,
            };
            match joined {
                Some((id, handle)) => Subscription {
                    state: self.state.clone(),
                    key: key.clone(),
                    id,
                    execution: Some(handle),
                },
                None => {
                    let id = inner.next_id;
                    inner.next_id += 1;
                    let (abort, registration) = AbortHandle::new_pair();
                    let execution: ExecutionFuture<V> = Self::new_execution(
                        self.state.clone(),
                        key.clone(),
                        id,
                        task,
                        registration,
                    );
                    inner.in_progress.insert(
                        key.clone(),
                        Execution {
                            id,
                            execution: execution
                                .downgrade()
                                .expect("a new execution has not completed"),
                            abort,
                            subscribers: 1,
                        },
                    );
                    Subscription {
                        state: self.state.clone(),
                        key: key.clone(),
                        id,
                        execution: Some(execution),
                    }
                }
            }
        };
        let execution = subscription
            .execution
            .clone()
            .expect("the subscription holds its execution");
        let outcome = execution.await;
        drop(subscription);
        outcome.map_err(|e| shared(&e))
    }

    /// The upstream task with the `Execution`'s `onSuccess` / `onError`: the first outcome
    /// removes the execution from `inProgress` (only while it is still registered, Java's
    /// `!terminated`), records a value in `finished` and checks for termination.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:197 AsyncTaskCache.Execution.onSuccess
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:217 AsyncTaskCache.Execution.onError
    fn new_execution<F>(
        state: Arc<State<K, V>>,
        key: K,
        id: u64,
        task: F,
        registration: futures::future::AbortRegistration,
    ) -> ExecutionFuture<V>
    where
        F: Future<Output = io::Result<V>> + Send + 'static,
    {
        async move {
            let outcome: Outcome<V> = match Abortable::new(task, registration).await {
                Ok(result) => result.map_err(Arc::new),
                // `cancel()` (shutdownNow) removed the execution and disposed the upstream:
                // `onError(new CancellationException("cancelled"))`.
                Err(Aborted) => Err(Arc::new(cancellation_exception("cancelled"))),
            };
            {
                let mut inner = state.inner.lock();
                if inner.in_progress.get(&key).is_some_and(|e| e.id == id) {
                    inner.in_progress.remove(&key);
                    if let Ok(value) = &outcome {
                        inner.finished.insert(key, value.clone());
                    }
                    Self::maybe_notify_termination(&mut inner);
                }
            }
            state.idle.notify_waiters();
            outcome
        }
        .boxed()
        .shared()
    }

    /// Initiates an orderly shutdown in which preexisting tasks will continue but new tasks will
    /// be immediately cancelled with `CancellationException`.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:373 AsyncTaskCache.shutdown
    pub fn shutdown(&self) {
        let mut inner = self.state.inner.lock();
        if inner.state == STATE_ACTIVE {
            inner.state = STATE_SHUTDOWN;
            Self::maybe_notify_termination(&mut inner);
        }
        drop(inner);
        self.state.idle.notify_waiters();
    }

    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:492 AsyncTaskCache.maybeNotifyTermination
    fn maybe_notify_termination(inner: &mut Inner<K, V>) {
        if inner.state == STATE_SHUTDOWN && inner.in_progress.is_empty() {
            inner.state = STATE_TERMINATED;
        }
    }

    /// Waits for the in-progress tasks to finish: the executions in progress when it is called
    /// (`ImmutableList.copyOf(inProgress.values())`); tasks submitted later are not waited for.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:386 AsyncTaskCache.awaitInProgressTasks
    pub async fn await_in_progress_tasks(&self) {
        let executions: Vec<(K, u64)> = {
            let inner = self.state.inner.lock();
            inner
                .in_progress
                .iter()
                .map(|(key, execution)| (key.clone(), execution.id))
                .collect()
        };
        // `Single.fromObservable(e.completion)` for each: an execution is complete once it is no
        // longer registered under its key.
        loop {
            let notified = self.state.idle.notified();
            {
                let inner = self.state.inner.lock();
                if executions.iter().all(|(key, id)| {
                    inner
                        .in_progress
                        .get(key)
                        .is_none_or(|execution| execution.id != *id)
                }) {
                    return;
                }
            }
            notified.await;
        }
    }

    /// Waits for the channel to become terminated.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:416 AsyncTaskCache.awaitTermination
    pub async fn await_termination(&self) {
        loop {
            let notified = self.state.idle.notified();
            {
                let mut inner = self.state.inner.lock();
                if inner.state == STATE_TERMINATED {
                    // Reduce retained size in case references to the cache are held after
                    // shutdown.
                    inner.in_progress = HashMap::new();
                    inner.finished.clear();
                    return;
                }
            }
            notified.await;
        }
    }

    /// Initiates a forceful shutdown in which preexisting and new tasks are cancelled with
    /// `CancellationException`. Although forceful, the shutdown process is still not
    /// instantaneous; [`AsyncTaskCache::is_terminated`] will likely return `false` immediately
    /// after this method returns.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:458 AsyncTaskCache.shutdownNow
    pub fn shutdown_now(&self) {
        self.shutdown();

        let mut inner = self.state.inner.lock();
        if inner.state == STATE_SHUTDOWN {
            // Java: AsyncTaskCache.Execution.cancel: dispose the upstream, then onError(new
            // CancellationException("cancelled")), which removes the execution and checks for
            // termination. The subscribers are woken by the abort and get the exception.
            let executions: Vec<K> = inner.in_progress.keys().cloned().collect();
            for key in executions {
                let execution = inner.in_progress.remove(&key).expect("listed above");
                execution.abort.abort();
                Self::maybe_notify_termination(&mut inner);
            }
        }
        drop(inner);
        self.state.idle.notify_waiters();
    }

    /// Returns whether the cache is in shutdown state.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:474 AsyncTaskCache.isShutdown
    pub fn is_shutdown(&self) -> bool {
        let state = self.state.inner.lock().state;
        state == STATE_SHUTDOWN || state == STATE_TERMINATED
    }

    /// Returns whether the cache is in terminated state.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:484 AsyncTaskCache.isTerminated
    pub fn is_terminated(&self) -> bool {
        self.state.inner.lock().state == STATE_TERMINATED
    }
}

/// An `AsyncTaskCache` without result.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:503 AsyncTaskCache.NoResult
pub type AsyncTaskCacheNoResult<K> = AsyncTaskCache<K, ()>;
