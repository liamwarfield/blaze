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
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:376 ActionExecutionContext.getActionFileSystem
    fn get_action_file_system(&self) -> Option<FileSystemRef> {
        self.get_remote_action_file_system()
            .map(|fs| fs.as_file_system())
    }

    /// `getActionFileSystem()` when it is the `RemoteActionFileSystem` of build without the
    /// bytes (the only action file system Blaze has): `RemoteOutputService.createActionFileSystem`.
    /// `None` without the remote output service.
    fn get_remote_action_file_system(
        &self,
    ) -> Option<Arc<crate::exec::remote_action_file_system::RemoteActionFileSystem>> {
        None
    }

    /// `getPathResolver().convertPath(path)`: `path` on the action file system
    /// (`TransformResolver`), or `path` itself (`IdentityResolver`).
    // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:124 TransformResolver.convertPath
    // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:86 IdentityResolver.convertPath
    fn convert_path(&self, path: &Path) -> Path {
        match self.get_action_file_system() {
            Some(file_system) => Path::create(path.as_fragment().clone(), &file_system),
            None => path.clone(),
        }
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
            Some(prefetcher) => {
                match prefetcher.prefetch_files(owner, inputs, input_metadata_provider) {
                    Ok(()) => Ok(()),
                    // Futures.catchingAsync(..., BulkTransferException.class, ...)
                    // Java: com/google/devtools/build/lib/exec/AbstractSpawnStrategy.java:285 SpawnExecutionContextImpl.prefetchInputs
                    Err(e) => match e
                        .get_ref()
                        .and_then(|inner| inner.downcast_ref::<BulkTransferError>())
                    {
                        Some(bulk) => {
                            let input_metadata_provider = self.get_input_metadata_provider();
                            let lost = bulk.get_lost_artifacts(&|exec_path| {
                                input_metadata_provider.get_input(exec_path)
                            });
                            lost.throw_if_not_empty()?;
                            Err(crate::actions::EnvironmentalExecException::with_cause(
                                &bulk.message,
                                blaze_protos::failure_details::FailureDetail {
                                    message: "Failed to fetch blobs because of a remote cache error."
                                        .to_string(),
                                    category: Some(
                                        blaze_protos::failure_details::failure_detail::Category::Spawn(
                                            blaze_protos::failure_details::Spawn {
                                                code: blaze_protos::failure_details::spawn::Code::RemoteCacheEvicted
                                                    as i32,
                                                ..Default::default()
                                            },
                                        ),
                                    ),
                                },
                            )
                            .into())
                        }
                        None => Err(crate::exec::abstract_spawn_strategy::exec_io_exception(&e)),
                    },
                }
            }
            // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:28 ActionInputPrefetcher.NONE
            None => Ok(()),
        }
    }

    /// `isRewindingEnabled()`.
    fn is_rewinding_enabled(&self) -> bool {
        false
    }

    /// `getInputPath(ActionInput)`: the path of `input` for this action (the execroot path, on
    /// the action file system when there is one).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
    fn get_input_path(&self, input: &dyn crate::actions::ActionInput) -> Path {
        self.convert_path(&self.get_exec_root().get_relative(input.get_exec_path()))
    }

    /// `getVerboseFailures()` (`--verbose_failures`).
    fn get_verbose_failures(&self) -> bool {
        false
    }

    /// `setRichArtifactData(RichArtifactData)`: the runfiles tree value a `RunfilesTreeAction`
    /// produces (stored in its `ActionExecutionValue`).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:440 ActionExecutionContext.setRichArtifactData
    fn set_rich_artifact_data(&self, _data: Arc<crate::actions::RunfilesArtifactValue>) {}

    /// `checkForLostInputs()`: `lostInputsCheck.checkForLostInputs()`, the check the action
    /// executor gives the context (`SkyframeActionExecutor.lostInputsCheck`: the action file
    /// system's record of lost inputs, or `LostInputsCheck.NONE` without one, the default).
    ///
    /// PORT: Java throws `LostInputsActionExecutionException`; the only caller,
    /// `SpawnExecutionContextImpl.checkForLostInputs`, converts it with `toExecException()`, so
    /// the context returns that form (a [`LostInputsExecException`]) directly.
    ///
    /// [`LostInputsExecException`]: crate::actions::LostInputsExecException
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:388 ActionExecutionContext.checkForLostInputs
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:623 ActionExecutionContext.LostInputsCheck.NONE
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

    /// `getOutputMetadataStore().getTreeArtifactValue(treeArtifact)` for a tree artifact output
    /// of this action (`StandaloneTestStrategy`'s split coverage post-processing reads the
    /// children of the coverage directory).
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:72 OutputMetadataStore.getTreeArtifactValue
    fn get_tree_artifact_value(
        &self,
        _tree_artifact: &ActionInputRef,
    ) -> io::Result<crate::actions::TreeArtifactValue> {
        blaze_util::unported!(
            "ActionExecutionContext.getOutputMetadataStore",
            "this execution context has no output metadata store"
        )
    }

    /// `getOutputMetadataStore().resetOutputs(outputs)`: forgets the metadata recorded for
    /// `outputs` (an action that re-runs its spawn, e.g. `JavaCompileAction`'s reduced-classpath
    /// fallback).
    // Java: com/google/devtools/build/lib/actions/cache/OutputMetadataStore.java:92 OutputMetadataStore.resetOutputs
    fn reset_outputs(&self, _outputs: &[crate::actions::ActionInputRef]) {
        blaze_util::unported!(
            "ActionExecutionContext.getOutputMetadataStore",
            "this execution context has no output metadata store"
        )
    }
}

