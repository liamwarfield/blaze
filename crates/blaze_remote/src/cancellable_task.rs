//! Port of `lib/concurrent/CancellableTask.java`.
//!
//! PORT: the class lives in `lib/concurrent` (`blaze_util::concurrent` marks it skipped); its one
//! user here is the output upload of `RemoteExecutionService`, which `RemoteRewoundActionSynchronizer`
//! cancels, and it interrupts threads through `blaze_shell::ThreadInterrupt`, which `blaze_util`
//! does not depend on. So it is in this crate.

use std::sync::Arc;
use std::thread::ThreadId;

use blaze_shell::{InterruptedException, ThreadInterrupt};
use parking_lot::{Condvar, Mutex};

// Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:43 CancellableTask.Phase
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    NotStarted,
    Running,
    CompletingAfterRunning,
    CompletingWithoutRunning,
    CompletedAfterRunning,
    CompletedWithoutRunning,
}

struct State {
    phase: Phase,
    /// `executingThread`: the thread running the body or the completion action, and its
    /// interrupt handle.
    executing_thread: Option<(ThreadId, ThreadInterrupt)>,
}

/// The body of a [`CancellableTask`] (`CancellableTask.Task`).
pub type Task<E> = Box<dyn FnOnce() -> Result<(), E> + Send>;

/// A one-shot task whose cancellation can wait until its execution has quiesced.
///
/// An optional completion action runs exactly once when the task's lifecycle ends, no matter how
/// it ends: on the task thread after the body has run (even if it threw), or on the thread of the
/// canceler that prevented the task from starting. Once the completion action begins,
/// cancellation does not interrupt it. Quiescence includes the completion action: a normal return
/// from [`CancellableTask::run_if_not_cancelled`], [`CancellableTask::await_completion`],
/// [`CancellableTask::await_completion_uninterruptibly`], or
/// [`CancellableTask::cancel_and_await`] guarantees that both the body and the completion action
/// have finished. [`CancellableTask::request_cancellation`] only initiates cancellation and does
/// not provide this guarantee.
///
/// PORT: an exception of the body is the `Err` of the body; a `RuntimeException` (of the body or
/// the completion action) is a panic, after which the terminal state is still published (Java's
/// `finally`). `IllegalStateException` is a panic with that name. A thread is interrupted through
/// its `ThreadInterrupt`.
// Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:41 CancellableTask
pub struct CancellableTask<E> {
    task: Mutex<Option<Task<E>>>,
    on_completion: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    state: Mutex<State>,
    /// `done`, a `CountDownLatch(1)`.
    done: Mutex<bool>,
    done_changed: Condvar,
}

/// Publishes the terminal state when dropped: the `finally` of `runCompletion`.
struct CompletionGuard<'a, E>(&'a CancellableTask<E>);

impl<E> Drop for CompletionGuard<'_, E> {
    fn drop(&mut self) {
        {
            let mut state = self.0.state.lock();
            state.executing_thread = None;
            state.phase = if state.phase == Phase::CompletingWithoutRunning {
                Phase::CompletedWithoutRunning
            } else {
                Phase::CompletedAfterRunning
            };
        }
        *self.0.done.lock() = true;
        self.0.done_changed.notify_all();
    }
}

/// The `finally` of `runIfNotCancelled` after the body: completing, then the completion action.
struct AfterBodyGuard<'a, E>(&'a CancellableTask<E>);

impl<E> Drop for AfterBodyGuard<'_, E> {
    fn drop(&mut self) {
        self.0.state.lock().phase = Phase::CompletingAfterRunning;
        self.0.run_completion();
    }
}

fn current_thread() -> (ThreadId, ThreadInterrupt) {
    (std::thread::current().id(), ThreadInterrupt::current())
}

impl<E> CancellableTask<E> {
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:69 CancellableTask.<init>
    pub fn new(task: Task<E>) -> Arc<CancellableTask<E>> {
        CancellableTask::with_completion(task, Box::new(|| {}))
    }

    /// Creates a task with a completion action.
    ///
    /// The completion action must not throw or reenter this task.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:80 CancellableTask.<init>
    pub fn with_completion(
        task: Task<E>,
        on_completion: Box<dyn FnOnce() + Send>,
    ) -> Arc<CancellableTask<E>> {
        Arc::new(CancellableTask {
            task: Mutex::new(Some(task)),
            on_completion: Mutex::new(Some(on_completion)),
            state: Mutex::new(State {
                phase: Phase::NotStarted,
                executing_thread: None,
            }),
            done: Mutex::new(false),
            done_changed: Condvar::new(),
        })
    }

