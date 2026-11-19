//! Port of `lib/remote/RemoteActionFileSystem.java`: the action file system of
//! `RemoteOutputService` (build without the bytes).
//!
//! PORT: the class lives in `lib/remote`, but the action executor (`blaze_sky_exec`) creates it
//! for every action (`OutputService.createActionFileSystem`) and the in-process actions of this
//! crate read through it, so it lives here; `blaze_remote` (which depends on this crate) injects
//! into it and gives it its input fetcher through the [`ActionInputPrefetcher`] context.
//!
//! One instance per action execution (Java: `ActionExecutionFunction` creates one per action),
//! so the outputs a remote result injects are seen by that action only
//! ([`RemoteActionFileSystem::inject_remote_file`]); the outputs of other actions are reached
//! through the action's input metadata, never through another action's remote output tree.
//!
//! Every in-process reader of an action (a `SymlinkAction`'s executable check, template
//! expansion, the test strategy's `test.xml` check, ...) reaches its files through
//! `ActionExecutionContext.getInputPath` / `getPathResolver().convertPath`, which give a
//! [`Path`] on this file system: `stat` answers from the metadata of a remote-only file and
//! `getInputStream` downloads it first.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::{Arc, Weak};

use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemRef,
    NotASymlinkException, Path, PathFragment,
};

use crate::actions::{
    ActionExecutionMetadata, FileArtifactValue, FileStateType, InputMetadataProvider,
};
use crate::exec::action_execution_context::{ActionInputPrefetcher, RemoteOutputTree};

/// `FollowMode`.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:121 RemoteActionFileSystem.FollowMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // Java's names.
enum FollowMode {
    /// Canonicalize neither the parent nor the base name.
    FollowNone,
    /// Canonicalize the parent but not the base name.
    FollowParent,
    /// Canonicalize both the parent and the base name.
    FollowAll,
}

/// `StatSources`.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:131 RemoteActionFileSystem.StatSources
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatSources {
    /// Stat both the in-memory sources and the local file system.
    All,
    /// Stat only the in-memory sources (input metadata and the remote output tree).
    InMemoryOnly,
}

/// The result of `statInternal`: Java returns a `FileStatus`, which for the in-memory sources
/// is a `FileStatusWithMetadata` (the `FileArtifactValue` is kept here).
#[derive(Clone, Debug)]
enum Stat {
    /// `statFromMetadata(metadata)`, or a file of the remote output tree.
    Metadata(FileArtifactValue),
    /// `DIRECTORY_FILE_STATUS`, or a directory of the remote output tree.
    Directory,
    /// The local file system's status.
    Local(FileStatus),
}

impl Stat {
    fn is_symbolic_link(&self) -> bool {
        match self {
            Stat::Metadata(m) => m.get_type().is_symlink(),
            Stat::Directory => false,
            Stat::Local(s) => s.is_symbolic_link(),
        }
    }

    /// The `FileStatus` an outside caller sees.
    fn to_file_status(&self) -> FileStatus {
        match self {
            Stat::Metadata(m) => stat_from_metadata(m),
            Stat::Directory => directory_file_status(),
            Stat::Local(s) => *s,
        }
    }
}

/// `DIRECTORY_FILE_STATUS`: a directory, size 0, modified at 0.
///
/// PORT: Java's anonymous `FileStatus` has no permissions (`getPermissions()` is -1);
/// `blaze_vfs::FileStatus` carries a mode, so it is `r-xr-xr-x`.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:138 RemoteActionFileSystem.DIRECTORY_FILE_STATUS
fn directory_file_status() -> FileStatus {
    FileStatus::new(0o040_555, 0, 0, 0, 0)
}

