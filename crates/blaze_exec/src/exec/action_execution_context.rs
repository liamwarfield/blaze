//! The seam between spawn strategies and the action executor: the subset of
//! `lib/actions/ActionExecutionContext.java` (and of `ActionContext.ActionContextRegistry`) that
//! `AbstractSpawnStrategy`, `SpawnStrategyResolver` and the strategies read, plus the
//! `lib/exec/SpawnCache.java` interface they look up through it and the
//! `lib/actions/SpawnExecutedEvent.java` they post.
//!
//! Implemented by the action executor (`blaze_sky_exec`) for each action execution, and by tests.

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use blaze_events::{Clock, ExtendedEventHandler, Postable};
use blaze_vfs::{FileSystemRef, Path};
use indexmap::IndexMap;

use crate::actions::action_execution::ActionContextRef;
use crate::actions::{
    ActionExecutionMetadata, ActionInputRef, ActionOwner, ExecException, FileArtifactValue,
    InputMetadataProvider, Spawn, SpawnResult,
};
use crate::exec::spawn_log_context::SpawnLogContext;
use crate::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext};
use crate::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use crate::util::FileOutErr;

/// `ActionContext.ActionContextRegistry`: `getContext(Class)` for the context types the execution
/// layer looks up, as typed getters. `None` where Java's `getContext` returns null.
// Java: com/google/devtools/build/lib/actions/ActionContext.java:35 ActionContext.ActionContextRegistry
pub trait ActionContextRegistry: Send + Sync {
    /// `getContext(SpawnStrategyRegistry.class)` (also registered as `DynamicStrategyRegistry`
    /// and `RemoteLocalFallbackRegistry`).
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        None
    }
    /// `getContext(SpawnLogContext.class)`: set when an execution log is requested.
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        None
    }
    /// `getContext(SpawnCache.class)`: `NoSpawnCacheModule` registers [`no_cache`] in Bazel
    /// unless a remote or disk cache replaces it.
    fn get_spawn_cache(&self) -> Option<Arc<dyn SpawnCache>> {
        None
    }
    /// `getContext(Class)` for any other context type (`TestActionContext`, ...): the registered
    /// context whose concrete type is `type_id`, or `None`. Use [`get_context`].
    fn get_context_any(
        &self,
        _type_id: std::any::TypeId,
    ) -> Option<crate::actions::action_execution::ActionContextRef> {
        None
    }
}

/// `getContext(T.class)` for a context registered through
/// [`ActionContextRegistry::get_context_any`] under `TypeId::of::<T>()`. Register the context as
/// `Arc<T>` (wrapped as `Arc<dyn Any>`), so trait-object contexts use `T = Arc<dyn Trait>`.
pub fn get_context<T: Send + Sync + 'static>(
    registry: &(impl ActionContextRegistry + ?Sized),
) -> Option<Arc<T>> {
    registry
        .get_context_any(std::any::TypeId::of::<T>())
        .and_then(|c| c.downcast::<T>().ok())
}

/// The parts of `ActionExecutionContext` that spawn strategies read.
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:64 ActionExecutionContext
pub trait ActionExecutionContext: ActionContextRegistry {
    /// `getExecRoot()`.
    fn get_exec_root(&self) -> &Path;

    /// `getInputMetadataProvider()`.
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider;

    /// `getActionFileSystem()`: `None` unless an action file system is in use.
    fn get_action_file_system(&self) -> Option<FileSystemRef> {
        None
    }

    /// `getFileOutErr()`.
    fn get_file_out_err(&self) -> &FileOutErr;

    /// `getClientEnv()`.
    fn get_client_env(&self) -> &IndexMap<String, String>;

    /// `getEventHandler()`.
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler;

    /// `getClock()`.
    fn get_clock(&self) -> &dyn Clock;

    /// `maybeReportSubcommand(spawn, spawnRunner)`: prints the command when `--subcommands` is set.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:459 ActionExecutionContext.maybeReportSubcommand
    fn maybe_report_subcommand(&self, spawn: &dyn Spawn, spawn_runner: Option<&str>);