    fn is_current_thread(state: &State) -> bool {
        state
            .executing_thread
            .as_ref()
            .is_some_and(|(id, _)| *id == std::thread::current().id())
    }

    /// Runs the task synchronously unless cancellation prevented it from starting.
    ///
    /// A normal return guarantees that the completion action has finished, including when
    /// cancellation prevented the task body from running. Returns whether the task ran.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:93 CancellableTask.runIfNotCancelled
    pub fn run_if_not_cancelled(&self) -> Result<bool, E> {
        let was_cancelled = {
            let mut state = self.state.lock();
            match state.phase {
                Phase::NotStarted => {
                    state.phase = Phase::Running;
                    state.executing_thread = Some(current_thread());
                    false
                }
                Phase::CompletingWithoutRunning | Phase::CompletedWithoutRunning => {
                    if Self::is_current_thread(&state) {
                        panic!(
                            "IllegalStateException: completion action must not attempt to \
                             execute its task, this would deadlock"
                        );
                    }
                    true
                }
                _ => panic!("IllegalStateException: task executed more than once"),
            }
        };
        if was_cancelled {
            self.await_done_uninterruptibly();
            return Ok(false);
        }
        let task = self
            .task
            .lock()
            .take()
            .unwrap_or_else(|| panic!("IllegalStateException: task executed more than once"));
        let _finally = AfterBodyGuard(self);
        task()?;
        Ok(true)
    }

    /// Requests cancellation without waiting for the task to quiesce.
    ///
    /// If the task has not started, this prevents it from starting and runs the completion
    /// action on its behalf before returning. Otherwise, it interrupts the thread executing the
    /// task body, which may still be executing when this method returns.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:138 CancellableTask.requestCancellation
    pub fn request_cancellation(&self) {
        let prevented_start = {
            let mut state = self.state.lock();
            match state.phase {
                Phase::NotStarted => {
                    state.phase = Phase::CompletingWithoutRunning;
                    state.executing_thread = Some(current_thread());
                    true
                }
                Phase::Running => {
                    if Self::is_current_thread(&state) {
                        panic!("IllegalStateException: task cannot cancel itself");
                    }
                    // Interrupt while holding the state lock so that the task cannot enter its
                    // completion action between publishing the executing thread and receiving the
                    // interrupt.
                    state
                        .executing_thread
                        .as_ref()
                        .unwrap_or_else(|| panic!("NullPointerException: executingThread"))
                        .1
                        .interrupt();
                    false
                }
                Phase::CompletingAfterRunning | Phase::CompletingWithoutRunning => {
                    if Self::is_current_thread(&state) {
                        panic!("IllegalStateException: task cannot cancel itself");
                    }
                    false
                }
                Phase::CompletedAfterRunning | Phase::CompletedWithoutRunning => false,
            }
        };
        if prevented_start {
            // The task will never run, so run the completion action on its behalf. Signal
            // completion only afterward so that concurrent awaiters only return once it has
            // finished.
            self.run_completion();
        }
    }

