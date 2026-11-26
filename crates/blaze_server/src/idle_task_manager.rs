//! `IdleTaskManager`: runs cleanup-related tasks during an idle period in the server.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use blaze_shell::thread_interrupt::ThreadInterrupt;
use blaze_util::check_state;
use parking_lot::Mutex;

use crate::idle_task::{IdleTask, IdleTaskError, IdleTaskResult, IdleTaskStatus};
use crate::interruptible::Monitor;

// Java: com/google/devtools/build/lib/server/IdleTaskManager.java:43 IdleTaskManager.State
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    INITIALIZED,
    IDLE,
    BUSY,
}

/// Runs `task` and turns its outcome into a result.
// Java: com/google/devtools/build/lib/server/IdleTaskManager.java:58 IdleTaskWrapper.call
fn idle_task_wrapper_call(task: &dyn IdleTask) -> IdleTaskResult {
    const CLASS: &str = "com.google.devtools.build.lib.server.IdleTaskManager$IdleTaskWrapper";
    let name = task.display_name();
    let stopwatch = Instant::now();
    blaze_util::server_logger::info(CLASS, "call", &format!("{} idle task started", name));
    match task.run() {
        Ok(()) => {
            blaze_util::server_logger::info(CLASS, "call", &format!("{} idle task finished", name));
            IdleTaskResult::new(name, IdleTaskStatus::SUCCESS, stopwatch.elapsed())
        }
        Err(IdleTaskError::IdleTask(e)) => {
            // withCause(e.getCause())
            blaze_util::server_logger::warning(
                CLASS,
                "call",
                &format!("{} idle task failed", name),
                Some(&e.get_cause().to_string()),
            );
            IdleTaskResult::new(name, IdleTaskStatus::FAILURE, stopwatch.elapsed())
        }
        // There's no point in restoring the interrupt bit since this thread belongs to an
        // executor service that is shutting down.
        Err(IdleTaskError::Interrupted(_)) => {
            blaze_util::server_logger::warning(
                CLASS,
                "call",
                &format!("{} idle task interrupted", name),
                Some("java.lang.InterruptedException"),
            );
            IdleTaskResult::new(name, IdleTaskStatus::INTERRUPTED, stopwatch.elapsed())
        }
    }
}

/// The single-threaded `ScheduledThreadPoolExecutor` of Java, reduced to what the manager uses:
/// tasks scheduled once, run serially in order of their delay (ties in submission order),
/// `shutdownNow` and `awaitTermination`.
struct Executor {
    /// Set by `shutdownNow`; guarded by the monitor so that a waiting worker wakes at once.
    shutdown: Monitor<bool>,
    /// The worker thread's interrupt status, for `shutdownNow`.
    worker: Mutex<Option<(ThreadInterrupt, JoinHandle<()>)>>,
}

/// Runs cleanup-related tasks during an idle period in the server.
///
/// A fresh instance must be constructed to manage each individual idle period. The idle period
/// begins when [`IdleTaskManager::idle`] is called and ends when [`IdleTaskManager::busy`] is
/// called.
// Java: com/google/devtools/build/lib/server/IdleTaskManager.java:40 IdleTaskManager
pub struct IdleTaskManager {
    state: Mutex<State>,
    executor: Arc<Executor>,
    idle_tasks: Vec<Arc<dyn IdleTask>>,
    /// The futures: one slot per task, filled when the task completes.
    task_futures: Arc<Mutex<Vec<Option<IdleTaskResult>>>>,
}

impl IdleTaskManager {
    /// Creates a new `IdleTaskManager` that runs `idle_tasks` while idle.
    // Java: com/google/devtools/build/lib/server/IdleTaskManager.java:94 IdleTaskManager.<init>
    pub fn new(idle_tasks: Vec<Arc<dyn IdleTask>>) -> IdleTaskManager {
        let n = idle_tasks.len();
        IdleTaskManager {
            state: Mutex::new(State::INITIALIZED),
            executor: Arc::new(Executor {
                shutdown: Monitor::new(false),
                worker: Mutex::new(None),
            }),
            idle_tasks,
            task_futures: Arc::new(Mutex::new(vec![None; n])),
        }
    }