    /// `progress.postTo(getEventHandler(), action)` (`SpawnRunner.ProgressStatus.postTo`): posts
    /// the `SchedulingActionEvent` / `RunningActionEvent` / ... for `progress`.
    ///
    /// Seam: the posted events hold the action itself, which only the action executor owns, so
    /// the executor builds and posts them.
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:116 SpawnRunner.ProgressStatus.postTo
    fn post_progress(&self, action: &dyn ActionExecutionMetadata, progress: &ProgressStatus);

    /// `getActionInputPrefetcher().prefetchFiles(owner, inputs, metadataProvider, MEDIUM,
    /// INPUTS)`, waited for (`SpawnExecutionContext.prefetchInputsAndWait`). The prefetcher is
    /// the [`ActionInputPrefetcher`] registered as a context (`ExecutorBuilder
    /// .setActionInputPrefetcher`, the remote module's `RemoteActionInputFetcher`), else
    /// `ActionInputPrefetcher.NONE` (no remote outputs).
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:170 SpawnRunner.SpawnExecutionContext.prefetchInputsAndWait
    fn prefetch_files(
        &self,
        owner: &dyn ActionExecutionMetadata,
        inputs: &[ActionInputRef],
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<(), ExecException> {
        match get_context::<Arc<dyn ActionInputPrefetcher>>(self) {
            Some(prefetcher) => prefetcher
                .prefetch_files(owner, inputs, input_metadata_provider)
                .map_err(|e| crate::exec::abstract_spawn_strategy::exec_io_exception(&e)),
            // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:28 ActionInputPrefetcher.NONE
            None => Ok(()),
        }
    }

    /// `isRewindingEnabled()`.
    fn is_rewinding_enabled(&self) -> bool {
        false
    }

    /// `getInputPath(ActionInput)`: the path of `input` for this action (the execroot path; no
    /// action file system).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
    fn get_input_path(&self, input: &dyn crate::actions::ActionInput) -> Path {
        self.get_exec_root().get_relative(input.get_exec_path())
    }

    /// `getVerboseFailures()` (`--verbose_failures`).
    fn get_verbose_failures(&self) -> bool {
        false
    }

    /// `setRichArtifactData(RichArtifactData)`: the runfiles tree value a `RunfilesTreeAction`
    /// produces (stored in its `ActionExecutionValue`).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:440 ActionExecutionContext.setRichArtifactData
    fn set_rich_artifact_data(&self, _data: Arc<crate::actions::RunfilesArtifactValue>) {}

    /// `checkForLostInputs()`.
    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        Ok(())
    }

    /// `getOutputMetadataStore().getOutputMetadata(output)` for a file output of this action
    /// (read by `withOutputsAsInputs`, e.g. for the test log given to the test XML generation
    /// spawn). The default constructs the value from the file system, as
    /// `ActionOutputMetadataStore.constructFileArtifactValueFromFilesystem` does for a plain
    /// file (stat + digest).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:570 ActionExecutionContext.withOutputsAsInputs
    fn get_output_metadata(
        &self,
        output: &dyn crate::actions::ActionInput,
    ) -> io::Result<crate::actions::FileArtifactValue> {
        let path = self.get_input_path(output);
        let stat = path.stat_with(blaze_vfs::Symlinks::Follow)?;
        crate::actions::FileArtifactValue::create_from_stat(&path, &stat)
    }
}

