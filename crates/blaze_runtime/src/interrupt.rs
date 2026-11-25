//! Interruption of the running command: Java's `Thread.interrupt()` of the thread that runs the
//! command, and the SIGINT handlers of batch mode (`BlazeRuntime.captureSigint`) and server mode
//! (`BlazeRuntime.serverMain`), `server/signal/InterruptSignalHandler.java`.
//!
//! Java interrupts the command thread, whose blocking calls then throw `InterruptedException`.
//! The command thread's interrupt status is its `blaze_shell::thread_interrupt::ThreadInterrupt`:
//! the server's `CommandManager` interrupts the `grpc-command-N` thread of the command on a
//! `Cancel` (`CommandManager.doCancel`), batch mode's SIGINT handler interrupts the main thread
//! (`mainThread.interrupt()`), and the in-process server's cancel interrupts the thread that runs
//! its one command ([`command_thread_canceller`]).
//!
//! PORT: the command's evaluators do not wait on the command thread's status; they carry their
//! own flag (`blaze_skyframe::Evaluator::interrupt_flag`: dependency requests and `evaluate`
//! return `Interrupted` once it is set). [`register_interrupt_flag`] bridges the two: called on
//! the command thread, it registers an interrupt listener on that thread's status that sets the
//! evaluator's flag, so that an interrupt of the command's thread reaches its evaluators at once,
//! and an interrupt that arrived before the evaluator was created reaches it when it registers
//! (a Java thread's interrupt status is sticky until a wait consumes it). The status belongs to
//! the thread, so it never reaches a command on another thread: each command of a server runs on
//! a fresh `grpc-command-N` thread (`GrpcServerImpl.commandExecutorPool`), so a later command
//! does not start interrupted, and a command waiting for the command lock is interrupted only by
//! its own `Cancel`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use blaze_events::OutErr;
use blaze_shell::thread_interrupt::{InterruptRegistration, ThreadInterrupt};
use parking_lot::Mutex;

/// The evaluator flags registered with [`register_interrupt_flag`] and the listener that keeps
/// each one in step with the interrupt status of the thread that registered it. Entries whose
/// evaluator is gone are dropped at the next registration (which unregisters their listener).
static FLAG_REGISTRATIONS: Mutex<Vec<(Weak<AtomicBool>, InterruptRegistration)>> =
    Mutex::new(Vec::new());

/// Registers the interrupt flag of an evaluator of the running command
/// (`Evaluator::interrupt_flag`) with the interrupt status of the calling thread, the command
/// thread (Java: the thread that calls `evaluate` is the one interrupted). An interrupt of that
/// thread sets the flag; an interrupt that is already pending sets it at once.
pub fn register_interrupt_flag(flag: &Arc<AtomicBool>) {
    let weak = Arc::downgrade(flag);
    let listener_flag = weak.clone();
    let registration = ThreadInterrupt::current().add_interrupt_listener(Arc::new(move || {
        if let Some(flag) = listener_flag.upgrade() {
            flag.store(true, Ordering::SeqCst);
        }
    }));
    let mut registrations = FLAG_REGISTRATIONS.lock();
    registrations.retain(|(f, _)| f.strong_count() > 0);
    registrations.push((weak, registration));
}

/// `commandThread.interrupt()` for the thread that calls this function: the cancel of the
/// in-process server (`GrpcServerImpl.cancel` / `CommandManager.doCancel` interrupt the
/// command's thread), which the client's cancel thread calls on Ctrl-C. Call it on the thread
/// that then runs the command.
// Java: com/google/devtools/build/lib/server/CommandManager.java:99 CommandManager.doCancel
pub fn command_thread_canceller() -> Arc<dyn Fn() + Send + Sync> {
    let command_thread = ThreadInterrupt::current();
    Arc::new(move || command_thread.interrupt())
}

