//! Port of `lib/sandbox/AsynchronousTreeDeleter.java`.
//!
//! PORT: the deletions run synchronously in the calling thread (the task allows it). Java moves
//! the tree to `<trashBase>/<n>` and deletes it on a background pool; here the tree is moved the
//! same way and then deleted at once, so the file system ends up in the state Java reaches when
//! its pool drains.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use blaze_vfs::Path;

use crate::exec::tree_deleter::TreeDeleter;

/// `AsynchronousTreeDeleter.MOVED_TRASH_DIR`.
// Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:44 AsynchronousTreeDeleter.MOVED_TRASH_DIR
pub const MOVED_TRASH_DIR: &str = "_moved_trash_dir";

/// Executes file system tree deletions "asynchronously" (synchronously, see the module docs).
// Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:42 AsynchronousTreeDeleter
#[derive(Debug)]
pub struct AsynchronousTreeDeleter {
    trash_count: AtomicI32,
    trash_base: Path,
    trash_base_created: AtomicBool,
    shut_down: AtomicBool,
}

impl AsynchronousTreeDeleter {
    /// Constructs a new asynchronous tree deleter.
    // Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:58 AsynchronousTreeDeleter.<init>
    pub fn new(trash_base: Path) -> AsynchronousTreeDeleter {
        AsynchronousTreeDeleter {
            trash_count: AtomicI32::new(0),
            trash_base,
            trash_base_created: AtomicBool::new(false),
            shut_down: AtomicBool::new(false),
        }
    }

    /// Resizes the thread pool to the given number of threads (no pool here).
    // Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:84 AsynchronousTreeDeleter.setThreads
    pub fn set_threads(&self, threads: i32) {
        blaze_util::check_state!(
            threads > 0,
            "Use SynchronousTreeDeleter if no async behavior is desired"
        );
        blaze_util::check_state!(
            !self.shut_down.load(Ordering::SeqCst),
            "Cannot call setThreads after shutdown"
        );
    }

    /// `getTrashBase()`.
    pub fn get_trash_base(&self) -> &Path {
        &self.trash_base
    }
}

impl TreeDeleter for AsynchronousTreeDeleter {
    // Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:91 AsynchronousTreeDeleter.deleteTree
    fn delete_tree(&self, path: &Path) -> std::io::Result<()> {
        if !self.trash_base_created.load(Ordering::SeqCst) {
            self.trash_base.create_directory()?;
            self.trash_base_created.store(true, Ordering::SeqCst);
        }
        if !path.exists() {
            return Ok(());
        }
        let trash_path = self
            .trash_base
            .get_relative_str(&self.trash_count.fetch_add(1, Ordering::SeqCst).to_string());
        if path.rename_to(&trash_path).is_err() {
            // PORT: skipped, tracing (Java log) ("Failed to rename %s -> %s for asynchronous removal.
            // Removing synchronously.").
            return path.delete_tree();
        }
        blaze_util::check_state!(
            !self.shut_down.load(Ordering::SeqCst),
            "Cannot call deleteTree after shutdown"
        );
        // PORT: skipped, tracing (Java log) (on error: "Failed to delete tree %s asynchronously").
        let _ = trash_path.delete_tree();
        Ok(())
    }

    // Java: com/google/devtools/build/lib/sandbox/AsynchronousTreeDeleter.java:122 AsynchronousTreeDeleter.shutdown
    fn shutdown(&self) {
        self.shut_down.store(true, Ordering::SeqCst);
    }
}
