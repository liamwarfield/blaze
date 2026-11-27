//! Port of `lib/remote/RemoteExternalOverlayFileSystem.java` (and its `SubtreeMaterializer`
//! interface, `lib/remote/SubtreeMaterializer.java`).
//!
//! PORT notes for the whole file:
//! - Java's `materializationExecutor` (a virtual thread per materialization) and
//!   `TaskDeduplicator` are a per-repo lock here: a materialization runs on the calling thread,
//!   and a second caller for the same repo waits for the first one (interruptibly, as Java's
//!   `getFromFuture` waits) and then finds the repo materialized. An interrupted materialization
//!   leaves the repo in memory for the next caller. `afterCommand` has nothing to wait for (no
//!   materialization outlives its caller).
//! - `invalidateRepoDirectories` deletes nodes of the server's `MemoizingEvaluator`; Blaze's
//!   runtime hands a callback for it (`CommandState::invalidate_repo_directories`), which is a
//!   no-op where the evaluator does not outlive the command.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use blaze_cmdline::RepositoryName;
use blaze_events::{Event, ExtendedEventHandler, FetchProgress, Postable};
use blaze_exec::actions::{ActionInputRef, BasicActionInput, FileArtifactValue};
use blaze_exec::exec::remote_action_file_system::RemoteInMemoryFileSystem;
use blaze_util::string_encoding::unicode_to_internal;
use blaze_vfs::{
    DigestHashFunction, Dirent, DirentType, FileStatus, FileSystem, FileSystemRef, Path,
    PathFragment,
};
use indexmap::IndexSet;
use parking_lot::{Mutex, RwLock};

use crate::abstract_action_input_prefetcher::{AbstractActionInputPrefetcher, Priority, Reason};
use crate::combined_cache::CombinedCache;
use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::remote_action_execution_context::{CachePolicy, RemoteActionExecutionContext};
use crate::reapi::{Digest, Directory, Tree};
use crate::remote_action_input_fetcher::RemoteActionInputFetcher;
use crate::runtime::{block_on_interruptibly, get_from_future};
use crate::util::digest_util::DigestUtil;
use crate::util::tracing_metadata_utils;
use crate::util::utils::wait_for_bulk_transfer;

/// A file system that can materialize a subtree that it holds in memory to the native file
/// system.
// Java: com/google/devtools/build/lib/remote/SubtreeMaterializer.java:23 SubtreeMaterializer
pub trait SubtreeMaterializer: Send + Sync {
    /// Materializes the subtree rooted at the given path to the native file system if it is only
    /// available in memory.
    // Java: com/google/devtools/build/lib/remote/SubtreeMaterializer.java:30 SubtreeMaterializer.ensureSubtreeMaterialized
    fn ensure_subtree_materialized(&self, path: &PathFragment) -> io::Result<()>;
}

/// Per-build information that is set in `beforeCommand` and cleared in `afterCommand`.
pub struct CommandState {
    pub cache: Arc<CombinedCache>,
    pub input_prefetcher: Arc<RemoteActionInputFetcher>,
    pub reporter: Arc<dyn ExtendedEventHandler>,
    pub build_request_id: String,
    pub command_id: String,
    /// `evaluator.delete(REPOSITORY_DIRECTORY nodes of these repos)` (see the module comment).
    pub invalidate_repo_directories: Arc<dyn Fn(&HashSet<String>) + Send + Sync>,
    pub remote_cache_ttl: Duration,
}

/// What [`RemoteExternalOverlayFileSystem`] and its in-memory part share.
struct Shared {
    external_directory: PathFragment,
    native_fs: FileSystemRef,
    state: RwLock<Option<Arc<CommandState>>>,
    repos_with_lost_files: Mutex<HashSet<String>>,
}

impl Shared {
    fn state(&self) -> Arc<CommandState> {
        self.state
            .read()
            .clone()
            .expect("RemoteExternalOverlayFileSystem used outside of a command")
    }
}

/// A file system that overlays the native file system with a `RemoteExternalFileSystem` for the
/// "external" directory, which contains the contents of external repositories.
///
/// Each external repository can either be materialized to the native file system or kept in
/// memory in the `RemoteExternalFileSystem`.
// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:90 RemoteExternalOverlayFileSystem
pub struct RemoteExternalOverlayFileSystem {
    external_directory_segment_count: usize,
    shared: Arc<Shared>,
    external_fs: Arc<RemoteExternalFileSystem>,
    /// `materializations` (see the module comment): one lock per repo name.
    /// An async lock, so that a caller waiting for another's materialization can be interrupted,
    /// as Java's wait on the shared future (`getFromFuture`) can.
    materializations: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The names of the repos whose contents have been fully materialized to nativeFs.
    materialized_repos: Mutex<HashSet<String>>,
    /// As long as a repo name appears as a key in this map, the repo contents are available in
    /// externalFs.
    marker_file_contents: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for RemoteExternalOverlayFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteExternalOverlayFileSystem")
            .field("external_directory", &self.shared.external_directory)
            .finish_non_exhaustive()
    }
}