/// The write end of the pipe the signal handler writes to (-1: no handler installed).
static SIGINT_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// The SIGINT handler: wakes the thread that runs `InterruptSignalHandler.run`. Only
/// async-signal-safe calls are allowed here.
extern "C" fn on_sigint(_signal: libc::c_int) {
    let fd = SIGINT_PIPE_WRITE.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte: u8 = 1;
        // SAFETY: write(2) is async-signal-safe; the buffer is one valid byte.
        unsafe {
            libc::write(fd, (&raw const byte).cast::<libc::c_void>(), 1);
        }
    }
}

/// The installed SIGINT disposition of an [`InterruptSignalHandler`], as its `run` sees it (to
/// call `uninstall()`).
#[derive(Clone)]
pub struct InterruptSignalHandlerRef {
    /// `oldHandler`; `None` once uninstalled.
    old_handler: Arc<Mutex<Option<libc::sighandler_t>>>,
}

impl InterruptSignalHandlerRef {
    /// Disables SIGINT handling.
    // Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:45 InterruptSignalHandler.uninstall
    pub fn uninstall(&self) {
        if let Some(old) = self.old_handler.lock().take() {
            // SAFETY: restores the disposition that was in place before the handler.
            unsafe {
                libc::signal(libc::SIGINT, old);
            }
        }
    }
}

/// A facade providing special-purpose SIGINT handling: until [`InterruptSignalHandler::uninstall`]
/// is invoked (or the value is dropped), the delivery of a SIGINT signal to this process causes
/// the handler's `run` to be invoked in another thread.
///
/// PORT: Java's `uninstall()` checks it is called once; here a second call (or the drop after
/// one) does nothing.
// Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:28 InterruptSignalHandler
pub struct InterruptSignalHandler {
    handle: InterruptSignalHandlerRef,
}

impl InterruptSignalHandler {
    /// Constructs an InterruptSignalHandler instance. Until the uninstall() method is invoked,
    /// the delivery of a SIGINT signal to this process will cause `run` to be invoked in
    /// another thread (named `interrupt-signal-handler`).
    // Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:37 InterruptSignalHandler.<init>
    pub fn new(
        mut run: impl FnMut(&InterruptSignalHandlerRef) + Send + 'static,
    ) -> InterruptSignalHandler {
        let mut fds: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `fds` is a valid array of two ints.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            panic!(
                "IllegalStateException: pipe2: {}",
                std::io::Error::last_os_error()
            );
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        SIGINT_PIPE_WRITE.store(write_fd, Ordering::SeqCst);
        // SAFETY: `on_sigint` only calls async-signal-safe functions.
        let old = unsafe {
            libc::signal(
                libc::SIGINT,
                on_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
            )
        };
        let handle = InterruptSignalHandlerRef {
            old_handler: Arc::new(Mutex::new(Some(old))),
        };

        let watcher_handle = handle.clone();
        std::thread::Builder::new()
            .name("interrupt-signal-handler".to_owned())
            // Java: the JVM's signal dispatch thread; an exception of the handler reaches the
            // default uncaught exception handler.
            .spawn(move || {
                blaze_util::uncaught_exception::run_catching(|| {
                    loop {
                        let mut byte: u8 = 0;
                        // SAFETY: reads one byte into a valid buffer.
                        let n = unsafe {
                            libc::read(read_fd, (&raw mut byte).cast::<libc::c_void>(), 1)
                        };
                        if n < 0
                            && std::io::Error::last_os_error().kind()
                                == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        if n <= 0 {
                            return;
                        }
                        run(&watcher_handle);
                    }
                });
            })
            .unwrap_or_else(|e| panic!("OutOfMemoryError: unable to create native thread: {e}"));

        InterruptSignalHandler { handle }
    }

    /// Disables SIGINT handling.
    // Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:45 InterruptSignalHandler.uninstall
    pub fn uninstall(&self) {
        self.handle.uninstall();
    }
}

