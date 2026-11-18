//! Java thread interruption (`Thread.interrupt`, `Thread.interrupted`, `Thread.isInterrupted`)
//! and `InterruptedException`, as far as `lib/shell` needs them.
//!
//! Rust threads have no interrupt flag. Each thread gets one here, plus an `eventfd` that becomes
//! readable when the thread is interrupted, so blocking waits (`poll`) wake up exactly like a Java
//! `Object.wait` does.
// PORT: move to blaze_util (thread interruption is needed by every blocking port).

use std::cell::RefCell;
use std::fmt;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

/// Port of `java.lang.InterruptedException`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InterruptedException;

impl fmt::Display for InterruptedException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("java.lang.InterruptedException")
    }
}

impl std::error::Error for InterruptedException {}

struct InterruptState {
    flag: AtomicBool,
    event_fd: OwnedFd,
    /// Callbacks that [`ThreadInterrupt::interrupt`] runs (see
    /// [`ThreadInterrupt::add_interrupt_listener`]), by registration id.
    listeners: Mutex<Vec<(u64, InterruptListener)>>,
}

/// A wake-up callback run when a thread is interrupted (see
/// [`ThreadInterrupt::add_interrupt_listener`]).
pub type InterruptListener = Arc<dyn Fn() + Send + Sync>;

impl InterruptState {
    fn new() -> InterruptState {
        // SAFETY: plain syscall; the result is checked.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            panic!("eventfd failed: {}", std::io::Error::last_os_error());
        }
        InterruptState {
            flag: AtomicBool::new(false),
            // SAFETY: fd is a fresh descriptor we own.
            event_fd: unsafe { OwnedFd::from_raw_fd(fd) },
            listeners: Mutex::new(Vec::new()),
        }
    }

    fn drain(&self) {
        let mut value: libc::eventfd_t = 0;
        // SAFETY: non-blocking read into a local.
        unsafe {
            libc::eventfd_read(self.event_fd.as_raw_fd(), &mut value);
        }
    }
}

thread_local! {
    static OWN: Arc<InterruptState> = Arc::new(InterruptState::new());
    /// The status this thread stands in for (see [`ThreadInterrupt::adopt`]), if any.
    static ADOPTED: RefCell<Option<Arc<InterruptState>>> = const { RefCell::new(None) };
}

/// Runs `f` with the interrupt status of the current thread: its own, or the one it adopted.
fn with_current<R>(f: impl FnOnce(&Arc<InterruptState>) -> R) -> R {
    let adopted = ADOPTED.with(|a| a.borrow().clone());
    match adopted {
        Some(state) => f(&state),
        None => OWN.with(f),
    }
}

static NEXT_LISTENER_ID: AtomicU64 = AtomicU64::new(0);

/// A handle on a thread's interrupt status (the part of `java.lang.Thread` used for
/// interruption). Obtain one with [`ThreadInterrupt::current`] and send it to other threads.
#[derive(Clone)]
pub struct ThreadInterrupt(Arc<InterruptState>);

impl ThreadInterrupt {
    /// `Thread.currentThread()`.
    pub fn current() -> ThreadInterrupt {
        ThreadInterrupt(with_current(Arc::clone))
    }

    /// `Thread.interrupt()`: sets the flag, wakes any interruptible wait of that thread and runs
    /// the thread's interrupt listeners (on the calling thread).
    pub fn interrupt(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        // SAFETY: write to our own eventfd.
        unsafe {
            libc::eventfd_write(self.0.event_fd.as_raw_fd(), 1);
        }
        let listeners: Vec<InterruptListener> = self
            .0
            .listeners
            .lock()
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for listener in listeners {
            listener();
        }
    }

    /// `Thread.isInterrupted()`: does not clear the flag.
    pub fn is_interrupted(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }

    /// Whether both handles are the interrupt status of the same thread.
    pub fn same_thread(&self, other: &ThreadInterrupt) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Makes the current thread stand in for the thread of `self` until the guard is dropped:
    /// [`ThreadInterrupt::current`], [`interrupted`], [`is_current_interrupted`],
    /// [`interrupt_current`] and every interruptible wait of this crate then use `self`'s status.
    /// An interrupt of that thread therefore reaches this one, and a status this thread sets or
    /// clears is that thread's.
    ///
    /// PORT: Blaze only. Java runs a whole `Action.execute` on the Skyframe worker thread; Blaze
    /// runs parts of it on a helper thread with a larger stack while the worker waits for it
    /// (`blaze_exec::exec::evaluation_stack`). The helper adopts the worker's status, so the
    /// evaluator's `Thread.interrupt()` of the worker (`AbstractQueueVisitor.interruptInFlightTasks`)
    /// reaches the waits in the spawn runners.
    pub fn adopt(&self) -> AdoptedInterrupt {
        let previous = ADOPTED.with(|a| a.borrow_mut().replace(self.0.clone()));
        AdoptedInterrupt {
            previous,
            _not_send: std::marker::PhantomData,
        }
    }

    /// Registers `listener` to run whenever this thread is interrupted, until the returned
    /// registration is dropped. If the thread is interrupted already, `listener` also runs once
    /// at once, on the calling thread. Listeners run on the interrupting thread after the flag is
    /// set; they must be quick and must not block (typically they cancel a call or wake a wait
    /// that is not one of this crate's).
    ///
    /// PORT: Blaze only. Java code blocked in a gRPC call or a `ListenableFuture` is woken by
    /// `Thread.interrupt()` through the JDK; a Rust wait outside this crate registers here
    /// instead.
    pub fn add_interrupt_listener(&self, listener: InterruptListener) -> InterruptRegistration {
        let id = NEXT_LISTENER_ID.fetch_add(1, Ordering::Relaxed);
        self.0.listeners.lock().push((id, listener.clone()));
        if self.is_interrupted() {
            listener();
        }
        InterruptRegistration {
            state: self.0.clone(),
            id,
        }
    }
}