impl RemoteExternalOverlayFileSystem {
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:114 RemoteExternalOverlayFileSystem.<init>
    pub fn new(
        external_directory: PathFragment,
        native_fs: FileSystemRef,
    ) -> RemoteExternalOverlayFileSystem {
        let shared = Arc::new(Shared {
            external_directory: external_directory.clone(),
            native_fs: native_fs.clone(),
            state: RwLock::new(None),
            repos_with_lost_files: Mutex::new(HashSet::new()),
        });
        RemoteExternalOverlayFileSystem {
            external_directory_segment_count: external_directory.segment_count(),
            external_fs: Arc::new(RemoteExternalFileSystem {
                inner: RemoteInMemoryFileSystem::new(native_fs.get_digest_function()),
                shared: shared.clone(),
            }),
            shared,
            materializations: Mutex::new(HashMap::new()),
            materialized_repos: Mutex::new(HashSet::new()),
            marker_file_contents: Mutex::new(HashMap::new()),
        }
    }

    /// `fs instanceof RemoteExternalOverlayFileSystem`: the overlay behind a file system handle.
    pub fn of(fs: &FileSystemRef) -> Option<Arc<RemoteExternalOverlayFileSystem>> {
        let any: Arc<dyn std::any::Any + Send + Sync> = fs.clone();
        any.downcast::<RemoteExternalOverlayFileSystem>().ok()
    }

    fn external_directory(&self) -> &PathFragment {
        &self.shared.external_directory
    }