/// `statFromMetadata(m)`.
///
/// PORT: Java's `FileStatusWithMetadata` has no permissions (`getPermissions()` is -1, and
/// `SymlinkAction` then asks `isExecutable()`, which is true for a file that is not local);
/// `blaze_vfs::FileStatus` carries a mode, so a file is `r-xr-xr-x`, which gives the same
/// answers. The modification time is `m.getModifiedTime()`, 0 where that is unsupported.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:709 RemoteActionFileSystem.statFromMetadata
fn stat_from_metadata(m: &FileArtifactValue) -> FileStatus {
    let type_ = m.get_type();
    let kind = if type_ == FileStateType::Directory {
        0o040_000
    } else if type_.is_symlink() {
        0o120_000
    } else if type_.is_special_file() {
        0o010_000
    } else {
        0o100_000
    };
    let mtime = match m {
        FileArtifactValue::Directory { mtime } => *mtime,
        _ => 0,
    };
    FileStatus::new(kind | 0o555, mtime, mtime, m.get_size(), 0)
}

fn file_not_found(path: &PathFragment) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} (No such file or directory)", path.get_path_string()),
    )
}

/// The action file system of build without the bytes: the local file system, with the metadata
/// of the action's inputs and of its own remote outputs layered on top.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:106 RemoteActionFileSystem
pub struct RemoteActionFileSystem {
    this: Weak<RemoteActionFileSystem>,
    exec_root: PathFragment,
    output_base: PathFragment,
    input_artifact_data: Arc<dyn InputMetadataProvider>,
    /// `TreeArtifactDirectoryCache`: the directories of the input tree artifacts, by exec path.
    input_tree_artifact_directory_cache: parking_lot::Mutex<TreeArtifactDirectoryCache>,
    /// `inputFetcher`; `None` only in tests that never read a remote file.
    input_fetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    local_fs: FileSystemRef,
    remote_output_tree: RemoteOutputTree,
    /// `action` (`updateContext`).
    action: parking_lot::Mutex<Option<Arc<dyn ActionExecutionMetadata>>>,
}

impl std::fmt::Debug for RemoteActionFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteActionFileSystem")
            .field("exec_root", &self.exec_root)
            .field("output_base", &self.output_base)
            .finish_non_exhaustive()
    }
}

/// `TreeArtifactDirectoryCache`: the contents of the intermediate subdirectories of tree artifact
/// inputs.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:188 RemoteActionFileSystem.TreeArtifactDirectoryCache
#[derive(Default)]
struct TreeArtifactDirectoryCache {
    cached_trees: HashSet<PathFragment>,
    dir_to_entries: HashMap<PathFragment, HashSet<(String, bool)>>,
}

impl RemoteActionFileSystem {
    /// `new RemoteActionFileSystem(localFs, execRootFragment, relativeOutputPath,
    /// inputArtifactData, inputFetcher)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:239 RemoteActionFileSystem.<init>
    pub fn new(
        local_fs: FileSystemRef,
        exec_root_fragment: PathFragment,
        relative_output_path: &str,
        input_artifact_data: Arc<dyn InputMetadataProvider>,
        input_fetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    ) -> Arc<RemoteActionFileSystem> {
        let output_base = exec_root_fragment.get_relative_str(relative_output_path);
        Arc::new_cyclic(|this| RemoteActionFileSystem {
            this: this.clone(),
            exec_root: exec_root_fragment,
            output_base,
            input_artifact_data,
            input_tree_artifact_directory_cache: parking_lot::Mutex::new(
                TreeArtifactDirectoryCache::default(),
            ),
            input_fetcher,
            local_fs,
            remote_output_tree: RemoteOutputTree::new(),
            action: parking_lot::Mutex::new(None),
        })
    }

    /// This file system as a [`FileSystemRef`].
    pub fn as_file_system(&self) -> FileSystemRef {
        self.this
            .upgrade()
            .expect("RemoteActionFileSystem used after it was dropped")
    }

    /// `getPath(fragment)`: the path on this file system.
    pub fn get_path(&self, fragment: &PathFragment) -> Path {
        Path::create(fragment.clone(), &self.as_file_system())
    }

    /// `getRemoteOutputTree()`: the outputs of this action that exist only remotely.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:283 RemoteActionFileSystem.getRemoteOutputTree
    pub fn get_remote_output_tree(&self) -> &RemoteOutputTree {
        &self.remote_output_tree
    }

    /// `getLocalFileSystem()`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:288 RemoteActionFileSystem.getLocalFileSystem
    pub fn get_local_file_system(&self) -> &FileSystemRef {
        &self.local_fs
    }

