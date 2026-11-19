//! The remote output tree of `RemoteActionFileSystem`, as far as the execution phase reads it:
//! the metadata of action outputs that a remote cache hit or a remote execution left in the
//! cache instead of writing them to disk (`RemoteActionFileSystem.injectRemoteFile`), which the
//! output metadata store of the action reports (`ActionOutputMetadataStore` stats outputs
//! through the action file system, whose `statInternal` asks the remote output tree before the
//! local file system).
//!
//! PORT: Java creates a `RemoteActionFileSystem` (with its own in-memory remote output tree) for
//! every action through `RemoteOutputService.createActionFileSystem`. Blaze has no action file
//! systems: one tree serves the build, keyed by the absolute output path, and the executor drops
//! the entries of an action's outputs before the action runs, so an action only ever sees the
//! files injected during its own execution, as with a fresh per-action file system.
//! [`RemoteActionFileSystem`] is the read view of it that the spawn log uses; reading the
//! contents of remote files, writes and prefetching are not ported:
//! `--remote_download_outputs=all` keeps every output on disk except in-memory
//! outputs (`ExecutionRequirements.REMOTE_EXECUTION_INLINE_OUTPUTS`, e.g. the `.d` files of
//! C++ compiles), which Java does not write to disk either.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::Arc;

use blaze_exec::actions::FileArtifactValue;
use blaze_vfs::{DigestHashFunction, Dirent, FileStatus, FileSystem, FileSystemRef, PathFragment};
use parking_lot::Mutex;

/// The injected remote outputs of the running actions.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
#[derive(Debug, Default)]
pub struct RemoteOutputTree {
    files: Mutex<HashMap<PathFragment, FileArtifactValue>>,
}

impl RemoteOutputTree {
    pub fn new() -> RemoteOutputTree {
        RemoteOutputTree::default()
    }

    /// `remoteOutputTree.injectFile(path, metadata)`; `path` is absolute.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:317 RemoteActionFileSystem.injectRemoteFile
    pub fn inject_file(&self, path: PathFragment, metadata: FileArtifactValue) {
        self.files.lock().insert(path, metadata);
    }

    /// The metadata injected for the absolute `path`, if any (`remoteOutputTree.statIfFound`
    /// as a `FileStatusWithMetadata`).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:697 RemoteActionFileSystem.statInternal
    pub fn get(&self, path: &PathFragment) -> Option<FileArtifactValue> {
        self.files.lock().get(path).cloned()
    }

    /// Forgets the file at the absolute `path` (the action that declares it is about to run
    /// with a fresh action file system).
    pub fn remove(&self, path: &PathFragment) {
        self.files.lock().remove(path);
    }
}

/// The view of an action's `RemoteActionFileSystem` that readers of its outputs use (the spawn
/// log reads the outputs of a spawn through `ActionExecutionContext.getActionFileSystem()`): a
/// file of the remote output tree is a regular file with the injected digest and size, every
/// other path is the local file system's.
///
/// PORT: only the reads of `RemoteActionFileSystem` that answer from the remote output tree are
/// ported (`statInternal`'s second source, `getDigest`/`getFastDigest`/`getFileSize` from the
/// metadata); the first source (the action's input metadata), symlink canonicalization within
/// the remote tree and every write are the local file system's.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:106 RemoteActionFileSystem
#[derive(Debug)]
pub struct RemoteActionFileSystem {
    delegate: FileSystemRef,
    remote_output_tree: Arc<RemoteOutputTree>,
}

impl RemoteActionFileSystem {
    pub fn new(
        delegate: FileSystemRef,
        remote_output_tree: Arc<RemoteOutputTree>,
    ) -> RemoteActionFileSystem {
        RemoteActionFileSystem {
            delegate,
            remote_output_tree,
        }
    }

    /// `remoteOutputTree.statIfFound(path)`: the metadata of a remote output.
    fn remote(&self, path: &PathFragment) -> Option<FileArtifactValue> {
        self.remote_output_tree.get(path)
    }

    /// `statFromMetadata(m)` for a remote file.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:709 RemoteActionFileSystem.statFromMetadata
    fn stat_from_metadata(m: &FileArtifactValue) -> FileStatus {
        // A regular file; `getLastModifiedTime` is 0 (a remote file has no modified time), the
        // node id is not used by the readers.
        FileStatus::new(
            libc::S_IFREG | 0o555,
            /* mtime= */ 0,
            /* ctime= */ 0,
            m.get_size(),
            /* ino= */ 0,
        )
    }
}

impl FileSystem for RemoteActionFileSystem {
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:667 RemoteActionFileSystem.statInternal
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        match self.remote(path) {
            Some(m) => Ok(RemoteActionFileSystem::stat_from_metadata(&m)),
            None => self.delegate.stat(path, follow_symlinks),
        }
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.remote(path).is_some() || self.delegate.exists(path, follow_symlinks)
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        match self.remote(path) {
            Some(m) => Ok(m.get_size()),
            None => self.delegate.get_file_size(path, follow_symlinks),
        }
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        match self.remote(path) {
            Some(_) => Ok(0),
            None => self.delegate.get_last_modified_time(path, follow_symlinks),
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:463 RemoteActionFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        match self.remote(path) {
            Some(m) => Ok(m.get_digest().map(<[u8]>::to_vec)),
            None => self.delegate.get_fast_digest(path),
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:476 RemoteActionFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        match self
            .remote(path)
            .and_then(|m| m.get_digest().map(<[u8]>::to_vec))
        {
            Some(digest) => Ok(digest),
            None => self.delegate.get_digest(path),
        }
    }

    // DelegateFileSystem.
    fn get_digest_function(&self) -> DigestHashFunction {
        self.delegate.get_digest_function()
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.delegate.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.delegate.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.create_directory_and_parents(path)
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.delete(path)
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.delegate.set_last_modified_time(path, new_time)
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.read_symbolic_link(path)
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        self.delegate.get_directory_entries(path)
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.delegate.readdir(path, follow_symlinks)
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_readable(path)
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.delegate.set_readable(path, readable)
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_writable(path)
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.delegate.set_writable(path, writable)
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_executable(path)
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.delegate.set_executable(path, executable)
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        if self.remote(path).is_some() {
            // Java downloads the file first (`downloadIfRemote`, through the action input
            // fetcher), which is not ported.
            blaze_util::unported!(
                "RemoteActionFileSystem.getInputStream (download of a remote-only file)",
                "{}",
                path.get_path_string()
            );
        }
        self.delegate.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.delegate.get_output_stream(path, append, internal)
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        self.delegate.rename_to(source_path, target_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}
