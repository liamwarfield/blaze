//! Java thread interruption (`Thread.interrupt`, `Thread.interrupted`, `Thread.isInterrupted`)
//! and `InterruptedException`, as far as `lib/shell` needs them.
//!
//! Rust threads have no interrupt flag. Each thread gets one here, plus an `eventfd` that becomes
//! readable when the thread is interrupted, so blocking waits (`poll`) wake up exactly like a Java
//! `Object.wait` does.
// PORT: move to blaze_util (thread interruption is needed by every blocking port).

use std::fmt;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
}

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
    static CURRENT: Arc<InterruptState> = Arc::new(InterruptState::new());
}

/// A handle on a thread's interrupt status (the part of `java.lang.Thread` used for
/// interruption). Obtain one with [`ThreadInterrupt::current`] and send it to other threads.
#[derive(Clone)]
pub struct ThreadInterrupt(Arc<InterruptState>);

impl ThreadInterrupt {
    /// `Thread.currentThread()`.
    pub fn current() -> ThreadInterrupt {
        ThreadInterrupt(CURRENT.with(Arc::clone))
    }

    /// `Thread.interrupt()`: sets the flag and wakes any interruptible wait of that thread.
    pub fn interrupt(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        // SAFETY: write to our own eventfd.
        unsafe {
            libc::eventfd_write(self.0.event_fd.as_raw_fd(), 1);
        }
    }

    /// `Thread.isInterrupted()`: does not clear the flag.
    pub fn is_interrupted(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }
}

/// `Thread.interrupted()`: returns and clears the current thread's interrupt flag.
pub fn interrupted() -> bool {
    CURRENT.with(|s| {
        s.drain();
        s.flag.swap(false, Ordering::SeqCst)
    })
}

/// `Thread.currentThread().isInterrupted()`.
pub fn is_current_interrupted() -> bool {
    CURRENT.with(|s| s.flag.load(Ordering::SeqCst))
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
    let event_fd = CURRENT.with(|s| s.event_fd.as_raw_fd());
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