    fn native_fs(&self) -> &FileSystemRef {
        &self.shared.native_fs
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:122 RemoteExternalOverlayFileSystem.beforeCommand
    pub fn before_command(&self, state: CommandState) {
        let mut current = self.shared.state.write();
        blaze_util::check_state!(current.is_none());
        *current = Some(Arc::new(state));
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:151 RemoteExternalOverlayFileSystem.afterCommand
    pub fn after_command(&self) {
        let Some(state) = self.shared.state.write().take() else {
            // Not all commands cause beforeCommand to be called, but afterCommand is called
            // unconditionally.
            return;
        };

        // Clean up the in-memory contents of materialized repos to save memory, or those that
        // need to be refetched to recover files that the remote cache has lost. This wouldn't be
        // safe to do eagerly as ongoing repo rule evaluations may still refer to the in-memory
        // content and refetching is not atomic.
        let materialized: Vec<String> = self.materialized_repos.lock().iter().cloned().collect();
        for repo_name in &materialized {
            self.evict_in_memory_repo(repo_name);
        }
        let lost: HashSet<String> = std::mem::take(&mut *self.shared.repos_with_lost_files.lock());
        for repo_name in &lost {
            self.evict_in_memory_repo(repo_name);
        }
        invalidate_repo_directories_with(&*state.invalidate_repo_directories, &lost);
    }

    /// Removes the contents of the given repo from the in-memory overlay file system.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:183 RemoteExternalOverlayFileSystem.evictInMemoryRepo
    fn evict_in_memory_repo(&self, repo_name: &str) {
        if let Err(e) = self
            .external_fs
            .delete_tree(&self.external_directory().get_child(repo_name))
        {
            panic!("In-memory file system is not expected to throw: {e}");
        }
        self.materialized_repos.lock().remove(repo_name);
        self.marker_file_contents.lock().remove(repo_name);
    }

    /// Injects the given remote contents, possibly prefetching some files, and returns true on
    /// success.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:208 RemoteExternalOverlayFileSystem.injectRemoteRepo
    pub fn inject_remote_repo(
        &self,
        repo: &RepositoryName,
        remote_contents: &Tree,
        marker_file: &str,
    ) -> io::Result<bool> {
        let state = self.shared.state();
        let repo_dir = self.external_directory().get_child(repo.get_name());
        self.delete_tree(&repo_dir)?;
        self.materialized_repos.lock().remove(repo.get_name());
        let _unused = self.delete(
            &self
                .external_directory()
                .get_child(&repo.get_marker_file_name()),
        )?;
        // toImmutableMap(cache.digestUtil::compute, directory -> directory, (a, b) -> a)
        let digest_util = state.cache.digest_util();
        let mut child_map: HashMap<Digest, &Directory> = HashMap::new();
        for child in &remote_contents.children {
            child_map
                .entry(digest_util.compute_message(child))
                .or_insert(child);
        }
        let mut files_to_prefetch: IndexSet<PathFragment> = IndexSet::new();
        let mut symlinks_to_prefetch: Vec<PathFragment> = Vec::new();
        self.external_fs
            .inner
            .create_directory_and_parents(&repo_dir.get_parent_directory().expect("parent"))?;
        let default_root = Directory::default();
        inject_recursively(
            &self.external_fs,
            &repo_dir,
            &repo_dir,
            remote_contents.root.as_ref().unwrap_or(&default_root),
            &child_map,
            &mut files_to_prefetch,
            &mut symlinks_to_prefetch,
            expiration_time_millis(state.remote_cache_ttl),
        )?;
        self.add_symlink_targets_to_prefetch(&symlinks_to_prefetch, &mut files_to_prefetch);
        // TODO: This prefetches a large number of small files. Investigate whether
        // BatchReadBlobs would be more efficient.
        if let Err(e) = self.prefetch(&state, files_to_prefetch.iter()) {
            if BulkTransferException::all_caused_by_cache_not_found_exception_of(&e) {
                // The cache has lost the prefetched files, which should be treated just like a
                // cache miss.
                self.external_fs.delete_tree(&repo_dir)?;
                return Ok(false);
            }
            return Err(e);
        }
        // Create the repo directory on disk so that readdir reflects the overlaid state of the
        // external directory.
        self.native_fs().create_directory_and_parents(&repo_dir)?;
        // Keep the marker file contents in memory so that it can be written out when the repo is
        // materialized. This doubles as a presence marker for the in-memory repo contents.
        self.marker_file_contents
            .lock()
            .insert(repo.get_name().to_string(), marker_file.to_string());
        Ok(true)
    }

    /// Collects the targets of the given symlinks into `files_to_prefetch`.
    ///
    /// Whether a read is served from the native file system is decided by the path it is made
    /// through, which for a symlink is not the path of the file that would be prefetched. A
    /// symlink that should be prefetched thus requires its target to be materialized, no matter
    /// whether the target's own path calls for prefetching.
    ///
    /// Symlink targets can only be resolved once the entire repo has been injected, which is why
    /// this doesn't happen in `inject_recursively`.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:265 RemoteExternalOverlayFileSystem.addSymlinkTargetsToPrefetch
    fn add_symlink_targets_to_prefetch(
        &self,
        symlinks: &[PathFragment],
        files_to_prefetch: &mut IndexSet<PathFragment>,
    ) {
        for symlink in symlinks {
            let target = match self.external_fs.resolve_symbolic_links(symlink) {
                Ok(target) => target,
                // Dangling symlinks and symlink loops are reproduced verbatim and only fail when
                // read.
                Err(_) => continue,
            };
            if self.external_fs.is_file(&target, true) {
                files_to_prefetch.insert(target);
            }
        }
    }

    /// Materializes the given external repository to the native file system if it hasn't been
    /// materialized yet. This method blocks until the materialization is complete.
    ///
    /// This should only be used for cases in which the given repo is accessed non-hermetically,
    /// such as when another repo rule that depends on its files executes a command. Selective
    /// reads by Bazel or local actions are handled automatically by the file system or
    /// `AbstractActionInputPrefetcher`.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:395 RemoteExternalOverlayFileSystem.ensureMaterialized
    pub fn ensure_materialized(
        &self,
        repo: &RepositoryName,
        reporter: &dyn ExtendedEventHandler,
    ) -> io::Result<()> {
        if !self
            .marker_file_contents
            .lock()
            .contains_key(repo.get_name())
        {
            // The repo has not been injected into the in-memory file system.
            return Ok(());
        }
        let lock = self
            .materializations
            .lock()
            .entry(repo.get_name().to_string())
            .or_default()
            .clone();
        // getFromFuture: an interrupt while waiting for another caller's materialization throws
        // InterruptedException.
        let Some(_guard) = block_on_interruptibly(lock.lock_owned()) else {
            return Err(crate::common::errors::interrupted());
        };
        // Another caller may have finished since the presence check above.
        if !self
            .marker_file_contents
            .lock()
            .contains_key(repo.get_name())
        {
            return Ok(());
        }
        self.do_materialize(repo, reporter)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:420 RemoteExternalOverlayFileSystem.doMaterialize
    fn do_materialize(
        &self,
        repo: &RepositoryName,
        reporter: &dyn ExtendedEventHandler,
    ) -> io::Result<()> {
        reporter.handle(&Event::debug(format!("Materializing remote repo {repo}")));
        self.materialize_subtree(&self.external_directory().get_child(repo.get_name()))?;
        self.materialized_repos
            .lock()
            .insert(repo.get_name().to_string());

        // After the repo has been copied, atomically materialize the marker file. This ensures
        // that the repo doesn't have to be refetched after the next server restart.
        let marker_file = Path::create(
            self.external_directory()
                .get_child(&repo.get_marker_file_name()),
            self.native_fs(),
        );
        let marker_file_sibling = Path::create(
            self.external_directory()
                .get_child(&format!("{}.tmp", repo.get_marker_file_name())),
            self.native_fs(),
        );
        let content = self
            .marker_file_contents
            .lock()
            .remove(repo.get_name())
            .unwrap_or_default();
        // FileSystemUtils.writeContentAsLatin1
        {
            let mut out = marker_file_sibling.get_output_stream()?;
            out.write_all(&blaze_util::string_encoding::latin1_encode_lossy(&content))?;
        }
        marker_file_sibling.rename_to(&marker_file)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:436 RemoteExternalOverlayFileSystem.prefetch
    fn prefetch<'a>(
        &self,
        state: &CommandState,
        paths: impl Iterator<Item = &'a PathFragment> + Clone,
    ) -> io::Result<()> {
        // These paths may have been prefetched and then deleted again earlier in this invocation,
        // e.g. by an injection whose fetch was subsequently restarted due to memory pressure. The
        // prefetcher's download cache would otherwise consider them downloaded already and not
        // even verify they exist on the local file system.
        let prefetcher: &dyn AbstractActionInputPrefetcher = &*state.input_prefetcher;
        prefetcher.invalidate_downloads(paths.clone());
        let inputs: Vec<ActionInputRef> = paths
            .map(|p| ActionInputRef(Arc::new(BasicActionInput::from_path_fragment(p.clone()))))
            .collect();
        if inputs.is_empty() {
            return Ok(());
        }
        let external_fs = self.external_fs.clone();
        let metadata_supplier =
            move |input: &ActionInputRef| -> io::Result<Option<FileArtifactValue>> {
                external_fs.get_metadata(input.get_exec_path()).map(Some)
            };
        get_from_future(prefetcher.prefetch_files_interruptibly(
            None,
            &inputs,
            &metadata_supplier,
            Priority::CRITICAL,
            Reason::INPUTS,
        ))
    }

    /// Informs the FS that no cache is available and in-memory repos can no longer be used.
    ///
    /// Must not be called while accessing external repos.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:456 RemoteExternalOverlayFileSystem.notifyNoCacheAvailable
    pub fn notify_no_cache_available(
        &self,
        invalidate_repo_directories: &dyn Fn(&HashSet<String>),
    ) {
        blaze_util::check_state!(
            self.shared.state.read().is_none(),
            "must not be called when active"
        );
        let repos_to_discard: HashSet<String> =
            self.marker_file_contents.lock().keys().cloned().collect();
        for repo_name in &repos_to_discard {
            self.evict_in_memory_repo(repo_name);
        }
        invalidate_repo_directories_with(invalidate_repo_directories, &repos_to_discard);
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:480 RemoteExternalOverlayFileSystem.materializeSubtree
    fn materialize_subtree(&self, path: &PathFragment) -> io::Result<()> {
        let state = self.shared.state();
        let mut files: IndexSet<PathFragment> = IndexSet::new();
        let mut symlinks: IndexSet<PathFragment> = IndexSet::new();
        let mut root = path.clone();
        if self.external_fs.is_symbolic_link(&root) {
            symlinks.insert(path.clone());
            root = self.external_fs.resolve_symbolic_links(&root)?;
        }
        self.collect_and_create_directories(&root, &mut files, &mut symlinks, &mut HashSet::new())?;
        self.prefetch(&state, files.iter())?;
        // Create symlinks last as some platforms don't allow creating a symlink to a
        // non-existent target.
        self.prefetch(&state, symlinks.iter())
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:496 RemoteExternalOverlayFileSystem.collectAndCreateDirectories
    fn collect_and_create_directories(
        &self,
        dir: &PathFragment,
        files: &mut IndexSet<PathFragment>,
        symlinks: &mut IndexSet<PathFragment>,
        visited_dirs: &mut HashSet<PathFragment>,
    ) -> io::Result<()> {
        if !visited_dirs.insert(dir.clone()) {
            return Ok(());
        }
        self.native_fs().create_directory_and_parents(dir)?;
        for dirent in self.external_fs.readdir(dir, false)? {
            let child = dir.get_child(dirent.get_name());
            match dirent.get_type() {
                DirentType::File => {
                    files.insert(child);
                }
                DirentType::Symlink => {
                    symlinks.insert(child.clone());
                    // The symlink chain is reproduced verbatim on the native file system, but
                    // its target may lie outside the materialized subtree and has to be
                    // materialized as well so that the chain doesn't dangle.
                    let target = match self.external_fs.resolve_symbolic_links(&child) {
                        Ok(target) => target,
                        // Dangling symlinks and symlink loops are reproduced verbatim.
                        Err(e) if is_not_found_or_loop(&e) => continue,
                        Err(e) => return Err(e),
                    };
                    // TODO(#30160): RepositoryUtils.replantSymlinks currently ensures that all
                    // symlinks within a remotely cacheable external repo stay within that repo.
                    // If that changes, new logic has to be added here to prefetch such files
                    // correctly.
                    if self.external_fs.is_directory(&target, false) {
                        self.collect_and_create_directories(
                            &target,
                            files,
                            symlinks,
                            visited_dirs,
                        )?;
                    } else {
                        files.insert(target);
                    }
                }
                DirentType::Directory => {
                    self.collect_and_create_directories(&child, files, symlinks, visited_dirs)?;
                }
                _ => {
                    return Err(io::Error::other(format!(
                        "Unsupported file type: {dirent:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// All other methods delegate to the file system given by this method.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:579 RemoteExternalOverlayFileSystem.fsForPath
    fn fs_for_path(&self, path: &PathFragment) -> &dyn FileSystem {
        let external_directory = self.external_directory();
        if path.starts_with(external_directory) && path != external_directory {
            let repo_name = path.get_segment(self.external_directory_segment_count);
            let has_been_injected = self.marker_file_contents.lock().contains_key(repo_name);
            let has_been_materialized = self.materialized_repos.lock().contains(repo_name);
            if has_been_injected && !has_been_materialized {
                // The repo may have been deleted due to refetching. Clean up in-memory state if
                // that is the case.
                if self
                    .external_fs
                    .exists(&external_directory.get_child(repo_name), true)
                {
                    return &*self.external_fs;
                }
                self.materialized_repos.lock().remove(repo_name);
                self.marker_file_contents.lock().remove(repo_name);
            }
            // Fall back to the native file system if the repo has been materialized, deleted, or
            // never injected.
        }
        &**self.native_fs()
    }
}

impl SubtreeMaterializer for RemoteExternalOverlayFileSystem {
    /// Materializes the subtree rooted at the given path to the native file system if it lies in
    /// a repo whose contents are currently only available in memory.
    ///
    /// This is used to make the files below a source directory action input available to local
    /// actions, which access them through the native file system.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:472 RemoteExternalOverlayFileSystem.ensureSubtreeMaterialized
    fn ensure_subtree_materialized(&self, path: &PathFragment) -> io::Result<()> {
        if !std::ptr::addr_eq(self.fs_for_path(path), &*self.external_fs) {
            return Ok(());
        }
        self.materialize_subtree(path)
    }
}

/// Invalidates the `REPOSITORY_DIRECTORY` nodes of the given repos.
// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:195 RemoteExternalOverlayFileSystem.invalidateRepoDirectories
fn invalidate_repo_directories_with(
    evaluator: &dyn Fn(&HashSet<String>),
    repo_names: &HashSet<String>,
) {
    if repo_names.is_empty() {
        return;
    }
    evaluator(repo_names);
}

/// `Instant.now().plus(remoteCacheTtl)` in milliseconds since the epoch.
fn expiration_time_millis(remote_cache_ttl: Duration) -> Option<i64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Some((now + remote_cache_ttl).as_millis() as i64)
}

fn is_not_found_or_loop(e: &io::Error) -> bool {
    // FileNotFoundException | FileSymlinkLoopException
    e.kind() == io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ELOOP)
}

// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:283 RemoteExternalOverlayFileSystem.isValidName
fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && !PathFragment::contains_separator(name)
        && !PathFragment::contains_uplevel_references_str(name)
        && PathFragment::is_normalized_relative_path(name)
}

// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:290 RemoteExternalOverlayFileSystem.injectRecursively
#[allow(clippy::too_many_arguments)]
fn inject_recursively(
    fs: &RemoteExternalFileSystem,
    repo_dir: &PathFragment,
    path: &PathFragment,
    dir: &Directory,
    child_map: &HashMap<Digest, &Directory>,
    files_to_prefetch: &mut IndexSet<PathFragment>,
    symlinks_to_prefetch: &mut Vec<PathFragment>,
    expiration_time: Option<i64>,
) -> io::Result<()> {
    // The parent directory always exists at this point: the repo's parent is created by
    // injectRemoteRepo and subdirectories are only visited after their parent has been created.
    // PORT: Java passes the number of entries as a capacity hint.
    let _unused = fs.inner.create_directory(path)?;
    for file in &dir.files {
        let name = unicode_to_internal(&file.name).into_owned();
        if !is_valid_name(&name) {
            return Err(io::Error::other(format!(
                "invalid remote repo tree node name: {name}"
            )));
        }
        let file_path = path.get_relative_str(&name);
        if !file_path.starts_with(repo_dir) {
            return Err(io::Error::other(format!(
                "Path traversal detected: {file_path} is outside {repo_dir}"
            )));
        }
        if should_prefetch(&file_path) {
            files_to_prefetch.insert(file_path.clone());
        }
        let default_digest = Digest::default();
        let digest = file.digest.as_ref().unwrap_or(&default_digest);
        fs.inner.inject_file(
            &file_path,
            // Using the *WithMaterializationData variant ensures that the file benefits from the
            // FileContentsProxy optimization to avoid widespread invalidation when it is
            // materialized later, even if expiration times aren't relevant (depends on the usage
            // of the lease extension).
            FileArtifactValue::create_for_remote_file_with_materialization_data(
                DigestUtil::to_binary_digest(digest),
                digest.size_bytes,
                /* location_index= */ 1,
                expiration_time,
                /* in_memory_output= */ false,
            ),
        )?;
        fs.inner.set_executable(&file_path, file.is_executable)?;
        // The RE API does not track whether a file is readable or writable. We choose to make all
        // files readable and not writable to ensure that other repo rules can't accidentally
        // modify the cached repo.
        fs.inner.set_writable(&file_path, false)?;
    }
    for symlink in &dir.symlinks {
        let name = unicode_to_internal(&symlink.name).into_owned();
        if !is_valid_name(&name) {
            return Err(io::Error::other(format!(
                "invalid remote repo tree node name: {name}"
            )));
        }
        let link_path = path.get_relative_str(&name);
        if !link_path.starts_with(repo_dir) {
            return Err(io::Error::other(format!(
                "Path traversal detected: {link_path} is outside {repo_dir}"
            )));
        }
        if should_prefetch(&link_path) {
            symlinks_to_prefetch.push(link_path.clone());
        }
        let target = unicode_to_internal(&symlink.target).into_owned();
        let target_fragment = PathFragment::create(&target);
        let resolved_target = if target_fragment.is_absolute() {
            target_fragment.clone()
        } else {
            link_path
                .get_parent_directory()
                .expect("a child has a parent")
                .get_relative(&target_fragment)
        };
        if !resolved_target.starts_with(repo_dir) {
            return Err(io::Error::other(format!(
                "Path traversal detected: symlink target {resolved_target} is outside {repo_dir}"
            )));
        }
        fs.inner
            .create_symbolic_link(&link_path, &target_fragment)?;
    }
    for subdir_node in &dir.directories {
        let name = unicode_to_internal(&subdir_node.name).into_owned();
        if !is_valid_name(&name) {
            return Err(io::Error::other(format!(
                "invalid remote repo tree node name: {name}"
            )));
        }
        let subdir_path = path.get_relative_str(&name);
        if !subdir_path.starts_with(repo_dir) {
            return Err(io::Error::other(format!(
                "Path traversal detected: {subdir_path} is outside {repo_dir}"
            )));
        }
        let default_digest = Digest::default();
        let subdir_digest = subdir_node.digest.as_ref().unwrap_or(&default_digest);
        let Some(subdir) = child_map.get(subdir_digest) else {
            return Err(io::Error::other(format!(
                "Directory {} with digest {} not found in tree",
                subdir_path, subdir_digest.hash
            )));
        };
        inject_recursively(
            fs,
            repo_dir,
            &subdir_path,
            subdir,
            child_map,
            files_to_prefetch,
            symlinks_to_prefetch,
            expiration_time,
        )?;
    }
    Ok(())
}

/// Whether reads of the given path should be served from the native file system, which requires
/// its contents to be materialized eagerly when injecting a repo.
///
/// This is decided by the path a read is made through, which for a symlink is not the path of
/// the file that ends up being materialized.
// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:547 RemoteExternalOverlayFileSystem.shouldPrefetch
fn should_prefetch(path: &PathFragment) -> bool {
    // .bzl files are typically small and the loads between them can form complex DAGs that can
    // only be discovered layer by layer, so prefetching is worthwhile to reduce the number of
    // sequential cache requests.
    // The REPO.bazel file, if present, is a dependency of any package and will thus have to be
    // fetched anyway.
    path.get_file_extension() == "bzl" || path.get_base_name() == "REPO.bazel"
}

/// Delegates one `FileSystem` method to `self.fs_for_path(path)`.
macro_rules! delegate {
    ($self:ident, $path:expr, $method:ident($($arg:expr),*)) => {
        $self.fs_for_path($path).$method($($arg),*)
    };
}

impl FileSystem for RemoteExternalOverlayFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.native_fs().get_digest_function()
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:556 RemoteExternalOverlayFileSystem.getHostFileSystem
    fn get_host_file_system(&self) -> Option<FileSystemRef> {
        Some(
            self.native_fs()
                .get_host_file_system()
                .unwrap_or_else(|| self.native_fs().clone()),
        )
    }

    // Always mirror tree deletions to the underlying native file system to support bazel clean
    // and repository refetching.

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:564 RemoteExternalOverlayFileSystem.deleteTree
    fn delete_tree(&self, path: &PathFragment) -> io::Result<()> {
        self.native_fs().delete_tree(path)?;
        self.external_fs.delete_tree(path)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:570 RemoteExternalOverlayFileSystem.deleteTreesBelow
    fn delete_trees_below(&self, dir: &PathFragment) -> io::Result<()> {
        self.native_fs().delete_trees_below(dir)?;
        self.external_fs.delete_trees_below(dir)
    }

    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        delegate!(self, path, delete(path))
    }

    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        delegate!(self, path, get_digest(path))
    }

    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        delegate!(self, path, get_fast_digest(path))
    }

    fn supports_modifications(&self, path: &PathFragment) -> bool {
        delegate!(self, path, supports_modifications(path))
    }

    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        delegate!(self, path, supports_symbolic_links_natively(path))
    }

    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        delegate!(self, path, supports_hard_links_natively(path))
    }

    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.fs_for_path(&self.shared.external_directory.clone())
            .may_be_case_or_normalization_insensitive()
    }

    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        delegate!(self, path, create_directory(path))
    }

    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        delegate!(self, path, create_directory_and_parents(path))
    }

    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        delegate!(self, path, get_file_size(path, follow_symlinks))
    }

    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        delegate!(self, path, get_last_modified_time(path, follow_symlinks))
    }

    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        delegate!(self, path, set_last_modified_time(path, new_time))
    }

    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        delegate!(self, path, stat(path, follow_symlinks))
    }

    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<Arc<dyn blaze_vfs::StatMetadata>>> {
        delegate!(self, path, stat_metadata(path, follow_symlinks))
    }

    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        delegate!(
            self,
            link_path,
            create_symbolic_link(link_path, target_fragment)
        )
    }

    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        delegate!(self, path, read_symbolic_link(path))
    }

    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        delegate!(self, path, exists(path, follow_symlinks))
    }

    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        delegate!(self, path, get_directory_entries(path))
    }

    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        delegate!(self, path, is_readable(path))
    }

    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        delegate!(self, path, set_readable(path, readable))
    }

    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        delegate!(self, path, is_writable(path))
    }

    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        delegate!(self, path, set_writable(path, writable))
    }

    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        delegate!(self, path, is_executable(path))
    }

    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        delegate!(self, path, set_executable(path, executable))
    }

    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        delegate!(self, path, get_input_stream(path))
    }

    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        delegate!(self, path, get_output_stream(path, append, internal))
    }

    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        delegate!(self, source_path, rename_to(source_path, target_path))
    }

    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        delegate!(
            self,
            original_path,
            create_fs_dependent_hard_link(link_path, original_path)
        )
    }

    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        delegate!(self, path, resolve_one_link(path))
    }

    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        // Ensure that the return value doesn't leave the overlay file system (a fragment here).
        delegate!(self, path, resolve_symbolic_links(path))
    }

    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        delegate!(self, path, stat_nullable(path, follow_symlinks))
    }

    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        delegate!(self, path, stat_if_found(path, follow_symlinks))
    }

    fn is_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        delegate!(self, path, is_file(path, follow_symlinks))
    }

    fn is_special_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        delegate!(self, path, is_special_file(path, follow_symlinks))
    }

    fn is_symbolic_link(&self, path: &PathFragment) -> bool {
        delegate!(self, path, is_symbolic_link(path))
    }

    fn is_directory(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        delegate!(self, path, is_directory(path, follow_symlinks))
    }

    fn read_symbolic_link_unchecked(&self, path: &PathFragment) -> io::Result<PathFragment> {
        delegate!(self, path, read_symbolic_link_unchecked(path))
    }

    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        delegate!(self, path, readdir(path, follow_symlinks))
    }

    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        delegate!(self, path, chmod(path, mode))
    }

    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        delegate!(self, link_path, create_hard_link(link_path, original_path))
    }

    fn create_temp_directory(
        &self,
        parent: &PathFragment,
        prefix: &str,
    ) -> io::Result<PathFragment> {
        delegate!(self, parent, create_temp_directory(parent, prefix))
    }
}

