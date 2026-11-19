//! Port of `lib/remote/util/AsyncTaskCache.java`.
//!
//! PORT: the Java class is built on RxJava `Single`s with reference-counted subscribers. Here
//! the first caller of [`AsyncTaskCache::execute`] for a key drives the task itself and the
//! other callers wait for its outcome. Kept from Java: at most one execution of a key at a time,
//! finished keys are not executed again unless `force`, an outcome is delivered to every waiter
//! and an error is forgotten, `shutdownNow` cancels every running execution (its waiters get a
//! `CancellationException`), and the cache only terminates when an execution ends, `shutdown`
//! or `shutdownNow` finds nothing in progress (`maybeNotifyTermination`).
//!
//! Not kept (documented deviation, review 2026-11-19 R11): the tasks are futures that borrow
//! from their callers, so an execution cannot outlive the caller that drives it. When that
//! caller goes away while others still wait (Java: one of several subscribers disposes, and the
//! execution continues for the others), the execution is cancelled and a waiting caller starts
//! its own copy of the task. Keeping one execution for every subscriber needs `'static` tasks
//! (owned inputs and a detached context), which the callers do not provide yet.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::io;
use std::sync::Arc;

use futures::future::{AbortHandle, Abortable, Aborted};
use parking_lot::Mutex;
use tokio::sync::watch;

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

type Outcome<V> = Option<Result<V, Arc<io::Error>>>;

struct Execution<V> {
    /// Unique per execution, so that a driver only removes its own entry.
    id: u64,
    rx: watch::Receiver<Outcome<V>>,
    /// Delivers the outcome to the waiters (also used by `shutdownNow`'s `cancel()`).
    tx: Arc<watch::Sender<Outcome<V>>>,
    /// Stops the driver's task (`upstreamDisposable.dispose()`).
    abort: AbortHandle,
    subscribers: Arc<()>,
}

struct Inner<K, V> {
    state: u8,
    next_id: u64,
    finished: HashMap<K, V>,
    in_progress: HashMap<K, Execution<V>>,
}

/// A cache which de-duplicates the executions and stores the results of asynchronous tasks.
/// Each task is identified by a key of type `K` and has a result of type `V`.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:64 AsyncTaskCache
pub struct AsyncTaskCache<K, V> {
    inner: Mutex<Inner<K, V>>,
    /// Notified whenever an execution ends (for `awaitInProgressTasks`).
    idle: tokio::sync::Notify,
}

/// Removes the in-progress entry when the driving future is dropped before it finished
/// (`Execution.remove` of the last observer: no termination notification).
struct DriverGuard<'a, K: Eq + Hash + Clone, V: Clone> {
    cache: &'a AsyncTaskCache<K, V>,
    key: K,
    id: u64,
    done: bool,
}

