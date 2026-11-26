//! Port of `lib/remote/AbstractActionInputPrefetcher.java`, with the interface it implements,
//! `lib/actions/ActionInputPrefetcher.java`.
//!
//! PORT: the Java abstract class is split in three parts:
//! - [`AbstractActionInputPrefetcherBase`]: the fields of the abstract class;
//! - the trait [`AbstractActionInputPrefetcher`]: the abstract methods (`canDownloadFile`,
//!   `forceRefetch`, `doDownloadFile`) and the overridable `prefetchVirtualActionInput`, which a
//!   concrete prefetcher (`RemoteActionInputFetcher`) implements;
//! - the inherent methods of `dyn AbstractActionInputPrefetcher`: the concrete methods of the
//!   abstract class (`prefetchFilesInterruptibly`, `downloadedFiles`, `finalizeAction`, ...).
//!
//! RxJava `Completable`s and `ListenableFuture`s are `async fn`s (docs/remote.md); the blocking
//! entry points (`finalizeAction`, `shutdown`, `flushOutputTree`, [`prefetch_files_blocking`])
//! block with `crate::runtime`.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use blaze_actions::{Artifact as BlazeArtifact, ArtifactClass};
use blaze_events::Reporter;
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::actions::cache::{
    IoOrInterruptedException, OutputMetadataStore, OutputPermissions,
};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInput, ActionInputRef, FileArtifactValue, FileContentsProxy,
    FileStateType, InputMetadataProvider, OutputChecker, VirtualActionInput,
};
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{FileSymlinkLoopException, Path, PathFragment, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, RwLock};

use crate::common::bulk_transfer_exception::BulkTransferException;
use crate::common::errors;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::runtime::{block_on, get_from_future};
use crate::util::async_task_cache::AsyncTaskCacheNoResult;
use crate::util::temp_path_generator::TempPathGenerator;
use crate::util::utils::merge_bulk_transfer;
use blaze_exec::actions::LostInputsExecException;
use blaze_exec::exec::action_execution_context::SpawnCacheError;

// ---------------------------------------------------------------------------------------------
// ActionInputPrefetcher (lib/actions)
// ---------------------------------------------------------------------------------------------

/// Priority for the staging task.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:30 ActionInputPrefetcher.Priority
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    /// Critical priority tasks are tasks that are critical to the execution time e.g. staging
    /// files for in-process actions.
    CRITICAL,
    /// High priority tasks are tasks that may have impact on the execution time e.g. staging
    /// outputs that are inputs to local actions which will be executed later.
    HIGH,
    /// Medium priority tasks are tasks that may or may not have the impact on the execution time
    /// e.g. staging inputs for local branch of dynamically scheduled actions.
    MEDIUM,
    /// Low priority tasks are tasks that don't have impact on the execution time e.g. staging
    /// outputs of toplevel targets/aspects.
    LOW,
}

/// The reason for prefetching.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:54 ActionInputPrefetcher.Reason
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The requested files are needed as inputs to the given action.
    INPUTS,

    /// The requested files are requested as outputs of the given action.
    OUTPUTS,
}

/// Prefetches files to local disk.
///
/// PORT: the interface belongs to `lib/actions` (`blaze_exec`); it lives here until
/// `blaze_exec` owns it (`ActionExecutionContext::prefetch_files` is the call site that will
/// use it). Java's `ListenableFuture<Void>` is an `async fn`; [`prefetch_files_blocking`] waits
/// for it. (`requiresTreeMetadataWhenTreeFileIsInput` does not exist in the interface at the
/// pin.)
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:23 ActionInputPrefetcher
#[async_trait]
pub trait ActionInputPrefetcher: Send + Sync {
    /// Initiates best-effort prefetching of all given inputs.
    ///
    /// For any path not under this prefetcher's control, the call should be a no-op.
    ///
    /// Returns success if prefetch is finished or an `IOException`.
    // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:69 ActionInputPrefetcher.prefetchFiles
    async fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()>;
}

/// `ActionInputPrefetcher.NONE`: does nothing.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:24 ActionInputPrefetcher.NONE
#[derive(Debug, Clone, Copy, Default)]
pub struct NoneActionInputPrefetcher;

#[async_trait]
impl ActionInputPrefetcher for NoneActionInputPrefetcher {
    // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:24 ActionInputPrefetcher.NONE
    async fn prefetch_files(
        &self,
        _action: Option<&dyn ActionExecutionMetadata>,
        _inputs: &[ActionInputRef],
        _metadata_provider: &dyn InputMetadataProvider,
        _priority: Priority,
        _reason: Reason,
    ) -> io::Result<()> {
        // Do nothing.
        Ok(())
    }
}

/// `ActionInputPrefetcher.NONE`.
pub static NONE: NoneActionInputPrefetcher = NoneActionInputPrefetcher;

/// `getFromFuture(prefetcher.prefetchFiles(action, inputs, metadataProvider, priority, reason))`:
/// the blocking entry point of the execution phase.
///
/// PORT: what `ActionExecutionContext::prefetch_files` (blaze_exec) is to call, with
/// `Priority::MEDIUM` and `Reason::INPUTS` for the inputs of a local spawn. Must not be called
/// from inside the remote runtime.
// Java: com/google/devtools/build/lib/remote/util/Utils.java:98 Utils.getFromFuture
pub fn prefetch_files_blocking(
    prefetcher: &dyn ActionInputPrefetcher,
    action: Option<&dyn ActionExecutionMetadata>,
    inputs: &[ActionInputRef],
    metadata_provider: &dyn InputMetadataProvider,
    priority: Priority,
    reason: Reason,
) -> io::Result<()> {
    get_from_future(prefetcher.prefetch_files(action, inputs, metadata_provider, priority, reason))
}

// ---------------------------------------------------------------------------------------------
// AbstractActionInputPrefetcher
// ---------------------------------------------------------------------------------------------

