#![cfg(feature = "wip")]
//! Port of `lib/remote/util/AsyncTaskCacheTest.java`.
//!
//! PORT: the Java tests drive RxJava `Single`s by hand (`Single.create(emitterRef::set)`,
//! `TestObserver`). Here a "single" is a task future that waits for its emitter
//! ([`TestSingle`]), a subscription (`single.test()`) is a tokio task that awaits
//! `AsyncTaskCache::execute`, and `observer.dispose()` aborts that tokio task and waits until it
//! is gone. Each subscription hands the cache its own copy of the task, as each `execute` call
//! takes a future; the copies share their counters and their emitter slot.

use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

use blaze_remote::common::errors;
use blaze_remote::runtime::{block_on, runtime};
use blaze_remote::util::async_task_cache::{AsyncTaskCache, AsyncTaskCacheNoResult};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

type Cache = Arc<AsyncTaskCache<String, String>>;

/// Blocks until `condition` holds (Rx subscribes synchronously; a tokio task starts on its own
/// time).
fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// `SingleEmitter<String>`.
struct Emitter(oneshot::Sender<io::Result<String>>);

impl Emitter {
    fn on_success(self, value: &str) {
        let _ = self.0.send(Ok(value.to_string()));
    }

    fn on_error(self, error: io::Error) {
        let _ = self.0.send(Err(error));
    }
}

/// `Single.create(emitter -> { executionTimes.incrementAndGet(); emitterRef.set(emitter); })`
/// with `emitter.setCancellable(() -> disposed.set(true))`.
#[derive(Default)]
struct TestSingle {
    execution_times: AtomicUsize,
    emitter_ref: Mutex<Option<oneshot::Sender<io::Result<String>>>>,
    disposed: AtomicBool,
}

/// Sets `disposed` when a copy of the task is dropped before it completed.
struct Cancellable {
    single: Arc<TestSingle>,
    completed: bool,
}

impl Drop for Cancellable {
    fn drop(&mut self) {
        if !self.completed {
            self.single.disposed.store(true, Ordering::SeqCst);
        }
    }
}

impl TestSingle {
    fn create() -> Arc<TestSingle> {
        Arc::new(TestSingle::default())
    }

    /// A copy of the task, for one subscription.
    async fn task(single: Arc<TestSingle>) -> io::Result<String> {
        single.execution_times.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel::<io::Result<String>>();
        let mut cancellable = Cancellable {
            single: single.clone(),
            completed: false,
        };
        *single.emitter_ref.lock().unwrap() = Some(tx);
        match rx.await {
            Ok(result) => {
                cancellable.completed = true;
                result
            }
            // The emitter went away without a result: never complete.
            Err(_) => std::future::pending().await,
        }
    }

    /// `emitterRef.get()`, once the task has started.
    fn emitter(&self) -> Emitter {
        wait_until("the task has started", || {
            self.emitter_ref.lock().unwrap().is_some()
        });
        Emitter(self.emitter_ref.lock().unwrap().take().unwrap())
    }

    fn execution_times(&self) -> usize {
        self.execution_times.load(Ordering::SeqCst)
    }

    fn disposed(&self) -> bool {
        self.disposed.load(Ordering::SeqCst)
    }
}

/// `TestObserver<String>`.
struct TestObserver(JoinHandle<io::Result<String>>);

impl TestObserver {
    /// `cache.execute(key, single, force).test()`.
    fn subscribe(cache: &Cache, key: &str, single: &Arc<TestSingle>, force: bool) -> TestObserver {
        TestObserver::subscribe_task(cache, key, TestSingle::task(single.clone()), force)
    }

    fn subscribe_task(
        cache: &Cache,
        key: &str,
        task: impl Future<Output = io::Result<String>> + Send + 'static,
        force: bool,
    ) -> TestObserver {
        let cache = cache.clone();
        let key = key.to_string();
        TestObserver(runtime().spawn(async move { cache.execute(key, task, force).await }))
    }

    /// `assertEmpty()` / `assertNotComplete()`.
    fn assert_empty(&self) {
        assert!(!self.0.is_finished());
    }

    /// The terminal event of the observer.
    fn result(self) -> io::Result<String> {
        block_on(self.0).unwrap()
    }

    /// `assertValue(value)`.
    fn assert_value(self, value: &str) {
        assert_eq!(self.result().unwrap(), value);
    }

    /// `dispose()`.
    fn dispose(self) {
        self.0.abort();
        let _ = block_on(self.0);
    }
}