impl<K: Eq + Hash + Clone, V: Clone> Drop for DriverGuard<'_, K, V> {
    fn drop(&mut self) {
        if !self.done {
            let mut inner = self.cache.inner.lock();
            if inner
                .in_progress
                .get(&self.key)
                .is_some_and(|e| e.id == self.id)
            {
                inner.in_progress.remove(&self.key);
            }
            drop(inner);
            self.cache.idle.notify_waiters();
        }
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Default for AsyncTaskCache<K, V> {
    fn default() -> Self {
        AsyncTaskCache::create()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> AsyncTaskCache<K, V> {
    /// `AsyncTaskCache.create()`.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:85 AsyncTaskCache.create
    pub fn create() -> AsyncTaskCache<K, V> {
        AsyncTaskCache {
            inner: Mutex::new(Inner {
                state: STATE_ACTIVE,
                next_id: 0,
                finished: HashMap::new(),
                in_progress: HashMap::new(),
            }),
            idle: tokio::sync::Notify::new(),
        }
    }

    /// Returns a set of keys for tasks which is finished.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:90 AsyncTaskCache.getFinishedTasks
    pub fn get_finished_tasks(&self) -> Vec<K> {
        self.inner.lock().finished.keys().cloned().collect()
    }

    /// Removes a finished task from the cache, so that it is executed again when requested.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:99 AsyncTaskCache.invalidate
    pub fn invalidate(&self, key: &K) {
        self.inner.lock().finished.remove(key);
    }

    /// Records a task as finished with the given value.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:108 AsyncTaskCache.put
    pub fn put(&self, key: K, value: V) {
        self.inner.lock().finished.insert(key, value);
    }

    /// Returns a set of keys for tasks which is in progress.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:113 AsyncTaskCache.getInProgressTasks
    pub fn get_in_progress_tasks(&self) -> Vec<K> {
        self.inner.lock().in_progress.keys().cloned().collect()
    }

    /// Returns count of subscribers for a task.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:131 AsyncTaskCache.getSubscriberCount
    pub fn get_subscriber_count(&self, key: &K) -> usize {
        self.inner
            .lock()
            .in_progress
            .get(key)
            // The map itself holds one reference.
            .map_or(0, |e| Arc::strong_count(&e.subscribers) - 1)
    }

    /// Executes a task if it hasn't been executed.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:126 AsyncTaskCache.executeIfNot
    pub async fn execute_if_not<F>(&self, key: K, task: F) -> io::Result<V>
    where
        F: Future<Output = io::Result<V>>,
    {
        self.execute(key, task, false).await
    }

    /// Executes a task. If the task has already finished, this execution of the task is ignored
    /// unless `force` is true. If the task is in progress this execution of the task is always
    /// ignored.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:307 AsyncTaskCache.execute
    pub async fn execute<F>(&self, key: K, task: F, force: bool) -> io::Result<V>
    where
        F: Future<Output = io::Result<V>>,
    {
        enum Decision<V> {
            Finished(V),
            Wait(watch::Receiver<Outcome<V>>, Arc<()>),
            Drive(
                Arc<watch::Sender<Outcome<V>>>,
                futures::future::AbortRegistration,
                u64,
                Arc<()>,
            ),
        }

        let mut force = force;
        let mut task = Some(task);
        loop {
            // Either wait for a running execution or become the driver.
            let decision = {
                let mut inner = self.inner.lock();
                if inner.state != STATE_ACTIVE {
                    return Err(cancellation_exception("already shutdown"));
                }
                let cached = if force {
                    inner.finished.remove(&key);
                    None
                } else {
                    inner.finished.get(&key).cloned()
                };
                match cached {
                    Some(cached) => Decision::Finished(cached),
                    None => match inner.in_progress.get(&key) {
                        Some(execution) => {
                            Decision::Wait(execution.rx.clone(), execution.subscribers.clone())
                        }
                        None => {
                            let (tx, rx) = watch::channel(None);
                            let tx = Arc::new(tx);
                            let (abort, registration) = AbortHandle::new_pair();
                            let id = inner.next_id;
                            inner.next_id += 1;
                            let subscribers = Arc::new(());
                            inner.in_progress.insert(
                                key.clone(),
                                Execution {
                                    id,
                                    rx,
                                    tx: tx.clone(),
                                    abort,
                                    subscribers: subscribers.clone(),
                                },
                            );
                            Decision::Drive(tx, registration, id, subscribers)
                        }
                    },
                }
            };
            match decision {
                Decision::Finished(cached) => return Ok(cached),
                Decision::Drive(tx, registration, id, _subscription) => {
                    let mut guard = DriverGuard {
                        cache: self,
                        key: key.clone(),
                        id,
                        done: false,
                    };
                    let task = task.take().expect("a call drives at most once");
                    let result = match Abortable::new(task, registration).await {
                        Ok(result) => result,
                        Err(Aborted) => {
                            // `cancel()` (shutdownNow) removed the execution and delivered the
                            // CancellationException to the waiters; the task was dropped.
                            guard.done = true;
                            return Err(cancellation_exception("cancelled"));
                        }
                    };
                    let outcome = result.map_err(Arc::new);
                    {
                        let mut inner = self.inner.lock();
                        // `if (!terminated)`: only the execution that is still registered.
                        if inner.in_progress.get(&key).is_some_and(|e| e.id == id) {
                            inner.in_progress.remove(&key);
                            if let Ok(value) = &outcome {
                                inner.finished.insert(key.clone(), value.clone());
                            }
                        }
                        let _ = tx.send(Some(outcome.clone()));
                        Self::maybe_notify_termination(&mut inner);
                    }
                    guard.done = true;
                    self.idle.notify_waiters();
                    return outcome.map_err(|e| shared(&e));
                }
                Decision::Wait(mut rx, _subscription) => {
                    // A running execution always satisfies this call (Java ignores `force`
                    // then).
                    force = false;
                    loop {
                        let current = rx.borrow().clone();
                        if let Some(outcome) = current {
                            return outcome.map_err(|e| shared(&e));
                        }
                        if rx.changed().await.is_err() {
                            // The driver went away without finishing: try again (and maybe
                            // drive).
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Initiates an orderly shutdown in which preexisting tasks will continue but new tasks will
    /// be immediately cancelled with `CancellationException`.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:373 AsyncTaskCache.shutdown
    pub fn shutdown(&self) {
        let mut inner = self.inner.lock();
        if inner.state == STATE_ACTIVE {
            inner.state = STATE_SHUTDOWN;
            Self::maybe_notify_termination(&mut inner);
        }
        drop(inner);
        self.idle.notify_waiters();
    }

    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:492 AsyncTaskCache.maybeNotifyTermination
    fn maybe_notify_termination(inner: &mut Inner<K, V>) {
        if inner.state == STATE_SHUTDOWN && inner.in_progress.is_empty() {
            inner.state = STATE_TERMINATED;
        }
    }

    /// Waits for the in-progress tasks to finish.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:386 AsyncTaskCache.awaitInProgressTasks
    pub async fn await_in_progress_tasks(&self) {
        loop {
            let notified = self.idle.notified();
            if self.inner.lock().in_progress.is_empty() {
                return;
            }
            notified.await;
        }
    }

    /// Waits for the channel to become terminated.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:416 AsyncTaskCache.awaitTermination
    pub async fn await_termination(&self) {
        loop {
            let notified = self.idle.notified();
            {
                let mut inner = self.inner.lock();
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

        let mut inner = self.inner.lock();
        if inner.state == STATE_SHUTDOWN {
            // Java: AsyncTaskCache.Execution.cancel: dispose the upstream, then onError(new
            // CancellationException("cancelled")).
            let executions: Vec<K> = inner.in_progress.keys().cloned().collect();
            for key in executions {
                let execution = inner.in_progress.remove(&key).expect("listed above");
                execution.abort.abort();
                let _ = execution.tx.send(Some(Err(Arc::new(cancellation_exception(
                    "cancelled",
                )))));
                Self::maybe_notify_termination(&mut inner);
            }
        }
        drop(inner);
        self.idle.notify_waiters();
    }

    /// Returns whether the cache is in shutdown state.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:474 AsyncTaskCache.isShutdown
    pub fn is_shutdown(&self) -> bool {
        let state = self.inner.lock().state;
        state == STATE_SHUTDOWN || state == STATE_TERMINATED
    }

    /// Returns whether the cache is in terminated state.
    // Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:484 AsyncTaskCache.isTerminated
    pub fn is_terminated(&self) -> bool {
        self.inner.lock().state == STATE_TERMINATED
    }
}

/// An `AsyncTaskCache` without result.
// Java: com/google/devtools/build/lib/remote/util/AsyncTaskCache.java:503 AsyncTaskCache.NoResult
pub type AsyncTaskCacheNoResult<K> = AsyncTaskCache<K, ()>;