/// Returns the metadata for an `ActionInput`.
///
/// This will generally call through to a `InputMetadataProvider` and ask for the metadata of
/// either an input or an output artifact.
///
/// `InterruptedException` is an `io::Error` of kind `Interrupted` (`errors::interrupted`).
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:109 AbstractActionInputPrefetcher.MetadataSupplier
pub trait MetadataSupplier: Send + Sync {
    /// `getMetadata(actionInput)`.
    fn get_metadata(&self, action_input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>>;
}

impl<F> MetadataSupplier for F
where
    F: Fn(&ActionInputRef) -> io::Result<Option<FileArtifactValue>> + Send + Sync,
{
    fn get_metadata(&self, action_input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        self(action_input)
    }
}

/// The state of a directory tracked by [`DirectoryTracker`], as explained below.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:96 AbstractActionInputPrefetcher.DirectoryState
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryState {
    PERMANENTLY_WRITABLE,
    TEMPORARILY_WRITABLE,
    OUTPUT_PERMISSIONS,
}

/// Tracks directory permissions to minimize filesystem operations.
///
/// Throughout the prefetcher, `Path#setWritable` and `Path#chmod` calls on output directories
/// must go through the methods in this class.
///
/// PORT: Java's `ConcurrentHashMap.compute` runs the remapping function under the lock of the
/// key's bin; here one mutex guards the whole map for the duration of the remapping function.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:119 AbstractActionInputPrefetcher.DirectoryTracker
#[derive(Default)]
struct DirectoryTracker {
    directory_state_map: Mutex<HashMap<Path, DirectoryState>>,
}

impl DirectoryTracker {
    /// Marks a directory as temporarily writable.
    ///
    /// A temporarily writable directory may have its output permissions set by a later call to
    /// `setOutputPermissions`, unless `setPermanentlyWritable` is called in the interim.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:130 DirectoryTracker.setTemporarilyWritable
    fn set_temporarily_writable(
        &self,
        this: &dyn AbstractActionInputPrefetcher,
        dir: &Path,
    ) -> io::Result<()> {
        self.set_writable(this, dir, DirectoryState::TEMPORARILY_WRITABLE)
    }

    /// Marks a directory as permanently writable.
    ///
    /// A permanently writable directory will never have its output permissions set by a later
    /// call to `setOutputPermissions`.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:140 DirectoryTracker.setPermanentlyWritable
    fn set_permanently_writable(
        &self,
        this: &dyn AbstractActionInputPrefetcher,
        dir: &Path,
    ) -> io::Result<()> {
        self.set_writable(this, dir, DirectoryState::PERMANENTLY_WRITABLE)
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:144 DirectoryTracker.setWritable
    fn set_writable(
        &self,
        this: &dyn AbstractActionInputPrefetcher,
        dir: &Path,
        new_state: DirectoryState,
    ) -> io::Result<()> {
        let base = this.base();
        // Compare as fragments since execRoot may be located on a file system overlaying the host
        // file system where downloads are written to.
        if !dir.as_fragment().starts_with(base.exec_root.as_fragment()) {
            return Ok(());
        }

        // `directoryStateMap.compute(dir, (unusedKey, oldState) -> ...)`
        let mut directory_state_map = self.directory_state_map.lock();
        let old_state = directory_state_map.get(dir).copied();
        if !this.force_refetch(dir)
            && (old_state == Some(DirectoryState::TEMPORARILY_WRITABLE)
                || old_state == Some(DirectoryState::PERMANENTLY_WRITABLE))
        {
            // Already writable, but must potentially upgrade from temporary to permanent.
            if new_state == DirectoryState::PERMANENTLY_WRITABLE {
                directory_state_map.insert(dir.clone(), new_state);
            }
            return Ok(());
        }
        // `caughtException.set(e); return oldState;`: on failure the mapping is left unchanged.
        create_writable_directory(base, dir)?;
        directory_state_map.insert(dir.clone(), new_state);
        Ok(())
    }

    /// Sets the output permissions on a directory.
    ///
    /// If `setPermanentlyWritable` has been previously called on this directory, or if no
    /// `setTemporarilyWritable` call has intervened since the last call to
    /// `setOutputPermissions`, this is a no-op. Otherwise, the output permissions are set.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:187 DirectoryTracker.setOutputPermissions
    fn set_output_permissions(
        &self,
        this: &dyn AbstractActionInputPrefetcher,
        dir: &Path,
    ) -> io::Result<()> {
        let base = this.base();
        // `directoryStateMap.compute(dir, (unusedKey, oldState) -> ...)`
        let mut directory_state_map = self.directory_state_map.lock();
        let old_state = directory_state_map.get(dir).copied();
        if !this.force_refetch(dir)
            && (old_state == Some(DirectoryState::OUTPUT_PERMISSIONS)
                || old_state == Some(DirectoryState::PERMANENTLY_WRITABLE))
        {
            // Either the output permissions have already been set, or we're not changing the
            // permissions ever again.
            return Ok(());
        }
        dir.chmod(base.output_permissions.get_permissions_mode() as u32)?;
        directory_state_map.insert(dir.clone(), DirectoryState::OUTPUT_PERMISSIONS);
        Ok(())
    }
}

/// The `try` block of `DirectoryTracker.setWritable`'s remapping function.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:161 DirectoryTracker.setWritable
fn create_writable_directory(
    base: &AbstractActionInputPrefetcherBase,
    dir: &Path,
) -> io::Result<()> {
    match &base.output_directory_helper {
        Some(output_directory_helper) => output_directory_helper
            .create_output_directory(dir, base.exec_root.as_fragment())
            // `CreateOutputDirectoryException extends IOException`.
            .map_err(|e| io::Error::new(e.get_cause().kind(), e))?,
        None => dir.create_directory_and_parents()?,
    }
    dir.set_writable(true)
}

/// A symlink in the output tree that either points to another artifact's absolute path or
/// represents an unresolved symlink with its target path preserved verbatim.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:229 AbstractActionInputPrefetcher.Symlink
#[derive(Clone, Debug)]
struct Symlink {
    link_path: Path,
    target_path: PathFragment,
}

impl Symlink {
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:230 Symlink.<init>
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

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:237 Symlink.resolveOne
    fn resolve_one(&self) -> io::Result<Path> {
        resolve_one_symlink(&self.link_path, Some(&self.target_path))
    }
}

/// The fields of `AbstractActionInputPrefetcher`.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:82 AbstractActionInputPrefetcher
pub struct AbstractActionInputPrefetcherBase {
    reporter: Arc<Reporter>,
    download_cache: AsyncTaskCacheNoResult<Path>,
    temp_path_generator: Arc<TempPathGenerator>,
    output_permissions: OutputPermissions,

    exec_root: Path,
    remote_output_checker: Arc<RemoteOutputChecker>,

    output_directory_helper: Option<Arc<ActionOutputDirectoryHelper>>,

    directory_tracker: DirectoryTracker,

    // Tree artifact downloads use temporary paths. Coordinate by resolved tree root: a read lock
    // protects reopening ancestors and moving one child into place, while a write lock protects
    // restoring every directory touched by a prefetchFiles call to its output permissions. Weak
    // locks over the full hash space allow unrelated trees to finalize concurrently without
    // retaining every lock for the lifetime of the prefetcher.
    //
    // PORT: Java's `Striped.lazyWeakReadWriteLock(Integer.MAX_VALUE)` is one lock per tree root
    // fragment; the locks are kept for the lifetime of the prefetcher (one per tree root seen).
    tree_artifact_locks: Mutex<HashMap<PathFragment, Arc<RwLock<()>>>>,