/// A `BulkTransferException` of the [`ActionInputPrefetcher`], as the execution layer sees it
/// (the prefetcher returns it inside an `io::Error`): its message, and for each suppressed
/// `CacheNotFoundException` the missing digest (`hash/size`) and the exec path of the file
/// whose blob is missing. `causes` is `None` unless every suppressed exception is a
/// `CacheNotFoundException` (`allCausedByCacheNotFoundException()`); an entry's path is `None`
/// for a special output such as stdout/stderr (a `CacheNotFoundException` with a file name but
/// no exec path).
///
/// PORT: `BulkTransferException` lives in `lib/remote/common` (blaze_remote); this is the part
/// `AbstractSpawnStrategy.prefetchInputs` reads.
// Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:36 BulkTransferException
#[derive(Debug)]
pub struct BulkTransferError {
    /// `getMessage()`.
    pub message: String,
    /// The missing digests and exec paths, see the type's doc.
    pub causes: Option<Vec<(String, Option<blaze_vfs::PathFragment>)>>,
}

impl std::fmt::Display for BulkTransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BulkTransferError {}

impl BulkTransferError {
    /// `getLostArtifacts(actionInputResolver)`: non-empty if and only if all suppressed
    /// exceptions are cache misses of files the resolver knows.
    // Java: com/google/devtools/build/lib/remote/common/BulkTransferException.java:83 BulkTransferException.getLostArtifacts
    pub fn get_lost_artifacts(
        &self,
        action_input_resolver: &dyn Fn(&blaze_vfs::PathFragment) -> Option<ActionInputRef>,
    ) -> crate::actions::LostArtifacts {
        let Some(causes) = &self.causes else {
            return crate::actions::LostArtifacts::default();
        };
        let mut lost = crate::actions::LostArtifacts::default();
        for (missing_digest, exec_path) in causes {
            let Some(exec_path) = exec_path else {
                // This can happen if the lost artifact is not an input of the action, but a
                // special output such as stdout/stderr. This can't be solved by the rewinding
                // that LostArtifacts would trigger, but is rather a failure of the current
                // action execution.
                return crate::actions::LostArtifacts::default();
            };
            let Some(action_input) = action_input_resolver(exec_path) else {
                // This can happen if the lost artifact is not known to the resolver, e.g. an
                // output that failed to be retrieved from the remote cache after a cache hit
                // while resolving inputs. Rewinding can't recover an artifact it can't identify.
                return crate::actions::LostArtifacts::default();
            };
            lost.by_digest
                .entry(missing_digest.clone())
                .or_default()
                .insert(action_input);
        }
        lost
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
    /// `finalizeAction(action, outputMetadataStore) throws IOException, LostInputsExecException,
    /// InterruptedException`; `outputs` is `action.getOutputs()`. A [`SpawnCacheError::Exec`] is
    /// the `LostInputsExecException` (outputs the remote cache lost before they were
    /// downloaded); an `io::Error` of kind `Interrupted` is an `InterruptedException`.
    fn finalize_action(
        &self,
        action: &dyn crate::actions::ActionExecutionMetadata,
        outputs: &[ActionInputRef],
        output_metadata_store: &dyn crate::actions::cache::output_metadata_store::OutputMetadataStore,
    ) -> Result<(), SpawnCacheError>;
}

/// The remote output tree of a [`RemoteActionFileSystem`]: the outputs of one action that exist
/// only remotely (build without the bytes), what `RemoteExecutionService.downloadOutputs`
/// injects instead of writing a file (`RemoteActionFileSystem.injectRemoteFile`), keyed by
/// absolute path. The action's output metadata store stats its outputs through the action file
/// system, which consults this tree before the local file system.
///
/// One tree per action (Java's `RemoteInMemoryFileSystem` of the action's
/// `RemoteActionFileSystem`), so an action result can only describe that action's outputs.
/// A tree registered as the context `RemoteOutputTree` marks a build whose output service is
/// the remote one (the executor then gives every action a [`RemoteActionFileSystem`]); it is
/// also the injector of a spawn context without an action file system (tests, replay).
///
/// [`RemoteActionFileSystem`]: crate::exec::remote_action_file_system::RemoteActionFileSystem
// Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:927 RemoteActionFileSystem.RemoteInMemoryFileSystem
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
    // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:943 RemoteInMemoryFileSystem.injectFile
    pub fn inject_file(&self, path: blaze_vfs::PathFragment, metadata: FileArtifactValue) {
        blaze_util::check_argument!(
            metadata.is_remote(),
            "metadata is not remote: {:?}",
            metadata
        );
        self.files.lock().insert(path, metadata);
    }

