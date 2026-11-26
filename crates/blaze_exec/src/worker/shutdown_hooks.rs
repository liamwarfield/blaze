//! `Runtime.getRuntime().addShutdownHook(thread)` / `removeShutdownHook(thread)` for the worker
//! processes: `SingleplexWorker` and `WorkerMultiplexer` register a hook that destroys their
//! process when the JVM shuts down, "to avoid creating zombie processes".
//!
//! PORT: Blaze has no JVM. The hooks run from an `atexit` handler, which `std::process::exit`
//! and a return from `main` both reach, and from the terminating-signal path of the server
//! (`blaze_runtime::jvm_signals`, which ends with `_exit`). In-process mode runs one command per
//! process, so every worker of a command ends here; a server keeps its workers until it shuts
//! down, as in Java.

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

/// Runs every registered hook once, newest first (Java starts them concurrently, in no
/// particular order; the order here is a declared deviation, see the note below).
pub fn run_shutdown_hooks() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    // PORT: the JVM starts all hooks at once, in no order. Here they run one after the other,
    // latest first, so that a worker's hook (registered when its process starts) ends the worker
    // before the hook of `VirtualCgroup.deleteInstance` (registered when the server's cgroup is
    // made) removes the cgroup the worker was in; a cgroup with a process cannot be removed.
    let hooks: Vec<Hook> = std::mem::take(&mut *HOOKS.lock())
        .into_values()
        .rev()
        .collect();
    for hook in hooks {
        hook();
    }
}