    /// The prefetcher that owns these fields, for the downloads of `downloadCache`.
    ///
    /// PORT: a download is one execution shared by every caller that waits for it and keeps
    /// running while one of them is left (`AsyncTaskCache`), so it cannot borrow the prefetcher
    /// from the caller that started it; it holds this handle instead. Set by
    /// [`register_prefetcher`].
    self_ref: std::sync::OnceLock<std::sync::Weak<dyn AbstractActionInputPrefetcher>>,
}

impl std::fmt::Debug for AbstractActionInputPrefetcherBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbstractActionInputPrefetcher")
            .field("exec_root", &self.exec_root)
            .field("output_permissions", &self.output_permissions)
            .finish_non_exhaustive()
    }
}

impl AbstractActionInputPrefetcherBase {
    /// `new AbstractActionInputPrefetcher(reporter, execRoot, tempPathGenerator,
    /// remoteOutputChecker, outputDirectoryHelper, outputPermissions)`.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:242 AbstractActionInputPrefetcher.<init>
    pub fn new(
        reporter: Arc<Reporter>,
        exec_root: Path,
        temp_path_generator: Arc<TempPathGenerator>,
        remote_output_checker: Arc<RemoteOutputChecker>,
        output_directory_helper: Option<Arc<ActionOutputDirectoryHelper>>,
        output_permissions: OutputPermissions,
    ) -> AbstractActionInputPrefetcherBase {
        AbstractActionInputPrefetcherBase {
            reporter,
            download_cache: AsyncTaskCacheNoResult::create(),
            temp_path_generator,
            output_permissions,
            exec_root,
            remote_output_checker,
            output_directory_helper,
            directory_tracker: DirectoryTracker::default(),
            tree_artifact_locks: Mutex::new(HashMap::new()),
            self_ref: std::sync::OnceLock::new(),
        }
    }

    /// The prefetcher that owns these fields (see `self_ref`).
    fn prefetcher(&self) -> Arc<dyn AbstractActionInputPrefetcher> {
        self.self_ref
            .get()
            .and_then(std::sync::Weak::upgrade)
            .expect("the prefetcher was created without register_prefetcher")
    }

    /// `execRoot` (protected in Java).
    pub fn exec_root(&self) -> &Path {
        &self.exec_root
    }

    /// `outputDirectoryHelper` (protected in Java).
    pub fn output_directory_helper(&self) -> Option<&Arc<ActionOutputDirectoryHelper>> {
        self.output_directory_helper.as_ref()
    }

    /// `treeArtifactLocks.get(treeRoot)`.
    fn tree_artifact_lock(&self, tree_root: &PathFragment) -> Arc<RwLock<()>> {
        self.tree_artifact_locks
            .lock()
            .entry(tree_root.clone())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }
}

/// Shares a new prefetcher and lets its fields know it (see
/// `AbstractActionInputPrefetcherBase::self_ref`). Every prefetcher is created through this.
pub fn register_prefetcher<T: AbstractActionInputPrefetcher + 'static>(prefetcher: T) -> Arc<T> {
    let prefetcher = Arc::new(prefetcher);
    let as_dyn: Arc<dyn AbstractActionInputPrefetcher> = prefetcher.clone();
    blaze_util::check_state!(
        prefetcher
            .base()
            .self_ref
            .set(Arc::downgrade(&as_dyn))
            .is_ok(),
        "the prefetcher is already registered"
    );
    prefetcher
}

/// What a download task keeps of the action it downloads for (`ActionExecutionMetadata`): the
/// download is shared with other callers and outlives the call that started it, so it cannot
/// borrow the action.
#[derive(Debug)]
struct ActionMetadataSnapshot {
    mnemonic: String,
    progress_message: Option<String>,
    owner: blaze_exec::actions::action_execution_metadata::ActionOwner,
    primary_output: ActionInputRef,
    execution_platform: Option<blaze_exec::actions::action_execution_metadata::PlatformInfo>,
    pretty_print: String,
    describe: String,
    may_modify_spawn_outputs_after_execution: bool,
}

impl ActionMetadataSnapshot {
    fn of(action: &dyn ActionExecutionMetadata) -> ActionMetadataSnapshot {
        ActionMetadataSnapshot {
            mnemonic: action.get_mnemonic(),
            progress_message: action.get_progress_message(),
            owner: action.get_owner().clone(),
            primary_output: action.get_primary_output(),
            execution_platform: action.get_execution_platform().cloned(),
            pretty_print: action.pretty_print(),
            describe: action.describe(),
            may_modify_spawn_outputs_after_execution: action
                .may_modify_spawn_outputs_after_execution(),
        }
    }
}

impl ActionExecutionMetadata for ActionMetadataSnapshot {
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }
    fn get_progress_message(&self) -> Option<String> {
        self.progress_message.clone()
    }
    fn get_owner(&self) -> &blaze_exec::actions::action_execution_metadata::ActionOwner {
        &self.owner
    }
    fn get_primary_output(&self) -> ActionInputRef {
        self.primary_output.clone()
    }
    fn get_execution_platform(
        &self,
    ) -> Option<&blaze_exec::actions::action_execution_metadata::PlatformInfo> {
        self.execution_platform.as_ref()
    }
    fn pretty_print(&self) -> String {
        self.pretty_print.clone()
    }
    fn describe(&self) -> String {
        self.describe.clone()
    }
    fn may_modify_spawn_outputs_after_execution(&self) -> bool {
        self.may_modify_spawn_outputs_after_execution
    }
}

/// The abstract methods of `AbstractActionInputPrefetcher`, which the concrete prefetcher
/// implements. The orchestration (the concrete methods of the Java class) is in the inherent
/// methods of `dyn AbstractActionInputPrefetcher`.
///
/// Abstract implementation of `ActionInputPrefetcher` which implements the orchestration of
/// prefeching multiple inputs so subclasses can focus on prefetching / downloading single input.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:82 AbstractActionInputPrefetcher
#[async_trait]
pub trait AbstractActionInputPrefetcher: Send + Sync {
    /// The fields of the abstract class.
    fn base(&self) -> &AbstractActionInputPrefetcherBase;

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:289 AbstractActionInputPrefetcher.canDownloadFile
    fn can_download_file(&self, path: &Path, metadata: &FileArtifactValue) -> bool;

    /// If true, then all previously acquired knowledge of the file system state of this path
    /// (e.g. the existence of tree artifact directories or previously downloaded files) must be
    /// discarded.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:295 AbstractActionInputPrefetcher.forceRefetch
    fn force_refetch(&self, path: &Path) -> bool;