    /// `updateContext(action)`: the action the downloads of [`Self::download_if_remote`] are
    /// made for.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:304 RemoteActionFileSystem.updateContext
    pub fn update_context(&self, action: Arc<dyn ActionExecutionMetadata>) {
        *self.action.lock() = Some(action);
    }

    /// `isOutput(path)`: whether `path` is under the output base (`<execroot>/bazel-out`).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:768 RemoteActionFileSystem.isOutput
    fn is_output(&self, path: &PathFragment) -> bool {
        path.starts_with(&self.output_base)
    }

    /// `injectRemoteFile(path, digest, size, expirationTime, inMemoryOutput)`: records an output
    /// of this action that was not downloaded. A path outside the output base is ignored; a path
    /// that is not an output of this action is kept in this action's tree, which only this
    /// action's output metadata store reads, and only for its declared outputs.
    ///
    /// `expiration_time_millis` is the `Instant` as epoch milliseconds.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
    pub fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        expiration_time_millis: Option<i64>,
        in_memory_output: bool,
    ) -> io::Result<()> {
        if !self.is_output(path) {
            return Ok(());
        }
        let metadata = FileArtifactValue::create_for_remote_file_with_materialization_data(
            digest,
            size,
            /* location_index= */ 1,
            expiration_time_millis,
            in_memory_output,
        );
        // RemoteInMemoryFileSystem.injectFile: checkArgument(metadata.isRemote()).
        self.remote_output_tree.inject_file(path.clone(), metadata);
        Ok(())
    }

    /// `isRemote(path)`: whether the file at `path` (following symlinks) is known only by its
    /// remote metadata.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:297 RemoteActionFileSystem.isRemote
    pub fn is_remote(&self, path: &PathFragment) -> io::Result<bool> {
        // Files in the local filesystem are non-remote by definition, so stat only in-memory
        // sources.
        let status = self.stat_internal(path, FollowMode::FollowAll, StatSources::InMemoryOnly)?;
        Ok(matches!(status, Some(Stat::Metadata(m)) if m.is_remote()))
    }

    /// The metadata of the file at `path` when an in-memory source (the input metadata or the
    /// remote output tree) knows it: Java's `stat(...) instanceof FileStatusWithMetadata`, as
    /// `ActionOutputMetadataStore.fileArtifactValueFromArtifact` tests it. `None` for a local
    /// file, a directory or a missing file.
    pub fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileArtifactValue>> {
        let follow_mode = if follow_symlinks {
            FollowMode::FollowAll
        } else {
            FollowMode::FollowParent
        };
        Ok(
            match self.stat_internal(path, follow_mode, StatSources::InMemoryOnly)? {
                Some(Stat::Metadata(m)) => Some(m),
                _ => None,
            },
        )
    }

    /// `downloadIfRemote(path)`, waited for: prefetches the input at `path` when it is remote.
    ///
    /// PORT: Java prefetches with `Priority.CRITICAL`; the blocking [`ActionInputPrefetcher`]
    /// seam of this crate always uses `MEDIUM` (the priority only orders queued downloads).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:397 RemoteActionFileSystem.downloadIfRemote
    pub fn download_if_remote(&self, path: &PathFragment) -> io::Result<()> {
        if !self.is_remote(path)? {
            return Ok(());
        }
        let exec_path = path.relative_to(&self.exec_root);
        let Some(input) = self.input_artifact_data.get_input(&exec_path) else {
            // TODO(tjgq): Also look up the remote output tree.
            return Ok(());
        };
        let action =
            self.action.lock().clone().expect(
                "NullPointerException: RemoteActionFileSystem.updateContext was not called",
            );
        let fetcher = self.input_fetcher.as_ref().expect(
            "NullPointerException: RemoteActionFileSystem has no input fetcher for a remote file",
        );
        fetcher.prefetch_files(
            &*action,
            std::slice::from_ref(&input),
            &*self.input_artifact_data,
        )
    }

    /// `inputArtifactData.getInput(execPath)` and its metadata.
    fn input_metadata(&self, exec_path: &PathFragment) -> io::Result<Option<FileArtifactValue>> {
        let Some(action_input) = self.input_artifact_data.get_input(exec_path) else {
            return Ok(None);
        };
        self.input_artifact_data.get_input_metadata(&action_input)
    }

    /// `inputTreeArtifactDirectoryCache.get(execPath)`: the entries (name, is_directory) of a
    /// directory of an input tree artifact, `None` if `exec_path` is not one.
    ///
    /// PORT: `inputArtifactData.getEnclosingTreeMetadata(execPath)` is looked up through the
    /// input map: the closest ancestor of `exec_path` that is an input tree artifact.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:193 TreeArtifactDirectoryCache.get
    fn input_tree_directory(&self, exec_path: &PathFragment) -> Option<HashSet<(String, bool)>> {
        let mut cache = self.input_tree_artifact_directory_cache.lock();
        // ensureCached(execPath)
        let mut candidate = Some(exec_path.clone());
        while let Some(dir) = candidate {
            if let Some(input) = self.input_artifact_data.get_input(&dir)
                && input.as_artifact().is_some_and(|a| a.is_tree_artifact())
            {
                if let Some(tree) = self.input_artifact_data.get_tree_metadata(&input)
                    && !tree.child_values.is_empty()
                    && cache.cached_trees.insert(dir.clone())
                {
                    // insertTree(treeMetadata)
                    for (child, _) in &tree.child_values {
                        // insertChild(child)
                        let mut path = child.get_exec_path().clone();
                        while path != dir {
                            let Some(parent_path) = path.get_parent_directory() else {
                                break;
                            };
                            let name = path.get_base_name().to_string();
                            let is_directory = &path != child.get_exec_path();
                            let entries =
                                cache.dir_to_entries.entry(parent_path.clone()).or_default();
                            if !entries.insert((name, is_directory)) {
                                // Avoid wasted work on common prefixes.
                                break;
                            }
                            path = parent_path;
                        }
                    }
                }
                break;
            }
            candidate = dir.get_parent_directory();
        }
        cache.dir_to_entries.get(exec_path).cloned()
    }

    /// `statInternal(path, followMode, statSources)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:667 RemoteActionFileSystem.statInternal
    fn stat_internal(
        &self,
        path: &PathFragment,
        follow_mode: FollowMode,
        stat_sources: StatSources,
    ) -> io::Result<Option<Stat>> {
        // Canonicalize the path.
        let canonical = match follow_mode {
            FollowMode::FollowAll => self.resolve_symbolic_links(path),
            FollowMode::FollowParent => match path.get_parent_directory() {
                Some(parent) => self
                    .resolve_symbolic_links(&parent)
                    .map(|p| p.get_child(path.get_base_name())),
                None => Ok(path.clone()),
            },
            FollowMode::FollowNone => Ok(path.clone()),
        };
        let path = match canonical {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };

        // Since the path has been canonicalized, the operations below never need to follow
        // symlinks.

        if path.starts_with(&self.exec_root) {
            let exec_path = path.relative_to(&self.exec_root);
            if let Some(metadata) = self.input_metadata(&exec_path)? {
                return Ok(Some(Stat::Metadata(metadata)));
            }
            if self.input_tree_directory(&exec_path).is_some() {
                return Ok(Some(Stat::Directory));
            }
        }

        // remoteOutputTree.statIfFound(path, /* followSymlinks= */ false)
        if let Some(metadata) = self.remote_output_tree.get_metadata(&path) {
            return Ok(Some(Stat::Metadata(metadata)));
        }
        if self.remote_output_tree.is_directory(&path) {
            return Ok(Some(Stat::Directory));
        }

        if stat_sources == StatSources::All {
            return Ok(self.local_fs.stat_if_found(&path, false)?.map(Stat::Local));
        }

        Ok(None)
    }

    /// `resolveSymbolicLinksForParent(path)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:348 RemoteActionFileSystem.resolveSymbolicLinksForParent
    fn resolve_symbolic_links_for_parent(&self, path: &PathFragment) -> io::Result<PathFragment> {
        match path.get_parent_directory() {
            Some(parent_path) => Ok(self
                .resolve_symbolic_links(&parent_path)?
                .get_child(path.get_base_name())),
            None => Ok(path.clone()),
        }
    }

    /// `readSymbolicLinkInternal(path)`.
    ///
    /// PORT: Java also creates every symlink under the output base in the remote output tree
    /// (`createSymbolicLink`) and reads it back from there first; the same link always exists on
    /// the local file system too, so it is read there.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:567 RemoteActionFileSystem.readSymbolicLinkInternal
    fn read_symbolic_link_internal(&self, path: &PathFragment) -> io::Result<PathFragment> {
        if path.starts_with(&self.exec_root) {
            let exec_path = path.relative_to(&self.exec_root);
            let metadata = self.input_metadata(&exec_path)?;
            if let Some(metadata) = &metadata
                && metadata.get_type().is_symlink()
            {
                return Ok(PathFragment::create(
                    metadata
                        .get_unresolved_symlink_target()
                        .expect("an unresolved symlink has a target"),
                ));
            }
            if metadata.is_some() {
                // Other input artifacts are never symlinks.
                return Err(NotASymlinkException::new_error(path));
            }
            if self.input_tree_directory(&exec_path).is_some() {
                // Tree artifacts never contain symlinks.
                return Err(NotASymlinkException::new_error(path));
            }
        }
        self.local_fs.read_symbolic_link(path)
    }

    /// `getDirectoryContents(path, followSymlinks, transformer)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:835 RemoteActionFileSystem.getDirectoryContents
    fn get_directory_contents(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Vec<Dirent>> {
        let path = self.resolve_symbolic_links(path)?;

        let mut entries: HashMap<String, Dirent> = HashMap::new();
        let mut exists = false;

        if path.starts_with(&self.exec_root) {
            let exec_path = path.relative_to(&self.exec_root);
            if let Some(tree_entries) = self.input_tree_directory(&exec_path) {
                for (name, is_directory) in tree_entries {
                    let type_ = if is_directory {
                        DirentType::Directory
                    } else {
                        DirentType::File
                    };
                    entries.insert(name.clone(), Dirent::new(name, type_));
                }
                exists = true;
            }
        }

        // Since actions are assumed not to modify their inputs, a directory belonging to an
        // input tree artifact cannot also contain an output, so we can safely skip the other
        // sources.
        if !exists {
            if self.is_output(&path)
                && let Some(remote_entries) = self.remote_output_tree.readdir(&path)
            {
                for entry in remote_entries {
                    let entry = self.maybe_follow_symlink_for_dirent(&path, entry, follow_symlinks);
                    entries.insert(entry.get_name().to_string(), entry);
                }
                exists = true;
            }

            match self.local_fs.readdir(&path, false) {
                Ok(local_entries) => {
                    for entry in local_entries {
                        let entry =
                            self.maybe_follow_symlink_for_dirent(&path, entry, follow_symlinks);
                        entries.insert(entry.get_name().to_string(), entry);
                    }
                    exists = true;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    // Will be rethrown below if directory does not exist in any of the sources.
                }
                Err(e) => return Err(e),
            }
        }

        if !exists {
            return Err(file_not_found(&path));
        }

        // Sort entries to get a deterministic order.
        let mut result: Vec<Dirent> = entries.into_values().collect();
        result.sort();
        Ok(result)
    }

    /// `maybeFollowSymlinkForDirent(dirPath, entry, followSymlinks)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:892 RemoteActionFileSystem.maybeFollowSymlinkForDirent
    fn maybe_follow_symlink_for_dirent(
        &self,
        dir_path: &PathFragment,
        entry: Dirent,
        follow_symlinks: bool,
    ) -> Dirent {
        if !follow_symlinks || entry.get_type() != DirentType::Symlink {
            return entry;
        }
        let path = dir_path.get_child(entry.get_name());
        let st = self.stat_nullable(&path, /* follow_symlinks= */ true);
        Dirent::new(
            entry.get_name().to_string(),
            blaze_vfs::file_system::dirent_from_stat(st.as_ref()),
        )
    }

    /// Runs `op` on the local file system after canonicalizing `path`; a missing local file is
    /// remote, so `if_missing` is the answer then.
    fn local_or<T>(
        &self,
        path: &PathFragment,
        op: impl FnOnce(&PathFragment) -> io::Result<T>,
        if_missing: T,
    ) -> io::Result<T> {
        let path = self.resolve_symbolic_links(path)?;
        match op(&path) {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(if_missing),
            Err(e) => Err(e),
        }
    }
}