    /// Waits until the task body and completion action have finished without requesting
    /// cancellation.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:181 CancellableTask.awaitCompletion
    pub fn await_completion(self: &Arc<Self>) -> Result<(), InterruptedException>
    where
        E: 'static,
    {
        self.check_can_await();
        // done.await(): interruptible.
        let me = self.clone();
        let waiter = std::thread::current().id();
        let _registration =
            ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
                if std::thread::current().id() == waiter {
                    return;
                }
                let _done = me.done.lock();
                me.done_changed.notify_all();
            }));
        let mut done = self.done.lock();
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
            if *done {
                return Ok(());
            }
            self.done_changed.wait(&mut done);
        }
    }

    /// An uninterruptible variant of [`CancellableTask::await_completion`] that restores the
    /// interrupt bit before returning.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:193 CancellableTask.awaitCompletionUninterruptibly
    pub fn await_completion_uninterruptibly(&self) {
        self.check_can_await();
        self.await_done_uninterruptibly();
    }

    /// `Uninterruptibles.awaitUninterruptibly(done)`: the interrupt bit is left as it is.
    fn await_done_uninterruptibly(&self) {
        let mut done = self.done.lock();
        while !*done {
            self.done_changed.wait(&mut done);
        }
    }

    /// Cancels the task and waits until it no longer executes.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:210 CancellableTask.cancelAndAwait
    pub fn cancel_and_await(self: &Arc<Self>) -> Result<(), InterruptedException>
    where
        E: 'static,
    {
        self.request_cancellation();
        self.await_completion()
    }

    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:215 CancellableTask.checkCanAwait
    fn check_can_await(&self) {
        let state = self.state.lock();
        if Self::is_current_thread(&state) {
            panic!("IllegalStateException: task cannot await itself");
        }
    }

    /// Runs the completion action and publishes terminal state even if the action throws.
    // Java: com/google/devtools/build/lib/concurrent/CancellableTask.java:224 CancellableTask.runCompletion
    fn run_completion(&self) {
        let _finally = CompletionGuard(self);
        let on_completion = self.on_completion.lock().take();
        if let Some(on_completion) = on_completion {
            on_completion();
        }
    }
}