    /// Downloads file to the given path via its metadata.
    ///
    /// `temp_path`: the temporary path which the input should be written to.
    ///
    /// PORT: Java's method may throw synchronously or fail the returned future; both are the
    /// error of this `async fn`, which the caller treats alike (`toCompletable`).
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:302 AbstractActionInputPrefetcher.doDownloadFile
    async fn do_download_file(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        reporter: &Reporter,
        input: &ActionInputRef,
        temp_path: &Path,
        metadata: &FileArtifactValue,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()>;

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:312 AbstractActionInputPrefetcher.prefetchVirtualActionInput
    fn prefetch_virtual_action_input(&self, _input: &dyn VirtualActionInput) -> io::Result<()> {
        Ok(())
    }
}

/// `metadata.setContentsProxy(proxy)`: the cell is shared by every copy of the metadata (the
/// file system's, Skyframe's and the input map's), as Java's one object is.
// Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:883 RemoteFileArtifactValueWithMaterializationData.setContentsProxy
fn set_contents_proxy(metadata: &FileArtifactValue, proxy: FileContentsProxy) {
    metadata.set_contents_proxy(proxy);
}

/// The `blaze_actions` artifact behind an `ActionInput` that is an `Artifact`.
fn blaze_artifact(input: &dyn ActionInput) -> Option<&BlazeArtifact> {
    input.as_artifact()?;
    match input.as_any().downcast_ref::<BlazeArtifact>() {
        Some(artifact) => Some(artifact),
        None => blaze_util::unported!(
            "AbstractActionInputPrefetcher (missing hook: Artifact.getPath() / instanceof TreeFileArtifact on blaze_exec::actions::Artifact; the input is not a blaze_actions::Artifact)"
        ),
    }
}

/// `artifact.getPath()` for an input that is an `Artifact`.
fn artifact_get_path(input: &dyn ActionInput) -> Option<Path> {
    blaze_artifact(input).map(|artifact| artifact.get_path())
}

/// `input instanceof TreeFileArtifact treeFile ? treeFile.getParent() : null`.
fn tree_file_artifact_parent(input: &dyn ActionInput) -> Option<BlazeArtifact> {
    let artifact = blaze_artifact(input)?;
    if artifact.artifact_class() != ArtifactClass::TreeFileArtifact {
        return None;
    }
    Some(
        artifact
            .get_parent()
            .expect("TreeFileArtifact has a parent")
            .clone(),
    )
}

/// `input instanceof Artifact artifact && artifact.isChildOfDeclaredDirectory()`.
fn is_child_of_declared_directory(input: &dyn ActionInput) -> bool {
    input
        .as_artifact()
        .is_some_and(|artifact| artifact.is_child_of_declared_directory())
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:257 AbstractActionInputPrefetcher.shouldDownloadFile
fn should_download_file(path: &Path, metadata: &FileArtifactValue) -> io::Result<bool> {
    let Some(stat) = path.stat_if_found(Symlinks::Follow)? else {
        return Ok(true);
    };

    // If an action output is stale, Skyframe will delete it prior to action execution. However,
    // this doesn't apply to spawn outputs that aren't action outputs, or to files in external
    // repos that are remote repo contents cache hits. To avoid incorrectly reusing one such stale
    // file, check for its up-to-dateness here.
    if stat.get_size() != metadata.get_size() {
        return Ok(true);
    }
    let contents_proxy = metadata.get_contents_proxy();
    if contents_proxy
        .is_some_and(|contents_proxy| contents_proxy == FileContentsProxy::create(&stat))
    {
        return Ok(false);
    }

    let digest = match path.get_fast_digest()? {
        Some(digest) => digest,
        None => path.get_digest()?,
    };
    // `Arrays.equals(digest, metadata.getDigest())`
    if Some(digest.as_slice()) != metadata.get_digest() {
        return Ok(true);
    }
    // The file contents have been verified to be up to date. Record the contents proxy when
    // supported, just like after a fresh download, to make future modification checks cheaper.
    set_contents_proxy(metadata, FileContentsProxy::create(&stat));
    Ok(false)
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:588 AbstractActionInputPrefetcher.resolveOneSymlink
fn resolve_one_symlink(
    path: &Path,
    target_path_fragment: Option<&PathFragment>,
) -> io::Result<Path> {
    let target_path_fragment = match target_path_fragment {
        Some(target_path_fragment) => target_path_fragment.clone(),
        None => path.read_symbolic_link()?,
    };
    if target_path_fragment.is_absolute() {
        Ok(Path::create(target_path_fragment, path.get_file_system()))
    } else {
        Ok(path
            .get_parent_directory()
            .expect("NullPointerException: path.getParentDirectory()")
            .get_relative(&target_path_fragment))
    }
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:600 AbstractActionInputPrefetcher.maybeResolveSymlink
fn maybe_resolve_symlink(path: Path) -> io::Result<Path> {
    // Potentially resolves a symlink to its target path. This differs from
    // Path#resolveSymbolicLinks() that:
    //   1. Path#resolveSymbolicLinks() checks each segment of the path, but we assume there is no
    //      intermediate symlink because they should've been already normalized for outputs.
    //   2. In case of dangling symlink, we return the target path instead of throwing
    //      FileNotFoundException because we want to download output to that target path.
    let mut path = path;
    let mut max_attempt: i32 = 32;
    // `while (path.isSymbolicLink() && maxAttempt-- > 0)`
    loop {
        if !path.is_symbolic_link() {
            break;
        }
        let attempt = max_attempt;
        max_attempt -= 1;
        if attempt <= 0 {
            break;
        }
        let resolved_path = resolve_one_symlink(&path, /* targetPathFragment= */ None)?;
        if resolved_path.as_fragment() == path.as_fragment() {
            return Err(FileSymlinkLoopException::new_error(path.as_fragment()));
        }
        path = resolved_path;
    }
    if max_attempt <= 0 {
        return Err(FileSymlinkLoopException::new_error(path.as_fragment()));
    }
    Ok(path)
}

/// Deletes the temporary path of a download when the download ends without being finalized
/// (failure or cancellation).
///
/// PORT: Java's `usingTempPath` is `Completable.using(generateTempPath, task, cleanup, eager=false)`:
/// the cleanup runs when the download terminates or is disposed. Here the cleanup runs when this
/// guard is dropped: when the download future completes or is dropped (cancelled).
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:770 AbstractActionInputPrefetcher.usingTempPath
struct TempPathGuard {
    temp_path: Path,
    already_deleted: bool,
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        if !self.already_deleted {
            delete_partial_download(&self.temp_path);
        }
    }
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:784 AbstractActionInputPrefetcher.deletePartialDownload
fn delete_partial_download(path: &Path) {
    // PORT: skipped, logging (a failure is logged as a warning: "Failed to delete output file
    // after incomplete download: %s").
    let _ = path.delete();
}

/// `SetMultimap<Path, Path> directoriesByTreeRoot` of one `prefetchFiles` call.
///
/// PORT: Java's `HashMultimap` iterates in hash order; the order only decides in which order
/// the output permissions of the directories are restored.
type DirectoriesByTreeRoot = Mutex<IndexMap<Path, IndexSet<Path>>>;

impl<'t> dyn AbstractActionInputPrefetcher + 't {
    /// Fetches remotely stored action outputs and stores them under their path in the output
    /// base.
    ///
    /// The `inputs` may not contain any unexpanded directories.
    ///
    /// This method is safe to be called concurrently from spawn runners before running any local
    /// spawn.
    ///
    /// PORT: the body of `AbstractActionInputPrefetcher.prefetchFiles` (the
    /// `ActionInputPrefetcher` method); a concrete prefetcher's `ActionInputPrefetcher` impl
    /// calls it.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:325 AbstractActionInputPrefetcher.prefetchFiles
    pub async fn prefetch_files(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
        priority: Priority,
        reason: Reason,
    ) -> io::Result<()> {
        let metadata_supplier =
            |input: &ActionInputRef| metadata_provider.get_input_metadata(input);
        self.prefetch_files_interruptibly(action, inputs, &metadata_supplier, priority, reason)
            .await
    }

    /// Fetches remotely stored action outputs and stores them under their path in the output
    /// base.
    ///
    /// The `inputs` may not contain any unexpanded directories.
    ///
    /// This method is safe to be called concurrently from spawn runners before running any local
    /// spawn.
    ///
    /// This method is similar to `prefetchFiles()` above, but note that `metadataSupplier` may
    /// throw `InterruptedException`. If it does, this method will propagate this exception in
    /// the returned future.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:349 AbstractActionInputPrefetcher.prefetchFilesInterruptibly
    pub async fn prefetch_files_interruptibly(
        &self,
        action: Option<&dyn ActionExecutionMetadata>,
        inputs: &[ActionInputRef],
        metadata_supplier: &dyn MetadataSupplier,
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
        let directories_by_tree_root: DirectoriesByTreeRoot = Mutex::new(IndexMap::new());

        // Using plain futures to avoid RxJava overheads.
        // PORT: skipped, profiling (`compose prefetches`, `mergeBulkTransfer`).
        let transfers = files.iter().map(|file| {
            prefetch_file(
                self,
                action,
                &directories_by_tree_root,
                metadata_supplier,
                file,
                priority,
                reason,
            )
        });

        merge_bulk_transfer(transfers).await?;

        // Match the directory output permissions set by SkyframeActionExecutor#checkOutputs.
        let directories_by_tree_root = directories_by_tree_root.into_inner();
        let base = self.base();
        for (tree_root, dirs) in &directories_by_tree_root {
            let lock = base.tree_artifact_lock(tree_root.as_fragment());
            let _write_lock = lock.write();
            for dir in dirs {
                base.directory_tracker.set_output_permissions(self, dir)?;
            }
        }
        Ok(())
    }

    /// Forgets about completed downloads of the given paths so that the next prefetch of each of
    /// them verifies it against the local file system instead of assuming that it is still in
    /// place.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:817 AbstractActionInputPrefetcher.invalidateDownloads
    pub fn invalidate_downloads<'a>(&self, exec_paths: impl IntoIterator<Item = &'a PathFragment>) {
        let base = self.base();
        for path in exec_paths {
            // Downloads are written to the actual host file system, not any overlays.
            base.download_cache
                .invalidate(&base.exec_root.get_relative(path).for_host_file_system());
        }
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:824 AbstractActionInputPrefetcher.downloadedFiles
    pub fn downloaded_files(&self) -> HashSet<Path> {
        self.base()
            .download_cache
            .get_finished_tasks()
            .into_iter()
            .collect()
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:828 AbstractActionInputPrefetcher.downloadsInProgress
    pub fn downloads_in_progress(&self) -> HashSet<Path> {
        self.base()
            .download_cache
            .get_in_progress_tasks()
            .into_iter()
            .collect()
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:833 AbstractActionInputPrefetcher.getDownloadCache
    pub fn get_download_cache(&self) -> &AsyncTaskCacheNoResult<Path> {
        &self.base().download_cache
    }

    /// Shuts the download cache down and waits until it has terminated; an interrupt during the
    /// wait cancels the downloads still running (`shutdownNow`) and the wait starts again.
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:837 AbstractActionInputPrefetcher.shutdown
    pub fn shutdown(&self) {
        let download_cache = &self.base().download_cache;
        download_cache.shutdown();
        loop {
            // try { downloadCache.awaitTermination(); break; }
            if crate::runtime::block_on_interruptibly(download_cache.await_termination()).is_some()
            {
                break;
            }
            // catch (InterruptedException ignored) { downloadCache.shutdownNow(); }
            download_cache.shutdown_now();
        }
    }

    /// Downloads the outputs of `action` that the remote output checker wants downloaded.
    ///
    /// PORT: `action.getOutputs()` is `action_outputs`: blaze_exec's `ActionExecutionMetadata`
    /// has no `getOutputs()`. `OutputMetadataStore` errors: `InterruptedException` is
    /// `errors::interrupted()`. `throws IOException, LostInputsExecException,
    /// InterruptedException`: the `LostInputsExecException` is [`SpawnCacheError::Exec`].
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:849 AbstractActionInputPrefetcher.finalizeAction
    pub fn finalize_action(
        &self,
        action: &dyn ActionExecutionMetadata,
        action_outputs: &[ActionInputRef],
        output_metadata_store: &dyn OutputMetadataStore,
    ) -> Result<(), SpawnCacheError> {
        let base = self.base();
        let mut outputs_to_download: Vec<ActionInputRef> = Vec::new();
        for output in action_outputs {
            if output_metadata_store.artifact_omitted(output) {
                continue;
            }

            let metadata = output_metadata_store
                .get_output_metadata(output)
                .map_err(io_or_interrupted)?
                .expect("NullPointerException: outputMetadataStore.getOutputMetadata(output)");
            let output_path = artifact_get_path(&**output)
                .expect("ClassCastException: an action output is an Artifact");
            if !self.can_download_file(&output_path, &metadata) {
                continue;
            }

            if output
                .as_artifact()
                .is_some_and(|artifact| artifact.is_tree_artifact())
            {
                let tree_artifact_value = output_metadata_store
                    .get_tree_artifact_value(output)
                    .map_err(io_or_interrupted)?;
                for (child, child_metadata) in &tree_artifact_value.child_values {
                    if base
                        .remote_output_checker
                        .should_download_output(&**child, child_metadata)
                    {
                        outputs_to_download.push(child.clone());
                    }
                }
            } else if base
                .remote_output_checker
                .should_download_output(&**output, &metadata)
            {
                outputs_to_download.push(output.clone());
            }
        }

        if !outputs_to_download.is_empty() {
            // PORT: skipped, profiling (`ProfilerTask.REMOTE_DOWNLOAD`, "Download outputs").
            let metadata_supplier = |input: &ActionInputRef| {
                output_metadata_store
                    .get_output_metadata(input)
                    .map_err(io_or_interrupted)
            };
            let result = get_from_future(self.prefetch_files_interruptibly(
                Some(action),
                &outputs_to_download,
                &metadata_supplier,
                Priority::HIGH,
                Reason::OUTPUTS,
            ));
            if let Err(e) = result {
                let Some(bulk_transfer_exception) =
                    errors::downcast_ref::<BulkTransferException>(&e)
                else {
                    return Err(e.into());
                };
                // Outputs whose blobs the remote cache has lost are recreated by rewinding.
                // `Maps.uniqueIndex(outputsToDownload, Artifact::getExecPath)`
                let mut by_exec_path: HashMap<PathFragment, ActionInputRef> = HashMap::new();
                for output in &outputs_to_download {
                    if by_exec_path
                        .insert(output.get_exec_path().clone(), output.clone())
                        .is_some()
                    {
                        panic!(
                            "IllegalArgumentException: Multiple entries with same key: {}",
                            output.get_exec_path()
                        );
                    }
                }
                let lost_outputs =
                    bulk_transfer_exception.get_lost_artifacts(&|exec_path: &PathFragment| {
                        by_exec_path.get(exec_path).cloned()
                    });
                if lost_outputs.is_empty() {
                    return Err(e.into());
                }
                // `throw new LostInputsExecException(lostOutputs.byDigest(), e)`
                return Err(LostInputsExecException::create(lost_outputs).into());
            }
        }
        Ok(())
    }

    /// PORT: blocks the calling thread (Java's `awaitInProgressTasks` is blocking too).
    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:900 AbstractActionInputPrefetcher.flushOutputTree
    pub fn flush_output_tree(&self) {
        block_on(self.base().download_cache.await_in_progress_tasks());
    }

    // Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:904 AbstractActionInputPrefetcher.getRemoteOutputChecker
    pub fn get_remote_output_checker(&self) -> &Arc<RemoteOutputChecker> {
        &self.base().remote_output_checker
    }
}

/// `throws IOException, InterruptedException` of `OutputMetadataStore` as an `io::Error`.
fn io_or_interrupted(e: IoOrInterruptedException) -> io::Error {
    match e {
        IoOrInterruptedException::Io(e) => e,
        IoOrInterruptedException::Interrupted(_) => errors::interrupted(),
    }
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:419 AbstractActionInputPrefetcher.prefetchFile
async fn prefetch_file(
    this: &dyn AbstractActionInputPrefetcher,
    action: Option<&dyn ActionExecutionMetadata>,
    directories_by_tree_root: &DirectoriesByTreeRoot,
    metadata_supplier: &dyn MetadataSupplier,
    input: &ActionInputRef,
    priority: Priority,
    reason: Reason,
) -> io::Result<()> {
    let base = this.base();
    if let Some(virtual_action_input) = input.as_virtual_action_input() {
        this.prefetch_virtual_action_input(virtual_action_input)?;
        return Ok(());
    }

    let mut input_path = match artifact_get_path(&**input) {
        Some(path) => path,
        None => base.exec_root.get_relative(input.get_exec_path()),
    };

    // Metadata may legitimately be missing, e.g. if this is an optional test output.
    let Some(metadata) = metadata_supplier.get_metadata(input)? else {
        return Ok(());
    };

    if metadata.get_type() == FileStateType::Directory {
        // Tree artifacts have already been expanded into their children, so this is a source
        // directory. If it lies in an external repo backed by the remote repo contents cache, its
        // contents may only be available in memory and must be materialized to the local file
        // system for local actions to access them.
        // `inputPath.getFileSystem() instanceof SubtreeMaterializer subtreeMaterializer`: the one
        // implementation is `RemoteExternalOverlayFileSystem`.
        if let Some(subtree_materializer) =
            crate::remote_external_overlay_file_system::RemoteExternalOverlayFileSystem::of(
                input_path.get_file_system(),
            )
        {
            use crate::remote_external_overlay_file_system::SubtreeMaterializer as _;
            // PORT: the materialization blocks on remote transfers, which must not happen on a
            // thread of the remote runtime; it runs on a thread of its own while this task
            // waits.
            let path = input_path.as_fragment().clone();
            let (sender, receiver) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = sender.send(subtree_materializer.ensure_subtree_materialized(&path));
            });
            receiver.await.map_err(|_| {
                io::Error::other("subtree materialization ended without a result")
            })??;
        }
        return Ok(());
    }

    let symlinks = get_symlinks(this, &**input, &input_path, &metadata, metadata_supplier)?;
    // On Windows, the type of symlink depends on the target file and the target may have to
    // exist, so we plant symlinks in reverse order and only after any download has completed.
    // `concat(Lists.transform(symlinks.reverse(), this::plantSymlink))`
    let plant_symlinks = async {
        for symlink in symlinks.iter().rev() {
            plant_symlink(this, symlink).await?;
        }
        Ok::<(), io::Error>(())
    };

    if !this.can_download_file(&input_path, &metadata) {
        // If the artifact is a declared ("unresolved") symlink, it can't be "downloaded", but the
        // symlink logic above creates it.
        return plant_symlinks.await;
    }

    if !symlinks.is_empty() {
        // Symlink may track the parent of a TreeFileArtifact, so the parent relative path has to
        // be translated relative to it.
        let parent_relative_path = input_path.relative_to(&symlinks[0].link_path);
        let resolved = symlinks[symlinks.len() - 1].resolve_one()?;
        input_path = Path::create(
            resolved.as_fragment().get_relative(&parent_relative_path),
            input_path.get_file_system(),
        );
    }

    let tree_root_path = maybe_get_tree_root(&**input, metadata_supplier)?;

    download_file_no_check_rx(
        this,
        action,
        input,
        input_path,
        tree_root_path,
        directories_by_tree_root,
        input,
        &metadata,
        priority,
        reason,
    )
    .await?;
    plant_symlinks.await
}

/// For an input belonging to a tree artifact, returns the resolved path of the tree artifact
/// root. Otherwise, returns `None`.
///
/// Some artifacts (notably, those created by `ctx.actions.symlink`) are materialized in the
/// output tree as a symlink to another artifact, as indicated by the
/// `FileArtifactValue#getResolvedPath()` field in their metadata.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:506 AbstractActionInputPrefetcher.maybeGetTreeRoot
fn maybe_get_tree_root(
    input: &dyn ActionInput,
    metadata_supplier: &dyn MetadataSupplier,
) -> io::Result<Option<Path>> {
    let Some(tree_artifact) = tree_file_artifact_parent(input) else {
        return Ok(None);
    };
    let tree_artifact_ref = ActionInputRef::new(tree_artifact.clone());
    let Some(tree_metadata) = metadata_supplier.get_metadata(&tree_artifact_ref)? else {
        // There are two cases where tree metadata is legitimately not available:
        // (1) If the file is the output of an action expanded from an action template. In this
        //     case, the symlink optimization is intentionally not supported.
        // (2) If the file is part of an input fileset. In this case, a symlink has already been
        //     created, but we're currently unable to prefetch the file(s) it points to.
        // TODO: b/401575099 - Treating fileset more like runfiles could make the tree metadata
        //  available for case (2).
        return Ok(None);
    };
    if let Some(resolved_path) = tree_metadata.get_resolved_path() {
        return Ok(Some(Path::create(
            resolved_path.clone(),
            tree_artifact.get_path().get_file_system(),
        )));
    }
    Ok(Some(tree_artifact.get_path()))
}

/// Returns the symlinks to be planted on disk for inputs that are prefetched into a different
/// location, ordered from the input path to the final target.
///
/// Some artifacts (notably, those created by `ctx.actions.symlink`) are materialized in the
/// output tree as a symlink to another artifact, as indicated by the
/// `FileArtifactValue#getResolvedPath()` field in their (or their parent tree artifact's)
/// metadata.
///
/// Paths in external repos backed by the remote repo contents cache may be (part of) a chain of
/// symlinks created by the repo rule, which has to be reproduced verbatim on disk.
// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:541 AbstractActionInputPrefetcher.getSymlinks
fn get_symlinks(
    this: &dyn AbstractActionInputPrefetcher,
    input: &dyn ActionInput,
    input_path: &Path,
    metadata: &FileArtifactValue,
    metadata_supplier: &dyn MetadataSupplier,
) -> io::Result<Vec<Symlink>> {
    if let Some(tree_artifact) = tree_file_artifact_parent(input) {
        let tree_artifact_ref = ActionInputRef::new(tree_artifact.clone());
        let Some(tree_metadata) = metadata_supplier.get_metadata(&tree_artifact_ref)? else {
            // There are two cases where tree metadata is legitimately not available:
            // (1) If the file is the output of an action expanded from an action template. In
            //     this case, the symlink optimization is intentionally not supported.
            // (2) If the file is part of an input fileset. In this case, a symlink has already
            //     been created, but we're currently unable to prefetch the file(s) it points to.
            // TODO: b/401575099 - Treating fileset more like runfiles could make the tree
            //  metadata available for case (2).
            return Ok(Vec::new());
        };
        return get_symlinks(
            this,
            &tree_artifact,
            &tree_artifact.get_path(),
            &tree_metadata,
            metadata_supplier,
        );
    }
    let exec_root = &this.base().exec_root;
    if (metadata.is_remote() || metadata.get_type() == FileStateType::Symlink)
        && !input_path.starts_with(exec_root)
    {
        // A path in an external repo, e.g. a source artifact consumed by an action or a file
        // prefetched during the materialization of an external repo. It may be (part of) a chain
        // of symlinks created by the repo rule, which has to be reproduced verbatim on disk.
        // SECURITY: as in Java, the targets of the symlinks found on disk are not checked; they
        // are re-created verbatim by `plant_symlink`.
        let mut symlink_chain: Vec<Symlink> = Vec::new();
        let mut current_path = input_path.clone();
        let mut max_attempt: i32 = 32;
        loop {
            match current_path.stat_if_found(Symlinks::Nofollow)? {
                Some(stat) if stat.is_symbolic_link() => {}
                _ => break,
            }
            // `if (maxAttempt-- == 0)`
            let attempt = max_attempt;
            max_attempt -= 1;
            if attempt == 0 {
                return Err(FileSymlinkLoopException::new_error(
                    input_path.as_fragment(),
                ));
            }
            let symlink = Symlink::new(current_path.clone(), current_path.read_symbolic_link()?);
            symlink_chain.push(symlink.clone());
            current_path = symlink.resolve_one()?;
        }
        return Ok(symlink_chain);
    }
    let resolved_path = metadata.get_resolved_path();
    match resolved_path {
        None => Ok(Vec::new()),
        Some(resolved_path) if resolved_path == input_path.as_fragment() => Ok(Vec::new()),
        // SECURITY: as in Java, the resolved path comes from the action's metadata (Blaze's own
        // analysis or a remote action's output symlinks after RemoteExecutionService checked
        // them) and is not validated again here.
        Some(resolved_path) => Ok(vec![Symlink::new(
            input_path.clone(),
            resolved_path.clone(),
        )]),
    }
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:621 AbstractActionInputPrefetcher.downloadFileNoCheckRx
async fn download_file_no_check_rx(
    this: &dyn AbstractActionInputPrefetcher,
    action: Option<&dyn ActionExecutionMetadata>,
    input: &ActionInputRef,
    path: Path,
    tree_root: Option<Path>,
    directories_by_tree_root: &DirectoriesByTreeRoot,
    action_input: &ActionInputRef,
    metadata: &FileArtifactValue,
    priority: Priority,
    reason: Reason,
) -> io::Result<()> {
    let base = this.base();
    // If the path to be prefetched is a non-dangling symlink, prefetch its target path instead.
    // Note that this only applies to symlinks created by spawns (or, currently, with the internal
    // version of BwoB); symlinks created in-process through an ActionFileSystem should have
    // already been canonicalized by maybeGetSymlink.
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

    let final_tree_root: Option<Path> = match &tree_root {
        Some(tree_root) if is_child_of_declared_directory(&**action_input) => {
            Some(tree_root.for_host_file_system())
        }
        _ => None,
    };
    if let Some(final_tree_root) = &final_tree_root {
        // Record directories child-to-root. If a directory is already present for this tree root,
        // an earlier traversal also recorded every ancestor through the root.
        let mut directories_by_tree_root = directories_by_tree_root.lock();
        let directories = directories_by_tree_root
            .entry(final_tree_root.clone())
            .or_default();
        let mut dir = final_path
            .get_parent_directory()
            .expect("NullPointerException: finalPath.getParentDirectory()");
        while dir.starts_with(final_tree_root) {
            if !directories.insert(dir.clone()) {
                break;
            }
            dir = dir
                .get_parent_directory()
                .expect("NullPointerException: dir.getParentDirectory()");
        }
    }

    // `usingTempPath((tempPath, alreadyDeleted) -> toCompletable(() -> doDownloadFile(...))
    //     .doOnComplete(() -> { finalizeDownload(...); alreadyDeleted.set(true); }))`
    // SECURITY: the bytes from the cache are written to a fresh `TempPathGenerator` path, never to
    // the output path; the digest is verified by `CombinedCache.downloadFile` (called from
    // `doDownloadFile`); only then `finalize_download` sets the output permissions and moves the
    // file into place. A failed or cancelled download deletes the temporary path.
    // The download is shared by every caller that waits for `final_path` and outlives the caller
    // that starts it: it owns what it uses.
    let prefetcher = base.prefetcher();
    let action = action.map(ActionMetadataSnapshot::of);
    let input = input.clone();
    let metadata = metadata.clone();
    let task_final_path = final_path.clone();
    let download = async move {
        let this: &dyn AbstractActionInputPrefetcher = &*prefetcher;
        let base = this.base();
        // `Completable.defer(...)`
        if !should_download_file(&task_final_path, &metadata)? {
            return Ok::<(), io::Error>(());
        }
        let mut temp_path = TempPathGuard {
            temp_path: base.temp_path_generator.generate_temp_path(),
            already_deleted: false,
        };
        this.do_download_file(
            action
                .as_ref()
                .map(|action| action as &dyn ActionExecutionMetadata),
            &base.reporter,
            &input,
            &temp_path.temp_path.for_host_file_system(),
            &metadata,
            priority,
            reason,
        )
        .await?;
        finalize_download(
            this,
            &metadata,
            &temp_path.temp_path.for_host_file_system(),
            &task_final_path,
            final_tree_root.as_ref(),
        )?;
        temp_path.already_deleted = true;
        Ok::<(), io::Error>(())
    };

    let force = this.force_refetch(&final_path);
    base.download_cache
        .execute(final_path.clone(), download, force)
        .await
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:702 AbstractActionInputPrefetcher.finalizeDownload
fn finalize_download(
    this: &dyn AbstractActionInputPrefetcher,
    metadata: &FileArtifactValue,
    tmp_path: &Path,
    final_path: &Path,
    tree_root: Option<&Path>,
) -> io::Result<()> {
    let base = this.base();
    // Set file output permissions, matching SkyframeActionExecutor#checkOutputs for artifacts
    // produced by local actions. The temporary path is outside the output tree, so this can
    // happen before entering the tree artifact root's critical section.
    tmp_path.chmod(base.output_permissions.get_permissions_mode() as u32)?;

    let parent_dir = final_path
        .get_parent_directory()
        .expect("NullPointerException: finalPath.getParentDirectory()");
    let tree_artifact_lock =
        tree_root.map(|tree_root| base.tree_artifact_lock(tree_root.as_fragment()));
    {
        let _read_lock = tree_artifact_lock.as_ref().map(|lock| lock.read());

        // Compare as fragments since execRoot may be located on a file system overlaying the
        // host file system where the download is written to.
        if final_path
            .as_fragment()
            .starts_with(base.exec_root.as_fragment())
        {
            // Ensure the parent directory exists and is writable. We cannot rely on this
            // precondition to have been established by the execution of the owning action in a
            // previous invocation, since the output tree may have been externally modified in
            // between invocations.
            if let Some(tree_root) = tree_root {
                // Reopen every ancestor because ActionOutputDirectoryHelper caches existing
                // directories and may otherwise leave a previously restored ancestor read-only.
                let mut dir = tree_root.clone();
                base.directory_tracker
                    .set_temporarily_writable(this, &dir)?;
                for segment in parent_dir.relative_to(tree_root).segments() {
                    dir = dir.get_child(segment);
                    base.directory_tracker
                        .set_temporarily_writable(this, &dir)?;
                }
            } else {
                // One of the following must apply:
                //   (1) The file does not belong to a tree artifact.
                //   (2) The file belongs to a tree artifact created by an action template
                //       expansion.
                // In case (1), the parent directory is a package or a subdirectory of a package,
                // and should remain writable. In case (2), even though we arguably ought to set
                // the output permissions on the parent directory to match local execution, we
                // choose not to do it and avoid the additional implementation complexity required
                // to detect a race condition between concurrent calls touching the same
                // directory.
                base.directory_tracker
                    .set_permanently_writable(this, &parent_dir)?;
            }
        } else {
            // SECURITY: as in Java, a final path outside the exec root (an external repo of the
            // remote repo contents cache, or the target of a symlink in the output tree) gets its
            // parent directory created; the path itself comes from the artifact and its metadata,
            // not from the server.
            parent_dir.create_directory_and_parents()?;
        }

        fs_utils::move_file(tmp_path, final_path)?;
    }

    // Set the contents proxy when supported, to make future modification checks cheaper.
    set_contents_proxy(metadata, FileContentsProxy::create(&final_path.stat()?));
    Ok(())
}

// Java: com/google/devtools/build/lib/remote/AbstractActionInputPrefetcher.java:793 AbstractActionInputPrefetcher.plantSymlink
async fn plant_symlink(
    this: &dyn AbstractActionInputPrefetcher,
    symlink: &Symlink,
) -> io::Result<()> {
    let base = this.base();
    let link_path = symlink.link_path.for_host_file_system();
    let force = this.force_refetch(&link_path);
    // The task is shared by every caller that waits for `link_path`: it owns what it uses.
    let symlink = symlink.clone();
    let exec_root = base.exec_root.clone();
    let task_link_path = link_path.clone();
    base.download_cache
        .execute(
            link_path,
            // `Completable.defer(...)`
            async move {
                let link_path = task_link_path;
                if !symlink
                    .link_path
                    .as_fragment()
                    .starts_with(exec_root.as_fragment())
                {
                    // If the symlink is a source file in an external repo, its parent directory
                    // may not exist yet.
                    link_path
                        .get_parent_directory()
                        .expect("NullPointerException: linkPath.getParentDirectory()")
                        .create_directory_and_parents()?;
                }
                // Delete the link path if it already exists. This is the case for tree artifacts,
                // whose root directory is created before the action runs.
                // SECURITY: as in Java, `delete` removes a file, a symlink or an empty directory
                // at the link path (it does not follow symlinks), and the symlink target is
                // written verbatim.
                link_path.delete()?;
                link_path.create_symbolic_link_to_fragment(&symlink.target_path)?;
                Ok::<(), io::Error>(())
            },
            force,
        )
        .await
}