/// The in-memory part of the overlay: a `RemoteInMemoryFileSystem` whose reads download the
/// file from the remote cache.
// Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:822 RemoteExternalOverlayFileSystem.RemoteExternalFileSystem
#[derive(Debug)]
struct RemoteExternalFileSystem {
    inner: RemoteInMemoryFileSystem,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("external_directory", &self.external_directory)
            .finish_non_exhaustive()
    }
}

impl RemoteExternalFileSystem {
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:829 RemoteExternalFileSystem.makeRemoteContext
    fn make_remote_context(
        &self,
        state: &CommandState,
        relative_path: &PathFragment,
    ) -> RemoteActionExecutionContext<'static> {
        let repo_name = relative_path.sub_fragment(0, 1).get_base_name().to_string();
        let metadata = tracing_metadata_utils::build_metadata(
            &state.build_request_id,
            &state.command_id,
            &repo_name,
            None,
        );
        // Files in the remote external repo that Bazel reads are worth writing through to the
        // disk cache, as they are likely to be read again on future cold builds.
        RemoteActionExecutionContext::create(metadata)
            .with_read_cache_policy(CachePolicy::AnyCache)
            .with_write_cache_policy(CachePolicy::AnyCache)
    }

    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:840 RemoteExternalFileSystem.getMetadata
    fn get_metadata(self: &Arc<Self>, path: &PathFragment) -> io::Result<FileArtifactValue> {
        match self.inner.stat_if_found_with_metadata(path, false)? {
            Some((status, metadata)) if !status.is_symbolic_link() => {
                metadata.ok_or_else(|| io::Error::other(format!("{path} is not a remote file")))
            }
            Some(_) => FileArtifactValue::create_for_unresolved_symlink(&Path::create(
                path.clone(),
                &(self.clone() as FileSystemRef),
            )),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{path} (No such file or directory)"),
            )),
        }
    }
}

