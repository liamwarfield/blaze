//! Interruptible waits on the `blaze_shell::thread_interrupt` status: the parts of
//! `Thread.sleep` and `Object.wait` this crate needs.
//!
//! PORT: Blaze only. Java's `Object.wait` and `Thread.sleep` wake on `Thread.interrupt()`; Rust
//! condition variables do not, so a waiter registers an interrupt listener that notifies its
//! condition variable, and also re-checks the interrupt status every
//! [`INTERRUPT_POLL_INTERVAL`] (a notification sent between the waiter's check and its wait
//! would otherwise be lost).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use blaze_shell::thread_interrupt::{self, InterruptedException, ThreadInterrupt};
use parking_lot::{Condvar, Mutex, MutexGuard};

/// The longest an interrupt can go unnoticed by a waiter whose wake-up was lost.
const INTERRUPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// A monitor (`synchronized` + `wait`/`notify`) whose waits end on an interrupt of the waiting
/// thread, as Java's do.
pub struct Monitor<T> {
    state: Mutex<T>,
    shared: Arc<Shared>,
}

struct Shared {
    cond: Condvar,
    /// Incremented by every notification, so that a waiter returns only when notified, timed
    /// out or interrupted (no spurious returns).
    generation: AtomicU64,
    /// The number of threads in `wait` (Java's `Thread.State.WAITING`, for tests).
    waiters: AtomicU64,
}

impl<T: Default> Default for Monitor<T> {
    fn default() -> Self {
        Monitor::new(T::default())
    }
}

impl<T> Monitor<T> {
    pub fn new(value: T) -> Self {
        Monitor {
            state: Mutex::new(value),
            shared: Arc::new(Shared {
                cond: Condvar::new(),
                generation: AtomicU64::new(0),
                waiters: AtomicU64::new(0),
            }),
        }
    }

    /// `synchronized (this)`.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        self.state.lock()
    }

    /// `notifyAll()` (and `notify()`: every waiter wakes; Java allows either). Call it while
    /// holding the lock, as Java requires.
    pub fn notify_all(&self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.cond.notify_all();
    }

    /// `wait()` (`timeout` `None`) or `wait(timeout)` on a held lock. Returns
    /// `Err(InterruptedException)`, clearing the status, when the current thread is or becomes
    /// interrupted; otherwise returns when notified or when the timeout passes.
    pub fn wait(
        &self,
        guard: &mut MutexGuard<'_, T>,
        timeout: Option<Duration>,
    ) -> Result<(), InterruptedException> {
        if thread_interrupt::interrupted() {
            return Err(InterruptedException);
        }
        let start_generation = self.shared.generation.load(Ordering::SeqCst);
        self.shared.waiters.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting(&self.shared);
        let deadline = timeout.and_then(|t| Instant::now().checked_add(t));
        let shared = self.shared.clone();
        let _registration =
            ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
                shared.cond.notify_all();
            }));
        loop {
            if thread_interrupt::interrupted() {
                return Err(InterruptedException);
            }
            if self.shared.generation.load(Ordering::SeqCst) != start_generation {
                return Ok(());
            }
            let now = Instant::now();
            let slice = match deadline {
                Some(deadline) if now >= deadline => return Ok(()),
                Some(deadline) => (deadline - now).min(INTERRUPT_POLL_INTERVAL),
                // No timeout, or one too large to represent.
                None => INTERRUPT_POLL_INTERVAL,
            };
            let _ = self.shared.cond.wait_for(guard, slice);
        }
    }
}

/// Counts a thread in [`Monitor::wait`] until dropped.
struct Waiting<'a>(&'a Shared);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiters.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<T> Monitor<T> {
    /// The number of threads waiting in [`Monitor::wait`] (their `Thread.State` is `WAITING` or
    /// `TIMED_WAITING`); they have read the notification count, so a notification sent from now
    /// on wakes them.
    pub fn waiting_threads(&self) -> u64 {
        self.shared.waiters.load(Ordering::SeqCst)
    }
}

/// `Thread.sleep(duration)`: returns `Err(InterruptedException)` (and clears the status) when
/// the current thread is or becomes interrupted.
pub fn sleep(duration: Duration) -> Result<(), InterruptedException> {
    let monitor = Monitor::new(());
    let mut guard = monitor.lock();
    monitor.wait(&mut guard, Some(duration))
}

/// `Uninterruptibles.sleepUninterruptibly(duration)`.
pub fn sleep_uninterruptibly(duration: Duration) {
    std::thread::sleep(duration);
}