/// While alive, the current thread uses another thread's interrupt status (see
/// [`ThreadInterrupt::adopt`]).
pub struct AdoptedInterrupt {
    previous: Option<Arc<InterruptState>>,
    /// The guard restores the status of the thread that created it.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for AdoptedInterrupt {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ADOPTED.with(|a| *a.borrow_mut() = previous);
    }
}

/// Keeps an interrupt listener registered (see [`ThreadInterrupt::add_interrupt_listener`]);
/// dropping it removes the listener.
pub struct InterruptRegistration {
    state: Arc<InterruptState>,
    id: u64,
}

impl Drop for InterruptRegistration {
    fn drop(&mut self) {
        self.state.listeners.lock().retain(|(id, _)| *id != self.id);
    }
}

/// Runs `listener` if the current thread (or the thread it stands in for) is interrupted while
/// `f` runs; see [`ThreadInterrupt::add_interrupt_listener`].
pub fn with_interrupt_listener<R>(listener: InterruptListener, f: impl FnOnce() -> R) -> R {
    let _registration = ThreadInterrupt::current().add_interrupt_listener(listener);
    f()
}

/// `Thread.interrupted()`: returns and clears the current thread's interrupt flag.
pub fn interrupted() -> bool {
    with_current(|s| {
        s.drain();
        s.flag.swap(false, Ordering::SeqCst)
    })
}

/// `Thread.currentThread().isInterrupted()`.
pub fn is_current_interrupted() -> bool {
    with_current(|s| s.flag.load(Ordering::SeqCst))
}

/// `Thread.currentThread().interrupt()`.
pub fn interrupt_current() {
    ThreadInterrupt::current().interrupt();
}

/// Blocks until `fd` (if any) is readable, `timeout` passes, or the current thread is
/// interrupted. Like Java's `Object.wait`, an interrupt is reported (and the flag cleared) by
/// returning `Err(InterruptedException)`. Spurious returns are allowed; callers loop.
pub(crate) fn wait_readable(
    fd: Option<RawFd>,
    timeout: Option<Duration>,
) -> Result<(), InterruptedException> {
    if interrupted() {
        return Err(InterruptedException);
    }
    let event_fd = with_current(|s| s.event_fd.as_raw_fd());
    // Without a descriptor to watch, fall back to short sleeps.
    let fallback = Duration::from_millis(10);
    let timeout = match (fd, timeout) {
        (None, None) => Some(fallback),
        (None, Some(t)) => Some(t.min(fallback)),
        (Some(_), t) => t,
    };
    let timeout_ms: libc::c_int = match timeout {
        None => -1,
        Some(t) => {
            // Round up so that a positive remaining time never becomes a zero-timeout spin.
            let ms = t.as_nanos().div_ceil(1_000_000);
            ms.min(libc::c_int::MAX as u128) as libc::c_int
        }
    };
    let mut fds = [
        libc::pollfd {
            fd: event_fd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: fd.unwrap_or(-1),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: fds is a valid array of two pollfd; a negative fd is ignored by poll.
    unsafe {
        libc::poll(fds.as_mut_ptr(), 2, timeout_ms);
    }
    if interrupted() {
        return Err(InterruptedException);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn an_adopting_thread_sees_and_receives_the_interrupts_of_the_adopted_thread() {
        let worker = ThreadInterrupt::current();
        let (tx, rx) = std::sync::mpsc::channel();
        let helper = std::thread::spawn(move || {
            let _adopted = worker.adopt();
            assert!(ThreadInterrupt::current().same_thread(&worker));
            tx.send(()).unwrap();
            // Woken by the interrupt of the adopted (worker) thread's status.
            loop {
                if wait_readable(None, Some(Duration::from_secs(30))).is_err() {
                    return true;
                }
            }
        });
        rx.recv().unwrap();
        ThreadInterrupt::current().interrupt();
        assert!(helper.join().unwrap());
        // The helper's wait cleared the shared status, as a Java wait on the worker would.
        assert!(!is_current_interrupted());
    }

    #[test]
    fn a_status_set_while_adopted_is_the_adopted_threads() {
        let worker = ThreadInterrupt::current();
        std::thread::spawn(move || {
            let _adopted = worker.adopt();
            interrupt_current();
        })
        .join()
        .unwrap();
        assert!(interrupted());
    }

    #[test]
    fn adoption_ends_with_the_guard() {
        let other = std::thread::spawn(ThreadInterrupt::current).join().unwrap();
        {
            let _adopted = other.adopt();
            interrupt_current();
            assert!(other.is_interrupted());
        }
        assert!(!is_current_interrupted());
    }

    #[test]
    fn listeners_run_on_interrupt_until_unregistered() {
        let thread = ThreadInterrupt::current();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let registration = thread.add_interrupt_listener(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let t = thread.clone();
        std::thread::spawn(move || t.interrupt()).join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        drop(registration);
        thread.interrupt();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(interrupted());
    }

    #[test]
    fn a_listener_registered_on_an_interrupted_thread_runs_at_once() {
        interrupt_current();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        with_interrupt_listener(
            Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
            || {},
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(interrupted());
    }
}
