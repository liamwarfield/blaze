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
    let _ = std::panic::catch_unwind(|| {
        begin_shutdown();
        run_shutdown_hooks();
    });
}

/// The thread running the shutdown (`Shutdown.exit` holds the `Shutdown` class lock while the
/// hooks run and then halts; a second `exit`, from another thread or a signal, blocks for good).
static SHUTDOWN_OWNER: parking_lot::Mutex<Option<std::thread::ThreadId>> =
    parking_lot::Mutex::new(None);
static SHUTDOWN_STARTED: parking_lot::Condvar = parking_lot::Condvar::new();

/// Enters the process's shutdown: returns at once on the thread that already runs it, and
/// blocks forever on any other thread, so that the process cannot end (or start a second
/// shutdown) while the hooks of the first one are still running.
// Java: java/lang/Shutdown.java Shutdown.exit (JDK)
pub fn begin_shutdown() {
    let me = std::thread::current().id();
    let mut owner = SHUTDOWN_OWNER.lock();
    loop {
        match *owner {
            None => {
                *owner = Some(me);
                return;
            }
            Some(o) if o == me => return,
            Some(_) => SHUTDOWN_STARTED.wait(&mut owner),
        }
    }
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

/// The files of `File.deleteOnExit()`, in registration order (`DeleteOnExitHook.files`, a
/// `LinkedHashSet`).
static DELETE_ON_EXIT: LazyLock<Mutex<indexmap::IndexSet<std::path::PathBuf>>> =
    LazyLock::new(|| Mutex::new(indexmap::IndexSet::new()));

/// `File.deleteOnExit()`: the file or empty directory is deleted when the process shuts down,
/// after the shutdown hooks (`DeleteOnExitHook` is registered in a later `Shutdown` slot than
/// the application hooks). Panics with Java's `IllegalStateException` text once shutdown is in
/// progress.
// Java: java/io/DeleteOnExitHook.java DeleteOnExitHook.add (JDK)
pub fn delete_on_exit(path: std::path::PathBuf) {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        panic!("java.lang.IllegalStateException: Shutdown in progress");
    }
    if !INSTALLED.swap(true, Ordering::SeqCst) {
        // SAFETY: registering a plain extern "C" function.
        unsafe {
            libc::atexit(run_hooks_at_exit);
        }
    }
    DELETE_ON_EXIT.lock().insert(path);
}

/// `DeleteOnExitHook.runHooks`: deletes the files in reverse order of registration, ignoring
/// failures (`File.delete()`).
// Java: java/io/DeleteOnExitHook.java DeleteOnExitHook.runHooks (JDK)
fn run_delete_on_exit() {
    let files: Vec<std::path::PathBuf> = std::mem::take(&mut *DELETE_ON_EXIT.lock())
        .into_iter()
        .rev()
        .collect();
    for file in files {
        let _ = match std::fs::symlink_metadata(&file) {
            Ok(m) if m.is_dir() => std::fs::remove_dir(&file),
            Ok(_) => std::fs::remove_file(&file),
            Err(e) => Err(e),
        };
    }
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
    // Each Java hook is a thread of its own: an exception in one does not keep the others from
    // running; it reaches the uncaught exception handler. Here a panicking hook is caught, the
    // rest run, then the panics are reported.
    let mut panics = Vec::new();
    for hook in hooks {
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook())) {
            panics.push(payload);
        }
    }
    for payload in panics {
        report_hook_panic(&*payload);
    }
    // The next `Shutdown` slot: `DeleteOnExitHook`.
    run_delete_on_exit();
}

/// An uncaught exception of a shutdown hook's thread: the default uncaught exception handler.
pub fn report_hook_panic(payload: &(dyn std::any::Any + Send)) {
    match blaze_util::uncaught_exception::get_default_uncaught_exception_handler() {
        Some(handler) => handler(
            &blaze_util::uncaught_exception::panic_message(payload),
            blaze_util::uncaught_exception::CrashContextKind::Halt,
        ),
        None => eprintln!(
            "Exception in thread \"Thread-shutdown-hook\": {}",
            blaze_util::uncaught_exception::panic_message(payload)
        ),
    }
}
