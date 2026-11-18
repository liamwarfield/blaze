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

use crate::actions::{
    ActionExecutionMetadata, ActionInputRef, ActionOwner, ExecException, InputMetadataProvider,
    Spawn, SpawnResult,
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
    /// INPUTS)`, waited for. The default is `ActionInputPrefetcher.NONE` (no remote outputs).
    // Java: com/google/devtools/build/lib/actions/ActionInputPrefetcher.java:28 ActionInputPrefetcher.NONE
    fn prefetch_files(
        &self,
        _owner: &dyn ActionExecutionMetadata,
        _inputs: &[ActionInputRef],
        _input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<(), ExecException> {
        Ok(())
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
        crate::actions::FileArtifactValue::create_for_testing(&self.get_input_path(output))
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
    /// `lookup(spawn, context)`.
    fn lookup(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle>, SpawnCacheError>;

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
    fn lookup(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<Box<dyn CacheHandle>, SpawnCacheError> {
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
