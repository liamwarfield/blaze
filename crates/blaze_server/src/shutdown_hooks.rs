//! `ShutdownHooks`: wrapper for common shutdown operations.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use blaze_vfs::Path;
use parking_lot::Mutex;

use crate::pid_file_watcher::PidFileWatcher;

/// The hooks registered with [`ShutdownHooks::create_and_register`]: the stand-in for the JVM's
/// `Runtime.addShutdownHook` list.
static REGISTERED: LazyLock<Mutex<Vec<Arc<ShutdownHooks>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// Runs the hooks registered with [`ShutdownHooks::create_and_register`], as the JVM does on an
/// orderly exit.
///
/// PORT: Rust has no shutdown hooks. The process's `main` (the lead's `BlazeRuntime.serverMain`
/// caller, and the probe binary) calls this once before it exits normally; like the JVM's
/// hooks they do not run on `Runtime.halt` (`libc::_exit`), a crash or SIGKILL.
pub fn run_registered_shutdown_hooks() {
    let hooks: Vec<Arc<ShutdownHooks>> = std::mem::take(&mut *REGISTERED.lock());
    for hooks in hooks {
        hooks.run_hooks();
    }
}

struct Locked {
    files_to_delete_at_exit: Vec<Path>,
    pid_file_cleanup: Option<(Path, Arc<PidFileWatcher>)>,
}

/// Wrapper for common shutdown operations.
// Java: com/google/devtools/build/lib/server/ShutdownHooks.java:29 ShutdownHooks
pub struct ShutdownHooks {
    enabled: AtomicBool,
    locked: Mutex<Locked>,
}

impl ShutdownHooks {
    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:32 ShutdownHooks.createAndRegister
    pub fn create_and_register() -> Arc<ShutdownHooks> {
        let result = Arc::new(ShutdownHooks::new());
        REGISTERED.lock().push(result.clone());
        result
    }

    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:39 ShutdownHooks.createUnregistered
    pub fn create_unregistered() -> Arc<ShutdownHooks> {
        Arc::new(ShutdownHooks::new())
    }

    fn new() -> ShutdownHooks {
        ShutdownHooks {
            enabled: AtomicBool::new(true),
            locked: Mutex::new(Locked {
                files_to_delete_at_exit: Vec::new(),
                pid_file_cleanup: None,
            }),
        }
    }

    /// Schedules the specified file for (attempted) deletion at exit.
    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:54 ShutdownHooks.deleteAtExit
    pub fn delete_at_exit(&self, path: Path) {
        self.locked.lock().files_to_delete_at_exit.push(path);
    }

    /// Registers a cleanup operation for the given PID file.
    ///
    /// The PID file watcher will be stopped before the PID file is deleted.
    ///
    /// This operation will be executed when the process shuts down.
    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:65 ShutdownHooks.cleanupPidFile
    pub fn cleanup_pid_file(&self, pid_file: Path, pid_file_watcher: Arc<PidFileWatcher>) {
        self.locked.lock().pid_file_cleanup = Some((pid_file, pid_file_watcher));
    }

    /// Disables shutdown hook execution.
    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:78 ShutdownHooks.disable
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:83 ShutdownHooks.runHooks
    pub fn run_hooks(&self) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }

        let files = {
            let locked = self.locked.lock();
            if let Some((pid_file, pid_file_watcher)) = &locked.pid_file_cleanup {
                // Java: com/google/devtools/build/lib/server/ShutdownHooks.java:66 ShutdownHooks.cleanupPidFile (the lambda)
                pid_file_watcher.end_watch();
                if let Err(e) = pid_file.delete() {
                    print_stack(&e);
                }
            }
            locked.files_to_delete_at_exit.clone()
        };
        for path in files {
            if let Err(e) = path.delete() {
                print_stack(&e);
            }
        }
    }
}

// Java: com/google/devtools/build/lib/server/ShutdownHooks.java:104 ShutdownHooks.printStack
fn print_stack(e: &std::io::Error) {
    // Hopefully this never happens. It's not very nice to just write this to the user's console,
    // but I'm not sure what better choice we have.
    // PORT: `e.printStackTrace` is the exception's first line (Blaze has no Java stack).
    let err = format!(
        "=======[BAZEL SERVER: ENCOUNTERED IO EXCEPTION]=======\njava.io.IOException: {e}\n\
         =====================================================\n"
    );
    blaze_util::server_logger::severe(
        "com.google.devtools.build.lib.server.ShutdownHooks",
        "printStack",
        &err,
        None,
    );
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/server/ShutdownHooksTest.java
    use super::*;
    use blaze_vfs::{DigestHashFunction, UnixFileSystem};

    fn scratch() -> (tempfile::TempDir, blaze_vfs::FileSystemRef) {
        (
            tempfile::tempdir().unwrap(),
            UnixFileSystem::create(DigestHashFunction::Sha256),
        )
    }

    #[test]
    fn test_deletes_registered_paths() {
        let (dir, fs) = scratch();
        let root = dir.path().to_str().unwrap();
        let to_delete = Path::create_from_str(&format!("{root}/some-path-to-delete"), &fs);
        to_delete.create_directory_and_parents().unwrap();

        let to_keep = Path::create_from_str(&format!("{root}/some-path-to-keep"), &fs);
        to_keep.create_directory_and_parents().unwrap();

        let under_test = ShutdownHooks::create_unregistered();
        under_test.delete_at_exit(to_delete.clone());
        under_test.run_hooks();

        assert!(!to_delete.exists());
        assert!(to_keep.exists());
    }

    #[test]
    fn test_skip_hooks_if_disabled() {
        let (dir, fs) = scratch();
        let root = dir.path().to_str().unwrap();
        let to_delete = Path::create_from_str(&format!("{root}/some-path-to-delete"), &fs);
        to_delete.create_directory_and_parents().unwrap();

        let under_test = ShutdownHooks::create_unregistered();
        under_test.delete_at_exit(to_delete.clone());
        under_test.disable();
        under_test.run_hooks();

        assert!(to_delete.exists());
    }
}
