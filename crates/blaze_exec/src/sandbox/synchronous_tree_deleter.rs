//! Port of `lib/sandbox/SynchronousTreeDeleter.java`.

use blaze_vfs::Path;

use crate::exec::tree_deleter::TreeDeleter;

/// Deletes trees synchronously, in the calling thread.
// Java: com/google/devtools/build/lib/sandbox/SynchronousTreeDeleter.java:22 SynchronousTreeDeleter
#[derive(Debug, Default)]
pub struct SynchronousTreeDeleter;

impl TreeDeleter for SynchronousTreeDeleter {
    // Java: com/google/devtools/build/lib/sandbox/SynchronousTreeDeleter.java:25 SynchronousTreeDeleter.deleteTree
    fn delete_tree(&self, path: &Path) -> std::io::Result<()> {
        path.delete_tree()
    }
    fn shutdown(&self) {}
}