/// `CancellableTaskTest`.
///
/// PORT: `Semaphore.acquire()` / `tryAcquire` are [`TestSemaphore`], whose `acquire` is
/// interruptible through `ThreadInterrupt`; a `TestThread` is a `std::thread`.
// Java: src/test/java/com/google/devtools/build/lib/concurrent/CancellableTaskTest.java
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(60);
    const SHORT: Duration = Duration::from_millis(100);

    #[derive(Default)]
    struct TestSemaphore {
        permits: Mutex<usize>,
        changed: Condvar,
    }

    impl TestSemaphore {
        fn new() -> Arc<TestSemaphore> {
            Arc::new(TestSemaphore::default())
        }

        fn release(&self) {
            *self.permits.lock() += 1;
            self.changed.notify_all();
        }

        fn try_acquire(&self, timeout: Duration) -> bool {
            let deadline = std::time::Instant::now() + timeout;
            let mut permits = self.permits.lock();
            while *permits == 0 {
                if self.changed.wait_until(&mut permits, deadline).timed_out() {
                    return false;
                }
            }
            *permits -= 1;
            true
        }

        /// `acquire()`: interruptible.
        fn acquire(self: &Arc<Self>) -> Result<(), InterruptedException> {
            let me = self.clone();
            let waiter = std::thread::current().id();
            let _registration =
                ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
                    if std::thread::current().id() == waiter {
                        return;
                    }
                    let _p = me.permits.lock();
                    me.changed.notify_all();
                }));
            let mut permits = self.permits.lock();
            loop {
                if blaze_shell::thread_interrupt::interrupted() {
                    return Err(InterruptedException);
                }
                if *permits > 0 {
                    *permits -= 1;
                    return Ok(());
                }
                self.changed.wait(&mut permits);
            }
        }

        fn acquire_uninterruptibly(&self) {
            let mut permits = self.permits.lock();
            while *permits == 0 {
                self.changed.wait(&mut permits);
            }
            *permits -= 1;
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum CancelVariant {
        Interruptible,
        RequestThenAwait,
    }

    const VARIANTS: [CancelVariant; 2] = [
        CancelVariant::Interruptible,
        CancelVariant::RequestThenAwait,
    ];

    impl CancelVariant {
        fn cancel<E: 'static>(
            self,
            task: &Arc<CancellableTask<E>>,
        ) -> Result<(), InterruptedException> {
            match self {
                CancelVariant::Interruptible => task.cancel_and_await(),
                CancelVariant::RequestThenAwait => {
                    task.request_cancellation();
                    task.await_completion()
                }
            }
        }
    }

    type T = CancellableTask<String>;

    fn task(f: impl FnOnce() -> Result<(), String> + Send + 'static) -> Arc<T> {
        CancellableTask::new(Box::new(f))
    }

    fn task_with_completion(
        f: impl FnOnce() -> Result<(), String> + Send + 'static,
        c: impl FnOnce() + Send + 'static,
    ) -> Arc<T> {
        CancellableTask::with_completion(Box::new(f), Box::new(c))
    }

    fn panics_with(f: impl FnOnce(), prefix: &str) -> bool {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(()) => false,
            Err(p) => p
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| p.downcast_ref::<&str>().copied())
                .is_some_and(|m| m.starts_with(prefix)),
        }
    }

    #[test]
    fn run_twice_throws() {
        let t = task(|| Ok(()));
        assert!(t.run_if_not_cancelled().unwrap());
        let t2 = t.clone();
        assert!(panics_with(
            move || {
                let _ = t2.run_if_not_cancelled();
            },
            "IllegalStateException"
        ));
    }

    #[test]
    fn cancel_before_run_prevents_task_from_running() {
        for variant in VARIANTS {
            let ran = Arc::new(AtomicBool::new(false));
            let r = ran.clone();
            let t = task(move || {
                r.store(true, Ordering::SeqCst);
                Ok(())
            });
            variant.cancel(&t).unwrap();
            assert!(!t.run_if_not_cancelled().unwrap());
            assert!(!ran.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn cancel_twice_before_run_returns_without_blocking() {
        for variant in VARIANTS {
            let t = task(|| Ok(()));
            variant.cancel(&t).unwrap();
            variant.cancel(&t).unwrap();
        }
    }

    #[test]
    fn cancel_during_run_interrupts_and_awaits_task() {
        for variant in VARIANTS {
            let task_started = TestSemaphore::new();
            let task_interrupted = TestSemaphore::new();
            let task_may_finish = TestSemaphore::new();
            let cancellation_returned = TestSemaphore::new();
            let t = {
                let (started, interrupted, may_finish) = (
                    task_started.clone(),
                    task_interrupted.clone(),
                    task_may_finish.clone(),
                );
                task(move || {
                    started.release();
                    if may_finish.acquire().is_err() {
                        interrupted.release();
                        may_finish.acquire_uninterruptibly();
                    }
                    Ok(())
                })
            };
            let runner = {
                let t = t.clone();
                std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
            };
            assert!(task_started.try_acquire(WAIT));
            let canceller = {
                let (t, returned) = (t.clone(), cancellation_returned.clone());
                std::thread::spawn(move || {
                    variant.cancel(&t).unwrap();
                    returned.release();
                })
            };
            assert!(!cancellation_returned.try_acquire(SHORT));
            assert!(task_interrupted.try_acquire(WAIT));
            task_may_finish.release();
            assert!(cancellation_returned.try_acquire(WAIT));
            runner.join().unwrap();
            canceller.join().unwrap();
        }
    }

    #[test]
    fn cancel_after_run_returns_without_blocking() {
        for variant in VARIANTS {
            let t = task(|| Ok(()));
            assert!(t.run_if_not_cancelled().unwrap());
            variant.cancel(&t).unwrap();
            // Repeated cancellation is allowed and must not block either.
            variant.cancel(&t).unwrap();
        }
    }

    #[test]
    fn completion_runs_once_after_body() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let (e1, e2) = (events.clone(), events.clone());
        let t = task_with_completion(
            move || {
                e1.lock().push("body");
                Ok(())
            },
            move || e2.lock().push("completion"),
        );
        assert!(t.run_if_not_cancelled().unwrap());
        assert_eq!(*events.lock(), vec!["body", "completion"]);
        // Cancellation after completion must not run the completion action again.
        t.cancel_and_await().unwrap();
        assert_eq!(*events.lock(), vec!["body", "completion"]);
    }

    #[test]
    fn completion_runs_when_body_throws() {
        let completion_runs = Arc::new(AtomicUsize::new(0));
        let c = completion_runs.clone();
        let t = task_with_completion(
            || Err("task failed".to_owned()),
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(t.run_if_not_cancelled().unwrap_err(), "task failed");
        assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_before_run_runs_completion_exactly_once() {
        for variant in VARIANTS {
            let completion_runs = Arc::new(AtomicUsize::new(0));
            let c = completion_runs.clone();
            let t = task_with_completion(
                || Ok(()),
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                },
            );
            variant.cancel(&t).unwrap();
            assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
            // Neither a repeated cancellation nor the prevented run runs the completion action
            // again.
            variant.cancel(&t).unwrap();
            assert!(!t.run_if_not_cancelled().unwrap());
            assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
        }
    }

    /// A task whose completion action signals `started` and waits for `may_finish`.
    fn blocking_completion_task(
        started: &Arc<TestSemaphore>,
        may_finish: &Arc<TestSemaphore>,
    ) -> Arc<T> {
        let (s, m) = (started.clone(), may_finish.clone());
        task_with_completion(
            || Ok(()),
            move || {
                s.release();
                m.acquire_uninterruptibly();
            },
        )
    }

    #[test]
    fn cancel_before_run_concurrent_canceller_awaits_completion() {
        for variant in VARIANTS {
            let completion_started = TestSemaphore::new();
            let completion_may_finish = TestSemaphore::new();
            let t = blocking_completion_task(&completion_started, &completion_may_finish);
            let winner = {
                let t = t.clone();
                std::thread::spawn(move || variant.cancel(&t).unwrap())
            };
            assert!(completion_started.try_acquire(WAIT));
            let loser_returned = TestSemaphore::new();
            let loser = {
                let (t, r) = (t.clone(), loser_returned.clone());
                std::thread::spawn(move || {
                    variant.cancel(&t).unwrap();
                    r.release();
                })
            };
            // The losing canceler must not return while the winner's completion action is
            // running.
            assert!(!loser_returned.try_acquire(SHORT));
            completion_may_finish.release();
            assert!(loser_returned.try_acquire(WAIT));
            winner.join().unwrap();
            loser.join().unwrap();
        }
    }

    #[test]
    fn cancel_during_completion_awaits_without_interrupting_completion() {
        for variant in VARIANTS {
            let completion_started = TestSemaphore::new();
            let completion_may_finish = TestSemaphore::new();
            let completion_was_interrupted = Arc::new(AtomicBool::new(false));
            let t = {
                let (s, m, i) = (
                    completion_started.clone(),
                    completion_may_finish.clone(),
                    completion_was_interrupted.clone(),
                );
                task_with_completion(
                    || Ok(()),
                    move || {
                        s.release();
                        if m.acquire().is_err() {
                            i.store(true, Ordering::SeqCst);
                        }
                    },
                )
            };
            let runner = {
                let t = t.clone();
                std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
            };
            assert!(completion_started.try_acquire(WAIT));
            let cancellation_returned = TestSemaphore::new();
            let canceller = {
                let (t, r) = (t.clone(), cancellation_returned.clone());
                std::thread::spawn(move || {
                    variant.cancel(&t).unwrap();
                    r.release();
                })
            };
            // Completion is part of the terminal lifecycle, but cancellation only interrupts the
            // body.
            assert!(!cancellation_returned.try_acquire(SHORT));
            assert!(!completion_was_interrupted.load(Ordering::SeqCst));
            completion_may_finish.release();
            assert!(cancellation_returned.try_acquire(WAIT));
            runner.join().unwrap();
            canceller.join().unwrap();
            assert!(!completion_was_interrupted.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn run_losing_to_cancel_before_start_awaits_completion() {
        for variant in VARIANTS {
            let completion_started = TestSemaphore::new();
            let completion_may_finish = TestSemaphore::new();
            let t = blocking_completion_task(&completion_started, &completion_may_finish);
            let canceller = {
                let t = t.clone();
                std::thread::spawn(move || variant.cancel(&t).unwrap())
            };
            assert!(completion_started.try_acquire(WAIT));
            let run_returned = TestSemaphore::new();
            let ran = Arc::new(AtomicBool::new(true));
            let runner = {
                let (t, r, ran) = (t.clone(), run_returned.clone(), ran.clone());
                std::thread::spawn(move || {
                    ran.store(t.run_if_not_cancelled().unwrap(), Ordering::SeqCst);
                    r.release();
                })
            };
            assert!(!run_returned.try_acquire(SHORT));
            completion_may_finish.release();
            assert!(run_returned.try_acquire(WAIT));
            runner.join().unwrap();
            canceller.join().unwrap();
            assert!(!ran.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn task_throws_propagates_exception_and_unblocks_canceller() {
        for variant in VARIANTS {
            let task_started = TestSemaphore::new();
            let task_may_throw = TestSemaphore::new();
            let t = {
                let (s, m) = (task_started.clone(), task_may_throw.clone());
                task(move || {
                    s.release();
                    m.acquire_uninterruptibly();
                    Err("task failed".to_owned())
                })
            };
            let runner = {
                let t = t.clone();
                std::thread::spawn(move || t.run_if_not_cancelled().unwrap_err())
            };
            assert!(task_started.try_acquire(WAIT));
            let cancellation_returned = TestSemaphore::new();
            let canceller = {
                let (t, r) = (t.clone(), cancellation_returned.clone());
                std::thread::spawn(move || {
                    variant.cancel(&t).unwrap();
                    r.release();
                })
            };
            assert!(!cancellation_returned.try_acquire(SHORT));
            task_may_throw.release();
            // A task that leaves its body by throwing must still unblock cancelers.
            assert!(cancellation_returned.try_acquire(WAIT));
            assert_eq!(runner.join().unwrap(), "task failed");
            canceller.join().unwrap();
        }
    }

    #[test]
    fn cancel_from_task_body_throws() {
        for variant in VARIANTS {
            let task_ref: Arc<Mutex<Option<Arc<T>>>> = Arc::new(Mutex::new(None));
            let r = task_ref.clone();
            let t = task(move || {
                let t = r.lock().clone().unwrap();
                assert!(panics_with(
                    std::panic::AssertUnwindSafe(move || {
                        let _ = variant.cancel(&t);
                    }),
                    "IllegalStateException"
                ));
                Ok(())
            });
            *task_ref.lock() = Some(t.clone());
            assert!(t.run_if_not_cancelled().unwrap());
            *task_ref.lock() = None;
        }
    }

    #[test]
    fn cancel_from_completion_after_body_throws() {
        for variant in VARIANTS {
            let task_ref: Arc<Mutex<Option<Arc<T>>>> = Arc::new(Mutex::new(None));
            let r = task_ref.clone();
            let t = task_with_completion(
                || Ok(()),
                move || {
                    let t = r.lock().take().unwrap();
                    assert!(panics_with(
                        std::panic::AssertUnwindSafe(move || {
                            let _ = variant.cancel(&t);
                        }),
                        "IllegalStateException"
                    ));
                },
            );
            *task_ref.lock() = Some(t.clone());
            let runner = {
                let t = t.clone();
                std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
            };
            runner.join().unwrap();
        }
    }

    #[test]
    fn cancel_from_cancel_before_start_completion_throws() {
        for variant in VARIANTS {
            let task_ref: Arc<Mutex<Option<Arc<T>>>> = Arc::new(Mutex::new(None));
            let r = task_ref.clone();
            let t = task_with_completion(
                || Ok(()),
                move || {
                    let t = r.lock().take().unwrap();
                    assert!(panics_with(
                        std::panic::AssertUnwindSafe(move || {
                            let _ = variant.cancel(&t);
                        }),
                        "IllegalStateException"
                    ));
                },
            );
            *task_ref.lock() = Some(t.clone());
            let canceller = {
                let t = t.clone();
                std::thread::spawn(move || variant.cancel(&t).unwrap())
            };
            canceller.join().unwrap();
            assert!(!t.run_if_not_cancelled().unwrap());
        }
    }

    #[test]
    fn run_from_cancel_before_start_completion_throws() {
        let task_ref: Arc<Mutex<Option<Arc<T>>>> = Arc::new(Mutex::new(None));
        let r = task_ref.clone();
        let t = task_with_completion(
            || Ok(()),
            move || {
                let t = r.lock().take().unwrap();
                assert!(panics_with(
                    std::panic::AssertUnwindSafe(move || {
                        let _ = t.run_if_not_cancelled();
                    }),
                    "IllegalStateException"
                ));
            },
        );
        *task_ref.lock() = Some(t.clone());
        let canceller = {
            let t = t.clone();
            std::thread::spawn(move || t.cancel_and_await().unwrap())
        };
        canceller.join().unwrap();
    }

    fn assert_later_cancellation_returns(t: &Arc<T>, variant: CancelVariant) {
        let cancellation_returned = TestSemaphore::new();
        let canceller = {
            let (t, r) = (t.clone(), cancellation_returned.clone());
            std::thread::spawn(move || {
                variant.cancel(&t).unwrap();
                r.release();
            })
        };
        assert!(cancellation_returned.try_acquire(WAIT));
        canceller.join().unwrap();
    }

    #[test]
    fn completion_throws_after_body_still_signals_terminal_state() {
        for variant in VARIANTS {
            let t = task_with_completion(
                || Ok(()),
                || panic!("IllegalStateException: completion failed"),
            );
            let t2 = t.clone();
            assert!(panics_with(
                move || {
                    let _ = t2.run_if_not_cancelled();
                },
                "IllegalStateException: completion failed"
            ));
            assert_later_cancellation_returns(&t, variant);
        }
    }

    #[test]
    fn completion_throws_after_cancel_before_start_still_signals_terminal_state() {
        for variant in VARIANTS {
            let t = task_with_completion(
                || Ok(()),
                || panic!("IllegalStateException: completion failed"),
            );
            let t2 = t.clone();
            assert!(panics_with(
                move || {
                    let _ = variant.cancel(&t2);
                },
                "IllegalStateException: completion failed"
            ));
            assert_later_cancellation_returns(&t, variant);
            assert!(!t.run_if_not_cancelled().unwrap());
        }
    }

    #[test]
    fn request_cancellation_before_run_prevents_start_and_runs_completion() {
        let ran = Arc::new(AtomicBool::new(false));
        let completion_runs = Arc::new(AtomicUsize::new(0));
        let (r, c) = (ran.clone(), completion_runs.clone());
        let t = task_with_completion(
            move || {
                r.store(true, Ordering::SeqCst);
                Ok(())
            },
            move || {
                c.fetch_add(1, Ordering::SeqCst);
            },
        );
        t.request_cancellation();
        assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
        assert!(!t.run_if_not_cancelled().unwrap());
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_cancellation_during_run_interrupts_without_awaiting() {
        let task_started = TestSemaphore::new();
        let task_interrupted = TestSemaphore::new();
        let task_may_finish = TestSemaphore::new();
        let t = {
            let (s, i, m) = (
                task_started.clone(),
                task_interrupted.clone(),
                task_may_finish.clone(),
            );
            task(move || {
                s.release();
                if m.acquire().is_err() {
                    i.release();
                    m.acquire_uninterruptibly();
                }
                Ok(())
            })
        };
        let runner = {
            let t = t.clone();
            std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
        };
        assert!(task_started.try_acquire(WAIT));
        // Returns while the body is still running: it only finishes after the release below.
        t.request_cancellation();
        assert!(task_interrupted.try_acquire(WAIT));
        task_may_finish.release();
        runner.join().unwrap();
    }

    #[test]
    fn request_cancellation_then_await_completion_does_not_interrupt_task_again() {
        let task_started = TestSemaphore::new();
        let cleanup_started = TestSemaphore::new();
        let cleanup_may_finish = TestSemaphore::new();
        let interruption_count = Arc::new(AtomicUsize::new(0));
        let t = {
            let (s, cs, cm, n) = (
                task_started.clone(),
                cleanup_started.clone(),
                cleanup_may_finish.clone(),
                interruption_count.clone(),
            );
            task(move || {
                s.release();
                if TestSemaphore::new().acquire().is_err() {
                    n.fetch_add(1, Ordering::SeqCst);
                }
                cs.release();
                if cm.acquire().is_err() {
                    n.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })
        };
        let runner = {
            let t = t.clone();
            std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
        };
        assert!(task_started.try_acquire(WAIT));
        t.request_cancellation();
        assert!(cleanup_started.try_acquire(WAIT));
        let waiter = {
            let t = t.clone();
            std::thread::spawn(move || t.await_completion().unwrap())
        };
        // Awaiting a previously requested cancellation must not send another interrupt that
        // could abort the task's cleanup.
        std::thread::sleep(SHORT);
        assert!(!waiter.is_finished());
        assert_eq!(interruption_count.load(Ordering::SeqCst), 1);
        cleanup_may_finish.release();
        runner.join().unwrap();
        waiter.join().unwrap();
        assert_eq!(interruption_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_and_await_interrupted_while_awaiting_retry_interrupts_again_and_awaits() {
        let task_started = TestSemaphore::new();
        let task_may_finish = TestSemaphore::new();
        let task_interruptions = TestSemaphore::new();
        let interruption_count = Arc::new(AtomicUsize::new(0));
        let t = {
            let (s, m, ti, n) = (
                task_started.clone(),
                task_may_finish.clone(),
                task_interruptions.clone(),
                interruption_count.clone(),
            );
            task(move || {
                s.release();
                loop {
                    match m.acquire() {
                        Ok(()) => return Ok(()),
                        Err(_) => {
                            n.fetch_add(1, Ordering::SeqCst);
                            ti.release();
                        }
                    }
                }
            })
        };
        let runner = {
            let t = t.clone();
            std::thread::spawn(move || assert!(t.run_if_not_cancelled().unwrap()))
        };
        assert!(task_started.try_acquire(WAIT));
        let canceller_threw = TestSemaphore::new();
        let cancellation_returned = TestSemaphore::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let canceller = {
            let (t, threw, returned) = (
                t.clone(),
                canceller_threw.clone(),
                cancellation_returned.clone(),
            );
            std::thread::spawn(move || {
                tx.send(ThreadInterrupt::current()).unwrap();
                assert!(t.cancel_and_await().is_err());
                threw.release();
                // A retry interrupts the still-running task again and awaits quiescence.
                t.cancel_and_await().unwrap();
                returned.release();
            })
        };
        let canceller_interrupt = rx.recv().unwrap();
        assert!(task_interruptions.try_acquire(WAIT));
        canceller_interrupt.interrupt();
        assert!(canceller_threw.try_acquire(WAIT));
        assert!(task_interruptions.try_acquire(WAIT));
        assert!(!cancellation_returned.try_acquire(SHORT));
        assert_eq!(interruption_count.load(Ordering::SeqCst), 2);
        task_may_finish.release();
        assert!(cancellation_returned.try_acquire(WAIT));
        runner.join().unwrap();
        canceller.join().unwrap();
        assert_eq!(interruption_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn run_after_cancel_while_interrupted_restores_interrupt_bit() {
        let t = task(|| Ok(()));
        t.cancel_and_await().unwrap();
        ThreadInterrupt::current().interrupt();
        assert!(!t.run_if_not_cancelled().unwrap());
        assert!(blaze_shell::thread_interrupt::interrupted());
    }

    #[test]
    fn concurrent_run_and_cancel_exactly_one_claim_wins() {
        for i in 0..1000 {
            let body_runs = Arc::new(AtomicUsize::new(0));
            let body_running = Arc::new(AtomicBool::new(false));
            let completion_runs = Arc::new(AtomicUsize::new(0));
            let t = {
                let (runs, running, c) = (
                    body_runs.clone(),
                    body_running.clone(),
                    completion_runs.clone(),
                );
                task_with_completion(
                    move || {
                        running.store(true, Ordering::SeqCst);
                        runs.fetch_add(1, Ordering::SeqCst);
                        running.store(false, Ordering::SeqCst);
                        Ok(())
                    },
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    },
                )
            };
            let barrier = Arc::new(std::sync::Barrier::new(3));
            let ran = Arc::new(AtomicBool::new(false));
            let spawn_runner = || {
                let (t, b, ran) = (t.clone(), barrier.clone(), ran.clone());
                std::thread::spawn(move || {
                    b.wait();
                    ran.store(t.run_if_not_cancelled().unwrap(), Ordering::SeqCst);
                })
            };
            let spawn_canceller = || {
                let (t, b, running, c) = (
                    t.clone(),
                    barrier.clone(),
                    body_running.clone(),
                    completion_runs.clone(),
                );
                std::thread::spawn(move || {
                    b.wait();
                    t.cancel_and_await().unwrap();
                    // A normal return from cancellation guarantees that the body is not running
                    // and that the completion action has finished.
                    assert!(!running.load(Ordering::SeqCst));
                    assert_eq!(c.load(Ordering::SeqCst), 1);
                })
            };
            // Rotate which thread arrives at the barrier last.
            let threads = match i % 3 {
                0 => vec![spawn_runner(), spawn_canceller(), spawn_canceller()],
                1 => vec![spawn_canceller(), spawn_canceller(), spawn_runner()],
                _ => vec![spawn_canceller(), spawn_runner(), spawn_canceller()],
            };
            for thread in threads {
                thread.join().unwrap();
            }
            // The claim is atomic: either the task ran to completion exactly once and no
            // canceler prevented it, or cancellation prevented it from running.
            assert_eq!(
                ran.load(Ordering::SeqCst),
                body_runs.load(Ordering::SeqCst) == 1
            );
            assert!(body_runs.load(Ordering::SeqCst) <= 1);
            assert_eq!(completion_runs.load(Ordering::SeqCst), 1);
        }
    }
}