/// A fetch of a remote external file, as posted around a read (`ExtendedEventHandler.FetchProgress`).
#[derive(Debug)]
struct RemoteFileFetchProgress {
    resource_identifier: String,
    progress: String,
    finished: bool,
}

impl Postable for RemoteFileFetchProgress {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl FetchProgress for RemoteFileFetchProgress {
    fn get_resource_identifier(&self) -> String {
        self.resource_identifier.clone()
    }

    fn get_progress(&self) -> String {
        self.progress.clone()
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

impl RemoteExternalFileSystem {
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:849 RemoteExternalFileSystem.getInputStream
    fn read_remote(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        // Symlinks are never prefetched to the native file system themselves, only the regular
        // file they resolve to, so follow them before reading a prefetched file. Either end of
        // the chain can be what makes the read eligible: a symlink named `helper.bzl` pointing at
        // `helper.txt` as well as one named `helper.txt` pointing at `helper.bzl`.
        let prefetched = should_prefetch(path);
        let path = self.inner.resolve_symbolic_links(path)?;
        if prefetched || should_prefetch(&path) {
            return self.shared.native_fs.get_input_stream(&path);
        }
        let state = self.shared.state();
        let relative_path = path.relative_to(&self.shared.external_directory);
        let (info_size, metadata) = match self.inner.stat_if_found_with_metadata(&path, true)? {
            Some((status, Some(metadata))) => (status.get_size(), metadata),
            Some(_) => {
                return Err(io::Error::other(format!("{path} (Is a directory)")));
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{path} (No such file or directory)"),
                ));
            }
        };
        state.reporter.post(Arc::new(RemoteFileFetchProgress {
            resource_identifier: relative_path.get_path_string().to_string(),
            progress: format!(
                "({})",
                blaze_repo_fetch::downloader::download_progress_event::bytes_count_to_display_string(
                    info_size
                )
            ),
            finished: false,
        }));
        let digest = DigestUtil::build_digest_from_bytes(
            metadata.get_digest().unwrap_or_default(),
            info_size,
        );
        let context = self.make_remote_context(&state, &relative_path);
        let mut content: Option<Vec<u8>> = None;
        let result = wait_for_bulk_transfer([async {
            let bytes = state
                .cache
                .download_blob_named(&context, path.get_path_string(), None, &digest)
                .await?;
            content = Some(bytes);
            Ok::<(), io::Error>(())
        }]);
        state.reporter.post(Arc::new(RemoteFileFetchProgress {
            resource_identifier: relative_path.get_path_string().to_string(),
            progress: String::new(),
            finished: true,
        }));
        match result {
            Ok(()) => Ok(Box::new(io::Cursor::new(content.unwrap_or_default()))),
            Err(e) if BulkTransferException::all_caused_by_cache_not_found_exception_of(&e) => {
                self.shared
                    .repos_with_lost_files
                    .lock()
                    .insert(relative_path.get_segment(0).to_string());
                // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:911 RemoteExternalFileSystem.getInputStream
                Err(blaze_vfs::DetailedIOException::new(
                    format!(
                        "{}/{} with digest {} is no longer available in the remote cache",
                        self.shared.external_directory.get_base_name(),
                        relative_path,
                        DigestUtil::to_string(&digest)
                    ),
                    "REMOTE_FILE_EVICTED",
                    /* transient= */ true,
                )
                .into())
            }
            Err(e) => Err(e),
        }
    }
}

/// Delegates one `FileSystem` method to the in-memory file system.
macro_rules! inner {
    ($self:ident.$method:ident($($arg:expr),*)) => {
        $self.inner.$method($($arg),*)
    };
}

impl FileSystem for RemoteExternalFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        inner!(self.get_digest_function())
    }
    fn supports_modifications(&self, path: &PathFragment) -> bool {
        inner!(self.supports_modifications(path))
    }
    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        inner!(self.supports_symbolic_links_natively(path))
    }
    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        inner!(self.supports_hard_links_natively(path))
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        inner!(self.may_be_case_or_normalization_insensitive())
    }
    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        inner!(self.create_directory(path))
    }
    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        inner!(self.create_directory_and_parents(path))
    }
    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        inner!(self.get_file_size(path, follow_symlinks))
    }
    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        inner!(self.delete(path))
    }
    fn delete_tree(&self, path: &PathFragment) -> io::Result<()> {
        inner!(self.delete_tree(path))
    }
    fn delete_trees_below(&self, dir: &PathFragment) -> io::Result<()> {
        inner!(self.delete_trees_below(dir))
    }
    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        inner!(self.get_last_modified_time(path, follow_symlinks))
    }
    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        inner!(self.set_last_modified_time(path, new_time))
    }
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        inner!(self.get_fast_digest(path))
    }
    /// All regular files in this file system are remote files, whose digest is known in advance
    /// and returned by the base implementation of getFastDigest, which also correctly reports
    /// errors such as EISDIR for paths that don't resolve to regular files. The base
    /// implementation of getDigest would instead download the file contents to hash them.
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:943 RemoteExternalFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        self.inner
            .get_fast_digest(path)?
            .ok_or_else(|| io::Error::other(format!("{path} has no digest")))
    }
    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        inner!(self.resolve_one_link(path))
    }
    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        inner!(self.resolve_symbolic_links(path))
    }
    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        inner!(self.stat(path, follow_symlinks))
    }
    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<Arc<dyn blaze_vfs::StatMetadata>>> {
        inner!(self.stat_metadata(path, follow_symlinks))
    }
    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        inner!(self.stat_nullable(path, follow_symlinks))
    }
    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        inner!(self.stat_if_found(path, follow_symlinks))
    }
    fn is_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        inner!(self.is_file(path, follow_symlinks))
    }
    fn is_special_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        inner!(self.is_special_file(path, follow_symlinks))
    }
    fn is_symbolic_link(&self, path: &PathFragment) -> bool {
        inner!(self.is_symbolic_link(path))
    }
    fn is_directory(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        inner!(self.is_directory(path, follow_symlinks))
    }
    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        inner!(self.create_symbolic_link(link_path, target_fragment))
    }
    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        inner!(self.read_symbolic_link(path))
    }
    fn read_symbolic_link_unchecked(&self, path: &PathFragment) -> io::Result<PathFragment> {
        inner!(self.read_symbolic_link_unchecked(path))
    }
    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        inner!(self.exists(path, follow_symlinks))
    }
    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        inner!(self.get_directory_entries(path))
    }
    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        inner!(self.readdir(path, follow_symlinks))
    }
    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        inner!(self.is_readable(path))
    }
    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        inner!(self.set_readable(path, readable))
    }
    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        inner!(self.is_writable(path))
    }
    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        inner!(self.set_writable(path, writable))
    }
    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        inner!(self.is_executable(path))
    }
    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        inner!(self.set_executable(path, executable))
    }
    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        inner!(self.chmod(path, mode))
    }
    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        self.read_remote(path)
    }
    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        inner!(self.get_output_stream(path, append, internal))
    }
    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        inner!(self.rename_to(source_path, target_path))
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        inner!(self.create_fs_dependent_hard_link(link_path, original_path))
    }
}

impl blaze_repo_rules::repository::remote_helpers::RemoteRepoMaterializer
    for RemoteExternalOverlayFileSystem
{
    // Java: com/google/devtools/build/lib/remote/RemoteExternalOverlayFileSystem.java:395 RemoteExternalOverlayFileSystem.ensureMaterialized
    fn ensure_materialized(
        &self,
        repo: &RepositoryName,
        reporter: &dyn ExtendedEventHandler,
    ) -> io::Result<()> {
        RemoteExternalOverlayFileSystem::ensure_materialized(self, repo, reporter)
    }
}
