//! Interruption of the running command: Java's `Thread.interrupt()` of the thread that runs the
//! command, and the SIGINT handler of batch mode (`BlazeRuntime.captureSigint`,
//! `server/signal/InterruptSignalHandler.java`).
//!
//! PORT: Java interrupts the command thread, whose blocking calls then throw
//! `InterruptedException`. A Rust thread has no interrupt status; the command's evaluators carry
//! it instead (`blaze_skyframe::Evaluator::interrupt_flag`: dependency requests and `evaluate`
//! return `Interrupted` once it is set). The command registers the flag of every evaluator it
//! creates with [`register_interrupt_flag`], and [`interrupt_command_thread`] sets them all. Like
//! the thread's interrupt status, an interrupt that arrives while no evaluator exists stays
//! pending and reaches the next one registered.
//!
//! The in-process server of the client (`BlazeRuntime::server_main_single_command`) has no gRPC
//! `cancel` request: the client's own SIGINT handler has to call [`interrupt_command_thread`]
//! where Java's client sends the cancel request (`GrpcServerImpl.cancel` interrupts the command
//! thread: server/GrpcServerImpl.java).

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Weak};

use blaze_events::OutErr;
use parking_lot::Mutex;

/// The interrupt status of the command thread and the evaluators that observe it.
struct CommandThread {
    /// The interrupt status of the command thread (`Thread.isInterrupted()`).
    interrupted: AtomicBool,
    /// The interrupt flags of the command's evaluators.
    interrupt_flags: Mutex<Vec<Weak<AtomicBool>>>,
}

impl CommandThread {
    const fn new() -> CommandThread {
        CommandThread {
            interrupted: AtomicBool::new(false),
            interrupt_flags: Mutex::new(Vec::new()),
        }
    }

    fn register_interrupt_flag(&self, flag: &Arc<AtomicBool>) {
        let mut flags = self.interrupt_flags.lock();
        flags.retain(|f| f.strong_count() > 0);
        flags.push(Arc::downgrade(flag));
        if self.interrupted.load(Ordering::SeqCst) {
            flag.store(true, Ordering::SeqCst);
        }
    }

    fn interrupt(&self) {
        // The lock orders this with a concurrent registration: a flag is either in the list
        // here or sees the status when it is added.
        let flags = self.interrupt_flags.lock();
        self.interrupted.store(true, Ordering::SeqCst);
        for flag in flags.iter() {
            if let Some(flag) = flag.upgrade() {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// The thread that runs the command of this process (one command per process).
static COMMAND_THREAD: CommandThread = CommandThread::new();

/// Registers the interrupt flag of an evaluator of the running command
/// (`Evaluator::interrupt_flag`). An interrupt that is already pending is delivered at once.
pub fn register_interrupt_flag(flag: &Arc<AtomicBool>) {
    COMMAND_THREAD.register_interrupt_flag(flag);
}

/// `commandThread.interrupt()`: interrupts the running command.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1066 BlazeRuntime.captureSigint
pub fn interrupt_command_thread() {
    COMMAND_THREAD.interrupt();
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

/// A facade providing special-purpose SIGINT handling: until [`InterruptSignalHandler::uninstall`]
/// is invoked (or the value is dropped), the delivery of a SIGINT signal to this process causes
/// the handler's `run` to be invoked in another thread.
// Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:28 InterruptSignalHandler
pub struct InterruptSignalHandler {
    /// `oldHandler`; `None` once uninstalled.
    old_handler: Arc<Mutex<Option<libc::sighandler_t>>>,
}

impl InterruptSignalHandler {
    /// Disables SIGINT handling.
    // Java: com/google/devtools/build/lib/server/signal/InterruptSignalHandler.java:45 InterruptSignalHandler.uninstall
    pub fn uninstall(&self) {
        uninstall(&self.old_handler);
    }
}

impl Drop for InterruptSignalHandler {
    /// `batchMain`'s `finally { signalHandler.uninstall(); }` (a no-op when the second interrupt
    /// already reverted to the default handler).
    fn drop(&mut self) {
        uninstall(&self.old_handler);
    }
}

fn uninstall(old_handler: &Mutex<Option<libc::sighandler_t>>) {
    if let Some(old) = old_handler.lock().take() {
        // SAFETY: restores the disposition that was in place before `capture_sigint`.
        unsafe {
            libc::signal(libc::SIGINT, old);
        }
    }
}

/// Installs the SIGINT handler of batch mode: every SIGINT prints `Bazel received an interrupt`
/// and interrupts the command; the second one reverts to the previous handler, so that a third
/// one kills the process.
///
/// PORT: the `interrupt-watcher` thread only writes log lines and thread dumps to the server log
/// every 10 seconds (`ThreadUtils.warnAboutSlowInterrupt`); Blaze has no server logger, so it is
/// not started.
// Java: com/google/devtools/build/lib/runtime/BlazeRuntime.java:1045 BlazeRuntime.captureSigint
pub fn capture_sigint() -> InterruptSignalHandler {
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
    let old_handler = Arc::new(Mutex::new(Some(old)));

    let watcher_old_handler = old_handler.clone();
    std::thread::Builder::new()
        .name("interrupt-signal-handler".to_owned())
        .spawn(move || {
            // AtomicInteger numInterrupts
            let mut num_interrupts = 0;
            loop {
                let mut byte: u8 = 0;
                // SAFETY: reads one byte into a valid buffer.
                let n = unsafe { libc::read(read_fd, (&raw mut byte).cast::<libc::c_void>(), 1) };
                if n < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                if n <= 0 {
                    return;
                }
                // InterruptSignalHandler.run
                OutErr::system_out_err().print_err_ln("Bazel received an interrupt");
                interrupt_command_thread();

                num_interrupts += 1;
                if num_interrupts == 2 {
                    // Second --batch interrupt: Reverting to JVM SIGINT handler
                    uninstall(&watcher_old_handler);
                }
            }
        })
        .unwrap_or_else(|e| panic!("OutOfMemoryError: unable to create native thread: {e}"));

    InterruptSignalHandler { old_handler }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Thread.interrupt() semantics: the interrupt status is sticky, so an evaluator created after
    // the interrupt is interrupted as well.
    #[test]
    fn interrupt_reaches_registered_and_later_flags() {
        let command_thread = CommandThread::new();
        let before = Arc::new(AtomicBool::new(false));
        command_thread.register_interrupt_flag(&before);
        assert!(!before.load(Ordering::SeqCst));

        command_thread.interrupt();
        assert!(before.load(Ordering::SeqCst));

        let after = Arc::new(AtomicBool::new(false));
        command_thread.register_interrupt_flag(&after);
        assert!(after.load(Ordering::SeqCst));
    }

    #[test]
    fn dropped_evaluators_are_forgotten() {
        let command_thread = CommandThread::new();
        let dropped = Arc::new(AtomicBool::new(false));
        command_thread.register_interrupt_flag(&dropped);
        drop(dropped);
        let live = Arc::new(AtomicBool::new(false));
        command_thread.register_interrupt_flag(&live);
        assert_eq!(command_thread.interrupt_flags.lock().len(), 1);
    }
}
