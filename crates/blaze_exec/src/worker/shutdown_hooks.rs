//! `Runtime.getRuntime().addShutdownHook(thread)` / `removeShutdownHook(thread)` for the worker
//! processes: `SingleplexWorker` and `WorkerMultiplexer` register a hook that destroys their
//! process when the JVM shuts down, "to avoid creating zombie processes".
//!
//! PORT: Blaze has no JVM. The hooks run from an `atexit` handler, which `std::process::exit`
//! and a return from `main` both reach. Blaze has no persistent server yet, so this is where every
//! worker of a command ends; with a server it is where they end at server shutdown, as in Java.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;

type Hook = Arc<dyn Fn() + Send + Sync>;

static HOOKS: LazyLock<Mutex<BTreeMap<u64, Hook>>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static INSTALLED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn run_hooks_at_exit() {
    // A panic must not unwind out of an `atexit` handler.
    let _ = std::panic::catch_unwind(run_shutdown_hooks);
}

/// `process.destroyAndWait()` as a hook may run it: during `exit`, after the thread-local
/// interrupt state of the calling thread may be gone, so the wait polls `isAlive()` instead of
/// using the interruptible `waitFor()`.
///
/// PORT: Java waits without a limit; after 10s the process is killed with SIGKILL so that a
/// worker ignoring SIGTERM cannot keep Blaze from exiting.
pub fn destroy_and_wait_at_exit(process: &dyn blaze_shell::Subprocess) {
    process.destroy();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while process.is_alive() {
        if std::time::Instant::now() >= deadline {
            let pid = process.get_process_id() as libc::pid_t;
            if pid > 0 {
                // SAFETY: plain kill(2).
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// A registered hook; [`remove_shutdown_hook`] takes it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownHookId(u64);

/// `Runtime.getRuntime().addShutdownHook(hook)`. Panics with Java's `IllegalStateException`
/// text once shutdown is in progress.
pub fn add_shutdown_hook(hook: Hook) -> ShutdownHookId {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        panic!("java.lang.IllegalStateException: Shutdown in progress");
    }
    if !INSTALLED.swap(true, Ordering::SeqCst) {
        // SAFETY: registering a plain extern "C" function.
        unsafe {
            libc::atexit(run_hooks_at_exit);
        }
    }
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    HOOKS.lock().insert(id, hook);
    ShutdownHookId(id)
}

/// `Runtime.getRuntime().removeShutdownHook(hook)`. Returns false once shutdown is in progress
/// (Java throws `IllegalStateException`, which the callers ignore).
pub fn remove_shutdown_hook(id: ShutdownHookId) -> bool {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return false;
    }
    HOOKS.lock().remove(&id.0).is_some()
}

/// Runs every registered hook once (in registration order; Java starts them concurrently, in no
/// particular order).
pub fn run_shutdown_hooks() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let hooks: Vec<Hook> = std::mem::take(&mut *HOOKS.lock()).into_values().collect();
    for hook in hooks {
        hook();
    }
}