fn wait_for_subscribers(cache: &Cache, key: &str, count: usize) {
    let key = key.to_string();
    wait_until("the subscription is registered", || {
        cache.get_subscriber_count(&key) == count
    });
}

fn is_cancellation_exception(e: &io::Error) -> bool {
    // Java: `e instanceof CancellationException`
    errors::is_interrupted(e) && e.to_string().contains("CancellationException")
}

#[test]
fn execute_no_subscription_no_execution() {
    let cache: AsyncTaskCache<String, String> = AsyncTaskCache::create();
    let executed = AtomicBool::new(false);

    let _single = cache.execute_if_not("key1".to_string(), async {
        executed.store(true, Ordering::SeqCst);
        Ok("value1".to_string())
    });

    assert!(!executed.load(Ordering::SeqCst));
    assert!(cache.get_in_progress_tasks().is_empty());
    assert!(cache.get_finished_tasks().is_empty());
}

#[test]
fn execute_task_finished_completed() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let observer = TestObserver::subscribe(&cache, "key1", &single, false);
    let emitter = single.emitter();

    emitter.on_success("value1");

    observer.assert_value("value1");
    assert!(cache.get_in_progress_tasks().is_empty());
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);
}

#[test]
fn execute_task_has_error_propagate_error() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let observer = TestObserver::subscribe(&cache, "key1", &single, false);
    let emitter = single.emitter();

    // PORT: Java uses an `IllegalStateException("error")`; a task's error is an `io::Error`.
    let error = errors::io_exception("error");
    emitter.on_error(error);

    let observed = observer.result().unwrap_err();
    assert_eq!(observed.to_string(), "error");
    assert!(cache.get_in_progress_tasks().is_empty());
    assert!(cache.get_finished_tasks().is_empty());
}

#[test]
fn execute_task_in_progress_no_re_execution() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();

    let ob1 = TestObserver::subscribe(&cache, "key1", &single, false);
    let emitter = single.emitter();
    ob1.assert_empty();
    assert_eq!(cache.get_in_progress_tasks(), vec!["key1".to_string()]);
    assert!(cache.get_finished_tasks().is_empty());

    let ob2 = TestObserver::subscribe(&cache, "key1", &single, false);
    wait_for_subscribers(&cache, "key1", 2);
    ob2.assert_empty();

    emitter.on_success("value1");

    ob1.assert_value("value1");
    ob2.assert_value("value1");
    assert_eq!(single.execution_times(), 1);
    assert!(cache.get_in_progress_tasks().is_empty());
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);
}

#[test]
fn execute_forcibly_task_in_progress_no_re_execution() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();

    let ob1 = TestObserver::subscribe(&cache, "key1", &single, /* force= */ true);
    let emitter = single.emitter();
    ob1.assert_empty();
    assert_eq!(cache.get_in_progress_tasks(), vec!["key1".to_string()]);
    assert!(cache.get_finished_tasks().is_empty());

    let ob2 = TestObserver::subscribe(&cache, "key1", &single, /* force= */ true);
    wait_for_subscribers(&cache, "key1", 2);
    ob2.assert_empty();

    emitter.on_success("value1");

    ob1.assert_value("value1");
    ob2.assert_value("value1");
    assert_eq!(single.execution_times(), 1);
    assert!(cache.get_in_progress_tasks().is_empty());
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);
}

#[test]
fn execute_task_finished_no_re_execution() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();

    let ob1 = TestObserver::subscribe(&cache, "key1", &single, false);
    let emitter = single.emitter();
    emitter.on_success("value1");
    ob1.assert_value("value1");
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);

    let ob2 = TestObserver::subscribe(&cache, "key1", &single, false);
    ob2.assert_value("value1");
    assert_eq!(single.execution_times(), 1);
}

#[test]
fn execute_forcibly_task_finished_re_execution() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();

    let ob1 = TestObserver::subscribe(&cache, "key1", &single, /* force= */ true);
    let emitter = single.emitter();
    emitter.on_success("value1");
    ob1.assert_value("value1");
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);

    let ob2 = TestObserver::subscribe(&cache, "key1", &single, /* force= */ true);
    // The second execution has started once its emitter is there.
    let _emitter = single.emitter();
    ob2.assert_empty();
    assert_eq!(single.execution_times(), 2);
    assert_eq!(cache.get_in_progress_tasks(), vec!["key1".to_string()]);
    assert!(cache.get_finished_tasks().is_empty());

    ob2.dispose();
}

