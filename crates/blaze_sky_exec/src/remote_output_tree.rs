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
//! [`RemoteActionFileSystem`] is the view of it that actions get as their action file system
//! (in-process actions resolve their inputs through it, the spawn log reads outputs through it).
//!
//! PORT: the first source of Java's `statInternal`, the input metadata of the action, is per
//! action; the build-wide tree stands in for it by also holding the remote outputs of actions
//! that hit the action cache (`ActionOutputMetadataStore.injectFile`/`injectTree` with remote
//! metadata), so that every remote file of the output tree is known to it.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::sync::Arc;

use blaze_exec::actions::action_input_prefetcher::{ActionInputPrefetcher, Priority, Reason};
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, FileArtifactValue, FilesetOutputTree, InputMetadataProvider,
    RunfilesArtifactValue, TreeArtifactValue,
};
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemRef, PathFragment,
};
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

    /// `remoteOutputTree.getPath(path).delete()` of a file: whether there was one.
    pub fn delete(&self, path: &PathFragment) -> bool {
        self.files.lock().remove(path).is_some()
    }

    /// `remoteOutputTree.renameTo(src, dst)`: moves the file at `src`, or every file under the
    /// directory `src`; `false` when nothing is there (Java's `FileNotFoundException`).
    pub fn rename_to(&self, src: &PathFragment, dst: &PathFragment) -> bool {
        let mut files = self.files.lock();
        let moved: Vec<PathFragment> = files
            .keys()
            .filter(|p| p.starts_with(src))
            .cloned()
            .collect();
        for path in &moved {
            let metadata = files.remove(path).expect("listed above");
            files.insert(dst.get_relative(&path.relative_to(src)), metadata);
        }
        !moved.is_empty()
    }

    /// The entries of the directory `dir` in the remote output tree, by name, with whether each
    /// is a directory (a remote file's parent directories exist in the tree); `None` when no
    /// file was injected under `dir`.
    pub fn directory_entries(&self, dir: &PathFragment) -> Option<BTreeMap<String, bool>> {
        let files = self.files.lock();
        let mut entries: Option<BTreeMap<String, bool>> = None;
        for path in files.keys() {
            if !path.starts_with(dir) || path == dir {
                continue;
            }
            let relative = path.relative_to(dir);
            let segments: Vec<&str> = relative.segments().collect();
            let entries = entries.get_or_insert_with(BTreeMap::new);
            let is_dir = segments.len() > 1;
            let e = entries.entry(segments[0].to_string()).or_insert(is_dir);
            *e |= is_dir;
        }
        entries
    }

    /// The files injected under the absolute directory `dir`, with their paths relative to it
    /// (what the remote output tree's `readdir` contributes to a tree artifact's listing).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:835 RemoteActionFileSystem.getDirectoryContents
    pub fn files_under(&self, dir: &PathFragment) -> Vec<(PathFragment, FileArtifactValue)> {
        self.files
            .lock()
            .iter()
            .filter(|(path, _)| path.starts_with(dir) && *path != dir)
            .map(|(path, metadata)| (path.relative_to(dir), metadata.clone()))
            .collect()
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
pub struct RemoteActionFileSystem {
    delegate: FileSystemRef,
    exec_root: PathFragment,
    remote_output_tree: Arc<RemoteOutputTree>,
    /// `inputFetcher`: the remote module's `RemoteActionInputFetcher`.
    input_fetcher: Option<Arc<dyn ActionInputPrefetcher>>,
}

impl std::fmt::Debug for RemoteActionFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionFileSystem")
            .field("exec_root", &self.exec_root)
            .finish_non_exhaustive()
    }
}

impl RemoteActionFileSystem {
    pub fn new(
        delegate: FileSystemRef,
        exec_root: PathFragment,
        remote_output_tree: Arc<RemoteOutputTree>,
        input_fetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    ) -> RemoteActionFileSystem {
        RemoteActionFileSystem {
            delegate,
            exec_root,
            remote_output_tree,
            input_fetcher,
        }
    }

    /// `path` canonicalized through the local symlinks (`pathCanonicalizer
    /// .resolveSymbolicLinks`): each component that is a local symlink is replaced by its
    /// target; components that do not exist locally (remote directories and files) are kept.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:326 RemoteActionFileSystem.resolveSymbolicLinks
    fn canonicalize(&self, path: &PathFragment) -> io::Result<PathFragment> {
        let mut hops = 0;
        let mut pending: Vec<String> = path.segments().map(str::to_string).collect();
        pending.reverse();
        let mut current = PathFragment::create("/");
        while let Some(segment) = pending.pop() {
            let candidate = current.get_child(&segment);
            match self.delegate.stat(&candidate, /* follow_symlinks= */ false) {
                Ok(stat) if stat.is_symbolic_link() => {
                    hops += 1;
                    if hops > 32 {
                        return Err(io::Error::other(format!(
                            "{} (Too many levels of symbolic links)",
                            path.get_path_string()
                        )));
                    }
                    let target = self.delegate.read_symbolic_link(&candidate)?;
                    let resolved = if target.is_absolute() {
                        target
                    } else {
                        current.get_relative(&target)
                    };
                    let mut segments: Vec<String> =
                        resolved.segments().map(str::to_string).collect();
                    segments.reverse();
                    pending.extend(segments);
                    current = PathFragment::create("/");
                }
                _ => current = candidate,
            }
        }
        Ok(current)
    }