impl Drop for InterruptSignalHandler {
    /// `batchMain`'s and `serverMain`'s `finally { signalHandler.uninstall(); }` (a no-op when
    /// the handler was uninstalled already).
    fn drop(&mut self) {
        self.handle.uninstall();
    }
}

/// Installs the SIGINT handler of batch mode: every SIGINT prints `Bazel received an interrupt`
/// and interrupts the thread that called this function (the main thread, which runs the
/// command); the second one reverts to the previous handler, so that a third one kills the
/// process.
///
/// PORT: the `interrupt-watcher` thread only writes log lines and thread dumps to the server log
/// every 10 seconds (`ThreadUtils.warnAboutSlowInterrupt`); Blaze has no server logger, so it is
/// not started.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1045 BlazeRuntime.captureSigint
pub fn capture_sigint() -> InterruptSignalHandler {
    let main_thread = ThreadInterrupt::current();
    // AtomicInteger numInterrupts
    let mut num_interrupts = 0;
    InterruptSignalHandler::new(move |handler| {
        // PORT: skipped, logging ("User interrupt").
        OutErr::system_out_err().print_err_ln("Bazel received an interrupt");
        main_thread.interrupt();

        num_interrupts += 1;
        if num_interrupts == 2 {
            // PORT: skipped, logging ("Second --batch interrupt: Reverting to JVM SIGINT
            // handler").
            handler.uninstall();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Thread.interrupt() semantics: the interrupt status is sticky, so an evaluator created after
    // the interrupt is interrupted as well.
    #[test]
    fn interrupt_reaches_registered_and_later_flags() {
        std::thread::spawn(|| {
            let command_thread = ThreadInterrupt::current();
            let before = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&before);
            assert!(!before.load(Ordering::SeqCst));

            let t = command_thread.clone();
            std::thread::spawn(move || t.interrupt()).join().unwrap();
            assert!(before.load(Ordering::SeqCst));

            let after = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&after);
            assert!(after.load(Ordering::SeqCst));
        })
        .join()
        .unwrap();
    }

    // The interrupt belongs to the command's thread: a command on another thread (a later
    // command of the server, or one waiting for the command lock) is not interrupted.
    #[test]
    fn interrupt_does_not_reach_other_command_threads() {
        let (tx, rx) = std::sync::mpsc::channel();
        let first = std::thread::spawn(move || {
            let flag = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&flag);
            tx.send(ThreadInterrupt::current()).unwrap();
            while !flag.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            flag
        });
        let first_thread = rx.recv().unwrap();
        let second = std::thread::spawn(|| {
            let flag = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&flag);
            flag
        });
        let second_flag = second.join().unwrap();
        first_thread.interrupt();
        assert!(first.join().unwrap().load(Ordering::SeqCst));
        assert!(!second_flag.load(Ordering::SeqCst));
        // A command that starts after the interrupt, on its own thread, starts uninterrupted.
        let later = std::thread::spawn(|| {
            let flag = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&flag);
            flag.load(Ordering::SeqCst)
        });
        assert!(!later.join().unwrap());
    }

    #[test]
    fn dropped_evaluators_are_forgotten() {
        std::thread::spawn(|| {
            let dropped = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&dropped);
            let weak = Arc::downgrade(&dropped);
            drop(dropped);
            let live = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&live);
            assert!(
                !FLAG_REGISTRATIONS
                    .lock()
                    .iter()
                    .any(|(f, _)| f.ptr_eq(&weak))
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn command_thread_canceller_interrupts_the_thread_that_created_it() {
        std::thread::spawn(|| {
            let cancel = command_thread_canceller();
            let flag = Arc::new(AtomicBool::new(false));
            register_interrupt_flag(&flag);
            std::thread::spawn(move || cancel()).join().unwrap();
            assert!(flag.load(Ordering::SeqCst));
            assert!(blaze_shell::thread_interrupt::interrupted());
        })
        .join()
        .unwrap();
    }
}
