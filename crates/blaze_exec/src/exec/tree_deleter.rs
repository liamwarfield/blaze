//! Port of `lib/exec/TreeDeleter.java`.

use blaze_vfs::Path;

/// Executes tree deletions under different scheduling policies.
// Java: com/google/devtools/build/lib/exec/TreeDeleter.java:21 TreeDeleter
pub trait TreeDeleter: Send + Sync {
    /// `deleteTree(Path)`: does nothing if the path does not exist.
    fn delete_tree(&self, path: &Path) -> std::io::Result<()>;
    /// `shutdown()`.
    fn shutdown(&self);
}