    /// The metadata of the remote file `path` resolves to, following the local symlinks on the
    /// way (`isRemote(path)`: `statInternal(path, FOLLOW_ALL, IN_MEMORY_ONLY)`), with the
    /// canonical path.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:297 RemoteActionFileSystem.isRemote
    fn remote_following_symlinks(
        &self,
        path: &PathFragment,
    ) -> io::Result<Option<(PathFragment, FileArtifactValue)>> {
        if let Some(m) = self.remote(path) {
            return Ok(m.is_remote().then_some((path.clone(), m)));
        }
        let path = self.canonicalize(path)?;
        Ok(self
            .remote(&path)
            .filter(FileArtifactValue::is_remote)
            .map(|m| (path, m)))
    }

    /// Downloads the file at `path` if it is remote.
    ///
    /// PORT: Java looks the input up in the action's input metadata
    /// (`inputArtifactData.getInput(execPath)`) and does nothing for a path that is not an input;
    /// the build-wide tree has no per-action inputs, so the file is fetched as a plain input
    /// with the tree's metadata.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:397 RemoteActionFileSystem.downloadIfRemote
    fn download_if_remote(&self, path: &PathFragment) -> io::Result<()> {
        let Some((path, metadata)) = self.remote_following_symlinks(path)? else {
            return Ok(());
        };
        if !path.starts_with(&self.exec_root) {
            return Ok(());
        }
        let Some(input_fetcher) = &self.input_fetcher else {
            blaze_util::unported!(
                "RemoteActionFileSystem.downloadIfRemote (no RemoteActionInputFetcher)",
                "{}",
                path.get_path_string()
            );
        };
        let exec_path = path.relative_to(&self.exec_root);
        let input = ActionInputRef::new(BasicActionInput::from_path_fragment(exec_path));
        let provider = SingleInputMetadata {
            input: input.clone(),
            metadata,
        };
        input_fetcher.prefetch_files(
            None,
            &[input],
            &provider,
            Priority::CRITICAL,
            Reason::INPUTS,
        )
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
        if let Some(m) = self.remote(path) {
            return Ok(RemoteActionFileSystem::stat_from_metadata(&m));
        }
        // A local symlink (created in-process, e.g. by a SymlinkAction) to a remote file.
        if follow_symlinks && let Some((_, m)) = self.remote_following_symlinks(path)? {
            return Ok(RemoteActionFileSystem::stat_from_metadata(&m));
        }
        match self.delegate.stat(path, follow_symlinks) {
            // A directory of the remote output tree that does not exist locally (possibly
            // reached through a symlinked directory).
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && self
                        .remote_output_tree
                        .directory_entries(&self.canonicalize(path)?)
                        .is_some() =>
            {
                Ok(FileStatus::new(libc::S_IFDIR | 0o555, 0, 0, 0, 0))
            }
            r => r,
        }
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat(path, follow_symlinks).is_ok()
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
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:357 RemoteActionFileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        let deleted = self.delegate.delete(path)?;
        // The remote output tree only holds outputs (`isOutput(path)`).
        Ok(self.remote_output_tree.delete(path) || deleted)
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
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:826 RemoteActionFileSystem.getDirectoryEntries
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        Ok(self
            .readdir(path, /* follow_symlinks= */ false)?
            .into_iter()
            .map(|d| d.get_name().to_string())
            .collect())
    }
    /// The sorted union of the local and the remote entries (`getDirectoryContents`).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:835 RemoteActionFileSystem.getDirectoryContents
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        let Some(remote) = self
            .remote_output_tree
            .directory_entries(&self.canonicalize(path)?)
        else {
            return self.delegate.readdir(path, follow_symlinks);
        };
        let mut entries: BTreeMap<String, Dirent> = BTreeMap::new();
        match self.delegate.readdir(path, follow_symlinks) {
            Ok(local) => {
                for d in local {
                    entries.insert(d.get_name().to_string(), d);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        for (name, is_dir) in remote {
            let dirent_type = if is_dir {
                DirentType::Directory
            } else {
                DirentType::File
            };
            entries.insert(name.clone(), Dirent::new(name, dirent_type));
        }
        Ok(entries.into_values().collect())
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
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:380 RemoteActionFileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        // PORT: a lost input (`BulkTransferException.getLostArtifacts`) is recorded for action
        // rewinding in Java; without rewinding the exception is thrown as it is.
        self.download_if_remote(path)?;
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
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:773 RemoteActionFileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        let remote_renamed = self.remote_output_tree.rename_to(source_path, target_path);
        match self.delegate.rename_to(source_path, target_path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound && remote_renamed => Ok(()),
            r => r,
        }
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

/// The metadata of the one file `downloadIfRemote` fetches.
struct SingleInputMetadata {
    input: ActionInputRef,
    metadata: FileArtifactValue,
}

impl InputMetadataProvider for SingleInputMetadata {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        Ok((input.get_exec_path() == self.input.get_exec_path()).then(|| self.metadata.clone()))
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        (exec_path == self.input.get_exec_path()).then(|| self.input.clone())
    }
}