impl FileSystem for RemoteActionFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.local_fs.get_digest_function()
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:262 RemoteActionFileSystem.supportsModifications
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.local_fs.supports_modifications(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:267 RemoteActionFileSystem.supportsSymbolicLinksNatively
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.local_fs.supports_symbolic_links_natively(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:272 RemoteActionFileSystem.supportsHardLinksNatively
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.local_fs.supports_hard_links_natively(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:277 RemoteActionFileSystem.mayBeCaseOrNormalizationInsensitive
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        // remoteOutputTree is an InMemoryFileSystem: case-sensitive.
        self.local_fs.may_be_case_or_normalization_insensitive()
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:817 RemoteActionFileSystem.createDirectory
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        // PORT: the remote output tree's directories are the parents of its files; Java's
        // `remoteOutputTree.createDirectory(path)` only changes the return value when the local
        // directory already existed and the in-memory one did not.
        let created = self.local_fs.create_directory(path)?;
        Ok(created || (self.is_output(path) && !self.remote_output_tree.is_directory(path)))
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:808 RemoteActionFileSystem.createDirectoryAndParents
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.local_fs.create_directory_and_parents(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:616 RemoteActionFileSystem.getFileSize
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_size())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:357 RemoteActionFileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        let path = match self.resolve_symbolic_links_for_parent(path) {
            Ok(p) => p,
            // Failure to delete a nonexistent path is not an error.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let mut deleted = self.local_fs.delete(&path)?;
        if self.is_output(&path) {
            deleted = self.remote_output_tree.remove(&path) || deleted;
        }
        Ok(deleted)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:610 RemoteActionFileSystem.getLastModifiedTime
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_last_modified_time())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:427 RemoteActionFileSystem.setLastModifiedTime
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        // PORT: Java also sets the time of a remote output tree file; its metadata keeps no
        // modification time here.
        let path = self.resolve_symbolic_links(path)?;
        match self.local_fs.set_last_modified_time(&path, new_time) {
            Err(e)
                if e.kind() == io::ErrorKind::NotFound
                    && self.remote_output_tree.get_metadata(&path).is_some() =>
            {
                Ok(())
            }
            other => other,
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:463 RemoteActionFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        let path = self.resolve_symbolic_links(path)?;
        // Try to obtain a fast digest through a stat. This is only possible for in-memory files.
        // The parent path has already been canonicalized by resolveSymbolicLinks, so FOLLOW_NONE
        // is effectively the same as FOLLOW_PARENT, but more efficient.
        if let Some(Stat::Metadata(m)) =
            self.stat_internal(&path, FollowMode::FollowNone, StatSources::InMemoryOnly)?
        {
            return Ok(m.get_digest().map(<[u8]>::to_vec));
        }
        self.local_fs.get_fast_digest(&path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:476 RemoteActionFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        let path = self.resolve_symbolic_links(path)?;
        if let Some(Stat::Metadata(m)) =
            self.stat_internal(&path, FollowMode::FollowNone, StatSources::InMemoryOnly)?
            && let Some(digest) = m.get_digest()
        {
            return Ok(digest.to_vec());
        }
        self.local_fs.get_digest(&path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:332 RemoteActionFileSystem.resolveOneLink
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        // The parent path has already been canonicalized, so FOLLOW_NONE is effectively the same
        // as FOLLOW_PARENT, but much more efficient as it doesn't call stat recursively.
        let Some(stat) = self.stat_internal(path, FollowMode::FollowNone, StatSources::All)? else {
            return Err(file_not_found(path));
        };
        if stat.is_symbolic_link() {
            Ok(Some(self.read_symbolic_link_internal(path)?))
        } else {
            Ok(None)
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:631 RemoteActionFileSystem.stat
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        self.stat_if_found(path, follow_symlinks)?
            .ok_or_else(|| file_not_found(path))
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:641 RemoteActionFileSystem.statIfFound
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        let follow_mode = if follow_symlinks {
            FollowMode::FollowAll
        } else {
            FollowMode::FollowParent
        };
        Ok(self
            .stat_internal(path, follow_mode, StatSources::All)?
            .map(|s| s.to_file_status()))
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:648 RemoteActionFileSystem.statNullable
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        self.stat_if_found(path, follow_symlinks).ok().flatten()
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:597 RemoteActionFileSystem.createSymbolicLink
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        let link_path = self.resolve_symbolic_links_for_parent(link_path)?;
        self.local_fs
            .create_symbolic_link(&link_path, target_fragment)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:562 RemoteActionFileSystem.readSymbolicLink
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.read_symbolic_link_internal(&self.resolve_symbolic_links_for_parent(path)?)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:622 RemoteActionFileSystem.exists
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        matches!(self.stat_if_found(path, follow_symlinks), Ok(Some(_)))
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:826 RemoteActionFileSystem.getDirectoryEntries
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        let mut names: Vec<String> = self
            .get_directory_contents(path, false)?
            .into_iter()
            .map(|d| d.get_name().to_string())
            .collect();
        names.sort();
        Ok(names)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:831 RemoteActionFileSystem.readdir
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        self.get_directory_contents(path, follow_symlinks)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:489 RemoteActionFileSystem.isReadable
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        // Remote files are always readable since we can't control their permissions.
        self.local_or(path, |p| self.local_fs.is_readable(p), true)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:522 RemoteActionFileSystem.setReadable
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        // Intentionally ignored when the file is not local.
        self.local_or(path, |p| self.local_fs.set_readable(p, readable), ())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:500 RemoteActionFileSystem.isWritable
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        // Remote files are always writable since we can't control their permissions.
        self.local_or(path, |p| self.local_fs.is_writable(p), true)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:532 RemoteActionFileSystem.setWritable
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        // Intentionally ignored when the file is not local.
        self.local_or(path, |p| self.local_fs.set_writable(p, writable), ())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:511 RemoteActionFileSystem.isExecutable
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        // Remote files are always executable since we can't control their permissions.
        self.local_or(path, |p| self.local_fs.is_executable(p), true)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:542 RemoteActionFileSystem.setExecutable
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        // Intentionally ignored when the file is not local.
        self.local_or(path, |p| self.local_fs.set_executable(p, executable), ())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:552 RemoteActionFileSystem.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        // Intentionally ignored when the file is not local.
        self.local_or(path, |p| self.local_fs.chmod(p, mode), ())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:380 RemoteActionFileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        if let Err(e) = self.download_if_remote(path) {
            if e.kind() == io::ErrorKind::Interrupted {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!(
                        "Received interrupt while fetching file '{}'",
                        path.get_path_string()
                    ),
                ));
            }
            // PORT: Java records the lost inputs of a BulkTransferException for
            // checkForLostInputs (action rewinding, not ported) and rethrows it.
            return Err(e);
        }
        self.local_fs.get_input_stream(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:416 RemoteActionFileSystem.getOutputStream
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.local_fs.get_output_stream(path, append, internal)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:773 RemoteActionFileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        let src_path = self.resolve_symbolic_links_for_parent(source_path)?;
        let dst_path = self.resolve_symbolic_links_for_parent(target_path)?;

        blaze_util::check_argument!(self.is_output(&src_path), "srcPath must be an output path");
        blaze_util::check_argument!(self.is_output(&dst_path), "dstPath must be an output path");

        let remote_found = self.remote_output_tree.rename(&src_path, &dst_path);
        match self.local_fs.rename_to(&src_path, &dst_path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound && remote_found => Ok(()),
            other => other,
        }
    }
    fn create_fs_dependent_hard_link(
        &self,
        _link_path: &PathFragment,
        _original_path: &PathFragment,
    ) -> io::Result<()> {
        // Only called by the FileSystem#createHardLink base implementation, overridden below.
        panic!("UnsupportedOperationException")
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:910 RemoteActionFileSystem.createHardLink
    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.local_fs.create_hard_link(link_path, original_path)
    }
}
