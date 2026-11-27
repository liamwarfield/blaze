//! Port of `lib/remote/RemoteActionFileSystem.java`: the action file system of
//! `RemoteOutputService` (build without the bytes), with its `PathCanonicalizer`
//! (`lib/remote/PathCanonicalizer.java`) and its remote output tree, `RemoteInMemoryFileSystem`
//! (an `InMemoryFileSystem`, `lib/vfs/inmemoryfs`, ported here for this one use).
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::sync::{Arc, Weak};

use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSymlinkLoopException, FileSystem,
    FileSystemRef, MAX_SYMLINKS, NOW_SENTINEL_TIME, NotASymlinkException, Path, PathFragment,
};

use crate::actions::{
    ActionExecutionMetadata, FileArtifactValue, InputMetadataProvider, LostArtifacts,
};
use crate::exec::action_execution_context::{ActionInputPrefetcher, BulkTransferError};

/// `FollowMode`: how `statInternal` handles symlinks.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:121 RemoteActionFileSystem.FollowMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // Java's names.
enum FollowMode {
    /// Canonicalize the entire path. This is equivalent to `Symlinks.FOLLOW`.
    FollowAll,
    /// Canonicalize the parent path. This is equivalent to `Symlinks.NOFOLLOW`.
    FollowParent,
    /// Do not canonicalize. This is only used internally to resolve symlinks efficiently.
    FollowNone,
}

/// `StatSources`: which sources `statInternal` considers.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:131 RemoteActionFileSystem.StatSources
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatSources {
    /// Consider all sources (action input map, remote output tree and local filesystem).
    All,
    /// Consider only in-memory sources (action input map and remote output tree).
    InMemoryOnly,
}

/// The result of `statInternal`: the Java `FileStatus` it returns, by its source.
#[derive(Clone, Debug)]
enum Stat {
    /// `statFromMetadata(metadata)` (a `FileStatusWithMetadata`), with the canonical path it was
    /// found at.
    Metadata(FileArtifactValue, PathFragment),
    /// `DIRECTORY_FILE_STATUS`, with the canonical path it was found at.
    Directory(PathFragment),
    /// An inode of the remote output tree; its metadata when it is a `RemoteInMemoryFileInfo`
    /// (a `FileStatusWithMetadata`), `None` for a directory or a symlink.
    Remote(FileStatus, Option<FileArtifactValue>),
    /// The local file system's status, with the canonical path it was found at.
    Local(FileStatus, PathFragment),
}

impl Stat {
    fn is_symbolic_link(&self) -> bool {
        match self {
            Stat::Metadata(m, _) => m.get_type().is_symlink(),
            Stat::Directory(_) => false,
            Stat::Remote(s, _) => s.is_symbolic_link(),
            Stat::Local(s, _) => s.is_symbolic_link(),
        }
    }

    /// `stat instanceof FileStatusWithMetadata` (also `instanceof FileStatusWithDigest`):
    /// its `getMetadata()`.
    fn metadata(&self) -> Option<&FileArtifactValue> {
        match self {
            Stat::Metadata(m, _) | Stat::Remote(_, Some(m)) => Some(m),
            _ => None,
        }
    }
}

/// `DIRECTORY_FILE_STATUS`: a directory of size 0 whose times and node id are unsupported.
///
/// PORT: Java's times and node id throw `UnsupportedOperationException`; they are 0 here.
/// Permissions: see [`RemoteActionFileSystem::unknown_permissions`].
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:138 RemoteActionFileSystem.DIRECTORY_FILE_STATUS
fn directory_file_status(permissions: u32) -> FileStatus {
    FileStatus::new(S_IFDIR | permissions, 0, 0, 0, 0)
}

/// `statFromMetadata(m)`: a `FileStatusWithMetadata` whose type, size and digest are those of
/// `m`, modified at `m.getModifiedTime()` (0 where that is unsupported), changed at the same time.
///
/// PORT: the node id throws `UnsupportedOperationException` in Java; it is 0 here.
/// Permissions: see [`RemoteActionFileSystem::unknown_permissions`].
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:709 RemoteActionFileSystem.statFromMetadata
fn stat_from_metadata(m: &FileArtifactValue, permissions: u32) -> FileStatus {
    let type_ = m.get_type();
    let kind = if type_.is_symlink() {
        S_IFLNK
    } else if type_.is_directory() {
        S_IFDIR
    } else if type_.is_special_file() {
        S_IFIFO
    } else if type_.is_file() {
        S_IFREG
    } else {
        // FileStateType.NONEXISTENT: every is* is false.
        0
    };
    // Not every FileArtifactValue supports getModifiedTime: an UnsupportedOperationException
    // gives 0.
    let mtime = modified_time_or_zero(m);
    FileStatus::new(kind | permissions, mtime, mtime, m.get_size(), 0)
}

/// `m.getModifiedTime()`, with the `UnsupportedOperationException` that `statFromMetadata`
/// catches as 0.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java FileArtifactValue.getModifiedTime
fn modified_time_or_zero(m: &FileArtifactValue) -> i64 {
    match m {
        // DirectoryArtifactValue.getModifiedTime: mtime.
        FileArtifactValue::Directory { mtime } => *mtime,
        // HashedDirectoryArtifactValue, SingletonMarkerValue: 0.
        FileArtifactValue::HashedDirectory { .. }
        | FileArtifactValue::RunfilesTreeMarker
        | FileArtifactValue::MissingFileMarker => 0,
        // ConstantMetadataValue.getModifiedTime: -1.
        FileArtifactValue::ConstantMetadata => -1,
        // ResolvedSymlinkArtifactValue, ProxyFileArtifactValue: the delegate's.
        FileArtifactValue::ResolvedSymlink { delegate, .. }
        | FileArtifactValue::Proxy { delegate, .. } => modified_time_or_zero(delegate),
        // UnresolvedSymlinkArtifactValue.getModifiedTime throws IllegalStateException, which
        // statFromMetadata does not catch.
        // PORT: Java's FileStatus calls getModifiedTime only when its mtime is read (and then
        // crashes); this FileStatus is built eagerly, so the value is 0 and reading it does not
        // crash.
        FileArtifactValue::UnresolvedSymlink { .. } => 0,
        // RegularFileArtifactValue, RunfilesProxyArtifactValue, RemoteFileArtifactValue and
        // TreeArtifactCompositeFileArtifactValue throw UnsupportedOperationException.
        FileArtifactValue::RegularFile { .. }
        | FileArtifactValue::RunfilesProxy { .. }
        | FileArtifactValue::RemoteFile { .. }
        | FileArtifactValue::TreeArtifactComposite { .. } => 0,
    }
}

const S_IFLNK: u32 = 0o120_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFIFO: u32 = 0o010_000;

/// `new FileNotFoundException(path + " (No such file or directory)")`.
fn file_not_found(path: &PathFragment) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} (No such file or directory)", path.get_path_string()),
    )
}

/// `e instanceof FileNotFoundException`.
fn is_file_not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound
}

/// `localException.addSuppressed(remoteException); throw localException;`
fn with_suppressed(local: io::Error, remote: io::Error) -> io::Error {
    // PORT: io::Error has no suppressed list; the remote exception is appended to the message
    // the way Java's stack trace prints it ("Suppressed: ...").
    io::Error::new(local.kind(), format!("{local}\n\tSuppressed: {remote}"))
}

