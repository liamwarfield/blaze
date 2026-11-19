//! Port of `lib/remote/AbstractActionInputPrefetcher.java`.
//!
//! PORT: Java's subclasses override `canDownloadFile`, `forceRefetch`, `doDownloadFile` and
//! `prefetchVirtualActionInput`; here they implement [`ActionInputPrefetcherHooks`] and the
//! abstract class is a struct that holds them. The Rx/`ListenableFuture` chains are async
//! functions driven on the calling thread (`docs/remote.md`, convention 1).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use blaze_events::ExtendedEventHandler;
use blaze_exec::actions::action_input_prefetcher::{Priority, Reason};
use blaze_exec::actions::cache::{IoOrInterruptedException, OutputMetadataStore};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, FileArtifactValue, FileContentsProxy, FileStateType,
    InputMetadataProvider, OutputChecker,
};
use blaze_vfs::{FileSystemExt, Path, PathFragment, Symlinks};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::util::async_task_cache::AsyncTaskCache;
use crate::util::temp_path_generator::TempPathGenerator;
use crate::util::utils::merge_bulk_transfer;

/// The permissions of output files and directories (`OutputPermissions`).
// Java: com/google/devtools/build/lib/actions/cache/OutputPermissions.java OutputPermissions
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputPermissions {
    /// 0555.
    READONLY,
    /// 0755.
    WRITABLE,
}

impl OutputPermissions {
    /// `getPermissionsMode()`.
    pub fn get_permissions_mode(self) -> u32 {
        match self {
            OutputPermissions::READONLY => 0o555,
            OutputPermissions::WRITABLE => 0o755,
        }
    }
}

/// The state of a directory tracked by `DirectoryTracker`.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:95 AbstractActionInputPrefetcher.DirectoryState
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryState {
    PermanentlyWritable,
    TemporarilyWritable,
    OutputPermissions,
}

/// Returns the metadata for an `ActionInput` (`MetadataSupplier`).
///
/// This will generally call through to an `InputMetadataProvider` and ask for the metadata of
/// either an input or an output artifact.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:108 AbstractActionInputPrefetcher.MetadataSupplier
pub type MetadataSupplier<'a> =
    dyn Fn(&ActionInputRef) -> io::Result<Option<FileArtifactValue>> + Send + Sync + 'a;

/// What the subclasses of `AbstractActionInputPrefetcher` implement.
#[async_trait::async_trait]
pub trait ActionInputPrefetcherHooks: Send + Sync {
    /// `canDownloadFile(path, metadata)`.
    fn can_download_file(&self, path: &Path, metadata: &FileArtifactValue) -> bool;

    /// If true, then all previously acquired knowledge of the file system state of this path
    /// (e.g. the existence of tree artifact directories or previously downloaded files) must be
    /// discarded (`forceRefetch(path)`).
    fn force_refetch(&self, path: &Path) -> bool;