/// Prefetches files to local disk, as the execution phase uses it: `prefetchFiles(action,
/// inputs, metadataProvider, Priority.MEDIUM, Reason.INPUTS)`, waited for. Register it as the
/// context `Arc<dyn ActionInputPrefetcher>` (Java: `ExecutorBuilder.setActionInputPrefetcher`);
/// [`ActionExecutionContext::prefetch_files`] calls it.
///
/// PORT: Java's interface returns a future; the asynchronous form lives with its
/// implementation (`blaze_remote::abstract_action_input_prefetcher`). An `InterruptedException`
/// is an `io::Error` of kind `Interrupted`.
// Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:23 ActionInputPrefetcher
pub trait ActionInputPrefetcher: Send + Sync {
    /// `getFromFuture(prefetchFiles(action, inputs, metadataProvider, MEDIUM, INPUTS))`.
    // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:69 ActionInputPrefetcher.prefetchFiles
    fn prefetch_files(
        &self,
        action: &dyn ActionExecutionMetadata,
        inputs: &[ActionInputRef],
        metadata_provider: &dyn InputMetadataProvider,
    ) -> io::Result<()>;
}

/// `OutputService.finalizeAction(action, outputMetadataStore)` of an output service that does
/// something there (`RemoteOutputService`: `actionInputFetcher.finalizeAction`, which downloads
/// the outputs the remote output checker wants local). `SkyframeActionExecutor` calls it after
/// the outputs of an executed action are checked, when `--output_tree_tracking`
/// (`BuildRequestOptions.finalizeActions`) is set; register it as the context
/// `Arc<dyn ActionFinalizer>` only then. `LocalOutputService.finalizeAction` is a no-op (no
/// context).
// Java: com/google/devtools/build/lib/vfs/OutputService.java OutputService.finalizeAction
// Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:170 RemoteOutputService.finalizeAction
pub trait ActionFinalizer: Send + Sync {
    /// `finalizeAction(action, outputMetadataStore)`; `outputs` is `action.getOutputs()`. An
    /// `io::Error` of kind `Interrupted` is an `InterruptedException`.
    fn finalize_action(
        &self,
        action: &dyn crate::actions::ActionExecutionMetadata,
        outputs: &[ActionInputRef],
        output_metadata_store: &dyn crate::actions::cache::output_metadata_store::OutputMetadataStore,
    ) -> io::Result<()>;
}

/// The outputs of this build that exist only remotely (build without the bytes): what
/// `RemoteExecutionService.downloadOutputs` injects instead of writing a file
/// (`RemoteActionFileSystem.injectRemoteFile` into its remote output tree), keyed by absolute
/// path. The output metadata store of an action consults it before the local file system, as
/// `RemoteActionFileSystem.statInternal` stats the remote output tree before the local one, so
/// the action's value (and every downstream input) carries the remote metadata.
///
/// PORT: Java gives each action a `RemoteActionFileSystem` (the action file system of
/// `RemoteOutputService`) whose remote output tree holds the injected outputs of that action.
/// Blaze has no action file system; this one tree serves the whole build (one command per
/// process, and every action executes at most once in it). Register it as the context
/// `RemoteOutputTree`.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:308 RemoteActionFileSystem.injectRemoteFile
#[derive(Debug, Default)]
pub struct RemoteOutputTree {
    files:
        parking_lot::Mutex<std::collections::HashMap<blaze_vfs::PathFragment, FileArtifactValue>>,
    /// Whether an output metadata store took the tree ([`RemoteOutputTree::from_context`]): an
    /// output injected into a tree that no store consults would be reported as not created.
    attached: std::sync::atomic::AtomicBool,
}

impl RemoteOutputTree {
    pub fn new() -> RemoteOutputTree {
        RemoteOutputTree::default()
    }

    /// `remoteOutputTree.injectFile(path, metadata)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:316 RemoteActionFileSystem.injectRemoteFile
    pub fn inject_file(&self, path: blaze_vfs::PathFragment, metadata: FileArtifactValue) {
        self.files.lock().insert(path, metadata);
    }

    /// The injected metadata of the output at the absolute `path`, if any
    /// (`remoteOutputTree.statIfFound(path)` as a `FileStatusWithMetadata`).
    pub fn get_metadata(&self, path: &blaze_vfs::PathFragment) -> Option<FileArtifactValue> {
        self.files.lock().get(path).cloned()
    }