/// The action file system of build without the bytes: the local file system, with the metadata
/// of the action's inputs and of its own remote outputs layered on top.
///
/// It acts as a union filesystem over three different sources: the action input map (read-only
/// access to the metadata, not the contents, of the action's declared inputs), the remote output
/// tree (an in-memory file system holding the metadata of remotely stored files injected during
/// action execution) and the local file system. File operations consult the sources in that
/// order and operate on the first result found, although some (e.g. readdir) collate information
/// from all sources. The contents of remotely stored files are downloaded when an operation
/// requires them.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:106 RemoteActionFileSystem
pub struct RemoteActionFileSystem {
    this: Weak<RemoteActionFileSystem>,
    exec_root: PathFragment,
    output_base: PathFragment,
    input_artifact_data: Arc<dyn InputMetadataProvider>,
    /// `TreeArtifactDirectoryCache`: the directories of the input tree artifacts, by exec path.
    input_tree_artifact_directory_cache: parking_lot::Mutex<TreeArtifactDirectoryCache>,
    path_canonicalizer: PathCanonicalizer,
    /// `inputFetcher`; `None` only in tests that never read a remote file.
    input_fetcher: Option<Arc<dyn ActionInputPrefetcher>>,
    local_fs: FileSystemRef,
    /// `remoteOutputTree`.
    remote_output_tree: Arc<RemoteInMemoryFileSystem>,
    /// `lostInputs`: concurrent access is rare and most builds don't have lost inputs.
    lost_inputs: parking_lot::Mutex<Vec<LostArtifacts>>,
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

/// `TreeArtifactDirectoryCache`: caches the contents of intermediate subdirectories of tree
/// artifact inputs, to speed up `stat` and `readdir`. Actions are not expected to modify their
/// inputs.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:188 RemoteActionFileSystem.TreeArtifactDirectoryCache
#[derive(Default)]
struct TreeArtifactDirectoryCache {
    /// `cachedTrees`, by the tree's exec path.
    cached_trees: HashSet<PathFragment>,
    /// `dirToEntries`: (name, is_directory) per directory exec path.
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
        let remote_output_tree = Arc::new(RemoteInMemoryFileSystem::new(
            local_fs.get_digest_function(),
        ));
        Arc::new_cyclic(|this| RemoteActionFileSystem {
            this: this.clone(),
            exec_root: exec_root_fragment,
            output_base,
            input_artifact_data,
            input_tree_artifact_directory_cache: parking_lot::Mutex::new(
                TreeArtifactDirectoryCache::default(),
            ),
            path_canonicalizer: PathCanonicalizer::new(),
            input_fetcher,
            local_fs,
            remote_output_tree,
            lost_inputs: parking_lot::Mutex::new(Vec::new()),
            action: parking_lot::Mutex::new(None),
        })
    }

    /// This file system as a [`FileSystemRef`].
    pub fn as_file_system(&self) -> FileSystemRef {
        self.this
            .upgrade()
            .expect("RemoteActionFileSystem used after it was dropped")
    }

    /// This file system as an `Arc` (for work that outlives a borrow, such as a download run on
    /// its own thread).
    pub fn as_arc(&self) -> Arc<RemoteActionFileSystem> {
        self.this
            .upgrade()
            .expect("RemoteActionFileSystem used after it was dropped")
    }

    /// `getPath(fragment)`: the path on this file system.
    pub fn get_path(&self, fragment: &PathFragment) -> Path {
        Path::create(fragment.clone(), &self.as_file_system())
    }

    /// `getRemoteOutputTree()`: the in-memory file system of the outputs of this action that
    /// exist only remotely.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:283 RemoteActionFileSystem.getRemoteOutputTree
    pub fn get_remote_in_memory_file_system(&self) -> &Arc<RemoteInMemoryFileSystem> {
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
    /// of this action that was not downloaded. A path outside the output base is ignored.
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
        self.remote_output_tree.inject_file(path, metadata)
    }

    /// `isRemote(path)`: whether the file at `path` (following symlinks) is known only by its
    /// remote metadata.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:297 RemoteActionFileSystem.isRemote
    pub fn is_remote(&self, path: &PathFragment) -> io::Result<bool> {
        // Files in the local filesystem are non-remote by definition, so stat only in-memory
        // sources.
        let status = self.stat_internal(path, FollowMode::FollowAll, StatSources::InMemoryOnly)?;
        Ok(status
            .as_ref()
            .and_then(Stat::metadata)
            .is_some_and(FileArtifactValue::is_remote))
    }