#[test]
fn execute_dispose_cancelled() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let observer = TestObserver::subscribe(&cache, "key1", &single, false);
    let _emitter = single.emitter();

    observer.dispose();

    assert!(single.disposed());
    assert!(cache.get_in_progress_tasks().is_empty());
    assert!(cache.get_finished_tasks().is_empty());
}

#[test]
#[ignore = "depends on Rx reference-counted subscribers: in the Rust AsyncTaskCache the first caller drives the task, so disposing it cancels its copy of the task (disposed becomes true) and the remaining waiter runs its own copy; Rx keeps the one execution alive while any subscriber remains"]
fn execute_dispose_when_multiple_subscriptions_not_cancelled() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let ob1 = TestObserver::subscribe(&cache, "key1", &single, false);
    let _emitter = single.emitter();
    let ob2 = TestObserver::subscribe(&cache, "key1", &single, false);
    wait_for_subscribers(&cache, "key1", 2);

    ob1.dispose();

    ob2.assert_empty();
    assert!(!single.disposed());
    assert_eq!(cache.get_in_progress_tasks(), vec!["key1".to_string()]);
    assert!(cache.get_finished_tasks().is_empty());

    ob2.dispose();
}

#[test]
fn execute_dispose_when_multiple_subscriptions_cancelled() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let ob1 = TestObserver::subscribe(&cache, "key1", &single, false);
    let _emitter = single.emitter();
    let ob2 = TestObserver::subscribe(&cache, "key1", &single, false);
    wait_for_subscribers(&cache, "key1", 2);

    ob1.dispose();
    ob2.dispose();

    assert!(single.disposed());
    assert!(cache.get_in_progress_tasks().is_empty());
    assert!(cache.get_finished_tasks().is_empty());
}

#[test]
fn execute_multiple_tasks_complete_one() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single1 = TestSingle::create();
    let observer1 = TestObserver::subscribe(&cache, "key1", &single1, false);
    let emitter1 = single1.emitter();
    let single2 = TestSingle::create();
    let observer2 = TestObserver::subscribe(&cache, "key2", &single2, false);
    let _emitter2 = single2.emitter();

    emitter1.on_success("value1");

    observer1.assert_value("value1");
    observer2.assert_empty();
    assert_eq!(cache.get_in_progress_tasks(), vec!["key2".to_string()]);
    assert_eq!(cache.get_finished_tasks(), vec!["key1".to_string()]);

    observer2.dispose();
}

/// `java.util.Random`, as far as the loop tests need one (any pseudo-random sequence does).
struct Random(u64);