    /// The injected metadata of the output at the absolute `path`, if any
    /// (`remoteOutputTree.statIfFound(path, NOFOLLOW)` as a `FileStatusWithMetadata`).
    pub fn get_metadata(&self, path: &blaze_vfs::PathFragment) -> Option<FileArtifactValue> {
        self.files.lock().get(path).cloned()
    }

    /// Whether `path` is a directory of the tree: an ancestor of an injected file
    /// (`injectFile` creates the parent directories).
    pub fn is_directory(&self, path: &blaze_vfs::PathFragment) -> bool {
        self.files
            .lock()
            .keys()
            .any(|p| p != path && p.starts_with(path))
    }

    /// `remoteOutputTree.getPath(path).readdir(NOFOLLOW)`: `None` when `path` is not a
    /// directory of the tree.
    pub fn readdir(&self, path: &blaze_vfs::PathFragment) -> Option<Vec<blaze_vfs::Dirent>> {
        let files = self.files.lock();
        let mut entries: std::collections::BTreeMap<String, blaze_vfs::DirentType> =
            std::collections::BTreeMap::new();
        for file in files.keys() {
            if file == path || !file.starts_with(path) {
                continue;
            }
            let relative = file.relative_to(path);
            let mut segments = relative.segments();
            let Some(first) = segments.next() else {
                continue;
            };
            let type_ = if segments.next().is_some() {
                blaze_vfs::DirentType::Directory
            } else {
                blaze_vfs::DirentType::File
            };
            entries.insert(first.to_string(), type_);
        }
        if entries.is_empty() {
            return None;
        }
        Some(
            entries
                .into_iter()
                .map(|(name, type_)| blaze_vfs::Dirent::new(name, type_))
                .collect(),
        )
    }

    /// `remoteOutputTree.renameTo(src, dst)` for a file; false (Java's `FileNotFoundException`)
    /// when the tree has no file at `src`.
    pub fn rename(&self, src: &blaze_vfs::PathFragment, dst: &blaze_vfs::PathFragment) -> bool {
        let mut files = self.files.lock();
        match files.remove(src) {
            Some(metadata) => {
                files.insert(dst.clone(), metadata);
                true
            }
            None => false,
        }
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

    /// Forgets the output at `path` (`remoteOutputTree.getPath(path).delete()`); true iff it
    /// was there.
    pub fn remove(&self, path: &blaze_vfs::PathFragment) -> bool {
        self.files.lock().remove(path).is_some()
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