    /// The metadata of the file at `path` when an in-memory source (the input metadata or the
    /// remote output tree) knows it: Java's `stat(...) instanceof FileStatusWithMetadata`, as
    /// `ActionOutputMetadataStore.fileArtifactValueFromArtifact` tests it. `None` for a local
    /// file, a directory, a symlink of the remote output tree or a missing file.
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
        Ok(self
            .stat_internal(path, follow_mode, StatSources::InMemoryOnly)?
            .as_ref()
            .and_then(Stat::metadata)
            .cloned())
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
        // PORT: Java returns the prefetch future and its callers wait with `getFromFuture`. The
        // blocking prefetcher seam waits on the remote crate's runtime, which panics when the
        // calling thread is inside that runtime (an upload reads its file through this file
        // system: `uploadFile` -> `getInputStream`). The wait therefore runs on a thread of its
        // own; the calling thread blocks until it ends, as Java's caller does. The helper stands
        // in for the calling thread (`ThreadInterrupt::adopt`, as `on_evaluation_stack` does): an
        // interrupt of the caller reaches the wait of the prefetch (`getFromFuture` throws
        // `InterruptedException` at once), and the status the wait clears or leaves is the
        // caller's.
        let input_artifact_data = &*self.input_artifact_data;
        let caller = blaze_shell::ThreadInterrupt::current();
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _adopted = caller.adopt();
                    fetcher.prefetch_files(
                        &*action,
                        std::slice::from_ref(&input),
                        input_artifact_data,
                    )
                })
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        })
    }

    /// `checkForLostInputs(action)`: the inputs that `getInputStream` found lost (their blobs are
    /// missing from the remote cache), merged in recording order as
    /// `lostInputs.stream().map(l -> new LostInputsExecException(l.byDigest()))
    /// .reduce(LostInputsExecException::combine)` merges them; `None` when nothing was lost.
    ///
    /// PORT: Java throws `ActionExecutionException.fromExecException(mergedException, action)` (a
    /// `LostInputsActionExecutionException`); the caller builds it from the returned record with
    /// `LostInputsExecException::create`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:914 RemoteActionFileSystem.checkForLostInputs
    // Java: com/google/devtools/build/lib/actions/LostInputsExecException.java:68 LostInputsExecException.combine
    pub fn check_for_lost_inputs(&self) -> Option<LostArtifacts> {
        self.lost_inputs
            .lock()
            .iter()
            .cloned()
            .reduce(|mut combined, other| {
                // ImmutableSetMultimap.builder().putAll(lostInputs).putAll(other.lostInputs)
                for (digest, inputs) in other.by_digest {
                    combined.by_digest.entry(digest).or_default().extend(inputs);
                }
                combined
            })
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
    /// input map: the closest ancestor of `exec_path` (or itself) that is an input tree artifact.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:193 TreeArtifactDirectoryCache.get
    fn input_tree_directory(&self, exec_path: &PathFragment) -> Option<HashSet<(String, bool)>> {
        let mut cache = self.input_tree_artifact_directory_cache.lock();
        // ensureCached(execPath)
        // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:198 TreeArtifactDirectoryCache.ensureCached
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
                    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:216 TreeArtifactDirectoryCache.insertChild
                    for (child, _) in &tree.child_values {
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

    /// `pathCanonicalizer.resolveSymbolicLinks(path)`.
    fn canonicalize(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.path_canonicalizer.resolve_symbolic_links(path, self)
    }

    /// `statInternal(path, followMode, statSources)`: the status of `path` from the first source
    /// that has it, `None` if none of the considered sources has it.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:667 RemoteActionFileSystem.statInternal
    fn stat_internal(
        &self,
        path: &PathFragment,
        follow_mode: FollowMode,
        stat_sources: StatSources,
    ) -> io::Result<Option<Stat>> {
        // Canonicalize the path.
        let canonical = match follow_mode {
            FollowMode::FollowAll => self.canonicalize(path),
            FollowMode::FollowParent => match path.get_parent_directory() {
                Some(parent) => self
                    .canonicalize(&parent)
                    .map(|p| p.get_child(path.get_base_name())),
                None => Ok(path.clone()),
            },
            FollowMode::FollowNone => Ok(path.clone()),
        };
        let path = match canonical {
            Ok(p) => p,
            Err(e) if is_file_not_found(&e) => return Ok(None),
            Err(e) => return Err(e),
        };

        // Since the path has been canonicalized, the operations below never need to follow
        // symlinks.

        if path.starts_with(&self.exec_root) {
            let exec_path = path.relative_to(&self.exec_root);
            if let Some(metadata) = self.input_metadata(&exec_path)? {
                return Ok(Some(Stat::Metadata(metadata, path)));
            }
            if self.input_tree_directory(&exec_path).is_some() {
                return Ok(Some(Stat::Directory(path)));
            }
        }

        if let Some((stat, metadata)) = self
            .remote_output_tree
            .stat_if_found_with_metadata(&path, /* follow_symlinks= */ false)?
        {
            return Ok(Some(Stat::Remote(stat, metadata)));
        }

        if stat_sources == StatSources::All {
            return Ok(self
                .local_fs
                .stat_if_found(&path, false)?
                .map(|s| Stat::Local(s, path)));
        }

        Ok(None)
    }

    /// The `FileStatus` an outside caller of `stat` sees.
    fn to_file_status(&self, stat: &Stat) -> FileStatus {
        match stat {
            Stat::Metadata(m, path) => stat_from_metadata(m, self.unknown_permissions(path)),
            Stat::Directory(path) => directory_file_status(self.unknown_permissions(path)),
            Stat::Remote(s, _) | Stat::Local(s, _) => *s,
        }
    }

    /// The permission bits of a status whose Java `getPermissions()` is -1 (`statFromMetadata`
    /// and `DIRECTORY_FILE_STATUS` do not override the default).
    ///
    /// PORT: `blaze_vfs::FileStatus` has no "unknown permissions" value. A Java caller that gets
    /// -1 asks the file system instead (`SymlinkAction.maybeVerifyTargetIsExecutable` calls
    /// `inputPath.isExecutable()`), and this file system answers `isReadable` / `isWritable` /
    /// `isExecutable` from the local file at the canonical path, or true when there is none
    /// (`FileNotFoundException`). The bits are those answers: the local file's permission bits,
    /// 0777 when it does not exist locally, 0 when the local stat fails otherwise (Java would
    /// throw that error from `isExecutable`). Needed in `blaze_vfs` for an exact port: a
    /// `FileStatus` whose `get_permissions()` can be -1, and callers that check for it.
    // Java: com/google/devtools/build/lib/vfs/FileStatus.java:100 FileStatus.getPermissions
    fn unknown_permissions(&self, canonical_path: &PathFragment) -> u32 {
        let path = match self.canonicalize(canonical_path) {
            Ok(p) => p,
            Err(e) if is_file_not_found(&e) => return 0o777,
            Err(_) => return 0,
        };
        match self.local_fs.stat(&path, true) {
            Ok(s) => s.get_permissions(),
            Err(e) if is_file_not_found(&e) => 0o777,
            Err(_) => 0,
        }
    }

    /// `resolveSymbolicLinksForParent(path)`: like `resolveSymbolicLinks`, except that only the
    /// parent path is canonicalized.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:348 RemoteActionFileSystem.resolveSymbolicLinksForParent
    fn resolve_symbolic_links_for_parent(&self, path: &PathFragment) -> io::Result<PathFragment> {
        match path.get_parent_directory() {
            Some(parent_path) => Ok(self
                .canonicalize(&parent_path)?
                .get_child(path.get_base_name())),
            None => Ok(path.clone()),
        }
    }

    /// `readSymbolicLinkInternal(path)`: like `readSymbolicLink`, except that the parent path is
    /// assumed to be already canonical.
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

        if self.is_output(path) {
            match self.remote_output_tree.read_symbolic_link(path) {
                Ok(target) => return Ok(target),
                Err(e) if is_file_not_found(&e) => {
                    // Intentionally ignored.
                }
                Err(e) => return Err(e),
            }
        }

        self.local_fs.read_symbolic_link(path)
    }

    /// `getDirectoryContents(path, followSymlinks, transformer)`: the entries of the directory
    /// from every source, sorted.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:835 RemoteActionFileSystem.getDirectoryContents
    fn get_directory_contents(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Vec<Dirent>> {
        let path = self.canonicalize(path)?;

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
            if self.is_output(&path) {
                match FileSystem::readdir(&*self.remote_output_tree, &path, false) {
                    Ok(remote_entries) => {
                        for entry in remote_entries {
                            let entry =
                                self.maybe_follow_symlink_for_dirent(&path, entry, follow_symlinks);
                            entries.insert(entry.get_name().to_string(), entry);
                        }
                        exists = true;
                    }
                    Err(e) if is_file_not_found(&e) => {
                        // Will be rethrown below if directory does not exist in any of the
                        // sources.
                    }
                    Err(e) => return Err(e),
                }
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
                Err(e) if is_file_not_found(&e) => {
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

    /// `path = resolveSymbolicLinks(path); try { op(localFs) } catch (FileNotFoundException e)
    /// { return ifMissing; }`: the permission methods.
    fn local_or<T>(
        &self,
        path: &PathFragment,
        op: impl FnOnce(&PathFragment) -> io::Result<T>,
        if_missing: T,
    ) -> io::Result<T> {
        let path = self.canonicalize(path)?;
        match op(&path) {
            Ok(v) => Ok(v),
            Err(e) if is_file_not_found(&e) => Ok(if_missing),
            Err(e) => Err(e),
        }
    }
}

impl FileSystem for RemoteActionFileSystem {
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:257 RemoteActionFileSystem.getHostFileSystem
    fn get_host_file_system(&self) -> Option<FileSystemRef> {
        // `localFs.getHostFileSystem()` (`None` there is the local file system itself).
        Some(
            self.local_fs
                .get_host_file_system()
                .unwrap_or_else(|| self.local_fs.clone()),
        )
    }

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
        self.local_fs.may_be_case_or_normalization_insensitive()
            || self
                .remote_output_tree
                .may_be_case_or_normalization_insensitive()
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:817 RemoteActionFileSystem.createDirectory
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        let mut created = self.local_fs.create_directory(path)?;
        if self.is_output(path) {
            created = self.remote_output_tree.create_directory(path)? || created;
        }
        Ok(created)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:808 RemoteActionFileSystem.createDirectoryAndParents
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.local_fs.create_directory_and_parents(path)?;
        if self.is_output(path) {
            self.remote_output_tree.create_directory_and_parents(path)?;
        }
        Ok(())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:616 RemoteActionFileSystem.getFileSize
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_size())
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:357 RemoteActionFileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        let original_path = path;
        let path = match self.resolve_symbolic_links_for_parent(path) {
            Ok(p) => p,
            Err(e) if is_file_not_found(&e) => {
                // Failure to delete a nonexistent path is not an error.
                self.path_canonicalizer.clear_prefix(original_path);
                return Ok(false);
            }
            Err(e) => return Err(e),
        };

        // No action implementations call renameTo concurrently with other filesystem operations,
        // so there's no risk of a race condition below.
        self.path_canonicalizer.clear_prefix(original_path);

        let mut deleted = self.local_fs.delete(&path)?;
        if self.is_output(&path) {
            deleted = self.remote_output_tree.delete(&path)? || deleted;
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
        let path = self.canonicalize(path)?;

        // We can't set mtime for a remote file, set mtime of in-memory file node instead.
        let remote_exception = match self
            .remote_output_tree
            .set_last_modified_time(&path, new_time)
        {
            Ok(()) => None,
            Err(e) if is_file_not_found(&e) => Some(e),
            Err(e) => return Err(e),
        };

        let local_exception = match self.local_fs.set_last_modified_time(&path, new_time) {
            Ok(()) => None,
            Err(e) if is_file_not_found(&e) => Some(e),
            Err(e) => return Err(e),
        };

        match (remote_exception, local_exception) {
            (Some(remote), Some(local)) => Err(with_suppressed(local, remote)),
            _ => Ok(()),
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:463 RemoteActionFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        let path = self.canonicalize(path)?;
        // Try to obtain a fast digest through a stat. This is only possible for in-memory files.
        // The parent path has already been canonicalized by resolveSymbolicLinks, so FOLLOW_NONE
        // is effectively the same as FOLLOW_PARENT, but more efficient.
        let status =
            self.stat_internal(&path, FollowMode::FollowNone, StatSources::InMemoryOnly)?;
        if let Some(m) = status.as_ref().and_then(Stat::metadata) {
            return Ok(m.get_digest().map(<[u8]>::to_vec));
        }
        self.local_fs.get_fast_digest(&path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:476 RemoteActionFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        let path = self.canonicalize(path)?;
        // Try to obtain a fast digest through a stat. This is only possible for in-memory files.
        // The parent path has already been canonicalized by resolveSymbolicLinks, so FOLLOW_NONE
        // is effectively the same as FOLLOW_PARENT, but more efficient.
        let status =
            self.stat_internal(&path, FollowMode::FollowNone, StatSources::InMemoryOnly)?;
        if let Some(m) = status.as_ref().and_then(Stat::metadata) {
            return match m.get_digest() {
                Some(digest) => Ok(digest.to_vec()),
                // PORT: Java returns the metadata's null digest; the Rust signature has no null,
                // so it is an error rather than a digest computed from another source.
                None => Err(io::Error::other(format!(
                    "{}: RemoteActionFileSystem.getDigest: the metadata has no digest ({:?})",
                    path.get_path_string(),
                    m
                ))),
            };
        }
        self.local_fs.get_digest(&path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:332 RemoteActionFileSystem.resolveOneLink
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        // The base implementation attempts to readSymbolicLink first and falls back to stat, but
        // that unnecessarily allocates a NotASymlinkException in the overwhelmingly likely
        // non-symlink case. It's more efficient to stat unconditionally.
        //
        // The parent path has already been canonicalized, so FOLLOW_NONE is effectively the same
        // as FOLLOW_PARENT, but much more efficient as it doesn't call stat recursively. Likewise
        // for readSymbolicLinkInternal instead of readSymbolicLink.
        let Some(stat) = self.stat_internal(path, FollowMode::FollowNone, StatSources::All)? else {
            return Err(file_not_found(path));
        };
        if stat.is_symbolic_link() {
            Ok(Some(self.read_symbolic_link_internal(path)?))
        } else {
            Ok(None)
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:326 RemoteActionFileSystem.resolveSymbolicLinks
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.canonicalize(path)
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
            .map(|s| self.to_file_status(&s)))
    }
    /// The metadata of `stat`'s `FileStatusWithMetadata`: the input's metadata, the remote output
    /// tree's, or whatever the local file system's stat carries (it may be the remote repo
    /// contents cache's overlay).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:631 RemoteActionFileSystem.stat
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn blaze_vfs::StatMetadata>>> {
        let follow_mode = if follow_symlinks {
            FollowMode::FollowAll
        } else {
            FollowMode::FollowParent
        };
        match self.stat_internal(path, follow_mode, StatSources::All)? {
            None | Some(Stat::Directory(_)) => Ok(None),
            Some(Stat::Local(_, canonical)) => self.local_fs.stat_metadata(&canonical, false),
            Some(stat) => Ok(stat
                .metadata()
                .cloned()
                .map(|m| std::sync::Arc::new(m) as std::sync::Arc<dyn blaze_vfs::StatMetadata>)),
        }
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

        if self.is_output(&link_path) {
            self.remote_output_tree
                .create_symbolic_link(&link_path, target_fragment)?;
        }

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
        // getDirectoryContents(path, /* followSymlinks= */ false, Dirent::getName): the names,
        // sorted.
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
            // catch (BulkTransferException e)
            if let Some(bulk) = e
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<BulkTransferError>())
            {
                let newly_lost_inputs = bulk
                    .get_lost_artifacts(&|exec_path| self.input_artifact_data.get_input(exec_path));
                if !newly_lost_inputs.is_empty() {
                    self.lost_inputs.lock().push(newly_lost_inputs);
                }
                return Err(e);
            }
            // catch (InterruptedException e)
            if e.kind() == io::ErrorKind::Interrupted {
                // Thread.currentThread().interrupt();
                blaze_shell::thread_interrupt::interrupt_current();
                // PORT: Java throws a plain IOException; the error keeps the kind Interrupted,
                // which Blaze's callers read as the InterruptedException that the restored flag
                // makes their next wait throw in Java.
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!(
                        "Received interrupt while fetching file '{}'",
                        path.get_path_string()
                    ),
                ));
            }
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

        // No action implementations call renameTo concurrently with other filesystem operations,
        // so there's no risk of a race condition below.
        self.path_canonicalizer.clear_prefix(&src_path);
        self.path_canonicalizer.clear_prefix(&dst_path);

        let remote_exception = match self.remote_output_tree.rename_to(&src_path, &dst_path) {
            Ok(()) => None,
            Err(e) if is_file_not_found(&e) => Some(e),
            Err(e) => return Err(e),
        };

        let local_exception = match self.local_fs.rename_to(&src_path, &dst_path) {
            Ok(()) => None,
            Err(e) if is_file_not_found(&e) => Some(e),
            Err(e) => return Err(e),
        };

        match (remote_exception, local_exception) {
            (Some(remote), Some(local)) => Err(with_suppressed(local, remote)),
            _ => Ok(()),
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:903 RemoteActionFileSystem.createFSDependentHardLink
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
        // `localFs.getPath(linkPath).createHardLink(getPath(originalPath))`. `Path.createHardLink(
        // link)` is `fileSystem.createHardLink(link.asFragment(), asFragment())` on the receiver's
        // file system, so Java asks the local file system for a link at `originalPath` to
        // `linkPath` (the arguments end up swapped): it fails with "File ... does not exist"
        // unless `linkPath` exists. Ported as written.
        // Java: com/google/devtools/build/lib/vfs/Path.java:599 Path.createHardLink
        self.local_fs.create_hard_link(original_path, link_path)
    }
}

/// `PathCanonicalizer.resolveSymbolicLinks` asks `RemoteActionFileSystem.resolveOneLink`.
// Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:46 PathCanonicalizer.Resolver
trait PathCanonicalizerResolver {
    /// `resolveOneLink(path)`: the target if `path` is a symlink, otherwise `None`. All but the
    /// last path segment must be canonical.
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>>;
}

impl PathCanonicalizerResolver for RemoteActionFileSystem {
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        FileSystem::resolve_one_link(self, path)
    }
}

/// A trie node of [`PathCanonicalizer`].
// Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:58 PathCanonicalizer.Node
#[derive(Clone)]
enum CanonicalizerNode {
    /// `SymlinkNode(targetPath)`: a trie node corresponding to a symlink.
    Symlink(PathFragment),
    /// `NonSymlinkNode`: a trie node not corresponding to a symlink.
    NonSymlink(Arc<NonSymlinkNode>),
}

/// `NonSymlinkNode`: a `ConcurrentHashMap<String, Node>`.
// Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:64 PathCanonicalizer.NonSymlinkNode
#[derive(Default)]
struct NonSymlinkNode {
    children: parking_lot::RwLock<HashMap<String, CanonicalizerNode>>,
}

impl NonSymlinkNode {
    fn get(&self, segment: &str) -> Option<CanonicalizerNode> {
        self.children.read().get(segment).cloned()
    }

    fn remove(&self, segment: &str) {
        self.children.write().remove(segment);
    }
}

/// Canonicalizes paths like `FileSystem.resolveSymbolicLinks`, while storing the intermediate
/// results in a trie so they can be reused by future canonicalizations. Because
/// `RemoteActionFileSystem` implements a union filesystem and must account for the possibility of
/// symlinks straddling the underlying filesystems, the performance of large filesystem scans can
/// be greatly improved with a custom `resolveSymbolicLinks` that leverages the trie.
///
/// Thread-safe: concurrent calls to `resolve_symbolic_links` are supported; the result is
/// undefined if the filesystem is mutated concurrently.
///
/// PORT: Java's canonicalizer holds its resolver; here the resolver is passed to each call (the
/// file system owns the canonicalizer).
// Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:44 PathCanonicalizer
struct PathCanonicalizer {
    root: Arc<NonSymlinkNode>,
}

impl PathCanonicalizer {
    // Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:74 PathCanonicalizer.<init>
    fn new() -> PathCanonicalizer {
        PathCanonicalizer {
            root: Arc::new(NonSymlinkNode::default()),
        }
    }

    /// `getRootNode(path)`: Unix has a single root.
    // Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:79 PathCanonicalizer.getRootNode
    fn get_root_node(&self, path: &PathFragment) -> Arc<NonSymlinkNode> {
        blaze_util::check_argument!(path.is_absolute());
        // PORT: skipped, Windows only (one root per drive).
        self.root.clone()
    }

    /// `resolveSymbolicLinks(path)`: the canonical path, reusing cached information if possible.
    /// Fails with `FileSymlinkLoopException` if too many symlinks had to be followed.
    // Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:170 PathCanonicalizer.resolveSymbolicLinks
    fn resolve_symbolic_links(
        &self,
        path: &PathFragment,
        resolver: &dyn PathCanonicalizerResolver,
    ) -> io::Result<PathFragment> {
        self.resolve_symbolic_links_with_max_links(path.clone(), MAX_SYMLINKS, resolver)
    }

    // Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:99 PathCanonicalizer.resolveSymbolicLinks
    fn resolve_symbolic_links_with_max_links(
        &self,
        path: PathFragment,
        mut max_links: i32,
        resolver: &dyn PathCanonicalizerResolver,
    ) -> io::Result<PathFragment> {
        // Loop invariants:
        // - `segment_index` is the index of the current `segment` relative to the start of
        //   `path`. The first segment has index 0.
        // - `path` is the absolute path to canonicalize. If `segment_index` > 0, `path` is
        //   already canonical up to and including `segment_index` - 1.
        // - `node` is the trie node corresponding to the `path` prefix ending with
        //   `segment_index` - 1, or to the root path when `segment_index` is 0.
        let mut node = self.get_root_node(&path);
        for (segment_index, segment) in path.segments().enumerate() {
            let next_node = match node.get(segment) {
                Some(next_node) => next_node,
                None => {
                    let naive_path = path.sub_fragment(0, segment_index + 1);
                    let target_path = resolver.resolve_one_link(&naive_path)?;
                    node.children
                        .write()
                        .entry(segment.to_string())
                        .or_insert_with(|| match target_path {
                            Some(target_path) => CanonicalizerNode::Symlink(target_path),
                            None => {
                                CanonicalizerNode::NonSymlink(Arc::new(NonSymlinkNode::default()))
                            }
                        })
                        .clone()
                }
            };

            match next_node {
                CanonicalizerNode::Symlink(target_path) => {
                    if max_links == 0 {
                        return Err(FileSymlinkLoopException::new_error(&path));
                    }
                    max_links -= 1;

                    // Compute the path obtained by resolving the symlink.
                    // Note that path normalization already handles uplevel references.
                    let new_path = if target_path.is_absolute() {
                        target_path.get_relative(&path.sub_fragment_from(segment_index + 1))
                    } else {
                        path.sub_fragment(0, segment_index)
                            .get_relative(&target_path)
                            .get_relative(&path.sub_fragment_from(segment_index + 1))
                    };

                    // For absolute symlinks, we must start over.
                    // For relative symlinks, it would have been possible to restart after the
                    // already canonicalized prefix, but they're too rare to be worth optimizing
                    // for.
                    return self
                        .resolve_symbolic_links_with_max_links(new_path, max_links, resolver);
                }
                CanonicalizerNode::NonSymlink(non_symlink_node) => {
                    node = non_symlink_node;
                }
            }
        }

        Ok(path)
    }

    /// `clearPrefix(pathPrefix)`: removes cached information for a path prefix.
    // Java: com/google/devtools/build/lib/remote/PathCanonicalizer.java:175 PathCanonicalizer.clearPrefix
    fn clear_prefix(&self, path_prefix: &PathFragment) {
        let mut node = Some(CanonicalizerNode::NonSymlink(
            self.get_root_node(path_prefix),
        ));
        let mut parent: Option<Arc<NonSymlinkNode>> = None;
        let mut parent_segment: Option<String> = None;
        let mut segments = path_prefix.segments().peekable();
        let mut has_next = segments.peek().is_some();

        while let Some(current) = node.take() {
            if !has_next {
                break;
            }
            let segment = segments.next().expect("has_next");
            has_next = segments.peek().is_some();

            match current {
                CanonicalizerNode::Symlink(_) => {
                    // Invalidate all intermediate symlinks.
                    if let (Some(parent), Some(parent_segment)) = (&parent, &parent_segment) {
                        parent.remove(parent_segment);
                    }
                    return;
                }
                CanonicalizerNode::NonSymlink(non_symlink_node) => {
                    if !has_next {
                        // Found the path prefix.
                        non_symlink_node.remove(segment);
                    } else {
                        node = non_symlink_node.get(segment);
                        parent = Some(non_symlink_node);
                        parent_segment = Some(segment.to_string());
                    }
                }
            }
        }
    }
}

/// `InMemoryFileSystem.Errno`: the errors the in-memory file system issues.
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:96 InMemoryFileSystem.Errno
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Errno {
    Enoent,
    Eacces,
    Enotdir,
    Eexist,
    Ebusy,
    Enotempty,
    Eisdir,
    Eloop,
}

impl Errno {
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:118 InMemoryFileSystem.Errno.toString
    fn message(self) -> &'static str {
        match self {
            Errno::Enoent => "No such file or directory",
            Errno::Eacces => "Permission denied",
            Errno::Enotdir => "Not a directory",
            Errno::Eexist => "File exists",
            Errno::Ebusy => "Device or resource busy",
            Errno::Enotempty => "Directory not empty",
            Errno::Eisdir => "Is a directory",
            Errno::Eloop => "Too many levels of symbolic links",
        }
    }

    /// `exception(path)`: the `IOException` for this error, "<path> (<message>)".
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:127 InMemoryFileSystem.Errno.exception
    fn exception(self, path: &PathFragment) -> io::Error {
        let m = format!("{} ({})", path.get_path_string(), self.message());
        match self {
            // FileAccessException
            Errno::Eacces => io::Error::new(io::ErrorKind::PermissionDenied, m),
            // FileNotFoundException
            Errno::Enoent => io::Error::new(io::ErrorKind::NotFound, m),
            Errno::Eloop => FileSymlinkLoopException::new_error(path),
            _ => io::Error::other(m),
        }
    }
}

/// An inode of the in-memory file system (`InMemoryContentInfo`), by index.
type Ino = usize;

/// The root inode.
const ROOT_INODE: Ino = 0;

/// Maximum number of traversals before ELOOP is thrown.
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:70 InMemoryFileSystem.MAX_TRAVERSALS
const MAX_TRAVERSALS: i32 = 256;

/// `InMemoryContentInfo`: an inode, with its kind (`InMemoryDirectoryInfo`, `InMemoryLinkInfo` or
/// the `RemoteInMemoryFileInfo` that `RemoteInMemoryFileSystem.newFile` creates).
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:34 InMemoryContentInfo
struct Inode {
    kind: InodeKind,
    last_modified_time: i64,
    last_change_time: i64,
    /// Stores the file's permission bits.
    permissions: u32,
}

enum InodeKind {
    /// `InMemoryDirectoryInfo`: its children by name, including "." and "..".
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryDirectoryInfo.java InMemoryDirectoryInfo
    Directory(HashMap<String, Ino>),
    /// `InMemoryLinkInfo`: the link content (the normalized content is the same).
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryLinkInfo.java InMemoryLinkInfo
    Link(PathFragment),
    /// `RemoteInMemoryFileInfo`: its metadata, `None` until `set`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:956 RemoteActionFileSystem.RemoteInMemoryFileInfo
    File(Option<FileArtifactValue>),
}

/// `clock.currentTimeMillis()` of the `JavaClock` an `InMemoryFileSystem` is created with.
fn current_time_millis() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

impl Inode {
    /// `new InMemoryContentInfo(clock)`: when we create the file, it is modified.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:56 InMemoryContentInfo.<init>
    fn new(kind: InodeKind) -> Inode {
        let now = current_time_millis();
        Inode {
            kind,
            last_modified_time: now,
            last_change_time: now,
            permissions: 0o644,
        }
    }

    fn is_directory(&self) -> bool {
        matches!(self.kind, InodeKind::Directory(_))
    }

    /// `getSize()`: for files the length in bytes, for directories the number of children
    /// (including "." and ".."), for links the length of the link content.
    fn get_size(&self) -> i64 {
        match &self.kind {
            InodeKind::Directory(children) => children.len() as i64,
            // `getSafePathString().length()`: chars of the internal string (one per byte).
            // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryLinkInfo.java:60 InMemoryLinkInfo.getSize
            InodeKind::Link(link_content) => {
                link_content.get_safe_path_string().chars().count() as i64
            }
            InodeKind::File(metadata) => metadata
                .as_ref()
                .expect("NullPointerException: RemoteInMemoryFileInfo without metadata")
                .get_size(),
        }
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:141 InMemoryContentInfo.markModificationTime
    fn mark_modification_time(&mut self) {
        self.last_modified_time = current_time_millis();
        self.last_change_time = self.last_modified_time;
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:147 InMemoryContentInfo.markChangeTime
    fn mark_change_time(&mut self) {
        self.last_change_time = current_time_millis();
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:135 InMemoryContentInfo.setLastModifiedTime
    fn set_last_modified_time(&mut self, new_time: i64) {
        self.last_modified_time = new_time;
        self.mark_change_time();
    }

    fn is_readable(&self) -> bool {
        self.check_permissions(0o400)
    }

    fn set_readable(&mut self, readable: bool) {
        self.update_permissions(0o400, readable);
    }

    fn is_writable(&self) -> bool {
        self.check_permissions(0o200)
    }

    fn set_writable(&mut self, writable: bool) {
        self.update_permissions(0o200, writable);
    }

    fn is_executable(&self) -> bool {
        self.check_permissions(0o100)
    }

    fn set_executable(&mut self, executable: bool) {
        self.update_permissions(0o111, executable);
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryContentInfo.java:182 InMemoryContentInfo.chmod
    fn chmod(&mut self, permissions: u32) {
        self.permissions = permissions;
        self.mark_change_time();
    }

    fn check_permissions(&self, mask: u32) -> bool {
        (self.permissions & mask) != 0
    }

    fn update_permissions(&mut self, mask: u32, set: bool) {
        let permissions = if set {
            self.permissions | mask
        } else {
            self.permissions & !mask
        };
        self.chmod(permissions);
    }
}

/// The inodes of a [`RemoteInMemoryFileSystem`] (one lock: Java's methods are `synchronized` on
/// the file system).
struct Inodes {
    nodes: Vec<Inode>,
}

impl Inodes {
    fn get(&self, ino: Ino) -> &Inode {
        &self.nodes[ino]
    }

    fn get_mut(&mut self, ino: Ino) -> &mut Inode {
        &mut self.nodes[ino]
    }

    fn add(&mut self, inode: Inode) -> Ino {
        self.nodes.push(inode);
        self.nodes.len() - 1
    }

    /// `new InMemoryDirectoryInfo(clock)`: a directory, executable.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryDirectoryInfo.java:42 InMemoryDirectoryInfo.<init>
    fn new_directory(&mut self) -> Ino {
        let mut inode = Inode::new(InodeKind::Directory(HashMap::new()));
        inode.set_executable(true);
        self.add(inode)
    }

    /// `dir.getChild(name)`.
    fn get_child(&self, dir: Ino, name: &str) -> Option<Ino> {
        match &self.get(dir).kind {
            InodeKind::Directory(children) => children.get(name).copied(),
            _ => panic!("IllegalStateException: Not a directory"),
        }
    }

    /// `dir.addChild(name, inode)`.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryDirectoryInfo.java:52 InMemoryDirectoryInfo.addChild
    fn add_child(&mut self, dir: Ino, name: &str, inode: Ino) {
        let dir_inode = self.get_mut(dir);
        let InodeKind::Directory(children) = &mut dir_inode.kind else {
            panic!("IllegalStateException: Not a directory");
        };
        if children.insert(name.to_string(), inode).is_some() {
            panic!("IllegalArgumentException: File already exists: {name}");
        }
        dir_inode.mark_modification_time();
    }

    /// `dir.removeChild(name)`.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryDirectoryInfo.java:72 InMemoryDirectoryInfo.removeChild
    fn remove_child(&mut self, dir: Ino, name: &str) {
        let dir_inode = self.get_mut(dir);
        let InodeKind::Directory(children) = &mut dir_inode.kind else {
            panic!("IllegalStateException: Not a directory");
        };
        if children.remove(name).is_none() {
            panic!("IllegalArgumentException: {name} is not a member of this directory");
        }
        dir_inode.mark_modification_time();
    }

    /// `unlink(dir, child, errorPath)`: unlinks the entry `child` from its parent directory
    /// `dir`. Succeeds even if `child` names a non-empty directory (renameTo needs that). Fails
    /// if the directory was read-only.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:162 InMemoryFileSystem.unlink
    fn unlink(&mut self, dir: Ino, child: &str, error_path: &PathFragment) -> io::Result<()> {
        if !self.get(dir).is_executable() || !self.get(dir).is_writable() {
            return Err(Errno::Eacces.exception(error_path));
        }
        self.remove_child(dir, child);
        Ok(())
    }

    /// `insert(dir, child, childInode)`: fails if the directory was read-only.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:176 InMemoryFileSystem.insert
    fn insert(&mut self, dir: Ino, child: &str, child_inode: Ino) -> Option<Errno> {
        if !self.get(dir).is_writable() {
            return Some(Errno::Eacces);
        }
        self.add_child(dir, child, child_inode);
        None
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:185 InMemoryFileSystem.insert
    fn insert_or_throw(
        &mut self,
        dir: Ino,
        child: &str,
        child_inode: Ino,
        error_path: &PathFragment,
    ) -> io::Result<()> {
        match self.insert(dir, child, child_inode) {
            Some(error) => Err(error.exception(error_path)),
            None => Ok(()),
        }
    }

    /// `insertChildDirectory(parent, newDir, name)`.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:509 InMemoryFileSystem.insertChildDirectory
    fn insert_child_directory(&mut self, parent: Ino, new_dir: Ino, name: &str) -> Option<Errno> {
        self.add_child(new_dir, ".", new_dir);
        self.add_child(new_dir, "..", parent);
        self.insert(parent, name, new_dir)
    }

    /// `directoryLookupErrno(dir, name)`: looks up `name` within the existing directory `dir`.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:201 InMemoryFileSystem.directoryLookupErrno
    fn directory_lookup_errno(&self, dir: Ino, name: &str) -> Result<Ino, Errno> {
        if !self.get(dir).is_directory() {
            return Err(Errno::Enotdir);
        }
        if !self.get(dir).is_executable() {
            return Err(Errno::Eacces);
        }
        self.get_child(dir, name).ok_or(Errno::Enoent)
    }

    /// `pathWalkErrno(path, behavior)`: walks the directory tree from the root, resolving all
    /// symbolic links, and returns the designated inode. ENOENT along the walk is handled
    /// according to `behavior`. May fail with ENOTDIR, ENOENT, EACCES, ELOOP.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:237 InMemoryFileSystem.pathWalkErrno
    fn path_walk_errno(&mut self, path: &PathFragment, behavior: OnEnoent) -> Result<Ino, Errno> {
        // PORT: skipped, Windows only (the drive prefix).
        let mut it = path.segments().peekable();

        let mut inode = ROOT_INODE;
        let mut traversals = 0;

        // Stack of symlink targets.
        let mut symlinks: VecDeque<String> = VecDeque::new();

        while it.peek().is_some() || !symlinks.is_empty() {
            traversals += 1;

            let name = match symlinks.pop_front() {
                Some(name) => name,
                None => it.next().expect("has next").to_string(),
            };

            let child = match self.directory_lookup_errno(inode, &name) {
                Ok(child) => child,
                Err(Errno::Enoent) if behavior != OnEnoent::Halt => {
                    let parent = inode;
                    let (child, error) = if behavior == OnEnoent::CreateDirectoryAndParents {
                        // ENOENT anywhere with Create.DIRECTORY_AND_PARENTS => create a new
                        // directory.
                        let new_dir = self.new_directory();
                        (new_dir, self.insert_child_directory(parent, new_dir, &name))
                    } else if it.peek().is_none() && symlinks.is_empty() {
                        // ENOENT on last segment with Create.FILE => create a new file.
                        let child = self.new_file();
                        (child, self.insert(parent, &name, child))
                    } else {
                        return Err(Errno::Enoent);
                    };
                    if let Some(error) = error {
                        return Err(error);
                    }
                    child
                }
                Err(error) => return Err(error),
            };

            match &self.get(child).kind {
                InodeKind::Link(link_target) => {
                    let link_target = link_target.clone();
                    if link_target.is_absolute() {
                        inode = ROOT_INODE;
                    }
                    if traversals > MAX_TRAVERSALS {
                        return Err(Errno::Eloop);
                    }
                    // May include ".." segments.
                    for segment in link_target.split_to_list_of_segments().into_iter().rev() {
                        symlinks.push_front(segment);
                    }
                }
                _ => inode = child,
            }
        }
        Ok(inode)
    }

    /// `newFile(clock, path)`: `RemoteInMemoryFileSystem` creates a `RemoteInMemoryFileInfo`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:939 RemoteInMemoryFileSystem.newFile
    fn new_file(&mut self) -> Ino {
        self.add(Inode::new(InodeKind::File(None)))
    }

    /// `getDirectoryErrno(path)`: the existing directory inode `path` designates, following
    /// symbolic links.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:316 InMemoryFileSystem.getDirectoryErrno
    fn get_directory_errno(&mut self, path: &PathFragment) -> Result<Ino, Errno> {
        let ino = self.path_walk_errno(path, OnEnoent::Halt)?;
        if self.get(ino).is_directory() {
            Ok(ino)
        } else {
            Err(Errno::Enotdir)
        }
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:328 InMemoryFileSystem.getDirectory
    fn get_directory(&mut self, path: &PathFragment) -> io::Result<Ino> {
        self.get_directory_errno(path)
            .map_err(|e| e.exception(path))
    }

    /// `noFollowStatErrno(path)`: the path's status, not following a symlink at the end.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:333 InMemoryFileSystem.noFollowStatErrno
    fn no_follow_stat_errno(&mut self, path: &PathFragment) -> Result<Ino, Errno> {
        let parent = path.get_parent_directory().expect("not the root directory");
        let dir = self.get_directory_errno(&parent)?;
        self.directory_lookup_errno(dir, path.get_base_name())
    }

    /// `inodeStatErrno(path, followSymlinks)`.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:372 InMemoryFileSystem.inodeStatErrno
    fn inode_stat_errno(
        &mut self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> Result<Ino, Errno> {
        if follow_symlinks {
            return self.path_walk_errno(path, OnEnoent::Halt);
        }
        if is_root_directory(path) {
            Ok(ROOT_INODE)
        } else {
            self.no_follow_stat_errno(path)
        }
    }

    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:379 InMemoryFileSystem.inodeStat
    fn inode_stat(&mut self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Ino> {
        self.inode_stat_errno(path, follow_symlinks)
            .map_err(|e| e.exception(path))
    }

    /// The `FileStatus` of an inode (`InMemoryContentInfo implements FileStatus`).
    ///
    /// PORT: the node id is the inode's index (Java: `System.identityHashCode`).
    fn status(&self, ino: Ino) -> FileStatus {
        let inode = self.get(ino);
        let kind = match inode.kind {
            InodeKind::Directory(_) => S_IFDIR,
            InodeKind::Link(_) => S_IFLNK,
            InodeKind::File(_) => S_IFREG,
        };
        FileStatus::new(
            kind | (inode.permissions & 0o7777),
            inode.last_modified_time,
            inode.last_change_time,
            inode.get_size(),
            ino as i64,
        )
    }

    /// `statFile(path)`: the file inode at `path`, following symlinks.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:635 InMemoryFileSystem.statFile
    fn stat_file(&mut self, path: &PathFragment) -> io::Result<Ino> {
        let status = self.inode_stat(path, /* follow_symlinks= */ true)?;
        if self.get(status).is_directory() {
            return Err(Errno::Eisdir.exception(path));
        }
        if !self.get(status).is_readable() {
            return Err(Errno::Eacces.exception(path));
        }
        blaze_util::check_state!(matches!(self.get(status).kind, InodeKind::File(_)));
        Ok(status)
    }

    /// `getOrCreateWritableInode(path)`: creates a new file at the given path and returns its
    /// inode. Writing to a dangling link writes through the link, as on UNIX.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:666 InMemoryFileSystem.getOrCreateWritableInode
    fn get_or_create_writable_inode(&mut self, path: &PathFragment) -> io::Result<Ino> {
        let child = self
            .path_walk_errno(path, OnEnoent::CreateFile)
            .map_err(|e| e.exception(path))?;
        if self.get(child).is_directory() {
            return Err(Errno::Eisdir.exception(path));
        }
        if !self.get(child).is_writable() {
            return Err(Errno::Eacces.exception(path));
        }
        Ok(child)
    }
}

/// How to handle ENOENT during `pathWalkErrno`.
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:216 InMemoryFileSystem.OnEnoent
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum OnEnoent {
    /// Halt the walk with ENOENT.
    Halt,
    /// Create a file node if at the last segment of the walk, otherwise halt with ENOENT.
    CreateFile,
    /// Create a directory node.
    CreateDirectoryAndParents,
}

/// `isRootDirectory(path)`: on Unix the root directory is "/".
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:765 InMemoryFileSystem.isRootDirectory
fn is_root_directory(path: &PathFragment) -> bool {
    path.get_path_string() == "/"
}

/// The remote output tree of a [`RemoteActionFileSystem`]: an in-memory file system
/// (`InMemoryFileSystem`) whose files are `RemoteInMemoryFileInfo`s, holding the metadata (not
/// the contents) of the outputs that were not downloaded, plus the directories and symlinks the
/// action created under the output base.
///
/// PORT: Blaze has no `InMemoryFileSystem`; the part `RemoteInMemoryFileSystem` uses is ported
/// here (paths are Unix paths; the Windows drive handling is skipped).
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:925 RemoteActionFileSystem.RemoteInMemoryFileSystem
// Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:62 InMemoryFileSystem
pub struct RemoteInMemoryFileSystem {
    digest_function: DigestHashFunction,
    inodes: parking_lot::Mutex<Inodes>,
}

impl std::fmt::Debug for RemoteInMemoryFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteInMemoryFileSystem")
            .finish_non_exhaustive()
    }
}

impl RemoteInMemoryFileSystem {
    /// `new RemoteInMemoryFileSystem(hashFunction)`: an empty file system (the root directory).
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:927 RemoteInMemoryFileSystem.<init>
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:88 InMemoryFileSystem.newRootInode
    pub fn new(digest_function: DigestHashFunction) -> RemoteInMemoryFileSystem {
        let mut inodes = Inodes { nodes: Vec::new() };
        let root = inodes.new_directory();
        debug_assert_eq!(root, ROOT_INODE);
        inodes.add_child(root, ".", root);
        inodes.add_child(root, "..", root);
        RemoteInMemoryFileSystem {
            digest_function,
            inodes: parking_lot::Mutex::new(inodes),
        }
    }

    /// `injectFile(path, metadata)`: records the remote file `path` (creating its parent
    /// directories); fails if a node that is not a regular file is in the way.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:943 RemoteInMemoryFileSystem.injectFile
    pub fn inject_file(&self, path: &PathFragment, metadata: FileArtifactValue) -> io::Result<()> {
        blaze_util::check_argument!(
            metadata.is_remote(),
            "metadata is not remote: {:?}",
            metadata
        );
        self.create_directory_and_parents(
            &path
                .get_parent_directory()
                .expect("NullPointerException: path has no parent"),
        )?;
        let mut inodes = self.inodes.lock();
        let node = inodes.get_or_create_writable_inode(path)?;
        // If a node already exists but is not a regular file, throw an error.
        let inode = inodes.get_mut(node);
        match &mut inode.kind {
            InodeKind::File(file_metadata) => {
                *file_metadata = Some(metadata);
                Ok(())
            }
            // PORT: Java prints the node's toString (an identity hash code).
            InodeKind::Link(target) => Err(io::Error::other(format!(
                "Could not inject into InMemoryLinkInfo -> {}",
                target.get_path_string()
            ))),
            InodeKind::Directory(_) => Err(io::Error::other(
                "Could not inject into InMemoryDirectoryInfo".to_string(),
            )),
        }
    }

    /// `statIfFound(path, followSymlinks)` with the metadata of a `RemoteInMemoryFileInfo`
    /// (`FileStatusWithMetadata.getMetadata()`); `None` for ENOENT and ENOTDIR.
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:353 InMemoryFileSystem.statIfFound
    pub fn stat_if_found_with_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<(FileStatus, Option<FileArtifactValue>)>> {
        let mut inodes = self.inodes.lock();
        match inodes.inode_stat_errno(path, follow_symlinks) {
            Ok(ino) => {
                let metadata = match &inodes.get(ino).kind {
                    InodeKind::File(metadata) => metadata.clone(),
                    _ => None,
                };
                Ok(Some((inodes.status(ino), metadata)))
            }
            Err(Errno::Enoent | Errno::Enotdir) => Ok(None),
            Err(error) => Err(error.exception(path)),
        }
    }
}

impl FileSystem for RemoteInMemoryFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.digest_function
    }
    /// A regular file's stat here is a `RemoteInMemoryFileInfo`, a `FileStatusWithMetadata`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:956 RemoteInMemoryFileInfo
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn blaze_vfs::StatMetadata>>> {
        Ok(self
            .stat_if_found_with_metadata(path, follow_symlinks)?
            .and_then(|(_, metadata)| metadata)
            .map(|m| std::sync::Arc::new(m) as std::sync::Arc<dyn blaze_vfs::StatMetadata>))
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:454 InMemoryFileSystem.supportsModifications
    fn supports_modifications(&self, _path: &PathFragment) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:459 InMemoryFileSystem.supportsSymbolicLinksNatively
    fn supports_symbolic_links_natively(&self, _path: &PathFragment) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:464 InMemoryFileSystem.supportsHardLinksNatively
    fn supports_hard_links_natively(&self, _path: &PathFragment) -> bool {
        true
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:469 InMemoryFileSystem.mayBeCaseOrNormalizationInsensitive
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        false
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:482 InMemoryFileSystem.createDirectory
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        if is_root_directory(path) {
            return Err(Errno::Eacces.exception(path));
        }

        let parent_dir = path.get_parent_directory().expect("not the root directory");
        let name = path.get_base_name();
        let error;
        {
            let mut inodes = self.inodes.lock();
            let parent = inodes.get_directory(&parent_dir)?;
            if let Some(child) = inodes.get_child(parent, name) {
                // already exists
                if !inodes.get(child).is_directory() {
                    return Err(Errno::Eexist.exception(path));
                }
                return Ok(false);
            }
            let new_dir = inodes.new_directory();
            error = inodes.insert_child_directory(parent, new_dir, name);
        }
        if let Some(error) = error {
            return Err(error.exception(path));
        }
        Ok(true)
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:517 InMemoryFileSystem.createDirectoryAndParents
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let result = inodes
            .path_walk_errno(path, OnEnoent::CreateDirectoryAndParents)
            .map_err(|e| e.exception(path))?;
        if !inodes.get(result).is_directory() {
            return Err(io::Error::other(format!(
                "Not a directory: {}",
                path.get_path_string()
            )));
        }
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:553 InMemoryFileSystem.getFileSize
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_size())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:575 InMemoryFileSystem.delete
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        if is_root_directory(path) {
            return Err(Errno::Ebusy.exception(path));
        }

        let mut inodes = self.inodes.lock();
        let parent_path = path.get_parent_directory().expect("not the root directory");
        match inodes.get_directory_errno(&parent_path) {
            Ok(parent) => {
                let Some(child) = inodes.get_child(parent, path.get_base_name()) else {
                    return Ok(false);
                };
                if inodes.get(child).is_directory() && inodes.get(child).get_size() > 2 {
                    return Err(Errno::Enotempty.exception(path));
                }
                inodes.unlink(parent, path.get_base_name(), path)?;
                Ok(true)
            }
            Err(Errno::Enoent | Errno::Enotdir) => Ok(false),
            Err(error) => Err(error.exception(path)),
        }
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:607 InMemoryFileSystem.getLastModifiedTime
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        Ok(self.stat(path, follow_symlinks)?.get_last_modified_time())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:612 InMemoryFileSystem.setLastModifiedTime
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        inodes
            .get_mut(status)
            .set_last_modified_time(if new_time == NOW_SENTINEL_TIME {
                current_time_millis()
            } else {
                new_time
            });
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:631 InMemoryFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        let mut inodes = self.inodes.lock();
        let file = inodes.stat_file(path)?;
        // RemoteInMemoryFileInfo.getFastDigest(): metadata.getDigest().
        match &inodes.get(file).kind {
            InodeKind::File(metadata) => Ok(metadata
                .as_ref()
                .expect("NullPointerException: RemoteInMemoryFileInfo without metadata")
                .get_digest()
                .map(<[u8]>::to_vec)),
            _ => unreachable!("statFile returns a file"),
        }
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:399 InMemoryFileSystem.resolveOneLink
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        // Beware, this seemingly simple code belies the complex specification of
        // FileSystem.resolveOneLink().
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, false)?;
        match &inodes.get(status).kind {
            InodeKind::Link(link_content) => Ok(Some(link_content.clone())),
            _ => Ok(None),
        }
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:347 InMemoryFileSystem.stat
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        let mut inodes = self.inodes.lock();
        let ino = inodes.inode_stat(path, follow_symlinks)?;
        Ok(inodes.status(ino))
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:363 InMemoryFileSystem.statNullable
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        let mut inodes = self.inodes.lock();
        let ino = inodes.inode_stat_errno(path, follow_symlinks).ok()?;
        Some(inodes.status(ino))
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:353 InMemoryFileSystem.statIfFound
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        Ok(self
            .stat_if_found_with_metadata(path, follow_symlinks)?
            .map(|(status, _)| status))
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:526 InMemoryFileSystem.createSymbolicLink
    fn create_symbolic_link(
        &self,
        path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        if is_root_directory(path) {
            return Err(Errno::Eacces.exception(path));
        }

        let mut inodes = self.inodes.lock();
        let parent =
            inodes.get_directory(&path.get_parent_directory().expect("not the root directory"))?;
        if inodes.get_child(parent, path.get_base_name()).is_some() {
            return Err(Errno::Eexist.exception(path));
        }
        let link = inodes.add(Inode::new(InodeKind::Link(target_fragment.clone())));
        inodes.insert_or_throw(parent, path.get_base_name(), link, path)
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:543 InMemoryFileSystem.readSymbolicLink
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, false)?;
        match &inodes.get(status).kind {
            InodeKind::Link(link_content) => Ok(link_content.clone()),
            _ => Err(NotASymlinkException::new_error(path)),
        }
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:407 InMemoryFileSystem.exists
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.stat_nullable(path, follow_symlinks).is_some()
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:558 InMemoryFileSystem.getDirectoryEntries
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        let mut inodes = self.inodes.lock();
        let dir_info = inodes.get_directory(path)?;
        if !inodes.get(dir_info).is_readable() {
            return Err(Errno::Eacces.exception(path));
        }

        let InodeKind::Directory(all_children) = &inodes.get(dir_info).kind else {
            unreachable!("getDirectory returns a directory");
        };
        Ok(all_children
            .keys()
            .filter(|child| *child != "." && *child != "..")
            .cloned()
            .collect())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:418 InMemoryFileSystem.isReadable
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        Ok(inodes.get(status).is_readable())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:424 InMemoryFileSystem.setReadable
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        inodes.get_mut(status).set_readable(readable);
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:430 InMemoryFileSystem.isWritable
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        Ok(inodes.get(status).is_writable())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:436 InMemoryFileSystem.setWritable
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        inodes.get_mut(status).set_writable(writable);
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:442 InMemoryFileSystem.isExecutable
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        Ok(inodes.get(status).is_executable())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:448 InMemoryFileSystem.setExecutable
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        inodes.get_mut(status).set_executable(executable);
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:412 InMemoryFileSystem.chmod
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        let mut inodes = self.inodes.lock();
        let status = inodes.inode_stat(path, true)?;
        inodes.get_mut(status).chmod(mode);
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:619 InMemoryFileSystem.getInputStream
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.inodes.lock().stat_file(path)?;
        // RemoteInMemoryFileInfo.getInputStream()
        // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:973 RemoteInMemoryFileInfo.getInputStream
        panic!("IllegalStateException: Shouldn't be called directly")
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:932 RemoteInMemoryFileSystem.getOutputStream
    fn get_output_stream(
        &self,
        _path: &PathFragment,
        _append: bool,
        _internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        // To get an output stream from remote file, we need to first stage it.
        panic!("IllegalStateException: Shouldn't be called directly")
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:690 InMemoryFileSystem.renameTo
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        if is_root_directory(source_path) {
            return Err(Errno::Eacces.exception(source_path));
        }
        if is_root_directory(target_path) {
            return Err(Errno::Eacces.exception(target_path));
        }
        let mut inodes = self.inodes.lock();
        let source_parent = inodes.get_directory(
            &source_path
                .get_parent_directory()
                .expect("not the root directory"),
        )?;
        let target_parent = inodes.get_directory(
            &target_path
                .get_parent_directory()
                .expect("not the root directory"),
        )?;

        let source_name = source_path.get_base_name();
        let target_name = target_path.get_base_name();
        let Some(source_inode) = inodes.get_child(source_parent, source_name) else {
            return Err(Errno::Enoent.exception(source_path));
        };
        let target_inode = inodes.get_child(target_parent, target_name);

        inodes.unlink(source_parent, source_name, source_path)?;
        let result = (|| -> io::Result<()> {
            // Precondition checks:
            if let Some(target_inode) = target_inode {
                // already exists
                if inodes.get(target_inode).is_directory() {
                    if !inodes.get(source_inode).is_directory() {
                        return Err(io::Error::other(format!(
                            "{} -> {} ({})",
                            source_path.get_path_string(),
                            target_path.get_path_string(),
                            Errno::Eisdir.message()
                        )));
                    }
                    if inodes.get(target_inode).get_size() > 2 {
                        return Err(Errno::Enotempty.exception(target_path));
                    }
                } else if inodes.get(source_inode).is_directory() {
                    return Err(io::Error::other(format!(
                        "{} -> {} ({})",
                        source_path.get_path_string(),
                        target_path.get_path_string(),
                        Errno::Enotdir.message()
                    )));
                }
                inodes.unlink(target_parent, target_name, target_path)?;
            }
            inodes.insert_or_throw(target_parent, target_name, source_inode, target_path)
        })();
        if let Err(e) = result {
            // restore source
            inodes.insert_or_throw(source_parent, source_name, source_inode, source_path)?;
            return Err(e);
        }
        Ok(())
    }
    // Java: com/google/devtools/build/lib/vfs/inmemoryfs/InMemoryFileSystem.java:738 InMemoryFileSystem.createFSDependentHardLink
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        // Same check used when creating a symbolic link
        if is_root_directory(original_path) {
            return Err(Errno::Eacces.exception(original_path));
        }

        let mut inodes = self.inodes.lock();
        let link_parent = inodes.get_directory(
            &link_path
                .get_parent_directory()
                .expect("not the root directory"),
        )?;
        // Same check used when creating a symbolic link
        if inodes
            .get_child(link_parent, link_path.get_base_name())
            .is_some()
        {
            return Err(Errno::Eexist.exception(link_path));
        }
        let original_parent = inodes.get_directory(
            &original_path
                .get_parent_directory()
                .expect("not the root directory"),
        )?;
        let original = inodes
            .get_child(original_parent, original_path.get_base_name())
            .expect("NullPointerException: inode");
        inodes.insert_or_throw(link_parent, link_path.get_base_name(), original, link_path)
    }
}

#[cfg(test)]
#[path = "path_canonicalizer_tests.rs"]
mod path_canonicalizer_tests;