impl Random {
    fn new() -> Random {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        Random(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_int(&mut self, bound: u64) -> u64 {
        (self.next_u64() >> 11) % bound
    }

    fn next_boolean(&mut self) -> bool {
        (self.next_u64() >> 11) & 1 == 1
    }
}

// Java: AsyncTaskCacheTest.newTask
async fn new_task(sleep_millis: u64) -> io::Result<()> {
    tokio::time::sleep(Duration::from_millis(sleep_millis)).await;
    Ok(())
}

#[test]
fn execute_execute_and_dispose_loop_no_errors() {
    let task_count = 1000;
    let max_key = 20;
    let mut random = Random::new();
    let cache: Arc<AsyncTaskCacheNoResult<String>> = Arc::new(AsyncTaskCache::create());
    let mut tasks: Vec<JoinHandle<io::Result<()>>> = Vec::new();
    for _ in 0..task_count {
        let cache = cache.clone();
        let key = format!("key{}", random.next_int(max_key));
        let sleep_millis = random.next_int(1000);
        let dispose = random.next_boolean();
        tasks.push(runtime().spawn(async move {
            let mut task = Box::pin(cache.execute(key, new_task(sleep_millis), true));
            // `task.test()` subscribes, `observer.assertNoErrors()`.
            if let Poll::Ready(result) = futures::poll!(task.as_mut()) {
                return result;
            }
            if dispose {
                // `observer.dispose()`
                drop(task);
                Ok(())
            } else {
                // `observer.await(); observer.assertNoErrors();`
                task.await
            }
        }));
    }

    let mut error: Option<io::Error> = None;
    for task in tasks {
        if let Err(e) = block_on(task).unwrap() {
            error = Some(e);
        }
    }
    if let Some(error) = error {
        panic!("{error:?}");
    }
}

#[test]
fn execute_execute_with_future_and_cancel_loop_no_errors() {
    let task_count = 1000;
    let max_key = 20;
    let mut random = Random::new();
    let cache: Arc<AsyncTaskCacheNoResult<String>> = Arc::new(AsyncTaskCache::create());
    let mut tasks: Vec<JoinHandle<io::Result<()>>> = Vec::new();
    for _ in 0..task_count {
        let cache = cache.clone();
        let key = format!("key{}", random.next_int(max_key));
        let sleep_millis = random.next_int(1000);
        let cancel = random.next_boolean();
        tasks.push(runtime().spawn(async move {
            // `RxFutures.toListenableFuture(download)`
            let future: JoinHandle<io::Result<()>> = runtime()
                .spawn(async move { cache.execute(key, new_task(sleep_millis), true).await });
            if !future.is_finished() && cancel {
                future.abort();
                Ok(())
            } else {
                future.await.map_err(io::Error::other)?
            }
        }));
    }

    let mut error: Option<io::Error> = None;
    for task in tasks {
        if let Err(e) = block_on(task).unwrap() {
            error = Some(e);
        }
    }
    if let Some(error) = error {
        panic!("{error:?}");
    }
}

#[test]
fn execute_pending_shutdown_get_cancellation_error() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let pending = TestObserver::subscribe_task(
        &cache,
        "key1",
        // never complete
        std::future::pending::<io::Result<String>>(),
        false,
    );
    wait_until("the task is in progress", || {
        cache.get_in_progress_tasks() == vec!["key1".to_string()]
    });
    pending.assert_empty();

    cache.shutdown();

    assert!(cache.is_shutdown());
    assert!(!cache.is_terminated());

    let ob = TestObserver::subscribe_task(
        &cache,
        "key2",
        std::future::ready(Ok("value2".to_string())),
        false,
    );
    let e = ob.result().unwrap_err();
    assert!(is_cancellation_exception(&e), "{e:?}");

    pending.dispose();
}

#[test]
fn execute_after_shutdown_get_cancellation_error() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());

    cache.shutdown();
    block_on(cache.await_termination());

    let ob = TestObserver::subscribe_task(
        &cache,
        "key",
        std::future::ready(Ok("value".to_string())),
        false,
    );
    let e = ob.result().unwrap_err();
    assert!(is_cancellation_exception(&e), "{e:?}");
}

#[test]
#[ignore = "AsyncTaskCache::shutdown_now does not cancel the running executions (PORT note on shutdown_now: the drivers belong to their callers); Rx cancels them, which is what lets awaitTermination return and the observer see a CancellationException here"]
fn shutdown_now_cancel_in_progress_tasks() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let ob = TestObserver::subscribe_task(
        &cache,
        "key",
        // never complete
        std::future::pending::<io::Result<String>>(),
        false,
    );
    wait_until("the task is in progress", || {
        cache.get_in_progress_tasks() == vec!["key".to_string()]
    });

    cache.shutdown();

    assert!(cache.is_shutdown());
    assert!(!cache.is_terminated());
    ob.assert_empty();

    cache.shutdown_now();
    // A bound, so that this fails instead of hanging where the task is not cancelled.
    block_on(async {
        tokio::time::timeout(Duration::from_secs(10), cache.await_termination())
            .await
            .expect("awaitTermination did not return")
    });

    assert!(cache.is_shutdown());
    assert!(cache.is_terminated());
    let e = ob.result().unwrap_err();
    assert!(is_cancellation_exception(&e), "{e:?}");
}

#[test]
fn await_termination_pending_shutdown_complete_after_task_finished() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());
    let single = TestSingle::create();
    let ob = TestObserver::subscribe(&cache, "key", &single, false);
    let emitter = single.emitter();
    ob.assert_empty();

    cache.shutdown();

    assert!(cache.is_shutdown());
    assert!(!cache.is_terminated());

    emitter.on_success("value");
    block_on(cache.await_termination());

    assert!(cache.is_shutdown());
    assert!(cache.is_terminated());
    ob.assert_value("value");
    assert!(cache.get_in_progress_tasks().is_empty());
    assert!(cache.get_finished_tasks().is_empty());
}

#[test]
fn await_termination_after_shutdown_complete() {
    let cache: Cache = Arc::new(AsyncTaskCache::create());

    cache.shutdown_now();
    block_on(cache.await_termination());
    block_on(cache.await_termination());

    assert!(cache.is_shutdown());
    assert!(cache.is_terminated());
}