    /// Downloads file to the given path via its metadata (`doDownloadFile`); `temp_path` is the
    /// temporary path which the input should be written to.
    async fn do_download_file(
        &self,
        action: Option<&(dyn ActionExecutionMetadata + '_)>,
        reporter: &(dyn ExtendedEventHandler + '_),
        input: &ActionInputRef,
        temp_path: &Path,
        metadata: &FileArtifactValue,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()>;

    /// `prefetchVirtualActionInput(input)` (a no-op by default).
    fn prefetch_virtual_action_input(&self, _input: &ActionInputRef) -> io::Result<()> {
        Ok(())
    }
}

/// A symlink in the output tree that either points to another artifact's absolute path or
/// represents an unresolved symlink with its target path preserved verbatim.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:228 AbstractActionInputPrefetcher.Symlink
#[derive(Clone, Debug)]
struct Symlink {
    link_path: Path,
    target_path: PathFragment,
}

impl Symlink {
    fn new(link_path: Path, target_path: PathFragment) -> Symlink {
        blaze_util::check_argument!(
            *link_path.as_fragment() != target_path,
            "linkPath and targetPath must differ"
        );
        Symlink {
            link_path,
            target_path,
        }
    }

    fn resolve_one(&self) -> io::Result<Path> {
        resolve_one_symlink(&self.link_path, Some(&self.target_path))
    }
}

type BoxedTransfer<'a> = Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

/// Abstract implementation of `ActionInputPrefetcher` which implements the orchestration of
/// prefetching multiple inputs so subclasses can focus on prefetching / downloading single input.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:82 AbstractActionInputPrefetcher
pub struct AbstractActionInputPrefetcher {
    reporter: Arc<dyn ExtendedEventHandler>,
    download_cache: AsyncTaskCache<Path, ()>,
    temp_path_generator: Arc<TempPathGenerator>,
    output_permissions: OutputPermissions,
    pub(crate) exec_root: Path,
    pub(crate) remote_output_checker: Arc<RemoteOutputChecker>,
    // PORT: `@Nullable ActionOutputDirectoryHelper outputDirectoryHelper` is not taken: Blaze's
    // executor owns its own helper, so directories are created with
    // `createDirectoryAndParents()`, Java's branch for a null helper.
    /// `DirectoryTracker.directoryStateMap`.
    directory_state_map: Mutex<HashMap<Path, DirectoryState>>,
    /// `treeArtifactLocks` (`Striped.lazyWeakReadWriteLock`): one lock per resolved tree root.
    tree_artifact_locks: Mutex<HashMap<PathFragment, Arc<RwLock<()>>>>,
    hooks: Box<dyn ActionInputPrefetcherHooks>,
}

impl AbstractActionInputPrefetcher {
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:242 AbstractActionInputPrefetcher.<init>
    pub fn new(
        reporter: Arc<dyn ExtendedEventHandler>,
        exec_root: Path,
        temp_path_generator: Arc<TempPathGenerator>,
        remote_output_checker: Arc<RemoteOutputChecker>,
        output_permissions: OutputPermissions,
        hooks: Box<dyn ActionInputPrefetcherHooks>,
    ) -> AbstractActionInputPrefetcher {
        AbstractActionInputPrefetcher {
            reporter,
            download_cache: AsyncTaskCache::create(),
            temp_path_generator,
            output_permissions,
            exec_root,
            remote_output_checker,
            directory_state_map: Mutex::new(HashMap::new()),
            tree_artifact_locks: Mutex::new(HashMap::new()),
            hooks,
        }
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:144 DirectoryTracker.setWritable
    fn set_writable(&self, dir: &Path, new_state: DirectoryState) -> io::Result<()> {
        // Compare as fragments since execRoot may be located on a file system overlaying the
        // host file system where downloads are written to.
        if !dir.as_fragment().starts_with(self.exec_root.as_fragment()) {
            return Ok(());
        }
        let mut map = self.directory_state_map.lock();
        let old_state = map.get(dir).copied();
        if !self.hooks.force_refetch(dir)
            && matches!(
                old_state,
                Some(DirectoryState::TemporarilyWritable | DirectoryState::PermanentlyWritable)
            )
        {
            // Already writable, but must potentially upgrade from temporary to permanent.
            if new_state == DirectoryState::PermanentlyWritable {
                map.insert(dir.clone(), new_state);
            }
            return Ok(());
        }
        dir.create_directory_and_parents()?;
        dir.set_writable(true)?;
        map.insert(dir.clone(), new_state);
        Ok(())
    }

    /// Marks a directory as temporarily writable.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:128 DirectoryTracker.setTemporarilyWritable
    fn set_temporarily_writable(&self, dir: &Path) -> io::Result<()> {
        self.set_writable(dir, DirectoryState::TemporarilyWritable)
    }

    /// Marks a directory as permanently writable.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:138 DirectoryTracker.setPermanentlyWritable
    fn set_permanently_writable(&self, dir: &Path) -> io::Result<()> {
        self.set_writable(dir, DirectoryState::PermanentlyWritable)
    }

    /// Sets the output permissions on a directory.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:184 DirectoryTracker.setOutputPermissions
    fn set_output_permissions(&self, dir: &Path) -> io::Result<()> {
        let mut map = self.directory_state_map.lock();
        let old_state = map.get(dir).copied();
        if !self.hooks.force_refetch(dir)
            && matches!(
                old_state,
                Some(DirectoryState::OutputPermissions | DirectoryState::PermanentlyWritable)
            )
        {
            // Either the output permissions have already been set, or we're not changing the
            // permissions ever again.
            return Ok(());
        }
        dir.chmod(self.output_permissions.get_permissions_mode())?;
        map.insert(dir.clone(), DirectoryState::OutputPermissions);
        Ok(())
    }

    fn tree_artifact_lock(&self, tree_root: &Path) -> Arc<RwLock<()>> {
        self.tree_artifact_locks
            .lock()
            .entry(tree_root.as_fragment().clone())
            .or_default()
            .clone()
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:257 AbstractActionInputPrefetcher.shouldDownloadFile
    fn should_download_file(path: &Path, metadata: &FileArtifactValue) -> io::Result<bool> {
        let Some(stat) = path.stat_if_found(Symlinks::Follow)? else {
            return Ok(true);
        };

        // If an action output is stale, Skyframe will delete it prior to action execution.
        // However, this doesn't apply to spawn outputs that aren't action outputs, or to files in
        // external repos that are remote repo contents cache hits. To avoid incorrectly reusing
        // one such stale file, check for its up-to-dateness here.
        if stat.get_size() != metadata.get_size() {
            return Ok(true);
        }
        if let Some(contents_proxy) = metadata.get_contents_proxy()
            && contents_proxy == FileContentsProxy::create(&stat)
        {
            return Ok(false);
        }

        let digest = match path.get_fast_digest()? {
            Some(digest) => digest,
            None => path.get_digest()?,
        };
        if Some(digest.as_slice()) != metadata.get_digest() {
            return Ok(true);
        }
        // The file contents have been verified to be up to date.
        // PORT: `metadata.setContentsProxy(FileContentsProxy.create(stat))` is not ported
        // (Blaze's `FileArtifactValue` is immutable); a later check computes the digest again.
        Ok(false)
    }

    /// Fetches remotely stored action outputs and stores them under their path in the output
    /// base (`prefetchFiles`), and waits for it.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:325 AbstractActionInputPrefetcher.prefetchFiles
    pub fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        let supplier = |input: &ActionInputRef| metadata_provider.get_input_metadata(input);
        crate::runtime::get_from_future(
            self.prefetch_files_interruptibly(action, inputs, &supplier, priority, reason),
        )
    }

    /// Fetches remotely stored action outputs and stores them under their path in the output
    /// base.
    ///
    /// The `inputs` may not contain any unexpanded directories.
    ///
    /// This method is safe to be called concurrently from spawn runners before running any local
    /// spawn.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:349 AbstractActionInputPrefetcher.prefetchFilesInterruptibly
    pub async fn prefetch_files_interruptibly(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_supplier: &MetadataSupplier<'_>,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        let mut files: Vec<&ActionInputRef> = Vec::new();

        for input in inputs {
            if !RemoteOutputChecker::may_be_remote(&**input) {
                continue;
            }

            // Skip empty tree artifacts (non-empty tree artifacts should have already been
            // expanded).
            if input.is_directory() {
                continue;
            }

            files.push(input);
        }

        if files.is_empty() {
            return Ok(());
        }

        // Collect directories whose output permissions must be restored at the end of this call,
        // grouped by tree root for synchronization. Keeping permission restoration scoped to this
        // prefetchFiles call avoids toggling shared ancestors between child finalizations. A
        // concurrent call to prefetchFiles for the same child may await the same download, but
        // must still participate in permission restoration before returning.
        // PORT: Java's `HashMultimap`; the order of the restoration is not observable.
        let directories_by_tree_root: Mutex<IndexMap<Path, HashSet<Path>>> =
            Mutex::new(IndexMap::new());

        let mut transfers: Vec<BoxedTransfer<'_>> = Vec::with_capacity(files.len());
        for file in files {
            transfers.push(self.prefetch_file(
                action,
                &directories_by_tree_root,
                metadata_supplier,
                file,
                priority,
                reason,
            ));
        }

        merge_bulk_transfer(transfers).await?;

        // Match the directory output permissions set by SkyframeActionExecutor#checkOutputs.
        let directories_by_tree_root = directories_by_tree_root.into_inner();
        for (tree_root, dirs) in &directories_by_tree_root {
            let lock = self.tree_artifact_lock(tree_root);
            let _guard = lock.write();
            for dir in dirs {
                self.set_output_permissions(dir)?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:419 AbstractActionInputPrefetcher.prefetchFile
    fn prefetch_file<'a>(
        &'a self,
        action: Option<&'a dyn ActionExecutionMetadata>,
        directories_by_tree_root: &'a Mutex<IndexMap<Path, HashSet<Path>>>,
        metadata_supplier: &'a MetadataSupplier<'_>,
        input: &'a ActionInputRef,
        priority: Priority,
        reason: Reason,
    ) -> BoxedTransfer<'a> {
        let prepared = (|| -> io::Result<Option<BoxedTransfer<'a>>> {
            if input.as_virtual_action_input().is_some() {
                self.hooks.prefetch_virtual_action_input(input)?;
                return Ok(None);
            }

            // PORT: `artifact.getPath()` is the exec root path of the artifact for every artifact
            // Blaze prefetches (outputs, and source artifacts of external repositories under
            // `external/`).
            let mut input_path = self.exec_root.get_relative(input.get_exec_path());

            // Metadata may legitimately be missing, e.g. if this is an optional test output.
            let Some(metadata) = metadata_supplier(input)? else {
                return Ok(None);
            };

            if metadata.get_type() == FileStateType::Directory {
                // Tree artifacts have already been expanded into their children, so this is a
                // source directory.
                // PORT: the remote repo contents cache (`SubtreeMaterializer`) is not ported.
                return Ok(None);
            }

            let symlinks = self.get_symlinks(input, &input_path, &metadata, metadata_supplier)?;

            if !self.hooks.can_download_file(&input_path, &metadata) {
                // If the artifact is a declared ("unresolved") symlink, it can't be "downloaded",
                // but the symlink logic above creates it.
                let symlinks = symlinks.clone();
                return Ok(Some(Box::pin(async move {
                    self.plant_symlinks(symlinks).await
                })));
            }

            if let Some(first) = symlinks.first() {
                // Symlink may track the parent of a TreeFileArtifact, so the parent relative path
                // has to be translated relative to it.
                let parent_relative_path = input_path.relative_to(&first.link_path);
                input_path = symlinks
                    .last()
                    .expect("not empty")
                    .resolve_one()?
                    .get_relative(&parent_relative_path);
            }

            let tree_root_path = self.maybe_get_tree_root(input, metadata_supplier)?;

            Ok(Some(Box::pin(async move {
                self.download_file_no_check(
                    action,
                    input,
                    input_path,
                    tree_root_path,
                    directories_by_tree_root,
                    metadata,
                    priority,
                    reason,
                )
                .await?;
                self.plant_symlinks(symlinks).await
            })))
        })();
        match prepared {
            Ok(Some(transfer)) => transfer,
            Ok(None) => Box::pin(async { Ok(()) }),
            Err(e) => Box::pin(async move { Err(e) }),
        }
    }

    /// On Windows, the type of symlink depends on the target file and the target may have to
    /// exist, so we plant symlinks in reverse order and only after any download has completed.
    async fn plant_symlinks(&self, symlinks: Vec<Symlink>) -> io::Result<()> {
        for symlink in symlinks.into_iter().rev() {
            self.plant_symlink(symlink).await?;
        }
        Ok(())
    }

    /// For an input belonging to a tree artifact, returns the resolved path of the tree artifact
    /// root. Otherwise, returns `None`.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:506 AbstractActionInputPrefetcher.maybeGetTreeRoot
    fn maybe_get_tree_root(
        &self,
        input: &ActionInputRef,
        metadata_supplier: &MetadataSupplier<'_>,
    ) -> io::Result<Option<Path>> {
        let Some(tree_artifact) = tree_file_parent(input) else {
            return Ok(None);
        };
        let Some(tree_metadata) = metadata_supplier(&tree_artifact)? else {
            // There are two cases where tree metadata is legitimately not available:
            // (1) If the file is the output of an action expanded from an action template. In
            //     this case, the symlink optimization is intentionally not supported.
            // (2) If the file is part of an input fileset. In this case, a symlink has already
            //     been created, but we're currently unable to prefetch the file(s) it points to.
            return Ok(None);
        };
        if let Some(resolved_path) = tree_metadata.get_resolved_path() {
            return Ok(Some(
                self.exec_root
                    .get_file_system()
                    .get_path_fragment(resolved_path),
            ));
        }
        Ok(Some(
            self.exec_root.get_relative(tree_artifact.get_exec_path()),
        ))
    }

    /// Returns the symlinks to be planted on disk for inputs that are prefetched into a different
    /// location, ordered from the input path to the final target.
    ///
    /// Some artifacts (notably, those created by `ctx.actions.symlink`) are materialized in the
    /// output tree as a symlink to another artifact, as indicated by the
    /// `FileArtifactValue.getResolvedPath()` field in their (or their parent tree artifact's)
    /// metadata.
    ///
    /// Paths in external repos backed by the remote repo contents cache may be (part of) a chain
    /// of symlinks created by the repo rule, which has to be reproduced verbatim on disk.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:541 AbstractActionInputPrefetcher.getSymlinks
    fn get_symlinks(
        &self,
        input: &ActionInputRef,
        input_path: &Path,
        metadata: &FileArtifactValue,
        metadata_supplier: &MetadataSupplier<'_>,
    ) -> io::Result<Vec<Symlink>> {
        if let Some(tree_artifact) = tree_file_parent(input) {
            let Some(tree_metadata) = metadata_supplier(&tree_artifact)? else {
                // See maybe_get_tree_root.
                return Ok(Vec::new());
            };
            let tree_path = self.exec_root.get_relative(tree_artifact.get_exec_path());
            return self.get_symlinks(
                &tree_artifact,
                &tree_path,
                &tree_metadata,
                metadata_supplier,
            );
        }
        if (metadata.is_remote() || metadata.get_type() == FileStateType::Symlink)
            && !input_path.starts_with(&self.exec_root)
        {
            // A path in an external repo, e.g. a source artifact consumed by an action or a file
            // prefetched during the materialization of an external repo. It may be (part of) a
            // chain of symlinks created by the repo rule, which has to be reproduced verbatim on
            // disk.
            let mut symlink_chain = Vec::new();
            let mut current_path = input_path.clone();
            let mut max_attempt = 32;
            while let Some(stat) = current_path.stat_if_found(Symlinks::Nofollow)?
                && stat.is_symbolic_link()
            {
                if max_attempt == 0 {
                    return Err(symlink_loop(input_path.as_fragment()));
                }
                max_attempt -= 1;
                let symlink =
                    Symlink::new(current_path.clone(), current_path.read_symbolic_link()?);
                current_path = symlink.resolve_one()?;
                symlink_chain.push(symlink);
            }
            return Ok(symlink_chain);
        }
        let Some(resolved_path) = metadata.get_resolved_path() else {
            return Ok(Vec::new());
        };
        if resolved_path == input_path.as_fragment() {
            return Ok(Vec::new());
        }
        Ok(vec![Symlink::new(
            input_path.clone(),
            resolved_path.clone(),
        )])
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:621 AbstractActionInputPrefetcher.downloadFileNoCheckRx
    async fn download_file_no_check(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        input: &ActionInputRef,
        path: Path,
        tree_root: Option<Path>,
        directories_by_tree_root: &Mutex<IndexMap<Path, HashSet<Path>>>,
        metadata: FileArtifactValue,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        // If the path to be prefetched is a non-dangling symlink, prefetch its target path
        // instead. Note that this only applies to symlinks created by spawns (or, currently, with
        // the internal version of BwoB); symlinks created in-process through an ActionFileSystem
        // should have already been canonicalized by maybeGetSymlink.
        let (path, tree_root) = match tree_root {
            Some(tree_root) => {
                let tree_root_relative_path = path.relative_to(&tree_root);
                let tree_root = maybe_resolve_symlink(tree_root)?;
                (
                    tree_root.get_relative(&tree_root_relative_path),
                    Some(tree_root),
                )
            }
            None => (maybe_resolve_symlink(path)?, None),
        };

        // Downloads are written to the actual host file system, not any overlays.
        let final_path = path.for_host_file_system();

        let final_tree_root = match &tree_root {
            Some(tree_root) if is_child_of_declared_directory(input) => {
                Some(tree_root.for_host_file_system())
            }
            _ => None,
        };
        if let Some(final_tree_root) = &final_tree_root {
            // Record directories child-to-root. If a directory is already present for this tree
            // root, an earlier traversal also recorded every ancestor through the root.
            let mut map = directories_by_tree_root.lock();
            let dirs = map.entry(final_tree_root.clone()).or_default();
            let mut dir = final_path.get_parent_directory();
            while let Some(d) = dir {
                if !d.starts_with(final_tree_root) {
                    break;
                }
                if !dirs.insert(d.clone()) {
                    break;
                }
                dir = d.get_parent_directory();
            }
        }

        let force = self.hooks.force_refetch(&final_path);
        let task = async {
            if !AbstractActionInputPrefetcher::should_download_file(&final_path, &metadata)? {
                return Ok(());
            }
            // usingTempPath
            let temp_path = self.temp_path_generator.generate_temp_path();
            let result = async {
                self.hooks
                    .do_download_file(
                        action,
                        &*self.reporter,
                        input,
                        &temp_path.for_host_file_system(),
                        &metadata,
                        priority,
                        reason,
                    )
                    .await?;
                self.finalize_download(
                    &temp_path.for_host_file_system(),
                    &final_path,
                    final_tree_root.as_ref(),
                )
            }
            .await;
            if result.is_err() {
                // Java deletes the temporary path unless the download was finalized
                // (`alreadyDeleted`); a failure after the move leaves nothing there.
                delete_partial_download(&temp_path);
            }
            result
        };
        self.download_cache
            .execute(final_path.clone(), task, force)
            .await
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:702 AbstractActionInputPrefetcher.finalizeDownload
    fn finalize_download(
        &self,
        tmp_path: &Path,
        final_path: &Path,
        tree_root: Option<&Path>,
    ) -> io::Result<()> {
        // Set file output permissions, matching SkyframeActionExecutor#checkOutputs for artifacts
        // produced by local actions. The temporary path is outside the output tree, so this can
        // happen before entering the tree artifact root's critical section.
        tmp_path.chmod(self.output_permissions.get_permissions_mode())?;

        let parent_dir = final_path
            .get_parent_directory()
            .expect("NullPointerException: parent of a download");
        let tree_artifact_lock = tree_root.map(|tree_root| self.tree_artifact_lock(tree_root));
        let _guard = tree_artifact_lock.as_ref().map(|lock| lock.read());

        // Compare as fragments since execRoot may be located on a file system overlaying the host
        // file system where the download is written to.
        if final_path
            .as_fragment()
            .starts_with(self.exec_root.as_fragment())
        {
            // Ensure the parent directory exists and is writable. We cannot rely on this
            // precondition to have been established by the execution of the owning action in a
            // previous invocation, since the output tree may have been externally modified in
            // between invocations.
            match tree_root {
                Some(tree_root) => {
                    // Reopen every ancestor because ActionOutputDirectoryHelper caches existing
                    // directories and may otherwise leave a previously restored ancestor
                    // read-only.
                    let mut dir = tree_root.clone();
                    self.set_temporarily_writable(&dir)?;
                    for segment in parent_dir.relative_to(tree_root).segments() {
                        dir = dir.get_child(segment);
                        self.set_temporarily_writable(&dir)?;
                    }
                }
                None => {
                    // One of the following must apply:
                    //   (1) The file does not belong to a tree artifact.
                    //   (2) The file belongs to a tree artifact created by an action template
                    //       expansion.
                    // In case (1), the parent directory is a package or a subdirectory of a
                    // package, and should remain writable. In case (2), even though we arguably
                    // ought to set the output permissions on the parent directory to match local
                    // execution, we choose not to do it and avoid the additional implementation
                    // complexity required to detect a race condition between concurrent calls
                    // touching the same directory.
                    self.set_permanently_writable(&parent_dir)?;
                }
            }
        } else {
            parent_dir.create_directory_and_parents()?;
        }

        blaze_repo_fetch::fs_utils::move_file(tmp_path, final_path)?;

        // PORT: `metadata.setContentsProxy(FileContentsProxy.create(finalPath.stat()))` is not
        // ported (immutable `FileArtifactValue`).
        Ok(())
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:793 AbstractActionInputPrefetcher.plantSymlink
    async fn plant_symlink(&self, symlink: Symlink) -> io::Result<()> {
        let link_path = symlink.link_path.for_host_file_system();
        let force = self.hooks.force_refetch(&link_path);
        let task = async {
            if !symlink
                .link_path
                .as_fragment()
                .starts_with(self.exec_root.as_fragment())
            {
                // If the symlink is a source file in an external repo, its parent directory may
                // not exist yet.
                link_path
                    .get_parent_directory()
                    .expect("NullPointerException")
                    .create_directory_and_parents()?;
            }
            // Delete the link path if it already exists. This is the case for tree artifacts,
            // whose root directory is created before the action runs.
            link_path.delete()?;
            link_path.create_symbolic_link_to_fragment(&symlink.target_path)?;
            Ok(())
        };
        self.download_cache
            .execute(link_path.clone(), task, force)
            .await
    }

    /// Forgets about completed downloads of the given paths so that the next prefetch of each of
    /// them verifies it against the local file system instead of assuming that it is still in
    /// place.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:817 AbstractActionInputPrefetcher.invalidateDownloads
    pub fn invalidate_downloads(&self, exec_paths: &[PathFragment]) {
        for path in exec_paths {
            // Downloads are written to the actual host file system, not any overlays.
            self.download_cache
                .invalidate(&self.exec_root.get_relative(path).for_host_file_system());
        }
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:824 AbstractActionInputPrefetcher.downloadedFiles
    pub fn downloaded_files(&self) -> Vec<Path> {
        self.download_cache.get_finished_tasks()
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:828 AbstractActionInputPrefetcher.downloadsInProgress
    pub fn downloads_in_progress(&self) -> Vec<Path> {
        self.download_cache.get_in_progress_tasks()
    }

    /// `getDownloadCache()` (for tests).
    pub fn get_download_cache(&self) -> &AsyncTaskCache<Path, ()> {
        &self.download_cache
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:837 AbstractActionInputPrefetcher.shutdown
    pub fn shutdown(&self) {
        self.download_cache.shutdown();
        crate::runtime::block_on(self.download_cache.await_termination());
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:849 AbstractActionInputPrefetcher.finalizeAction
    pub fn finalize_action(
        &self,
        action: &dyn ActionExecutionMetadata,
        outputs: &[ActionInputRef],
        output_metadata_store: &dyn OutputMetadataStore,
    ) -> io::Result<()> {
        let mut outputs_to_download: Vec<ActionInputRef> = Vec::new();
        for output in outputs {
            if output_metadata_store.artifact_omitted(output) {
                continue;
            }

            let metadata = match output_metadata_store.get_output_metadata(output) {
                Ok(Some(metadata)) => metadata,
                Ok(None) => panic!("NullPointerException: metadata of {output}"),
                Err(e) => return Err(io_or_interrupted(e)),
            };
            let output_path = self.exec_root.get_relative(output.get_exec_path());
            if !self.hooks.can_download_file(&output_path, &metadata) {
                continue;
            }

            if output.as_artifact().is_some_and(|a| a.is_tree_artifact()) {
                let tree = output_metadata_store
                    .get_tree_artifact_value(output)
                    .map_err(io_or_interrupted)?;
                for (child, child_metadata) in &tree.child_values {
                    if self
                        .remote_output_checker
                        .should_download_output(&**child, child_metadata)
                    {
                        outputs_to_download.push(child.clone());
                    }
                }
            } else if self
                .remote_output_checker
                .should_download_output(&**output, &metadata)
            {
                outputs_to_download.push(output.clone());
            }
        }

        if !outputs_to_download.is_empty() {
            // PORT: skipped, profiling ("Download outputs").
            let supplier = |input: &ActionInputRef| {
                output_metadata_store
                    .get_output_metadata(input)
                    .map_err(io_or_interrupted)
            };
            let result = crate::runtime::get_from_future(self.prefetch_files_interruptibly(
                Some(action),
                &outputs_to_download,
                &supplier,
                Priority::HIGH,
                Reason::OUTPUTS,
            ));
            if let Err(e) = result {
                if let Some(bulk) = errors::downcast_ref::<BulkTransferException>(&e) {
                    // Outputs whose blobs the remote cache has lost are recreated by rewinding.
                    let by_exec_path: HashMap<&PathFragment, &ActionInputRef> = outputs_to_download
                        .iter()
                        .map(|o| (o.get_exec_path(), o))
                        .collect();
                    let lost_outputs =
                        bulk.get_lost_artifacts(&|p| by_exec_path.get(p).map(|o| (*o).clone()));
                    if !lost_outputs.is_empty() {
                        blaze_util::unported!(
                            "LostInputsExecException (AbstractActionInputPrefetcher.finalizeAction: action rewinding for lost remote outputs)"
                        );
                    }
                }
                return Err(e);
            }
        }
        Ok(())
    }

    /// `flushOutputTree()`.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:900 AbstractActionInputPrefetcher.flushOutputTree
    pub fn flush_output_tree(&self) {
        crate::runtime::block_on(self.download_cache.await_in_progress_tasks());
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:904 AbstractActionInputPrefetcher.getRemoteOutputChecker
    pub fn get_remote_output_checker(&self) -> &Arc<RemoteOutputChecker> {
        &self.remote_output_checker
    }

    /// The reporter given to the constructor.
    pub fn get_reporter(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.reporter
    }
}

/// `input instanceof TreeFileArtifact treeFile ? treeFile.getParent() : null`.
fn tree_file_parent(input: &ActionInputRef) -> Option<ActionInputRef> {
    if is_child_of_declared_directory(input) {
        return input.as_artifact().and_then(|a| a.get_parent());
    }
    None
}

/// `actionInput instanceof Artifact artifact && artifact.isChildOfDeclaredDirectory()`.
fn is_child_of_declared_directory(input: &ActionInputRef) -> bool {
    input
        .as_artifact()
        .is_some_and(|a| a.is_child_of_declared_directory())
}

/// `IOException | InterruptedException` of an output metadata read.
fn io_or_interrupted(e: IoOrInterruptedException) -> io::Error {
    match e {
        IoOrInterruptedException::Io(e) => e,
        IoOrInterruptedException::Interrupted(_) => errors::interrupted(),
    }
}

/// `new FileSymlinkLoopException(path)`.
fn symlink_loop(path: &PathFragment) -> io::Error {
    io::Error::other(format!("{} (Too many levels of symbolic links)", path))
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:588 AbstractActionInputPrefetcher.resolveOneSymlink
fn resolve_one_symlink(
    path: &Path,
    target_path_fragment: Option<&PathFragment>,
) -> io::Result<Path> {
    let target_path_fragment = match target_path_fragment {
        Some(t) => t.clone(),
        None => path.read_symbolic_link()?,
    };
    if target_path_fragment.is_absolute() {
        Ok(path
            .get_file_system()
            .get_path_fragment(&target_path_fragment))
    } else {
        Ok(path
            .get_parent_directory()
            .expect("NullPointerException")
            .get_relative(&target_path_fragment))
    }
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:600 AbstractActionInputPrefetcher.maybeResolveSymlink
fn maybe_resolve_symlink(mut path: Path) -> io::Result<Path> {
    // Potentially resolves a symlink to its target path. This differs from
    // Path#resolveSymbolicLinks() that:
    //   1. Path#resolveSymbolicLinks() checks each segment of the path, but we assume there is no
    //      intermediate symlink because they should've been already normalized for outputs.
    //   2. In case of dangling symlink, we return the target path instead of throwing
    //      FileNotFoundException because we want to download output to that target path.
    let mut max_attempt = 32;
    while path.is_symbolic_link() && max_attempt > 0 {
        max_attempt -= 1;
        let resolved_path = resolve_one_symlink(&path, None)?;
        if resolved_path.as_fragment() == path.as_fragment() {
            return Err(symlink_loop(path.as_fragment()));
        }
        path = resolved_path;
    }
    if max_attempt <= 0 {
        return Err(symlink_loop(path.as_fragment()));
    }
    Ok(path)
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:784 AbstractActionInputPrefetcher.deletePartialDownload
fn delete_partial_download(path: &Path) {
    // PORT: Java logs a warning ("Failed to delete output file after incomplete download");
    // the logger is not ported.
    let _ = path.delete();
}