    /// The metadata of the output at `path` as the action file system's stat finds it in the
    /// remote output tree: the injected metadata of `path`, or, when `path` is a local symbolic
    /// link (an output a local action created, such as a `SolibSymlinkAction`'s) whose chain
    /// reaches an injected output, that output's metadata with the real path recorded
    /// (`FileArtifactValue.createFromExistingWithResolvedPath`). `None` when the remote output
    /// tree does not know the file.
    ///
    /// PORT: Java canonicalizes the path through the `RemoteActionFileSystem`
    /// (`resolveSymbolicLinks`, which consults the remote output tree for every component) and
    /// stats the real path there; the links followed here are local ones, each target resolved
    /// against the link's directory.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:667 RemoteActionFileSystem.statInternal
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:478 ActionOutputMetadataStore.fileArtifactValueFromArtifact
    pub fn get_output_metadata(
        &self,
        path: &blaze_vfs::Path,
    ) -> io::Result<Option<FileArtifactValue>> {
        if let Some(value) = self.get_metadata(path.as_fragment()) {
            return Ok(Some(value));
        }
        let mut current = path.clone();
        for _ in 0..blaze_vfs::MAX_SYMLINKS {
            let Some(stat) = current.stat_if_found(blaze_vfs::Symlinks::Nofollow)? else {
                return Ok(None);
            };
            if !stat.is_symbolic_link() {
                return Ok(None);
            }
            let target = current.read_symbolic_link()?;
            current = match current.get_parent_directory() {
                Some(parent) => parent.get_relative(&target),
                None => current.get_relative(&target),
            };
            if let Some(value) = self.get_metadata(current.as_fragment()) {
                return Ok(Some(
                    FileArtifactValue::create_from_existing_with_resolved_path(
                        value,
                        current.as_fragment().clone(),
                    ),
                ));
            }
        }
        Ok(None)
    }