    /// Called by the main thread when the server becomes idle.
    ///
    /// Does not block, but may schedule tasks in the background.
    // Java: com/google/devtools/build/lib/server/IdleTaskManager.java:104 IdleTaskManager.idle
    pub fn idle(&self) {
        let mut state = self.state.lock();
        check_state!(*state == State::INITIALIZED);
        *state = State::IDLE;

        // executor.schedule(new IdleTaskWrapper(task), task.delay().toSeconds(), SECONDS) for
        // each task. A ScheduledThreadPoolExecutor runs delayed tasks by trigger time, ties in
        // submission order.
        let now = Instant::now();
        let mut schedule: Vec<(Instant, usize)> = self
            .idle_tasks
            .iter()
            .enumerate()
            .map(|(i, task)| {
                let delay = Duration::from_secs(task.delay().as_secs());
                (
                    now.checked_add(delay)
                        .unwrap_or(now + Duration::from_secs(1 << 40)),
                    i,
                )
            })
            .collect();
        schedule.sort();
        if schedule.is_empty() {
            return;
        }
        let tasks = self.idle_tasks.clone();
        let futures = self.task_futures.clone();
        let executor = self.executor.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        // Java: new ThreadFactoryBuilder().setNameFormat("idle-server-tasks-%d")
        let handle = std::thread::Builder::new()
            .name("idle-server-tasks-0".to_string())
            .spawn(move || {
                tx.send(ThreadInterrupt::current()).unwrap();
                for (due, i) in schedule {
                    {
                        let mut shutdown = executor.shutdown.lock();
                        loop {
                            if *shutdown {
                                return;
                            }
                            let now = Instant::now();
                            if now >= due {
                                break;
                            }
                            // An interrupt here comes from shutdownNow, which also sets the flag.
                            let _ = executor.shutdown.wait(&mut shutdown, Some(due - now));
                        }
                    }
                    let result = idle_task_wrapper_call(&*tasks[i]);
                    futures.lock()[i] = Some(result);
                }
            })
            .expect("failed to start the idle task thread");
        let interrupt = rx.recv().expect("idle task thread died at start");
        *self.executor.worker.lock() = Some((interrupt, handle));
    }