    /// The injected files below the directory `dir` (absolute), with their paths relative to
    /// `dir`, sorted by path: the remote part of a directory listing of the action file system
    /// (`RemoteActionFileSystem.readdir` merges the remote output tree's entries with the local
    /// ones), as `TreeArtifactValue.visitTree` sees it for a tree artifact whose files were not
    /// downloaded.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:831 RemoteActionFileSystem.readdir
    pub fn get_files_under(
        &self,
        dir: &blaze_vfs::PathFragment,
    ) -> Vec<(blaze_vfs::PathFragment, FileArtifactValue)> {
        let mut files: Vec<(blaze_vfs::PathFragment, FileArtifactValue)> = self
            .files
            .lock()
            .iter()
            .filter(|(path, _)| path != &dir && path.starts_with(dir))
            .map(|(path, metadata)| (path.relative_to(dir), metadata.clone()))
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    /// Forgets the output at `path` (it was deleted or written locally).
    pub fn remove(&self, path: &blaze_vfs::PathFragment) {
        self.files.lock().remove(path);
    }

    /// The tree registered as the context `RemoteOutputTree` (`context` is the registry's
    /// entry for that type), if any.
    pub fn from_context(context: Option<ActionContextRef>) -> Option<Arc<RemoteOutputTree>> {
        context.and_then(|c| c.downcast::<RemoteOutputTree>().ok())
    }

    /// [`RemoteOutputTree::from_context`] for an output metadata store, which consults the tree
    /// from then on (see [`RemoteOutputTree::is_attached`]).
    pub fn for_output_metadata_store(
        context: Option<ActionContextRef>,
    ) -> Option<Arc<RemoteOutputTree>> {
        let tree = RemoteOutputTree::from_context(context)?;
        tree.attached
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Some(tree)
    }

    /// Whether an output metadata store consults this tree.
    pub fn is_attached(&self) -> bool {
        self.attached.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The local file system seen through the build's [`RemoteOutputTree`]: an output that was not
/// downloaded is a regular file with the size and digest of its remote metadata, as in the
/// `RemoteActionFileSystem` Java gives an action (`stat`, `getDigest`, `exists`, ...); every
/// other operation is the local file system's.
///
/// PORT: Java's spawn log context reads the outputs of a spawn through
/// `actionExecutionContext.getActionFileSystem()`; Blaze has no action file system, so
/// `AbstractSpawnStrategy` gives the log context this view when a remote output tree is
/// registered. Only the read side of the action file system is ported.
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:106 RemoteActionFileSystem
#[derive(Debug)]
pub struct RemoteOutputTreeFileSystem {
    local: FileSystemRef,
    tree: Arc<RemoteOutputTree>,
}

impl RemoteOutputTreeFileSystem {
    /// The view of `local` through `tree`.
    pub fn create(local: FileSystemRef, tree: Arc<RemoteOutputTree>) -> FileSystemRef {
        Arc::new(RemoteOutputTreeFileSystem { local, tree })
    }

    /// The remote metadata of `path` (following symbolic links when `follow_symlinks`).
    fn remote_metadata(
        &self,
        path: &blaze_vfs::PathFragment,
        follow_symlinks: bool,
    ) -> Option<FileArtifactValue> {
        if !follow_symlinks {
            return self.tree.get_metadata(path);
        }
        let local_path = Path::create(path.clone(), &self.local);
        self.tree.get_output_metadata(&local_path).ok().flatten()
    }

    /// `statFromMetadata(metadata)`: a regular file of the metadata's size.
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:709 RemoteActionFileSystem.statFromMetadata
    fn stat_from_metadata(metadata: &FileArtifactValue) -> blaze_vfs::FileStatus {
        // S_IFREG | r-xr-xr-x
        blaze_vfs::FileStatus::new(0o100_555, 0, 0, metadata.get_size(), 0)
    }
}

impl blaze_vfs::FileSystem for RemoteOutputTreeFileSystem {
    fn get_digest_function(&self) -> blaze_vfs::DigestHashFunction {
        self.local.get_digest_function()
    }
    fn supports_modifications(&self, path: &blaze_vfs::PathFragment) -> bool {
        self.local.supports_modifications(path)
    }
    fn supports_symbolic_links_natively(&self, path: &blaze_vfs::PathFragment) -> bool {
        self.local.supports_symbolic_links_natively(path)
    }
    fn supports_hard_links_natively(&self, path: &blaze_vfs::PathFragment) -> bool {
        self.local.supports_hard_links_natively(path)
    }
    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.local.may_be_case_or_normalization_insensitive()
    }
    fn create_directory(&self, path: &blaze_vfs::PathFragment) -> io::Result<bool> {
        self.local.create_directory(path)
    }
    fn create_directory_and_parents(&self, path: &blaze_vfs::PathFragment) -> io::Result<()> {
        self.local.create_directory_and_parents(path)
    }
    fn get_file_size(
        &self,
        path: &blaze_vfs::PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        match self.remote_metadata(path, follow_symlinks) {
            Some(metadata) => Ok(metadata.get_size()),
            None => self.local.get_file_size(path, follow_symlinks),
        }
    }
    fn delete(&self, path: &blaze_vfs::PathFragment) -> io::Result<bool> {
        self.local.delete(path)
    }
    fn get_last_modified_time(
        &self,
        path: &blaze_vfs::PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        match self.remote_metadata(path, follow_symlinks) {
            Some(_) => Ok(0),
            None => self.local.get_last_modified_time(path, follow_symlinks),
        }
    }
    fn set_last_modified_time(
        &self,
        path: &blaze_vfs::PathFragment,
        new_time: i64,
    ) -> io::Result<()> {
        self.local.set_last_modified_time(path, new_time)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:463 RemoteActionFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &blaze_vfs::PathFragment) -> io::Result<Option<Vec<u8>>> {
        match self.remote_metadata(path, true) {
            Some(metadata) => Ok(metadata.get_digest().map(<[u8]>::to_vec)),
            None => self.local.get_fast_digest(path),
        }
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:476 RemoteActionFileSystem.getDigest
    fn get_digest(&self, path: &blaze_vfs::PathFragment) -> io::Result<Vec<u8>> {
        match self.remote_metadata(path, true) {
            Some(metadata) if metadata.get_digest().is_some() => {
                Ok(metadata.get_digest().map(<[u8]>::to_vec).expect("checked"))
            }
            _ => self.local.get_digest(path),
        }
    }
    fn resolve_one_link(
        &self,
        path: &blaze_vfs::PathFragment,
    ) -> io::Result<Option<blaze_vfs::PathFragment>> {
        self.local.resolve_one_link(path)
    }
    fn resolve_symbolic_links(
        &self,
        path: &blaze_vfs::PathFragment,
    ) -> io::Result<blaze_vfs::PathFragment> {
        self.local.resolve_symbolic_links(path)
    }
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:631 RemoteActionFileSystem.stat
    fn stat(
        &self,
        path: &blaze_vfs::PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<blaze_vfs::FileStatus> {
        match self.remote_metadata(path, follow_symlinks) {
            Some(metadata) => Ok(Self::stat_from_metadata(&metadata)),
            None => self.local.stat(path, follow_symlinks),
        }
    }
    fn stat_if_found(
        &self,
        path: &blaze_vfs::PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<blaze_vfs::FileStatus>> {
        match self.remote_metadata(path, follow_symlinks) {
            Some(metadata) => Ok(Some(Self::stat_from_metadata(&metadata))),
            None => self.local.stat_if_found(path, follow_symlinks),
        }
    }
    fn create_symbolic_link(
        &self,
        link_path: &blaze_vfs::PathFragment,
        target_fragment: &blaze_vfs::PathFragment,
    ) -> io::Result<()> {
        self.local.create_symbolic_link(link_path, target_fragment)
    }
    fn read_symbolic_link(
        &self,
        path: &blaze_vfs::PathFragment,
    ) -> io::Result<blaze_vfs::PathFragment> {
        self.local.read_symbolic_link(path)
    }
    fn exists(&self, path: &blaze_vfs::PathFragment, follow_symlinks: bool) -> bool {
        self.remote_metadata(path, follow_symlinks).is_some()
            || self.local.exists(path, follow_symlinks)
    }
    fn get_directory_entries(&self, path: &blaze_vfs::PathFragment) -> io::Result<Vec<String>> {
        self.local.get_directory_entries(path)
    }
    fn is_readable(&self, path: &blaze_vfs::PathFragment) -> io::Result<bool> {
        self.local.is_readable(path)
    }
    fn set_readable(&self, path: &blaze_vfs::PathFragment, readable: bool) -> io::Result<()> {
        self.local.set_readable(path, readable)
    }
    fn is_writable(&self, path: &blaze_vfs::PathFragment) -> io::Result<bool> {
        self.local.is_writable(path)
    }
    fn set_writable(&self, path: &blaze_vfs::PathFragment, writable: bool) -> io::Result<()> {
        self.local.set_writable(path, writable)
    }
    fn is_executable(&self, path: &blaze_vfs::PathFragment) -> io::Result<bool> {
        self.local.is_executable(path)
    }
    fn set_executable(&self, path: &blaze_vfs::PathFragment, executable: bool) -> io::Result<()> {
        self.local.set_executable(path, executable)
    }
    fn chmod(&self, path: &blaze_vfs::PathFragment, mode: u32) -> io::Result<()> {
        self.local.chmod(path, mode)
    }
    fn get_input_stream(
        &self,
        path: &blaze_vfs::PathFragment,
    ) -> io::Result<Box<dyn std::io::Read + Send>> {
        self.local.get_input_stream(path)
    }
    fn get_output_stream(
        &self,
        path: &blaze_vfs::PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn std::io::Write + Send>> {
        self.local.get_output_stream(path, append, internal)
    }
    fn rename_to(
        &self,
        source_path: &blaze_vfs::PathFragment,
        target_path: &blaze_vfs::PathFragment,
    ) -> io::Result<()> {
        self.local.rename_to(source_path, target_path)
    }
    fn create_fs_dependent_hard_link(
        &self,
        link_path: &blaze_vfs::PathFragment,
        original_path: &blaze_vfs::PathFragment,
    ) -> io::Result<()> {
        self.local
            .create_fs_dependent_hard_link(link_path, original_path)
    }
}

/// An error of `SpawnCache.lookup` / `CacheHandle.store` (`ExecException`, `IOException` or
/// `InterruptedException`).
#[derive(Debug)]
pub enum SpawnCacheError {
    /// `ExecException` (and `InterruptedException` as [`ExecException::Interrupted`]).
    Exec(ExecException),
    /// `IOException`.
    Io(io::Error),
}

impl From<ExecException> for SpawnCacheError {
    fn from(e: ExecException) -> Self {
        SpawnCacheError::Exec(e)
    }
}

impl From<io::Error> for SpawnCacheError {
    fn from(e: io::Error) -> Self {
        SpawnCacheError::Io(e)
    }
}

/// `SpawnCache.CacheHandle`: the result of a cache lookup.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:138 SpawnCache.CacheHandle
pub trait CacheHandle: Send {
    /// `hasResult()`.
    fn has_result(&self) -> bool;
    /// `getResult()`.
    fn get_result(&self) -> SpawnResult;
    /// `willStore()`.
    fn will_store(&self) -> bool;
    /// `store(SpawnResult)`.
    fn store(&self, result: &SpawnResult) -> Result<(), SpawnCacheError>;
    /// `close()` (`SilentCloseable`).
    fn close(&self) {}
}

/// A cache that can lookup a SpawnResult given a Spawn, and can also upload the results of an
/// executed spawn to the cache.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:33 SpawnCache
pub trait SpawnCache: Send + Sync {
    /// `lookup(spawn, context)`. The handle may borrow the spawn and its context (Java's
    /// handles capture them); it is used and dropped while both are alive.
    fn lookup<'a>(
        &self,
        spawn: &'a dyn Spawn,
        context: &'a dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle + 'a>, SpawnCacheError>;

    /// `usefulInDynamicExecution()`.
    fn useful_in_dynamic_execution(&self) -> bool {
        true
    }
}

/// `SpawnCache.NO_RESULT_NO_STORE`.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:36 SpawnCache.NO_RESULT_NO_STORE
#[derive(Debug, Default)]
pub struct NoResultNoStore;

impl CacheHandle for NoResultNoStore {
    fn has_result(&self) -> bool {
        false
    }
    fn get_result(&self) -> SpawnResult {
        panic!("NoSuchElementException")
    }
    fn will_store(&self) -> bool {
        false
    }
    fn store(&self, _result: &SpawnResult) -> Result<(), SpawnCacheError> {
        // Do nothing.
        Ok(())
    }
}

/// `SpawnCache.success(result)`: a handle with a result that must not be stored.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:64 SpawnCache.success
pub fn success(result: SpawnResult) -> Box<dyn CacheHandle> {
    struct Success(SpawnResult);
    impl CacheHandle for Success {
        fn has_result(&self) -> bool {
            true
        }
        fn get_result(&self) -> SpawnResult {
            self.0.clone()
        }
        fn will_store(&self) -> bool {
            false
        }
        fn store(&self, _result: &SpawnResult) -> Result<(), SpawnCacheError> {
            panic!("IllegalStateException")
        }
    }
    Box::new(Success(result))
}

/// `SpawnCache.NoSpawnCache`.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:111 SpawnCache.NoSpawnCache
#[derive(Debug)]
pub struct NoSpawnCache {
    _private: (),
}

impl SpawnCache for NoSpawnCache {
    fn lookup<'a>(
        &self,
        _spawn: &'a dyn Spawn,
        _context: &'a dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle + 'a>, SpawnCacheError> {
        Ok(Box::new(NoResultNoStore))
    }
}

/// `SpawnCache.NO_CACHE`.
// Java: com/google/devtools/build/lib/exec/SpawnCache.java:131 SpawnCache.NO_CACHE
pub fn no_cache() -> Arc<dyn SpawnCache> {
    static NO_CACHE: std::sync::OnceLock<Arc<dyn SpawnCache>> = std::sync::OnceLock::new();
    NO_CACHE
        .get_or_init(|| Arc::new(NoSpawnCache { _private: () }))
        .clone()
}

/// Whether `cache` is `SpawnCache.NO_CACHE` (Java `==`).
pub fn is_no_cache(cache: &Arc<dyn SpawnCache>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(cache), Arc::as_ptr(&no_cache()))
}

/// This event is fired during the build, when a subprocess is executed.
///
/// PORT: Java keeps references to the spawn, its input metadata provider, action file system and
/// `FileOutErr`; a posted value must own its data here, so the event keeps the parts of the spawn
/// and its owning action that consumers read (mnemonic, owner, primary output, progress message).
// Java: com/google/devtools/build/lib/actions/SpawnExecutedEvent.java:28 SpawnExecutedEvent
#[derive(Debug, Clone)]
pub struct SpawnExecutedEvent {
    spawn_mnemonic: String,
    action_owner: ActionOwner,
    action_mnemonic: String,
    action_progress_message: Option<String>,
    action_primary_output: ActionInputRef,
    result: SpawnResult,
    start_time_instant: SystemTime,
    spawn_identifier: Option<String>,
}

impl SpawnExecutedEvent {
    /// `new SpawnExecutedEvent(spawn, inputMetadataProvider, actionFileSystem, fileOutErr,
    /// result, startTimeInstant, spawnIdentifier)`.
    // Java: com/google/devtools/build/lib/actions/SpawnExecutedEvent.java:37 SpawnExecutedEvent.<init>
    pub fn new(
        spawn: &dyn Spawn,
        result: SpawnResult,
        start_time_instant: SystemTime,
        spawn_identifier: Option<String>,
    ) -> SpawnExecutedEvent {
        let owner = spawn.get_resource_owner();
        SpawnExecutedEvent {
            spawn_mnemonic: spawn.get_mnemonic(),
            action_owner: owner.get_owner().clone(),
            action_mnemonic: owner.get_mnemonic(),
            action_progress_message: owner.get_progress_message(),
            action_primary_output: owner.get_primary_output(),
            result,
            start_time_instant,
            spawn_identifier,
        }
    }

    /// `getSpawn().getMnemonic()`.
    pub fn get_spawn_mnemonic(&self) -> &str {
        &self.spawn_mnemonic
    }
    /// `getActionMetadata().getOwner()`.
    pub fn get_action_owner(&self) -> &ActionOwner {
        &self.action_owner
    }
    /// `getActionMetadata().getMnemonic()`.
    pub fn get_action_mnemonic(&self) -> &str {
        &self.action_mnemonic
    }
    /// `getActionMetadata().getProgressMessage()`.
    pub fn get_action_progress_message(&self) -> Option<&str> {
        self.action_progress_message.as_deref()
    }
    /// `getActionMetadata().getPrimaryOutput()`.
    pub fn get_action_primary_output(&self) -> &ActionInputRef {
        &self.action_primary_output
    }
    /// `getExitCode()`.
    pub fn get_exit_code(&self) -> i32 {
        self.result.exit_code()
    }
    /// `getSpawnResult()`.
    pub fn get_spawn_result(&self) -> &SpawnResult {
        &self.result
    }
    /// `getStartTimeInstant()`.
    pub fn get_start_time_instant(&self) -> SystemTime {
        self.start_time_instant
    }
    /// `getSpawnIdentifier()`.
    pub fn get_spawn_identifier(&self) -> Option<&str> {
        self.spawn_identifier.as_deref()
    }
}

impl Postable for SpawnExecutedEvent {}