    /// Called by the main thread when the server gets to work.
    ///
    /// Interrupts any pending idle tasks and blocks for their completion before returning.
    ///
    /// Returns the results of the idle tasks, in the order they were registered.
    // Java: com/google/devtools/build/lib/server/IdleTaskManager.java:121 IdleTaskManager.busy
    pub fn busy(&self) -> Vec<IdleTaskResult> {
        let mut state = self.state.lock();
        check_state!(*state == State::IDLE);
        *state = State::BUSY;

        // Interrupt pending tasks (executor.shutdownNow()).
        {
            let mut shutdown = self.executor.shutdown.lock();
            *shutdown = true;
            self.executor.shutdown.notify_all();
        }
        let worker = self.executor.worker.lock().take();
        if let Some((interrupt, handle)) = worker {
            interrupt.interrupt();
            // Wait for all tasks to complete, so they cannot interfere with a subsequent command.
            // PORT: a panic of a task is Java's "Unexpected exception thrown by idle task".
            if handle.join().is_err() {
                panic!("IllegalStateException: Unexpected exception thrown by idle task");
            }
        }

        let futures = self.task_futures.lock();
        let mut results = Vec::with_capacity(self.idle_tasks.len());
        for (i, task) in self.idle_tasks.iter().enumerate() {
            let name = task.display_name();
            // Don't wait: task might not have had a chance to start.
            let result = match &futures[i] {
                Some(result) => result.clone(),
                // Task was never started (TimeoutException). Java's CancellationException
                // ("Task was interrupted") cannot happen: shutdownNow does not cancel futures.
                None => IdleTaskResult::new(name, IdleTaskStatus::NOT_STARTED, Duration::ZERO),
            };
            results.push(result);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/server/IdleTaskManagerTest.java
    use super::*;
    use crate::idle_task::IdleTaskException;
    use crate::interruptible;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    struct FnTask<F> {
        name: String,
        delay: Duration,
        f: F,
    }

    impl<F: Fn() -> Result<(), IdleTaskError> + Send + Sync> IdleTask for FnTask<F> {
        fn display_name(&self) -> String {
            self.name.clone()
        }
        fn delay(&self) -> Duration {
            self.delay
        }
        fn run(&self) -> Result<(), IdleTaskError> {
            (self.f)()
        }
    }

    fn make_task(
        name: &str,
        delay: Duration,
        f: impl Fn() -> Result<(), IdleTaskError> + Send + Sync + 'static,
    ) -> Arc<dyn IdleTask> {
        Arc::new(FnTask {
            name: name.to_string(),
            delay,
            f,
        })
    }

    fn zero(stats: &[IdleTaskResult]) -> Vec<IdleTaskResult> {
        stats
            .iter()
            .map(|s| IdleTaskResult::new(s.name(), s.status(), Duration::ZERO))
            .collect()
    }

    #[test]
    fn registered_task_task_successful() {
        let (running_tx, running_rx) = mpsc::channel();
        let running_tx = Mutex::new(running_tx);
        let task_done = Arc::new(AtomicBool::new(false));
        let done = task_done.clone();
        let task = make_task("task", Duration::ZERO, move || {
            running_tx.lock().send(()).unwrap();
            interruptible::sleep_uninterruptibly(Duration::from_millis(200));
            done.store(true, Ordering::SeqCst);
            Ok(())
        });
        let manager = IdleTaskManager::new(vec![task]);

        manager.idle();
        running_rx.recv().unwrap(); // wait for task to start
        let stats = manager.busy();

        assert!(task_done.load(Ordering::SeqCst));
        assert_eq!(
            zero(&stats),
            vec![IdleTaskResult::new(
                "task",
                IdleTaskStatus::SUCCESS,
                Duration::ZERO
            )]
        );
    }

    #[test]
    fn registered_task_task_failed() {
        let (running_tx, running_rx) = mpsc::channel();
        let running_tx = Mutex::new(running_tx);
        let task_done = Arc::new(AtomicBool::new(false));
        let done = task_done.clone();
        let task = make_task("task", Duration::ZERO, move || {
            running_tx.lock().send(()).unwrap();
            interruptible::sleep_uninterruptibly(Duration::from_millis(200));
            done.store(true, Ordering::SeqCst);
            Err(IdleTaskException::new("failed").into())
        });
        let manager = IdleTaskManager::new(vec![task]);

        manager.idle();
        running_rx.recv().unwrap();
        let stats = manager.busy();

        assert!(task_done.load(Ordering::SeqCst));
        assert_eq!(
            zero(&stats),
            vec![IdleTaskResult::new(
                "task",
                IdleTaskStatus::FAILURE,
                Duration::ZERO
            )]
        );
    }

    #[test]
    fn registered_task_task_interrupted() {
        let (running_tx, running_rx) = mpsc::channel();
        let running_tx = Mutex::new(running_tx);
        let task_interrupted = Arc::new(AtomicBool::new(false));
        let interrupted = task_interrupted.clone();
        let task = make_task("task", Duration::ZERO, move || {
            running_tx.lock().send(()).unwrap();
            match interruptible::sleep(Duration::from_secs(24 * 3600)) {
                Ok(()) => Ok(()),
                Err(e) => {
                    interrupted.store(true, Ordering::SeqCst);
                    Err(e.into())
                }
            }
        });
        let manager = IdleTaskManager::new(vec![task]);

        manager.idle();
        running_rx.recv().unwrap();
        let stats = manager.busy();

        assert!(task_interrupted.load(Ordering::SeqCst));
        assert_eq!(
            zero(&stats),
            vec![IdleTaskResult::new(
                "task",
                IdleTaskStatus::INTERRUPTED,
                Duration::ZERO
            )]
        );
    }

    #[test]
    fn registered_task_task_not_started() {
        let task_started = Arc::new(AtomicBool::new(false));
        let started = task_started.clone();
        let task = make_task("task", Duration::from_secs(24 * 3600), move || {
            started.store(true, Ordering::SeqCst);
            Ok(())
        });
        let manager = IdleTaskManager::new(vec![task]);

        manager.idle();
        std::thread::sleep(Duration::from_millis(200)); // make it more likely that a bug will be caught
        let stats = manager.busy();

        assert!(!task_started.load(Ordering::SeqCst));
        assert_eq!(
            stats,
            vec![IdleTaskResult::new(
                "task",
                IdleTaskStatus::NOT_STARTED,
                Duration::ZERO
            )]
        );
    }

    #[test]
    fn registered_task_multiple_tasks() {
        let task_running = Arc::new(AtomicBool::new(false));
        let concurrent_tasks_detected = Arc::new(AtomicBool::new(false));
        let (finished_tx, finished_rx) = mpsc::channel();
        let finished_tx = Arc::new(Mutex::new(finished_tx));

        let make = |name: &str| {
            let task_running = task_running.clone();
            let concurrent = concurrent_tasks_detected.clone();
            let finished = finished_tx.clone();
            make_task(name, Duration::ZERO, move || {
                if task_running
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    concurrent.store(true, Ordering::SeqCst);
                }
                // make it more likely that a bug will be caught
                interruptible::sleep(Duration::from_millis(200))?;
                finished.lock().send(()).unwrap();
                if task_running
                    .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    concurrent.store(true, Ordering::SeqCst);
                }
                Ok(())
            })
        };
        let tasks = vec![make("a"), make("b"), make("c")];

        let manager = IdleTaskManager::new(tasks);

        manager.idle();
        for _ in 0..3 {
            finished_rx.recv().unwrap();
        }
        let stats = manager.busy();

        assert!(!concurrent_tasks_detected.load(Ordering::SeqCst));
        assert_eq!(
            zero(&stats),
            vec![
                IdleTaskResult::new("a", IdleTaskStatus::SUCCESS, Duration::ZERO),
                IdleTaskResult::new("b", IdleTaskStatus::SUCCESS, Duration::ZERO),
                IdleTaskResult::new("c", IdleTaskStatus::SUCCESS, Duration::ZERO),
            ]
        );
    }
}
