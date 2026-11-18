//! `SkyframeActionExecutor`: action execution shared across `ActionExecutionFunction` calls
//! (contexts, the action cache check and update, input discovery, output preparation,
//! `Action.execute`, output checks and permissions, error reporting, shared actions), plus
//! `ActionOutputMetadataStore` and the `ActionInputMap` of an action.

use std::any::TypeId;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_actions::actions::artifact_factory::ArtifactFactory;
use blaze_actions::actions::runfiles_tree_action::RunfilesTreeAction;
use blaze_actions::analysis::actions::spawn_action::{SpawnAction, SpawnActionClass};
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_actions::analysis::actions::symlink_tree_action::SymlinkTreeAction;
use blaze_actions::{ActionKeyContext, ActionLookupData, Artifact};
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_events::{
    Clock, Event, EventHandler, EventKind, ExtendedEventHandler, JavaClock, OutputFilter, Postable,
};
use blaze_exec::actions::action_adapters::{artifact_input, as_blaze_artifact};
use blaze_exec::actions::action_cache_checker::{
    ActionCacheAwareAction, ActionCacheChecker, ActionCacheCheckerCacheConfig, ArtifactResolver,
    InterruptedException, NoProxies, PackageRootExceptionOrInterrupted, PackageRootResolver, Token,
    TrustLocalOnly,
};
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper;
use blaze_exec::actions::cache::{IoOrInterruptedException, OutputMetadataStore};
use blaze_exec::actions::exec_exception::{EnvironmentalExecException, ExecException};
use blaze_exec::actions::{
    ActionExecutionException, ActionExecutionMetadata, ActionInput, ActionInputRef, ActionResult,
    CacheCheckAction, FileArtifactValue, FilesetOutputTree, InputMetadataProvider, OutputChecker,
    RunfilesArtifactValue, Spawn, TreeArtifactValue, as_executable,
};
use blaze_exec::analysis::actions::runfiles_tree_action::create_runfiles_artifact_value;
use blaze_exec::analysis::actions::to_action_input_set;
use blaze_exec::analysis::test::test_action_context::TestActionContext;
use blaze_exec::analysis::test::test_runner_action::TestRunnerAction;
use blaze_exec::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, SpawnCache,
};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::action_execution_context::ShowSubcommands;
use blaze_protos::failure_details::{FailureDetail, execution};
use blaze_skyframe::Environment;
use blaze_util::{Fingerprint, check_state};
use blaze_vfs::{Path, PathFragment, Root, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, RwLock};

use crate::action_execution_value::ActionExecutionValue;
use crate::artifact_function::execution_failure;
use crate::events::{
    ActionCompletionEvent, ActionExecutedEvent, ActionResultReceivedEvent, ActionStartedEvent,
    ErrorTiming,
};

/// `OutputPermissions`: the mode output files are set to after execution.
// Java: com/google/devtools/build/lib/vfs/OutputPermissions.java:22 OutputPermissions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum OutputPermissions {
    /// `--experimental_writable_outputs=false` (default): 0555.
    READONLY,
    /// 0755.
    WRITABLE,
}

impl OutputPermissions {
    // Java: com/google/devtools/build/lib/vfs/OutputPermissions.java:30 OutputPermissions.getPermissionsMode
    pub fn get_permissions_mode(self) -> u32 {
        match self {
            OutputPermissions::READONLY => 0o555,
            OutputPermissions::WRITABLE => 0o755,
        }
    }

    /// The same value as the action cache's `OutputPermissions` (one Java enum).
    fn for_action_cache(self) -> blaze_exec::actions::cache::OutputPermissions {
        match self {
            OutputPermissions::READONLY => blaze_exec::actions::cache::OutputPermissions::Readonly,
            OutputPermissions::WRITABLE => blaze_exec::actions::cache::OutputPermissions::Writable,
        }
    }
}

/// The action contexts registered for this build (`Executor.getContext`), plus the spawn
/// strategy registry, spawn log context and spawn cache.
// Java: com/google/devtools/build/lib/exec/BlazeExecutor.java:56 BlazeExecutor
#[derive(Default)]
pub struct ActionContexts {
    pub spawn_strategy_registry: Option<Arc<SpawnStrategyRegistry>>,
    pub spawn_log_context: Option<Arc<dyn SpawnLogContext>>,
    pub spawn_cache: Option<Arc<dyn SpawnCache>>,
    contexts: HashMap<TypeId, ActionContextRef>,
}

impl ActionContexts {
    /// Registers `context` as the implementation of `T` (Java `ActionContext.ActionContextRegistry`
    /// `register(T.class, context)`). For a trait-object context, `T = Arc<dyn Trait>`.
    pub fn register<T: Send + Sync + 'static>(&mut self, context: Arc<T>) {
        self.contexts
            .insert(TypeId::of::<T>(), context as ActionContextRef);
    }

    /// Registers a context under an explicit `TypeId` (for crates that hand out
    /// `(TypeId, ActionContextRef)` pairs).
    pub fn register_raw(&mut self, type_id: TypeId, context: ActionContextRef) {
        self.contexts.insert(type_id, context);
    }

    pub fn get_any(&self, type_id: TypeId) -> Option<ActionContextRef> {
        self.contexts.get(&type_id).cloned()
    }
}

/// Configuration of a [`SkyframeActionExecutor`] for one build.
pub struct SkyframeActionExecutorConfig {
    pub exec_root: Path,
    /// `<output_base>/action_outs`, where action stdout/stderr buffers live
    /// (`ActionLogBufferPathGenerator`).
    pub action_out_dir: Path,
    pub reporter: Arc<dyn ExtendedEventHandler>,
    pub contexts: ActionContexts,
    pub client_env: IndexMap<String, String>,
    pub keep_going: bool,
    pub verbose_failures: bool,
    pub output_permissions: OutputPermissions,
}

/// The `PackageRootResolverWithEnvironment` lookups (`CONTAINING_PACKAGE_LOOKUP` for the package
/// of each exec path, under the `--experimental_sibling_repository_layout` of the
/// `STARLARK_SEMANTICS`) that `ActionExecutionFunction` needs to resolve the source inputs an
/// input-discovering action recorded in the action cache.
///
/// Seam: `ContainingPackageLookupValue` is computed in `blaze_sky_loading`, which this crate does
/// not depend on. The implementation ports the body of
/// `ActionExecutionFunction.PackageRootResolverWithEnvironment.findPackageRootsForFiles`
/// (ActionExecutionFunction.java:590-647) after its `checkState`.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:578 ActionExecutionFunction.PackageRootResolverWithEnvironment
pub trait ContainingPackageRootLookup: Send + Sync {
    #[allow(clippy::type_complexity)]
    /// Returns mapping from exec path to the root of its containing package (`None`: no
    /// containing package), or `Ok(None)` if Skyframe values are missing.
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
        env: &mut Environment<'_>,
    ) -> Result<Option<HashMap<PathFragment, Option<Root>>>, PackageRootExceptionOrInterrupted>;
}

/// How the action cache is used by one build: Java's `ActionCacheChecker` (built by
/// `ExecutionTool.createBuilder`) and the salt and flags `SkyframeActionExecutor` reads.
pub struct ActionCacheSetup {
    pub action_cache_checker: Arc<ActionCacheChecker>,
    /// `ExecutorBuilder.getActionExecutionSalt()` (`""` unless a module sets one).
    pub action_execution_salt: String,
    /// `BuildRequestOptions.explanationPath != null` (`--explain`): the cache checker reports
    /// why actions are rebuilt to the reporter.
    pub explain: bool,
    /// See [`ContainingPackageRootLookup`].
    pub containing_package_root_lookup: Option<Arc<dyn ContainingPackageRootLookup>>,
}

/// An execution failure of `SkyframeActionExecutor`: Java's `ActionExecutionException` or
/// `InterruptedException`.
#[derive(Clone, Debug)]
pub enum ExecutionFailure {
    Exception(ActionExecutionException),
    Interrupted,
}

impl From<ActionExecutionException> for ExecutionFailure {
    fn from(e: ActionExecutionException) -> Self {
        ExecutionFailure::Exception(e)
    }
}

impl From<InterruptedException> for ExecutionFailure {
    fn from(_: InterruptedException) -> Self {
        ExecutionFailure::Interrupted
    }
}

/// `ActionPostprocessing.run`: post-processes the executed action (records newly discovered
/// inputs and writes the action cache). Implemented by `ActionExecutionFunction`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1046 SkyframeActionExecutor.ActionPostprocessing
pub type ActionPostprocessing<'a> = dyn FnMut(
        &mut Environment<'_>,
        &ActionRef,
        &ActionInputMetadata,
        &ActionOutputMetadataStore,
        &IndexMap<String, String>,
    ) -> Result<(), ExecutionFailure>
    + 'a;

/// The result of a (shared) action execution: `None` when Skyframe values were missing in the
/// post-processing step (Java `ActionPostprocessingStep` returns itself to run again).
type SharedResult = Result<Option<ActionExecutionValue>, ExecutionFailure>;

/// An action was not executed because of an action cache hit.
///
/// PORT: Java also carries the action's `InputMetadataProvider` (for the critical path and the
/// execution graph); neither subscriber is ported.
// Java: com/google/devtools/build/lib/actions/CachedActionEvent.java:20 CachedActionEvent
pub struct CachedActionEvent {
    pub action: ActionRef,
    pub nano_time_start: Instant,
    pub nano_time_finish: Instant,
}

impl fmt::Debug for CachedActionEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CachedActionEvent")
    }
}

impl Postable for CachedActionEvent {}

/// Action executor: it executes actions for `ActionExecutionFunction`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:144 SkyframeActionExecutor
pub struct SkyframeActionExecutor {
    exec_root: Path,
    action_out_dir: Path,
    reporter: Arc<dyn ExtendedEventHandler>,
    contexts: ActionContexts,
    client_env: IndexMap<String, String>,
    keep_going: bool,
    verbose_failures: bool,
    output_permissions: OutputPermissions,
    action_key_context: ActionKeyContext,
    action_cache_checker: Arc<ActionCacheChecker>,
    action_execution_salt: String,
    explain: bool,
    containing_package_root_lookup: Option<Arc<dyn ContainingPackageRootLookup>>,
    /// Java: `outputDirectoryHelper` (`SkyframeExecutor`'s `ActionOutputDirectoryHelper`).
    output_directory_helper: ActionOutputDirectoryHelper,
    /// The reporter's `OutputFilter` (`--output_filter` / `--auto_output_filter`); `None` is
    /// `OutputFilter.OUTPUT_EVERYTHING`.
    output_filter: Option<Arc<dyn OutputFilter>>,
    /// `ExecutionOptions.showSubcommands` (`--subcommands`; `executor.reportsSubcommands()`).
    show_subcommands: ShowSubcommands,
    /// Java: `buildActionMap`. Shared actions (same primary output) execute once; the entry
    /// records the `ActionLookupData` of the action that runs (`ActionExecutionState
    /// .actionLookupData`).
    build_action_map:
        Mutex<HashMap<OwnerlessArtifactWrapper, (ActionLookupData, Arc<OnceLock<SharedResult>>)>>,
    had_execution_error: AtomicBool,
    /// Java: `ActionLogBufferPathGenerator.actionUniquifier`.
    action_uniquifier: AtomicU64,
}

impl SkyframeActionExecutor {
    /// Creates an executor whose action cache is disabled (`--nouse_action_cache`); see
    /// [`SkyframeActionExecutor::with_action_cache`].
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:274 SkyframeActionExecutor.<init>
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:331 SkyframeActionExecutor.prepareForExecution
    pub fn new(config: SkyframeActionExecutorConfig) -> SkyframeActionExecutor {
        SkyframeActionExecutor {
            exec_root: config.exec_root,
            action_out_dir: config.action_out_dir,
            reporter: config.reporter,
            contexts: config.contexts,
            client_env: config.client_env,
            keep_going: config.keep_going,
            verbose_failures: config.verbose_failures,
            output_permissions: config.output_permissions,
            action_key_context: ActionKeyContext::new(),
            action_cache_checker: Arc::new(disabled_action_cache_checker()),
            action_execution_salt: String::new(),
            explain: false,
            containing_package_root_lookup: None,
            output_directory_helper: ActionOutputDirectoryHelper::new(),
            output_filter: None,
            show_subcommands: ShowSubcommands::FALSE,
            build_action_map: Mutex::new(HashMap::new()),
            had_execution_error: AtomicBool::new(false),
            action_uniquifier: AtomicU64::new(0),
        }
    }

    /// Sets the action cache checker and the salt of this build (the `actionCacheChecker` and
    /// `actionExecutionSalt` arguments of Java's `prepareForExecution` / `configureActionExecutor`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:331 SkyframeActionExecutor.prepareForExecution
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1030 SkyframeActionExecutor.configure
    pub fn with_action_cache(mut self, setup: ActionCacheSetup) -> SkyframeActionExecutor {
        self.action_cache_checker = setup.action_cache_checker;
        self.action_execution_salt = setup.action_execution_salt;
        self.explain = setup.explain;
        self.containing_package_root_lookup = setup.containing_package_root_lookup;
        self
    }

    /// Sets the reporter's output filter (`Reporter.setOutputFilter`, from `--output_filter` /
    /// `--auto_output_filter`).
    pub fn with_output_filter(
        mut self,
        output_filter: Arc<dyn OutputFilter>,
    ) -> SkyframeActionExecutor {
        self.output_filter = Some(output_filter);
        self
    }

    /// Sets `--subcommands` (`BlazeExecutor.reportsSubcommands`).
    pub fn with_show_subcommands(
        mut self,
        show_subcommands: ShowSubcommands,
    ) -> SkyframeActionExecutor {
        self.show_subcommands = show_subcommands;
        self
    }

    /// `reporter.showOutput(tag)`.
    // Java: com/google/devtools/build/lib/events/Reporter.java:188 Reporter.showOutput
    fn show_output(&self, tag: &str) -> bool {
        match &self.output_filter {
            Some(filter) => filter.show_output(tag),
            None => true,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:405 SkyframeActionExecutor.getExecRoot
    pub fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }

    pub fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }

    pub fn get_reporter(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.reporter
    }

    pub fn get_action_key_context(&self) -> &ActionKeyContext {
        &self.action_key_context
    }

    pub fn get_contexts(&self) -> &ActionContexts {
        &self.contexts
    }

    pub fn get_action_cache_checker(&self) -> &Arc<ActionCacheChecker> {
        &self.action_cache_checker
    }

    pub(crate) fn get_containing_package_root_lookup(
        &self,
    ) -> Option<&Arc<dyn ContainingPackageRootLookup>> {
        self.containing_package_root_lookup.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:413 SkyframeActionExecutor.useArchivedTreeArtifacts
    fn use_archived_tree_artifacts(&self, action: &dyn Action) -> bool {
        // Check that the action produces at least one tree artifact to simplify downstream logic:
        // we don't need to take archived tree artifacts into account if the action doesn't
        // produce at least one of them.
        archived_tree_artifacts_enabled_for_mnemonic(action) && has_tree_artifact_outputs(action)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:452 SkyframeActionExecutor.getOutputPermissions
    pub fn get_output_permissions(&self) -> OutputPermissions {
        self.output_permissions
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1012 SkyframeActionExecutor.recordExecutionError
    pub fn record_execution_error(&self) {
        self.had_execution_error.store(true, Ordering::SeqCst);
    }

    pub fn had_execution_error(&self) -> bool {
        self.had_execution_error.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1025 SkyframeActionExecutor.isBuilderAborting
    fn is_builder_aborting(&self) -> bool {
        self.had_execution_error() && !self.keep_going
    }

    /// `probeActionExecution`: whether this action's (shared) execution already started.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:505 SkyframeActionExecutor.probeActionExecution
    pub fn probe_action_execution(&self, action: &dyn Action) -> bool {
        self.build_action_map
            .lock()
            .contains_key(&OwnerlessArtifactWrapper(action.get_primary_output()))
    }

    /// `ActionLogBufferPathGenerator.generate`: fresh stdout/stderr files for one action.
    // Java: com/google/devtools/build/lib/exec/ActionLogBufferPathGenerator.java:42 ActionLogBufferPathGenerator.generate
    fn generate_file_out_err(&self) -> FileOutErr {
        let action_id = self.action_uniquifier.fetch_add(1, Ordering::SeqCst) + 1;
        FileOutErr::new(
            self.action_out_dir
                .get_child(&format!("stdout-{action_id}")),
            self.action_out_dir
                .get_child(&format!("stderr-{action_id}")),
        )
    }

    /// `ActionOutputMetadataStore.create(useArchivedTreeArtifacts(action), getOutputPermissions(),
    /// action.getOutputs(), ...)`, as `ActionExecutionFunction.checkCacheAndExecuteIfNeeded`
    /// creates it.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:713 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
    pub fn create_output_metadata_store(&self, action: &dyn Action) -> ActionOutputMetadataStore {
        ActionOutputMetadataStore::create(
            self.use_archived_tree_artifacts(action),
            self.output_permissions,
            action.get_outputs(),
        )
    }

    /// Executes `action` (shared actions run once), returning its value; `Ok(None)` when Skyframe
    /// values were missing during post-processing.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:622 SkyframeActionExecutor.executeAction
    pub fn execute_action(
        &self,
        env: &mut Environment<'_>,
        action: &ActionRef,
        input_metadata: Arc<ActionInputMetadata>,
        output_metadata_store: Arc<ActionOutputMetadataStore>,
        action_start_time: Instant,
        action_lookup_data: &ActionLookupData,
        postprocessing: &mut ActionPostprocessing<'_>,
    ) -> SharedResult {
        let adapter = CacheCheckActionAdapter::new(action.as_ref(), &self.action_key_context);
        if self
            .action_cache_checker
            .is_action_execution_prohibited(&adapter)
        {
            // We can't execute an action (e.g. because --check_???_up_to_date option was used).
            // Fail the build instead.
            let message = format!("{} is not up-to-date", action.pretty_print());
            let code = create_detailed_exit_code(&message, execution::Code::ActionNotUpToDate);
            let e =
                ActionExecutionException::new(message, Some(action.as_ref()), false, Some(code));
            self.reporter.handle(&Event::error(e.get_message()));
            return Err(e.into());
        }

        // Use computeIfAbsent to handle concurrent attempts to execute the same shared action.
        let (owner, cell) = {
            let mut map = self.build_action_map.lock();
            map.entry(OwnerlessArtifactWrapper(action.get_primary_output()))
                .or_insert_with(|| (action_lookup_data.clone(), Arc::new(OnceLock::new())))
                .clone()
        };
        // Java: com/google/devtools/build/lib/skyframe/ActionExecutionState.java:87 ActionExecutionState.getResultOrDependOnFuture
        if owner == *action_lookup_data {
            // This object is owned by the Skyframe node executed by the current thread, so we use
            // it to run the state machine.
            return cell
                .get_or_init(|| {
                    self.run_action(
                        env,
                        action,
                        input_metadata,
                        output_metadata_store,
                        action_start_time,
                        action_lookup_data,
                        postprocessing,
                    )
                })
                .clone();
        }

        // This is a shared action, and the primary action is owned by another Skyframe node.
        // PORT: Java registers a dependency on the primary action's completion future and
        // restarts; Blaze's evaluator does not restart, so this thread waits for the result.
        // SharedActionCallback (progress) and SharedActionEvent (ExecutionGraphModule) are not
        // ported.
        match cell.wait().clone() {
            Ok(Some(result)) => match result.transform_for_shared_action(action.as_ref()) {
                Ok(transformed) => Ok(Some(transformed)),
                Err(e) => panic!(
                    "IllegalStateException: Cannot share {:?} and {:?}: {}",
                    owner, action_lookup_data, e
                ),
            },
            other => other,
        }
    }

    /// `ActionRunner.run` followed by its `ActionPostprocessingStep`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1087 ActionRunner.run
    fn run_action(
        &self,
        env: &mut Environment<'_>,
        action: &ActionRef,
        input_metadata: Arc<ActionInputMetadata>,
        output_metadata_store: Arc<ActionOutputMetadataStore>,
        action_start_time: Instant,
        action_lookup_data: &ActionLookupData,
        postprocessing: &mut ActionPostprocessing<'_>,
    ) -> SharedResult {
        let context = ActionExecutionContextImpl {
            executor: self,
            input_metadata: input_metadata.clone(),
            output_metadata_store: Some(output_metadata_store.clone()),
            file_out_err: self.generate_file_out_err(),
            rich_artifact_data: Mutex::new(None),
            clock: JavaClock,
        };
        // PORT: reporter.startTask/finishTask (progress UI) are not ported.
        env.listener().post(Arc::new(ActionStartedEvent {
            action: action.clone(),
            nano_time_start: action_start_time,
        }));
        let result: Result<ActionExecutionValue, ExecutionFailure> = (|| {
            // This call generally deletes any files at locations that are declared outputs of the
            // action, although some actions perform additional work, while others intentionally
            // keep previous outputs in place.
            let Some(executable) = as_executable(action.as_ref()) else {
                blaze_util::unported!(&format!(
                    "{}.execute (no ExecutableAction registered for mnemonic {})",
                    std::any::type_name_of_val(action.as_any()),
                    action.get_mnemonic()
                ));
            };
            if let Err(e) = executable.prepare(&self.exec_root) {
                return Err(self
                    .to_action_execution_exception(
                        "failed to delete output files before executing action",
                        Some(&e),
                        action.as_ref(),
                        None,
                        execution::Code::ActionOutputsDeletionFailure,
                    )
                    .into());
            }
            self.create_output_directories(action.as_ref())?;
            self.execute_action_inner(env.listener(), action, &context)
        })();
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1199 ActionRunner.notifyActionCompletion
        env.listener().post(Arc::new(ActionCompletionEvent {
            related_time_start: action_start_time,
            finish_time: Instant::now(),
            action: action.clone(),
            action_lookup_data: action_lookup_data.clone(),
        }));
        let value = result?;

        // A closure to post-process the executed action, doing work like updating cached state
        // with any newly discovered inputs, and writing the result to the action cache.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1424 ActionPostprocessingStep.run
        postprocessing(
            env,
            action,
            &*input_metadata,
            &*output_metadata_store,
            &self.client_env,
        )?;
        if env.values_missing() {
            return Ok(None);
        }
        // Once the action has been written to the action cache, we can free its discovered
        // inputs.
        // PORT: freeDiscoveredInputsAfterExecution is `!trackIncrementalState && no
        // --experimental_action_listener`; Bazel tracks incremental state by default, so
        // discovered inputs are kept (resetDiscoveredInputs is not called).
        Ok(Some(value))
    }

    /// Executes the given action.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1253 ActionRunner.executeAction
    fn execute_action_inner(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        context: &ActionExecutionContextImpl<'_>,
    ) -> Result<ActionExecutionValue, ExecutionFailure> {
        let Some(executable) = as_executable(action.as_ref()) else {
            blaze_util::unported!(&format!(
                "{}.execute (no ExecutableAction registered for mnemonic {})",
                std::any::type_name_of_val(action.as_any()),
                action.get_mnemonic()
            ));
        };
        let result = match self
            .check_for_unsound_directory_inputs(
                action.as_ref(),
                context.get_input_metadata_provider(),
            )
            .and_then(|()| executable.execute(context))
        {
            Ok(r) => r,
            Err(e) => {
                let primary_output_path = context.get_input_path(&action.get_primary_output());
                return Err(self
                    .process_and_get_exception_to_throw(
                        listener,
                        &primary_output_path,
                        action,
                        e,
                        context.get_file_out_err(),
                        ErrorTiming::AFTER_EXECUTION,
                    )
                    .into());
            }
        };
        self.actually_complete_action(listener, action, context, result)
            .map_err(ExecutionFailure::from)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1302 ActionRunner.actuallyCompleteAction
    fn actually_complete_action(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        context: &ActionExecutionContextImpl<'_>,
        action_result: ActionResult,
    ) -> Result<ActionExecutionValue, ActionExecutionException> {
        let mut output_already_dumped = false;
        if !action_result.is_empty() {
            listener.post(Arc::new(ActionResultReceivedEvent {
                action: action.clone(),
                action_result: action_result.clone(),
            }));
        }
        // Action terminated fine, now report the output.
        let out_err_buffer = context.get_file_out_err();
        if out_err_buffer.has_recorded_output()
            && (action.shows_output_unconditionally()
                || self.show_output(&blaze_cmdline::Label::print(action.get_owner().get_label())))
        {
            self.dump_recorded_out_err_for_action(action.as_ref(), out_err_buffer);
            output_already_dumped = true;
        }
        let output_metadata_store = context
            .output_metadata_store
            .as_ref()
            .expect("output metadata store of an executing action");
        let primary_output = action.get_primary_output();
        let primary_output_path = context.get_input_path(&primary_output);
        check_state!(
            action.inputs_known(),
            "Action {} successfully executed, but inputs still not known",
            action.to_string_java()
        );
        if !self.check_outputs(action.as_ref(), output_metadata_store) {
            let e = self.to_action_execution_exception(
                "not all outputs were created or valid",
                None,
                action.as_ref(),
                if output_already_dumped {
                    None
                } else {
                    Some(out_err_buffer)
                },
                execution::Code::ActionOutputsNotCreated,
            );
            // Success in execution but failure in completion.
            self.report_action_execution(
                listener,
                &primary_output_path,
                action,
                Some(e.clone()),
                out_err_buffer,
                ErrorTiming::AFTER_EXECUTION,
            );
            return Err(e);
        }
        // PORT: finalizeActions / outputService.finalizeAction is a no-op for the local output
        // service.
        self.report_action_execution(
            listener,
            &primary_output_path,
            action,
            None,
            out_err_buffer,
            ErrorTiming::NO_ERROR,
        );
        let rich_artifact_data = context.rich_artifact_data.lock().clone();
        Ok(self.create_action_execution_value(
            action.as_ref(),
            output_metadata_store,
            rich_artifact_data,
            &context.input_metadata,
        ))
    }

    /// `ActionExecutionValue.create(outputMetadataStore, richArtifactData, action)`; the
    /// `RunfilesArtifactValue` metadata (`getMetadata()`) is computed here from the action's
    /// input metadata, as Java's `RunfilesArtifactValue` constructor does.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:145 ActionExecutionValue.create
    fn create_action_execution_value(
        &self,
        action: &dyn Action,
        output_metadata_store: &ActionOutputMetadataStore,
        rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
        input_metadata: &ActionInputMetadata,
    ) -> ActionExecutionValue {
        let rich = rich_artifact_data.map(|r| {
            let metadata = compute_runfiles_metadata(action, &r, input_metadata);
            (r, metadata)
        });
        ActionExecutionValue::create_with_rich_metadata(
            output_metadata_store.get_all_artifact_data(),
            output_metadata_store.get_all_tree_artifact_data(),
            rich,
        )
    }

    /// Validates that all action outputs were created or intentionally omitted. This can result
    /// in chmod calls on the output files; see [`ActionOutputMetadataStore`]. Returns false if
    /// some outputs are missing or invalid.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1627 SkyframeActionExecutor.checkOutputs
    fn check_outputs(
        &self,
        action: &dyn Action,
        output_metadata_store: &ActionOutputMetadataStore,
    ) -> bool {
        let mut success = true;
        for output in action.get_outputs() {
            // getOutputMetadata() has the side effect of adding the artifact to the cache if it's
            // not there already (e.g., due to a previous call to OutputMetadataStore.injectFile()),
            // therefore we only call it if we know the artifact is not omitted.
            let input = artifact_input(&output);
            if output_metadata_store.artifact_omitted(&input) {
                continue;
            }
            match output_metadata_store.get_output_metadata(&input) {
                Ok(metadata) => {
                    if let Some(metadata) = &metadata
                        && !self.check_for_unsound_directory_output(action, &output, metadata)
                    {
                        return false;
                    }
                    // PORT: addOutputToMetrics (BEP output-artifact metrics) is not ported.
                }
                Err(e) => {
                    success = false;
                    let e = match e {
                        IoOrInterruptedException::Io(e) => e,
                        IoOrInterruptedException::Interrupted(_) => {
                            std::io::Error::other("interrupted")
                        }
                    };
                    if output.is_tree_artifact() {
                        report_output_tree_artifact_errors(action, &output, &*self.reporter, &e);
                    } else {
                        // PORT: the `output.isSymlink() && e instanceof NotASymlinkException`
                        // branch ("declared output '%s' is not a symlink") is unreachable: the
                        // metadata of a declared symlink output
                        // (FileArtifactValue.createForUnresolvedSymlink) fails with unported!.
                        // Are all other exceptions caught due to missing files?
                        report_missing_output_file(
                            action,
                            &output,
                            &*self.reporter,
                            output.get_path().is_symbolic_link(),
                            &e,
                        );
                    }
                }
            }
        }
        success
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1469 SkyframeActionExecutor.createOutputDirectories
    fn create_output_directories(
        &self,
        action: &dyn Action,
    ) -> Result<(), ActionExecutionException> {
        if let Err(e) = self
            .output_directory_helper
            .create_output_directories(&action.get_outputs())
        {
            return Err(self.to_action_execution_exception(
                &format!(
                    "failed to create output directory '{}': {}",
                    e.get_directory_path(),
                    e.get_cause()
                ),
                Some(e.get_cause()),
                action,
                /* action_output= */ None,
                execution::Code::ActionOutputDirectoryCreationFailure,
            ));
        }
        Ok(())
    }

    /// Reports the "directory dependency checking" warning for source directory inputs.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1727 SkyframeActionExecutor.checkForUnsoundDirectoryInputs
    fn check_for_unsound_directory_inputs(
        &self,
        action: &dyn Action,
        metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<(), ActionExecutionException> {
        // TrackSourceDirectoriesFlag.trackSourceDirectories() is false by default (the startup
        // flag that sets it is not ported).

        if action.get_mnemonic() == "FilesetTraversal" {
            // Omit warning for filesets (b/1437948).
            return Ok(());
        }

        // Report "directory dependency checking" warning only for non-generated directories
        // (generated ones will have been reported earlier, in the checkForUnsoundDirectoryOutput
        // call for the respective producing action).
        for input in action.get_mandatory_inputs().to_list().iter() {
            if !input.is_source_artifact() {
                continue;
            }
            // Assume that if the file did not exist, we would not have gotten here.
            match metadata_provider.get_input_metadata(&artifact_input(input)) {
                Ok(metadata) => {
                    let metadata = metadata
                        .unwrap_or_else(|| panic!("NullPointerException: metadata of {input}"));
                    if metadata.get_type().is_directory() {
                        // TODO(ulfjack): What about dependency checking of special files?
                        self.reporter.handle(&get_event_for_unsound_directory(
                            EventKind::Warning,
                            |s| {
                                format!(
                                    "input {s} is a directory; dependency checking of \
                                     directories is unsound"
                                )
                            },
                            input,
                            action.get_owner(),
                        ));
                    }
                }
                Err(e) => {
                    // Java: com/google/devtools/build/lib/actions/EnvironmentalExecException.java:33 EnvironmentalExecException.<init>
                    let exec_exception: ExecException = EnvironmentalExecException {
                        message: format!("java.io.IOException: {e}"),
                        failure_detail: execution_failure(
                            String::new(),
                            execution::Code::InputDirectoryCheckIoException,
                        ),
                    }
                    .into();
                    return Err(ActionExecutionException::from_exec_exception(
                        &exec_exception,
                        None,
                        action,
                    ));
                }
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1762 SkyframeActionExecutor.checkForUnsoundDirectoryOutput
    fn check_for_unsound_directory_output(
        &self,
        action: &dyn Action,
        output: &Artifact,
        metadata: &FileArtifactValue,
    ) -> bool {
        if output.is_directory() || output.is_symlink() || !metadata.get_type().is_directory() {
            return true;
        }
        self.reporter.handle(&get_event_for_unsound_directory(
            EventKind::Error,
            |s| format!("output {s} is a directory but was not declared as such"),
            output,
            action.get_owner(),
        ));
        false
    }

    /// Checks the action cache to see if `action` needs to be executed, or is up to date. Returns
    /// a token with the semantics of [`ActionCacheChecker::get_token_if_need_to_execute`]: `None`
    /// if the action is up to date, and `Some` if it needs to be executed, in which case that
    /// token should be provided to the `ActionCacheChecker` after execution.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:759 SkyframeActionExecutor.checkActionCache
    pub fn check_action_cache(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        action: &ActionRef,
        input_metadata_provider: &ActionInputMetadata,
        output_metadata_store: &ActionOutputMetadataStore,
        action_start_time: Instant,
        resolved_cache_artifacts: Option<&[Artifact]>,
        client_env: &IndexMap<String, String>,
    ) -> Result<Option<Token>, ExecutionFailure> {
        // PORT: cacheHitSemaphore (--experimental_throttle_action_cache_check) only throttles
        // concurrent cache checks; its wait time is a BEP metric. Not ported.
        self.check_action_cache_inputs_supported(action.as_ref());
        let adapter = CacheCheckActionAdapter::new(action.as_ref(), &self.action_key_context);
        // Java passes state.actionInputMetadataProvider: the input map alone.
        let map_only = input_metadata_provider.action_input_map_only();
        // LocalOutputService.getOutputChecker(): no remote output service is ported.
        let output_checker = TrustLocalOnly;
        let handler: Option<&dyn EventHandler> = if self.explain {
            Some(&*self.reporter as &dyn EventHandler)
        } else {
            None
        };
        let resolved: Option<Vec<ActionInputRef>> =
            resolved_cache_artifacts.map(|a| a.iter().map(artifact_input).collect());
        let use_archived_tree_artifacts = self.use_archived_tree_artifacts(action.as_ref());
        let mut token = self.action_cache_checker.get_token_if_need_to_execute(
            &adapter,
            resolved.as_deref(),
            client_env,
            self.output_permissions.for_action_cache(),
            handler,
            &map_only,
            output_metadata_store,
            &self.action_execution_salt,
            Some(&output_checker as &dyn OutputChecker),
            use_archived_tree_artifacts,
        )?;

        if token.is_none() {
            let event_posted = false;

            if let Some(record_action_cache_hit) = self.notify_on_action_cache_hit(action)
                && !record_action_cache_hit
            {
                token = self
                    .action_cache_checker
                    .get_token_unconditionally_after_failure_to_record_action_cache_hit(
                        &adapter,
                        resolved.as_deref(),
                        client_env,
                        self.output_permissions.for_action_cache(),
                        handler,
                        &map_only,
                        output_metadata_store,
                        &self.action_execution_salt,
                        Some(&output_checker as &dyn OutputChecker),
                        use_archived_tree_artifacts,
                    )?;
            }

            // We still need to check the outputs so that output file data is available to the
            // value. Filesets cannot be cached in the action cache.
            let _ = self.check_outputs(action.as_ref(), output_metadata_store);
            if !event_posted {
                event_handler.post(Arc::new(CachedActionEvent {
                    action: action.clone(),
                    nano_time_start: action_start_time,
                    nano_time_finish: Instant::now(),
                }));
            }
        }
        Ok(token)
    }

    /// `action instanceof NotifyOnActionCacheHit notify` → `notify.actionCacheHit(context)`:
    /// `None` if the action does not implement `NotifyOnActionCacheHit`.
    ///
    /// PORT: among ported actions only `TestRunnerAction` implements `NotifyOnActionCacheHit`
    /// (`CoverageReportAction` is not ported); the `ActionCachedContext` is its arguments (event
    /// handler, exec root, `getContext(TestActionContext.class)`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:801 SkyframeActionExecutor.checkActionCache
    fn notify_on_action_cache_hit(&self, action: &ActionRef) -> Option<bool> {
        // Java's `instanceof`: the analysis-side TestRunnerAction is viewed as the execution one.
        let test_action: Arc<TestRunnerAction> =
            blaze_exec::analysis::test::as_test_runner_action(action.as_ref())?;
        let test_action_context = self
            .contexts
            .get_any(TypeId::of::<Arc<dyn TestActionContext>>())
            .and_then(|c| c.downcast::<Arc<dyn TestActionContext>>().ok())
            .expect("NullPointerException: getContext(TestActionContext.class)");
        Some(test_action.action_cache_hit(&*self.reporter, &self.exec_root, &**test_action_context))
    }

    /// `action instanceof RichDataProducingAction rdpa ?
    /// rdpa.reconstructRichDataOnActionCacheHit(inputMetadataProvider) : null`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:740 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
    pub fn reconstruct_rich_data_on_action_cache_hit(
        &self,
        action: &dyn Action,
        input_metadata_provider: &ActionInputMetadata,
    ) -> Option<(Arc<RunfilesArtifactValue>, FileArtifactValue)> {
        if let Some(runfiles_tree_action) = action.as_any().downcast_ref::<RunfilesTreeAction>() {
            // Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:97 RunfilesTreeAction.reconstructRichDataOnActionCacheHit
            let value = Arc::new(create_runfiles_artifact_value(runfiles_tree_action));
            let metadata = compute_runfiles_metadata(
                action,
                &value,
                &input_metadata_provider.action_input_map_only(),
            );
            return Some((value, metadata));
        }
        if let Some(symlink_tree_action) = action.as_any().downcast_ref::<SymlinkTreeAction>() {
            // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:229 SymlinkTreeAction.reconstructRichDataOnActionCacheHit
            if symlink_tree_action.get_primary_output().is_fileset() {
                blaze_util::unported!("FilesetOutputTree.forward");
            }
            return None;
        }
        if let Some(symlink_action) = action.as_any().downcast_ref::<SymlinkAction>() {
            // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:210 SymlinkAction.reconstructRichDataOnActionCacheHit
            if symlink_action.is_fileset() {
                blaze_util::unported!("FilesetOutputTree.forward");
            }
            return None;
        }
        None
    }

    /// Fails loudly for action cache inputs whose metadata the port does not hold as Java does
    /// (filesets: `ActionInputMap.getFileset`).
    fn check_action_cache_inputs_supported(&self, action: &dyn Action) {
        if !self.action_cache_checker.enabled() {
            return;
        }
        for input in action.get_inputs().to_list().iter() {
            if input.is_fileset() {
                blaze_util::unported!("ActionCacheChecker (fileset input)");
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:864 SkyframeActionExecutor.updateActionCache
    pub fn update_action_cache(
        &self,
        action: &ActionRef,
        input_metadata_provider: &dyn InputMetadataProvider,
        output_metadata_store: &ActionOutputMetadataStore,
        token: &Token,
        client_env: &IndexMap<String, String>,
    ) -> Result<(), ExecutionFailure> {
        if !self.action_cache_checker.enabled() {
            return Ok(());
        }
        let adapter = CacheCheckActionAdapter::new(action.as_ref(), &self.action_key_context);
        match self.action_cache_checker.update_action_cache(
            &adapter,
            token,
            input_metadata_provider,
            output_metadata_store,
            client_env,
            self.output_permissions.for_action_cache(),
            &self.action_execution_salt,
            self.use_archived_tree_artifacts(action.as_ref()),
        ) {
            Ok(()) => Ok(()),
            // Skyframe has already done all the filesystem access needed for outputs and swallows
            // IOExceptions for inputs. So an IOException is impossible here.
            Err(IoOrInterruptedException::Io(e)) => panic!(
                "IllegalStateException: failed to update action cache for {}, but all outputs \
                 should already have been checked: {}",
                action.pretty_print(),
                e
            ),
            Err(IoOrInterruptedException::Interrupted(_)) => Err(ExecutionFailure::Interrupted),
        }
    }

    /// The inputs an input-discovering action used in its cached execution, or `None` if
    /// Skyframe dependencies are missing.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:897 SkyframeActionExecutor.getActionCachedInputs
    pub fn get_action_cached_inputs(
        &self,
        action: &dyn Action,
        resolver: &dyn PackageRootResolver,
    ) -> Result<Option<Vec<Artifact>>, ExecutionFailure> {
        let adapter = CacheCheckActionAdapter::new(action, &self.action_key_context);
        match self
            .action_cache_checker
            .get_cached_inputs(&adapter, resolver)
        {
            Ok(Some(inputs)) => Ok(Some(
                inputs
                    .iter()
                    .map(|i| {
                        as_blaze_artifact(i)
                            .expect("action cache input is an Artifact")
                            .clone()
                    })
                    .collect(),
            )),
            Ok(None) => Ok(None),
            Err(PackageRootExceptionOrInterrupted::Interrupted(_)) => {
                Err(ExecutionFailure::Interrupted)
            }
            Err(PackageRootExceptionOrInterrupted::PackageRoot(e)) => {
                self.print_error(&e.message, action, None);
                // PORT: the exception's FailureDetails.IncludeScanning code (from the
                // BuildFileNotFoundException / InconsistentFilesystemException) is not carried by
                // the ported PackageRootException.
                blaze_util::unported!(
                    "SkyframeActionExecutor.getActionCachedInputs (PackageRootException.getError)"
                );
            }
        }
    }

    /// Perform dependency discovery for action, which must discover its inputs.
    ///
    /// This method is just a wrapper around `Action.discoverInputs` that properly processes any
    /// `ActionExecutionException` thrown before rethrowing it to the caller.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:922 SkyframeActionExecutor.discoverInputs
    pub fn discover_inputs(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        input_metadata: Arc<ActionInputMetadata>,
    ) -> Result<Option<NestedSet<Artifact>>, ExecutionFailure> {
        let context = ActionExecutionContextImpl {
            executor: self,
            input_metadata,
            // ActionExecutionContext.forInputDiscovery has no output metadata store.
            output_metadata_store: None,
            file_out_err: self.generate_file_out_err(),
            rich_artifact_data: Mutex::new(None),
            clock: JavaClock,
        };
        // PORT: ScanningActionEvent / StoppedScanningActionEvent feed the progress UI only.
        let Some(executable) = as_executable(action.as_ref()) else {
            blaze_util::unported!(&format!(
                "{}.discoverInputs (no ExecutableAction registered for mnemonic {})",
                std::any::type_name_of_val(action.as_any()),
                action.get_mnemonic()
            ));
        };
        match executable.discover_inputs(&context) {
            Ok(artifacts) => Ok(artifacts),
            Err(e) => {
                // PORT: lost inputs (action file system) are not ported.
                let primary_output_path = context.get_input_path(&action.get_primary_output());
                Err(self
                    .process_and_get_exception_to_throw(
                        listener,
                        &primary_output_path,
                        action,
                        e,
                        context.get_file_out_err(),
                        ErrorTiming::BEFORE_EXECUTION,
                    )
                    .into())
            }
        }
    }

    /// Must not be called with a `LostInputsActionExecutionException`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1512 SkyframeActionExecutor.processAndGetExceptionToThrow
    fn process_and_get_exception_to_throw(
        &self,
        listener: &dyn ExtendedEventHandler,
        primary_output_path: &Path,
        action: &ActionRef,
        e: ActionExecutionException,
        out_err_buffer: &FileOutErr,
        error_timing: ErrorTiming,
    ) -> ActionExecutionException {
        self.report_action_execution(
            listener,
            primary_output_path,
            action,
            Some(e.clone()),
            out_err_buffer,
            error_timing,
        );
        // If we already printed the error for the exception we mark it as already reported so
        // that we do not print it again in upper levels.
        // PORT: AlreadyReportedActionExecutionException is the same exception; the caller does not
        // print action errors again.
        self.print_error(e.get_message(), action.as_ref(), Some(out_err_buffer));
        e
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1802 SkyframeActionExecutor.toActionExecutionException
    fn to_action_execution_exception(
        &self,
        message: &str,
        cause: Option<&std::io::Error>,
        action: &dyn Action,
        action_output: Option<&FileOutErr>,
        detailed_code: execution::Code,
    ) -> ActionExecutionException {
        let code = create_detailed_exit_code(message, detailed_code);
        let ex = match cause {
            None => ActionExecutionException::new(message, Some(action), false, Some(code)),
            Some(c) => {
                ActionExecutionException::with_cause(message, c, Some(action), false, Some(code))
            }
        };
        let mut report_message = ex.get_message().to_string();
        if let Some(c) = cause {
            report_message.push_str(": ");
            report_message.push_str(&c.to_string());
        }
        self.print_error(&report_message, action, action_output);
        ex
    }

    /// For the action 'action' that failed due to 'message' with the output 'actionOutput',
    /// notify the user about the error.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1850 SkyframeActionExecutor.printError
    pub fn print_error(
        &self,
        message: &str,
        action: &dyn Action,
        action_output: Option<&FileOutErr>,
    ) -> bool {
        let message = format!("{} failed: {}", action.describe(), message);
        self.dump_recorded_out_err(
            Event::error_at(Some(action.get_owner().get_location().clone()), message),
            action_output,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1863 SkyframeActionExecutor.dumpRecordedOutErr
    fn dump_recorded_out_err_for_action(&self, action: &dyn Action, out_err_buffer: &FileOutErr) {
        let event = Event::info(format!("From {}:", action.describe()));
        self.dump_recorded_out_err(event, Some(out_err_buffer));
    }

    /// Dumps output from the action along with `prefix_event` if the build is not aborting.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1876 SkyframeActionExecutor.dumpRecordedOutErr
    fn dump_recorded_out_err(
        &self,
        prefix_event: Event,
        out_err_buffer: Option<&FileOutErr>,
    ) -> bool {
        // For some actions (e.g., many local actions) the pollInterruptedStatus() won't notice
        // that we had an interrupted job. It will continue. For that reason we must take care to
        // NOT report errors if we're in the 'aborting' mode: Any cancelled action would show up
        // here.
        if self.is_builder_aborting() {
            return false;
        }
        match out_err_buffer {
            Some(o) if o.has_recorded_output() => {
                // Bind the output to the prefix event.
                self.reporter
                    .handle(
                        &prefix_event.with_process_output(Arc::new(ActionOutputEventData {
                            out: o.out_as_bytes(),
                            err: o.err_as_bytes(),
                            out_path: o.get_output_path().get_path_string().to_string(),
                            err_path: o.get_error_path().get_path_string().to_string(),
                        })),
                    );
            }
            _ => self.reporter.handle(&prefix_event),
        }
        true
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1923 SkyframeActionExecutor.reportActionExecution
    fn report_action_execution(
        &self,
        listener: &dyn ExtendedEventHandler,
        primary_output_path: &Path,
        action: &ActionRef,
        exception: Option<ActionExecutionException>,
        out_err: &FileOutErr,
        error_timing: ErrorTiming,
    ) {
        let stdout = out_err
            .has_recorded_stdout()
            .then(|| out_err.get_output_path().clone());
        let stderr = out_err
            .has_recorded_stderr()
            .then(|| out_err.get_error_path().clone());
        listener.post(Arc::new(ActionExecutedEvent {
            action_id: action.get_primary_output().get_exec_path().clone(),
            action: action.clone(),
            exception,
            primary_output_path: primary_output_path.clone(),
            output_artifact: action.get_primary_output(),
            stdout,
            stderr,
            error_timing,
        }));
    }

    pub fn is_verbose_failures(&self) -> bool {
        self.verbose_failures
    }
}

/// An `ActionCacheChecker` with `CacheConfig.enabled() == false` (`--nouse_action_cache`).
fn disabled_action_cache_checker() -> ActionCacheChecker {
    ActionCacheChecker::new(
        None,
        Arc::new(ArtifactFactoryResolver(None)),
        Box::new(|_: &dyn CacheCheckAction| true),
        Arc::new(NoProxies),
        Some(
            ActionCacheCheckerCacheConfig::builder()
                .set_enabled(false)
                .set_store_output_metadata(false)
                .build(),
        ),
    )
}

/// `options.getOptions(CoreOptions.class).archivedArtifactsMnemonicsFilter.test(mnemonic)`.
///
/// PORT: `--archived_tree_artifact_mnemonics_filter` keeps its default `-.*` (disabled by
/// default), which matches no mnemonic; the option is not threaded to the executor.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:420 SkyframeActionExecutor.archivedTreeArtifactsEnabledForMnemonic
fn archived_tree_artifacts_enabled_for_mnemonic(_action: &dyn Action) -> bool {
    false
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:427 SkyframeActionExecutor.hasTreeArtifactOutputs
fn has_tree_artifact_outputs(action: &dyn Action) -> bool {
    action.get_outputs().iter().any(|o| o.is_tree_artifact())
}

/// The `RunfilesArtifactValue` metadata: `FileArtifactValue.createRunfilesProxy(computeDigest())`
/// over the runfiles tree's mapping and the metadata of the producing action's inputs, sorted by
/// exec path (`RunfilesTreeAction.createRunfilesArtifactValue`).
// Java: com/google/devtools/build/lib/actions/RunfilesArtifactValue.java:87 RunfilesArtifactValue.computeDigest
// Java: com/google/devtools/build/lib/actions/RunfilesTreeAction.java:62 RunfilesTreeAction.createRunfilesArtifactValue
fn compute_runfiles_metadata(
    action: &dyn Action,
    value: &RunfilesArtifactValue,
    input_metadata_provider: &dyn InputMetadataProvider,
) -> FileArtifactValue {
    let mut inputs: Vec<Artifact> = action.get_inputs().to_list().to_vec();
    // Sort for better equality in RunfilesArtifactValue.
    inputs.sort_by(Artifact::exec_path_comparator);
    let mut files: Vec<(Artifact, Option<FileArtifactValue>)> = Vec::new();
    let mut tree_values: Vec<Arc<TreeArtifactValue>> = Vec::new();
    for input in inputs {
        if input.is_fileset() {
            blaze_util::unported!("RunfilesArtifactValue.computeDigest (FilesetOutputTree.addTo)");
        } else if input.is_tree_artifact() {
            tree_values.push(
                input_metadata_provider
                    .get_tree_metadata(&artifact_input(&input))
                    .unwrap_or_else(|| panic!("NullPointerException: tree metadata of {input}")),
            );
        } else {
            let metadata = match input_metadata_provider.get_input_metadata(&artifact_input(&input))
            {
                Ok(m) => m,
                // On action cache hits, all input metadata should already be in RAM.
                Err(e) => panic!("IllegalStateException: {e}"),
            };
            files.push((input, metadata));
        }
    }

    let mut result = Fingerprint::new();
    let mapping = value.get_runfiles_tree().get_mapping();
    result.add_int(mapping.len() as i32);
    for (path, artifact) in &mapping {
        result.add_string(path.get_path_string());
        result.add_boolean(artifact.is_some());
        if let Some(artifact) = artifact {
            result.add_string(artifact.get_exec_path().get_path_string());
        }
    }
    result.add_int(files.len() as i32);
    for (file, metadata) in &files {
        let value = if file.is_constant_metadata() {
            FileArtifactValue::ConstantMetadata
        } else {
            metadata
                .clone()
                .unwrap_or_else(|| panic!("NullPointerException: metadata of {file}"))
        };
        blaze_exec::actions::cache::metadata_digest_utils::add_to(&value, &mut result);
    }
    result.add_int(tree_values.len() as i32);
    for tree_value in &tree_values {
        result.add_bytes(
            tree_value
                .get_digest()
                .unwrap_or_else(|| panic!("NullPointerException: TreeArtifactValue.getDigest()")),
        );
    }
    // No filesets (unported above).
    FileArtifactValue::RunfilesProxy {
        digest: result.digest_and_reset(),
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1823 SkyframeActionExecutor.createDetailedExitCode
fn create_detailed_exit_code(message: &str, detailed_code: execution::Code) -> FailureDetail {
    execution_failure(message.to_string(), detailed_code)
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1563 SkyframeActionExecutor.reportMissingOutputFile
fn report_missing_output_file(
    action: &dyn Action,
    output: &Artifact,
    reporter: &dyn ExtendedEventHandler,
    is_symlink: bool,
    exception: &std::io::Error,
) {
    let genrule = action.get_mnemonic() == "Genrule";
    let prefix = format!(
        "{}{}' ",
        if genrule {
            "declared output '"
        } else {
            "output '"
        },
        output.pretty_print()
    );
    // PORT: logger.atWarning() ("Error creating ...") is not ported.
    let _ = exception;
    if is_symlink {
        let msg = format!("{prefix}is a dangling symbolic link");
        reporter.handle(&Event::error_at(
            Some(action.get_owner().get_location().clone()),
            msg,
        ));
    } else {
        let suffix = if genrule {
            " by genrule. This is probably because the genrule actually didn't create this \
             output, or because the output was a directory and the genrule was run remotely \
             (note that only the contents of declared file outputs are copied from genrules run \
             remotely)"
        } else {
            ""
        };
        reporter.handle(&Event::error_at(
            Some(action.get_owner().get_location().clone()),
            format!("{prefix}was not created{suffix}"),
        ));
    }
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1586 SkyframeActionExecutor.reportOutputTreeArtifactErrors
fn report_output_tree_artifact_errors(
    action: &dyn Action,
    output: &Artifact,
    reporter: &dyn ExtendedEventHandler,
    e: &std::io::Error,
) {
    let error_message = if e.kind() == std::io::ErrorKind::NotFound {
        format!(
            "output tree artifact {} was not created",
            output.pretty_print()
        )
    } else {
        format!(
            "error while validating output tree artifact {}: {}",
            output.pretty_print(),
            e
        )
    };
    reporter.handle(&Event::error_at(
        Some(action.get_owner().get_location().clone()),
        error_message,
    ));
}

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1774 SkyframeActionExecutor.getEventForUnsoundDirectory
fn get_event_for_unsound_directory(
    kind: EventKind,
    format: impl Fn(&str) -> String,
    artifact: &Artifact,
    owner: &blaze_actions::ActionOwner,
) -> Event {
    let label = owner.get_label();
    let artifact_string = match label {
        Some(label) => format!("'{}' of {}", artifact.pretty_print(), label),
        None => artifact.pretty_print(),
    };
    let event = Event::of_at(
        kind,
        Some(owner.get_location().clone()),
        format(&artifact_string),
    );
    match label {
        Some(label) => event.with_tag(Some(&label.to_string())),
        None => event,
    }
}

fn set_tree_read_only_and_executable(dir: &Path, mode: u32) -> std::io::Result<()> {
    for child in dir.get_directory_entries()? {
        let st = child.stat_with(Symlinks::Nofollow)?;
        if st.is_directory() {
            set_tree_read_only_and_executable(&child, mode)?;
        } else if st.is_file() {
            child.chmod(mode)?;
        }
    }
    dir.chmod(0o755)
}

fn visit_tree(
    dir: &Path,
    rel: &PathFragment,
    f: &mut dyn FnMut(&PathFragment, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut entries = dir.get_directory_entries()?;
    entries.sort_by(|a, b| a.get_base_name().cmp(b.get_base_name()));
    for child in entries {
        let child_rel = rel.get_child(child.get_base_name());
        if child.is_directory_with(Symlinks::Nofollow) {
            visit_tree(&child, &child_rel, f)?;
        } else {
            f(&child_rel, &child)?;
        }
    }
    Ok(())
}

/// The stdout/stderr of an action attached to its `From ...:` event.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:2007 SkyframeActionExecutor.ActionOutputEventData
#[derive(Debug)]
struct ActionOutputEventData {
    out: Vec<u8>,
    err: Vec<u8>,
    out_path: String,
    err_path: String,
}

impl blaze_events::event::ProcessOutput for ActionOutputEventData {
    fn get_std_out_path(&self) -> String {
        self.out_path.clone()
    }
    fn get_std_out_size(&self) -> std::io::Result<i64> {
        Ok(self.out.len() as i64)
    }
    fn get_std_out(&self) -> Vec<u8> {
        self.out.clone()
    }
    fn get_std_err_path(&self) -> String {
        self.err_path.clone()
    }
    fn get_std_err_size(&self) -> std::io::Result<i64> {
        Ok(self.err.len() as i64)
    }
    fn get_std_err(&self) -> Vec<u8> {
        self.err.clone()
    }
}

/// Handles the metadata of the outputs of the action during its execution.
///
/// PORT: Java's `MISSING_TREE_ARTIFACT` marker is a `None` tree value.
// Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:79 ActionOutputMetadataStore
pub struct ActionOutputMetadataStore {
    archived_tree_artifacts_enabled: bool,
    output_permissions: OutputPermissions,
    execution_mode: AtomicBool,
    outputs: IndexSet<Artifact>,
    omitted_outputs: Mutex<HashSet<Artifact>>,
    artifact_data: Mutex<HashMap<Artifact, FileArtifactValue>>,
    tree_artifact_data: Mutex<HashMap<Artifact, Option<Arc<TreeArtifactValue>>>>,
}

impl ActionOutputMetadataStore {
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:83 ActionOutputMetadataStore.create
    pub fn create(
        archived_tree_artifacts_enabled: bool,
        output_permissions: OutputPermissions,
        outputs: Vec<Artifact>,
    ) -> ActionOutputMetadataStore {
        ActionOutputMetadataStore {
            archived_tree_artifacts_enabled,
            output_permissions,
            execution_mode: AtomicBool::new(false),
            outputs: outputs.into_iter().collect(),
            omitted_outputs: Mutex::new(HashSet::new()),
            artifact_data: Mutex::new(HashMap::new()),
            tree_artifact_data: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:130 ActionOutputMetadataStore.putArtifactData
    fn put_artifact_data(&self, artifact: &Artifact, value: FileArtifactValue) {
        blaze_util::check_argument!(
            !artifact.is_tree_artifact() && !artifact.is_child_of_declared_directory(),
            "{} should be stored in a TreeArtifactValue",
            artifact
        );
        self.artifact_data.lock().insert(artifact.clone(), value);
    }

    /// `getAllArtifactData()`.
    ///
    /// PORT: Java copies a `ConcurrentHashMap`; the entries are returned in the action's output
    /// order here.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:138 ActionOutputMetadataStore.getAllArtifactData
    pub fn get_all_artifact_data(&self) -> IndexMap<Artifact, FileArtifactValue> {
        let data = self.artifact_data.lock();
        let mut result = IndexMap::new();
        for output in &self.outputs {
            if let Some(v) = data.get(output) {
                result.insert(output.clone(), v.clone());
            }
        }
        for (k, v) in data.iter() {
            if !result.contains_key(k) {
                result.insert(k.clone(), v.clone());
            }
        }
        result
    }

    /// `getAllTreeArtifactData()`.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:142 ActionOutputMetadataStore.getAllTreeArtifactData
    pub fn get_all_tree_artifact_data(&self) -> IndexMap<Artifact, Arc<TreeArtifactValue>> {
        let data = self.tree_artifact_data.lock();
        let mut result = IndexMap::new();
        for output in &self.outputs {
            if let Some(Some(v)) = data.get(output) {
                result.insert(output.clone(), v.clone());
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:162 ActionOutputMetadataStore.isKnownOutput
    fn is_known_output(&self, artifact: &Artifact) -> bool {
        self.outputs.contains(artifact)
            || artifact
                .get_parent()
                .is_some_and(|p| self.outputs.contains(p))
    }

    fn artifact_of<'a>(&self, input: &'a ActionInputRef) -> &'a Artifact {
        as_blaze_artifact(input)
            .unwrap_or_else(|| panic!("ClassCastException: {} is not an Artifact", input))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:147 ActionOutputMetadataStore.checkExists
    fn check_exists(
        value: FileArtifactValue,
        artifact: &Artifact,
    ) -> Result<FileArtifactValue, IoOrInterruptedException> {
        if value == FileArtifactValue::MissingFileMarker {
            return Err(IoOrInterruptedException::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{artifact} does not exist"),
            )));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:155 ActionOutputMetadataStore.checkExists
    fn check_tree_exists(
        value: Option<Arc<TreeArtifactValue>>,
        artifact: &Artifact,
    ) -> Result<Arc<TreeArtifactValue>, IoOrInterruptedException> {
        value.ok_or_else(|| {
            IoOrInterruptedException::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{artifact} does not exist"),
            ))
        })
    }

    /// `getTreeArtifactValue(SpecialArtifact)`.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:221 ActionOutputMetadataStore.getTreeArtifactValue
    fn get_tree(
        &self,
        artifact: &Artifact,
    ) -> Result<Arc<TreeArtifactValue>, IoOrInterruptedException> {
        check_state!(
            artifact.is_tree_artifact(),
            "{} is not a tree artifact",
            artifact
        );
        if let Some(value) = self.tree_artifact_data.lock().get(artifact).cloned() {
            return ActionOutputMetadataStore::check_tree_exists(value, artifact);
        }
        let value = self.construct_tree_artifact_value_from_filesystem(artifact)?;
        self.tree_artifact_data
            .lock()
            .insert(artifact.clone(), value.clone());
        ActionOutputMetadataStore::check_tree_exists(value, artifact)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:236 ActionOutputMetadataStore.constructTreeArtifactValueFromFilesystem
    fn construct_tree_artifact_value_from_filesystem(
        &self,
        parent: &Artifact,
    ) -> std::io::Result<Option<Arc<TreeArtifactValue>>> {
        let tree_dir = parent.get_path();
        let chmod = self.execution_mode.load(Ordering::SeqCst);
        let stat = tree_dir.stat_if_found(Symlinks::Follow)?;
        let is_dir = stat.as_ref().is_some_and(|s| s.is_directory());
        if !is_dir {
            if chmod {
                self.set_path_permissions_if_file(&tree_dir)?;
            }
            return Ok(None);
        }
        if chmod {
            // PORT: Java chmods each visited entry (not through symlinks) during the visit.
            set_tree_read_only_and_executable(
                &tree_dir,
                self.output_permissions.get_permissions_mode(),
            )?;
        }
        let mut children = Vec::new();
        visit_tree(
            &tree_dir,
            &PathFragment::empty_fragment(),
            &mut |rel, child| {
                let value = FileArtifactValue::create_for_testing(child)?;
                let child_artifact = Artifact::create_tree_output(parent, rel.clone());
                children.push((artifact_input(&child_artifact), value));
                Ok(())
            },
        )?;
        if self.archived_tree_artifacts_enabled {
            blaze_util::unported!("ArchivedTreeArtifact.createForTree");
        }
        if tree_dir.is_symbolic_link() {
            blaze_util::unported!("TreeArtifactValue.Builder.setResolvedPath");
        }
        Ok(Some(Arc::new(TreeArtifactValue::build(
            children, None, None,
        ))))
    }

    /// Sets the output permissions of a freshly created output file.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:509 ActionOutputMetadataStore.setPathPermissionsIfFile
    fn set_path_permissions_if_file(&self, path: &Path) -> std::io::Result<()> {
        let Some(stat) = path.stat_if_found(Symlinks::Nofollow)? else {
            return Ok(());
        };
        let mode = self.output_permissions.get_permissions_mode();
        if stat.is_file() && (stat.get_permissions() & 0o7777) != mode {
            path.chmod(mode)?;
        }
        Ok(())
    }

    /// `constructFileArtifactValueFromFilesystem`.
    ///
    /// PORT: the value of a regular file is its stat (following symlinks) and digest
    /// (`FileArtifactValue.createFromStat`); a directory is `createForDirectoryWithMtime`. Output
    /// symlinks (`artifact.isSymlink()`) are `FileArtifactValue.createForUnresolvedSymlink`,
    /// not ported.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:404 ActionOutputMetadataStore.constructFileArtifactValueFromFilesystem
    fn construct_file_artifact_value_from_filesystem(
        &self,
        artifact: &Artifact,
    ) -> std::io::Result<FileArtifactValue> {
        check_state!(
            !artifact.is_tree_artifact(),
            "{} is a tree artifact",
            artifact
        );
        let path = artifact.get_path();
        let stat_no_follow = match path.stat_if_found(Symlinks::Nofollow)? {
            Some(s) => s,
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{} does not exist", artifact.pretty_print()),
                ));
            }
        };
        if artifact.is_symlink() {
            let _ = stat_no_follow;
            blaze_util::unported!("FileArtifactValue.createForUnresolvedSymlink");
        }
        FileArtifactValue::create_for_testing(&path)
    }

    /// `prepareForActionExecution()`: enters execution mode and discards cached metadata.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:383 ActionOutputMetadataStore.prepareForActionExecution
    pub fn prepare_for_action_execution(&self) {
        check_state!(
            !self.execution_mode.swap(true, Ordering::SeqCst),
            "Already in execution mode"
        );
        self.artifact_data.lock().clear();
        self.tree_artifact_data.lock().clear();
    }
}

impl OutputMetadataStore for ActionOutputMetadataStore {
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:321 ActionOutputMetadataStore.injectFile
    fn inject_file(&self, output: &ActionInputRef, metadata: FileArtifactValue) {
        let output = self.artifact_of(output);
        blaze_util::check_argument!(
            self.is_known_output(output),
            "{} is not a declared output of this action",
            output
        );
        blaze_util::check_argument!(
            !output.is_tree_artifact() && !output.is_child_of_declared_directory(),
            "Tree artifacts and their children must be injected via injectTree: {}",
            output
        );
        self.put_artifact_data(output, metadata);
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:333 ActionOutputMetadataStore.injectTree
    fn inject_tree(&self, output: &ActionInputRef, tree: TreeArtifactValue) {
        let output = self.artifact_of(output);
        blaze_util::check_argument!(
            self.is_known_output(output),
            "{} is not a declared output of this action",
            output
        );
        blaze_util::check_argument!(
            output.is_tree_artifact(),
            "Output must be a tree artifact: {}",
            output
        );
        self.tree_artifact_data
            .lock()
            .insert(output.clone(), Some(Arc::new(tree)));
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:167 ActionOutputMetadataStore.getOutputMetadata
    fn get_output_metadata(
        &self,
        output: &ActionInputRef,
    ) -> Result<Option<FileArtifactValue>, IoOrInterruptedException> {
        let artifact = self.artifact_of(output);
        if !self.is_known_output(artifact) {
            return Ok(None);
        }
        if artifact.is_runfiles_tree() {
            if let Some(value) = self.artifact_data.lock().get(artifact).cloned() {
                return ActionOutputMetadataStore::check_exists(value, artifact).map(Some);
            }
            self.put_artifact_data(artifact, FileArtifactValue::RunfilesTreeMarker);
            return Ok(Some(FileArtifactValue::RunfilesTreeMarker));
        }
        if artifact.is_tree_artifact() {
            let tree = self.get_tree(artifact)?;
            return Ok(Some(tree.metadata.clone().unwrap_or_else(|| {
                panic!("NullPointerException: TreeArtifactValue.getMetadata() of {artifact}")
            })));
        }
        if artifact.is_child_of_declared_directory() {
            let parent = artifact.get_parent().expect("tree child has a parent");
            let tree = self.get_tree(parent)?;
            let value = tree
                .child_values
                .iter()
                .find(|(c, _)| c.get_exec_path() == artifact.get_exec_path())
                .map(|(_, v)| v.clone())
                .unwrap_or(FileArtifactValue::MissingFileMarker);
            return ActionOutputMetadataStore::check_exists(value, artifact).map(Some);
        }
        if let Some(value) = self.artifact_data.lock().get(artifact).cloned() {
            return ActionOutputMetadataStore::check_exists(value, artifact).map(Some);
        }

        // No existing metadata; this can happen if the output metadata is not injected after a
        // spawn is executed. SkyframeActionExecutor.checkOutputs calls this method for every
        // output file of the action, which hits this code path. Another possibility is that an
        // action runs multiple spawns, and a subsequent spawn requests the metadata of an output
        // of a previous spawn.

        // If necessary, we first call chmod the output file. The FileArtifactValue may use a
        // FileContentsProxy, which is based on ctime (affected by chmod).
        if self.execution_mode.load(Ordering::SeqCst) {
            self.set_path_permissions_if_file(&artifact.get_path())?;
        }

        let value = self.construct_file_artifact_value_from_filesystem(artifact)?;
        self.put_artifact_data(artifact, value.clone());
        ActionOutputMetadataStore::check_exists(value, artifact).map(Some)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:221 ActionOutputMetadataStore.getTreeArtifactValue
    fn get_tree_artifact_value(
        &self,
        tree_artifact: &ActionInputRef,
    ) -> Result<TreeArtifactValue, IoOrInterruptedException> {
        let artifact = self.artifact_of(tree_artifact);
        self.get_tree(artifact).map(|t| (*t).clone())
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:341 ActionOutputMetadataStore.markOmitted
    fn mark_omitted(&self, output: &ActionInputRef) {
        let output = self.artifact_of(output);
        check_state!(
            self.execution_mode.load(Ordering::SeqCst),
            "Tried to mark {} omitted outside of execution",
            output
        );
        self.omitted_outputs.lock().insert(output.clone());
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:348 ActionOutputMetadataStore.artifactOmitted
    fn artifact_omitted(&self, artifact: &ActionInputRef) -> bool {
        self.omitted_outputs
            .lock()
            .contains(self.artifact_of(artifact))
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:353 ActionOutputMetadataStore.resetOutputs
    fn reset_outputs(&self, outputs: &[ActionInputRef]) {
        check_state!(
            self.execution_mode.load(Ordering::SeqCst),
            "resetOutputs() should only be called from within a running action."
        );
        for output in outputs {
            let output = self.artifact_of(output);
            self.omitted_outputs.lock().remove(output);
            if output.is_tree_artifact() {
                self.tree_artifact_data.lock().remove(output);
            } else {
                self.artifact_data.lock().remove(output);
            }
        }
    }
}

/// The members of `Action` the `ActionCacheChecker` reads, for a `blaze_actions` action (Java
/// passes the action itself and `skyframeExecutor.getActionKeyContext()`).
struct CacheCheckActionAdapter<'a> {
    action: &'a dyn Action,
    action_key_context: &'a ActionKeyContext,
}

impl<'a> CacheCheckActionAdapter<'a> {
    fn new(
        action: &'a dyn Action,
        action_key_context: &'a ActionKeyContext,
    ) -> CacheCheckActionAdapter<'a> {
        CacheCheckActionAdapter {
            action,
            action_key_context,
        }
    }
}

impl CacheCheckAction for CacheCheckActionAdapter<'_> {
    // Java: com/google/devtools/build/lib/actions/ActionKeyComputer.java:37 ActionKeyComputer.getKey
    fn get_key(
        &self,
        input_metadata_provider: &dyn InputMetadataProvider,
    ) -> Result<String, InterruptedException> {
        self.action
            .get_key(
                self.action_key_context,
                Some(&KeyMetadataAdapter(input_metadata_provider)),
            )
            .map_err(|_| InterruptedException)
    }

    fn get_inputs(&self) -> NestedSet<ActionInputRef> {
        to_action_input_set(&self.action.get_inputs())
    }

    fn get_mandatory_inputs(&self) -> NestedSet<ActionInputRef> {
        to_action_input_set(&self.action.get_mandatory_inputs())
    }

    fn get_allowed_derived_inputs(&self) -> NestedSet<ActionInputRef> {
        to_action_input_set(&self.action.get_allowed_derived_inputs())
    }

    fn get_outputs(&self) -> Vec<ActionInputRef> {
        self.action
            .get_outputs()
            .iter()
            .map(artifact_input)
            .collect()
    }

    fn get_primary_output(&self) -> ActionInputRef {
        artifact_input(&self.action.get_primary_output())
    }

    fn inputs_known(&self) -> bool {
        self.action.inputs_known()
    }

    fn discovers_inputs(&self) -> bool {
        self.action.discovers_inputs()
    }

    fn update_inputs(&self, inputs: NestedSet<ActionInputRef>) {
        self.action.update_inputs(from_action_input_set(&inputs));
    }

    fn execute_unconditionally(&self) -> bool {
        self.action.execute_unconditionally()
    }

    fn is_volatile(&self) -> bool {
        self.action.is_volatile()
    }

    fn get_client_environment_variables(&self) -> Vec<String> {
        self.action.get_client_environment_variables()
    }

    fn pretty_print(&self) -> String {
        self.action.pretty_print()
    }

    fn as_action_cache_aware_action(&self) -> Option<&dyn ActionCacheAwareAction> {
        // Only StarlarkAction.EnhancedStarlarkAction implements ActionCacheAwareAction.
        let spawn_action = self.action.as_any().downcast_ref::<SpawnAction>()?;
        match spawn_action.get_class() {
            SpawnActionClass::EnhancedStarlark(_) => Some(self),
            _ => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self.action.as_any()
    }

    // `action instanceof TestRunnerAction`.
    fn is_test_runner_action(&self) -> bool {
        blaze_exec::analysis::test::as_test_runner_action(self.action).is_some()
    }
}

impl ActionCacheAwareAction for CacheCheckActionAdapter<'_> {
    // Java: com/google/devtools/build/lib/analysis/actions/StarlarkAction.java:477 EnhancedStarlarkAction.storeInputsExecPathsInActionCache
    fn store_inputs_exec_paths_in_action_cache(&self) -> bool {
        let Some(spawn_action) = self.action.as_any().downcast_ref::<SpawnAction>() else {
            return false;
        };
        match spawn_action.get_class() {
            SpawnActionClass::EnhancedStarlark(d) => {
                d.unused_inputs_list.is_some()
                    || d.shadowed_action
                        .as_ref()
                        .is_some_and(|s| s.discovers_inputs())
            }
            _ => false,
        }
    }
}

/// The execution layer's `InputMetadataProvider` as the key-computation one of `blaze_actions`
/// (which only reads tree artifact children).
struct KeyMetadataAdapter<'a>(&'a dyn InputMetadataProvider);

impl blaze_actions::actions::action::InputMetadataProvider for KeyMetadataAdapter<'_> {
    // Java: com/google/devtools/build/lib/actions/InputMetadataProvider.java:56 InputMetadataProvider.getTreeMetadata
    fn get_tree_children(&self, tree: &Artifact) -> Option<Vec<Artifact>> {
        let value = self.0.get_tree_metadata(&artifact_input(tree))?;
        Some(
            value
                .get_children()
                .iter()
                .map(|c| match as_blaze_artifact(c) {
                    Some(a) => a.clone(),
                    None => blaze_util::unported!(
                        "TreeArtifactValue.getChildren",
                        "tree child {} is not a blaze_actions Artifact",
                        c.0.get_exec_path()
                    ),
                })
                .collect(),
        )
    }
}

/// Converts a `NestedSet<ActionInput>` of artifacts back into a `NestedSet<Artifact>`, keeping
/// the node structure (Java passes the same object).
fn from_action_input_set(set: &NestedSet<ActionInputRef>) -> NestedSet<Artifact> {
    let order = set.get_order();
    let convert = |i: &ActionInputRef| -> Artifact {
        as_blaze_artifact(i)
            .unwrap_or_else(|| panic!("ClassCastException: {i} is not an Artifact"))
            .clone()
    };
    if set.is_empty() {
        return NestedSetBuilder::empty_set(order);
    }
    if set.is_singleton() {
        return NestedSetBuilder::create(order, [convert(set.get_singleton())]);
    }
    let mut b = NestedSetBuilder::new_builder(order);
    for child in set.get_non_leaves() {
        b.add_transitive(&from_action_input_set(&child));
    }
    for leaf in set.get_leaves() {
        b.add(convert(&leaf));
    }
    b.build()
}

/// `ArtifactFactory` as the `ArtifactResolver` of the action cache checker (Java passes
/// `env.getSkyframeBuildView().getArtifactFactory()`).
///
/// `None` (no analysis artifact factory available) resolves no exec paths and fails loudly if
/// asked to resolve any.
pub struct ArtifactFactoryResolver(pub Option<Arc<ArtifactFactory>>);

impl ArtifactResolver for ArtifactFactoryResolver {
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:624 ArtifactFactory.resolveSourceArtifacts
    fn resolve_source_artifacts(
        &self,
        exec_paths: &[PathFragment],
        resolver: &dyn PackageRootResolver,
    ) -> Result<Option<HashMap<PathFragment, ActionInputRef>>, PackageRootExceptionOrInterrupted>
    {
        let Some(factory) = &self.0 else {
            if exec_paths.is_empty() {
                // ArtifactFactory.resolveSourceArtifacts of no paths asks the resolver for no
                // roots and returns an empty map.
                return match resolver.find_package_roots_for_files(&[])? {
                    Some(_) => Ok(Some(HashMap::new())),
                    None => Ok(None),
                };
            }
            blaze_util::unported!(
                "ArtifactResolver.resolveSourceArtifacts (no ArtifactFactory given to the executor)"
            );
        };
        let bridge = PackageRootResolverBridge {
            resolver,
            error: RefCell::new(None),
        };
        match factory.resolve_source_artifacts(exec_paths, &bridge) {
            Ok(Some(m)) => Ok(Some(
                m.into_iter()
                    .filter_map(|(path, artifact)| artifact.map(|a| (path, artifact_input(&a))))
                    .collect(),
            )),
            Ok(None) => Ok(None),
            Err(message) => Err(bridge.error.borrow_mut().take().unwrap_or(
                PackageRootExceptionOrInterrupted::PackageRoot(
                    blaze_exec::actions::action_cache_checker::PackageRootException { message },
                ),
            )),
        }
    }
}

/// The checker's `PackageRootResolver` as the `blaze_actions` one `ArtifactFactory` takes (one
/// Java interface).
struct PackageRootResolverBridge<'a> {
    resolver: &'a dyn PackageRootResolver,
    error: RefCell<Option<PackageRootExceptionOrInterrupted>>,
}

impl blaze_actions::actions::artifact_factory::PackageRootResolver
    for PackageRootResolverBridge<'_>
{
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
    ) -> Result<Option<std::collections::BTreeMap<PathFragment, Option<Root>>>, String> {
        match self.resolver.find_package_roots_for_files(exec_paths) {
            Ok(Some(m)) => Ok(Some(m.into_iter().collect())),
            Ok(None) => Ok(None),
            Err(e) => {
                let message = e.to_string();
                *self.error.borrow_mut() = Some(e);
                Err(message)
            }
        }
    }
}

/// `ActionInputMap` + `ActionInputMetadataProvider`: the metadata of an action's inputs, from
/// their Skyframe values. Inputs discovered by the action are added to the same map.
// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:53 ActionInputMap
// Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:45 ActionInputMetadataProvider
#[allow(clippy::type_complexity)]
pub struct ActionInputMetadata {
    exec_root: Path,
    files: RwLock<HashMap<PathFragment, (ActionInputRef, FileArtifactValue)>>,
    trees: RwLock<HashMap<PathFragment, Arc<TreeArtifactValue>>>,
    /// A runfiles tree's `RunfilesArtifactValue` and its `getMetadata()` (`None` for values built
    /// without the producing action's input metadata).
    runfiles:
        RwLock<HashMap<PathFragment, (Arc<RunfilesArtifactValue>, Option<FileArtifactValue>)>>,
    /// PORT: metadata of files the map does not hold (runfiles members, files written by an
    /// earlier spawn of the same action), stat'ed on first use; see `get_input_metadata`.
    stat_cache: Mutex<HashMap<PathFragment, Option<FileArtifactValue>>>,
}

impl ActionInputMetadata {
    pub fn new(exec_root: Path) -> ActionInputMetadata {
        ActionInputMetadata {
            exec_root,
            files: RwLock::new(HashMap::new()),
            trees: RwLock::new(HashMap::new()),
            runfiles: RwLock::new(HashMap::new()),
            stat_cache: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:236 ActionInputMap.put
    pub fn put(&self, input: &Artifact, value: FileArtifactValue) {
        // putIfAbsent: the first metadata for an exec path wins.
        self.files
            .write()
            .entry(input.get_exec_path().clone())
            .or_insert_with(|| (artifact_input(input), value));
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:264 ActionInputMap.putTreeArtifact
    pub fn put_tree_artifact(&self, tree: &Artifact, value: Arc<TreeArtifactValue>) {
        self.trees
            .write()
            .entry(tree.get_exec_path().clone())
            .or_insert(value);
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:285 ActionInputMap.putRunfilesMetadata
    pub fn put_runfiles_metadata(
        &self,
        input: &Artifact,
        value: Arc<RunfilesArtifactValue>,
        metadata: Option<FileArtifactValue>,
    ) {
        self.runfiles
            .write()
            .entry(input.get_exec_path().clone())
            .or_insert((value, metadata));
    }

    /// `ActionInputMap.getInputMetadata(input) != null`: whether the map (not the file system
    /// fallback) knows `input`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1256 InputDiscoveryState.getKnownDiscoveredInputMetadata
    pub fn is_known(&self, input: &Artifact) -> bool {
        let exec_path = input.get_exec_path();
        self.files.read().contains_key(exec_path)
            || self.trees.read().contains_key(exec_path)
            || self.runfiles.read().contains_key(exec_path)
    }
}

impl ActionInputMetadata {
    /// The `ActionInputMetadataProvider` over the map alone, without the file-system fallback
    /// that stands in for Java's `SkyframeInputMetadataProvider` (Java's
    /// `state.actionInputMetadataProvider`, used for the action cache check).
    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:45 ActionInputMetadataProvider
    pub fn action_input_map_only(&self) -> ActionInputMapOnly<'_> {
        ActionInputMapOnly(self)
    }

    fn get_input_metadata_impl(
        &self,
        input: &ActionInputRef,
        file_system_fallback: bool,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        if let Some(v) = input.as_virtual_action_input() {
            if v.is_empty_action_input() {
                return Ok(None);
            }
            if let Some(m) = v.get_metadata()? {
                return Ok(Some(m));
            }
            let bytes = v.get_bytes()?;
            use sha2::Digest as _;
            return Ok(Some(FileArtifactValue::create_for_virtual_action_input(
                sha2::Sha256::digest(&bytes).to_vec(),
                bytes.len() as i64,
            )));
        }
        let exec_path = input.get_exec_path();
        // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:212 ActionInputMap.getInputMetadataChecked
        if input.as_artifact().is_some_and(|a| a.is_runfiles_tree()) {
            if let Some((_, metadata)) = self.runfiles.read().get(exec_path) {
                return match metadata {
                    Some(m) => Ok(Some(m.clone())),
                    None => blaze_util::unported!(
                        "RunfilesArtifactValue.getMetadata (value built without its action's input metadata)"
                    ),
                };
            }
        }
        // A tree artifact's metadata is its TreeArtifactValue.getMetadata().
        if input.as_artifact().is_some_and(|a| a.is_tree_artifact())
            && let Some(tree) = self.trees.read().get(exec_path).cloned()
        {
            return Ok(Some(tree.metadata.clone().unwrap_or_else(|| {
                panic!("NullPointerException: TreeArtifactValue.getMetadata() of {exec_path}")
            })));
        }
        if let Some((_, v)) = self.files.read().get(exec_path) {
            return Ok(Some(v.clone()));
        }
        if !file_system_fallback {
            return Ok(None);
        }
        // PORT: Java consults ActionInputMap, then the per-build file cache
        // (SkyframeInputMetadataProvider) for runfiles members and outputs of earlier spawns.
        // In a single non-incremental build those values equal a stat of the file, done here.
        if let Some(cached) = self.stat_cache.lock().get(exec_path) {
            return Ok(cached.clone());
        }
        let path = self.exec_root.get_relative(exec_path);
        let value = if path.exists() {
            Some(FileArtifactValue::create_for_testing(&path)?)
        } else {
            None
        };
        self.stat_cache
            .lock()
            .insert(exec_path.clone(), value.clone());
        Ok(value)
    }
}

/// See [`ActionInputMetadata::action_input_map_only`].
pub struct ActionInputMapOnly<'a>(&'a ActionInputMetadata);

impl InputMetadataProvider for ActionInputMapOnly<'_> {
    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:76 ActionInputMetadataProvider.getInputMetadataChecked
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        self.0.get_input_metadata_impl(input, false)
    }

    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.0.get_tree_metadata(input)
    }

    fn get_fileset(&self, input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        self.0.get_fileset(input)
    }

    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.0.get_runfiles_metadata(input)
    }

    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.0.get_input(exec_path)
    }
}

impl InputMetadataProvider for ActionInputMetadata {
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        self.get_input_metadata_impl(input, true)
    }

    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees.read().get(input.get_exec_path()).cloned()
    }

    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles
            .read()
            .get(input.get_exec_path())
            .map(|(v, _)| v.clone())
    }

    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.files.read().get(exec_path).map(|(i, _)| i.clone())
    }
}

/// The `ActionExecutionContext` for one action (or, without an output metadata store, the
/// `ActionExecutionContext.forInputDiscovery` of one action).
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:59 ActionExecutionContext
struct ActionExecutionContextImpl<'a> {
    executor: &'a SkyframeActionExecutor,
    input_metadata: Arc<ActionInputMetadata>,
    output_metadata_store: Option<Arc<ActionOutputMetadataStore>>,
    file_out_err: FileOutErr,
    rich_artifact_data: Mutex<Option<Arc<RunfilesArtifactValue>>>,
    clock: JavaClock,
}

impl ActionContextRegistry for ActionExecutionContextImpl<'_> {
    fn get_spawn_strategy_registry(&self) -> Option<Arc<SpawnStrategyRegistry>> {
        self.executor.contexts.spawn_strategy_registry.clone()
    }
    fn get_spawn_log_context(&self) -> Option<Arc<dyn SpawnLogContext>> {
        self.executor.contexts.spawn_log_context.clone()
    }
    fn get_spawn_cache(&self) -> Option<Arc<dyn SpawnCache>> {
        self.executor.contexts.spawn_cache.clone()
    }
    fn get_context_any(&self, type_id: TypeId) -> Option<ActionContextRef> {
        self.executor.contexts.get_any(type_id)
    }
}

impl ActionExecutionContext for ActionExecutionContextImpl<'_> {
    fn get_exec_root(&self) -> &Path {
        &self.executor.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &*self.input_metadata
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.executor.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        &*self.executor.reporter
    }
    fn get_clock(&self) -> &dyn Clock {
        &self.clock
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:459 ActionExecutionContext.maybeReportSubcommand
    fn maybe_report_subcommand(&self, spawn: &dyn Spawn, spawn_runner: Option<&str>) {
        blaze_exec::actions::action_execution::maybe_report_subcommand(
            self.executor.show_subcommands,
            spawn,
            spawn_runner,
            &self.executor.exec_root,
            &*self.executor.reporter,
        );
    }
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {
        // PORT: progress events (SchedulingActionEvent, RunningActionEvent) feed the UI only.
    }
    fn get_verbose_failures(&self) -> bool {
        self.executor.verbose_failures
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
    fn get_input_path(&self, input: &dyn ActionInput) -> Path {
        // ArtifactPathResolver.IDENTITY: an artifact resolves to `artifact.getPath()` (source
        // artifacts live under their source root); other inputs under the execroot.
        match input.as_any().downcast_ref::<Artifact>() {
            Some(a) => a.get_path(),
            None => self.executor.exec_root.get_relative(input.get_exec_path()),
        }
    }
    fn set_rich_artifact_data(&self, data: Arc<RunfilesArtifactValue>) {
        *self.rich_artifact_data.lock() = Some(data);
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:570 ActionExecutionContext.withOutputsAsInputs
    fn get_output_metadata(&self, output: &dyn ActionInput) -> std::io::Result<FileArtifactValue> {
        let Some(store) = &self.output_metadata_store else {
            panic!("NullPointerException: no OutputMetadataStore during input discovery");
        };
        let Some(artifact) = output.as_any().downcast_ref::<Artifact>() else {
            panic!(
                "ClassCastException: {} is not an Artifact",
                output.get_exec_path()
            );
        };
        match store.get_output_metadata(&artifact_input(artifact)) {
            Ok(Some(v)) => Ok(v),
            // ImmutableMap.Builder.put(input, null).
            Ok(None) => panic!("NullPointerException: null value in entry: {artifact}=null"),
            Err(IoOrInterruptedException::Io(e)) => Err(e),
            Err(IoOrInterruptedException::Interrupted(_)) => {
                Err(std::io::Error::other("interrupted"))
            }
        }
    }
}

/// The `blaze_actions` artifact behind an execution-layer input, if any.
pub fn input_artifact(input: &ActionInputRef) -> Option<&Artifact> {
    as_blaze_artifact(input)
}

/// The error texts of `SkyframeActionExecutor.reportMissingOutputFile`,
/// `reportOutputTreeArtifactErrors` and `getEventForUnsoundDirectory` (Java strings).
#[cfg(test)]
mod output_error_text_tests {
    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};

    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::actions::platform_info::empty_platform_info;
    use blaze_actions::analysis::actions::file_write_action::{Compression, FileWriteAction};
    use blaze_actions::{ActionLookupKeyRef, ActionOwner, Artifact, ArtifactRoot, RootType};
    use blaze_cmdline::Label;
    use blaze_events::location::Location;
    use blaze_events::{EventKind, StoredEventHandler};
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};
    use indexmap::IndexMap;

    use super::{
        get_event_for_unsound_directory, report_missing_output_file,
        report_output_tree_artifact_errors,
    };

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct Key(String);

    impl fmt::Display for Key {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "ConfiguredTargetKey{{label={}}}", self.0)
        }
    }

    impl ActionLookupKey for Key {
        fn get_label(&self) -> Option<Label> {
            Some(Label::parse_canonical(&self.0).unwrap())
        }
        fn get_configuration_checksum(&self) -> Option<String> {
            Some("abc".to_string())
        }
        fn function_name(&self) -> &'static str {
            "CONFIGURED_TARGET"
        }
        fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
            other.as_any().downcast_ref::<Key>() == Some(self)
        }
        fn key_hash(&self) -> u64 {
            let mut h = DefaultHasher::new();
            self.hash(&mut h);
            h.finish()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn owner() -> ActionOwner {
        ActionOwner::create_dummy(
            Some(Label::parse_canonical("//p:t").unwrap()),
            Location::from_file_line_column("/ws/p/BUILD", 1, 1),
            "genrule rule",
            "k8-fastbuild",
            "abc",
            None,
            false,
            Some(empty_platform_info()),
            Vec::new(),
            IndexMap::new(),
        )
    }

    fn output() -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = fs.get_path("/execroot");
        let root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["bazel-out", "bin"]);
        let exec_path = root.get_exec_path().get_relative_str("p/out");
        Artifact::create_derived_artifact(
            root,
            exec_path,
            ActionLookupKeyRef::new(Key("//p:t".to_string())),
        )
    }

    fn action(output: &Artifact) -> FileWriteAction {
        FileWriteAction::create(
            owner(),
            output.clone(),
            "x".to_string(),
            false,
            Compression::DISALLOW,
        )
    }

    fn messages(events: &StoredEventHandler) -> Vec<(EventKind, String)> {
        events
            .get_events()
            .iter()
            .map(|e| (e.get_kind(), e.get_message().to_string()))
            .collect()
    }

    #[test]
    fn missing_output_of_non_genrule() {
        let out = output();
        let events = StoredEventHandler::new();
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        report_missing_output_file(&action(&out), &out, &events, false, &not_found);
        assert_eq!(
            messages(&events),
            vec![(
                EventKind::Error,
                "output 'p/out' was not created".to_string()
            )]
        );
    }

    #[test]
    fn dangling_symlink_output_of_non_genrule() {
        let out = output();
        let events = StoredEventHandler::new();
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        report_missing_output_file(&action(&out), &out, &events, true, &not_found);
        assert_eq!(
            messages(&events),
            vec![(
                EventKind::Error,
                "output 'p/out' is a dangling symbolic link".to_string()
            )]
        );
    }

    #[test]
    fn missing_tree_artifact() {
        let out = output();
        let events = StoredEventHandler::new();
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        report_output_tree_artifact_errors(&action(&out), &out, &events, &not_found);
        assert_eq!(
            messages(&events),
            vec![(
                EventKind::Error,
                "output tree artifact p/out was not created".to_string()
            )]
        );
    }

    #[test]
    fn invalid_tree_artifact() {
        let out = output();
        let events = StoredEventHandler::new();
        let e = std::io::Error::other("child x has an unsupported type");
        report_output_tree_artifact_errors(&action(&out), &out, &events, &e);
        assert_eq!(
            messages(&events),
            vec![(
                EventKind::Error,
                "error while validating output tree artifact p/out: child x has an unsupported type"
                    .to_string()
            )]
        );
    }

    #[test]
    fn unsound_directory_output_event() {
        let out = output();
        let event = get_event_for_unsound_directory(
            EventKind::Error,
            |s| format!("output {s} is a directory but was not declared as such"),
            &out,
            &owner(),
        );
        assert_eq!(
            event.get_message(),
            "output 'p/out' of //p:t is a directory but was not declared as such"
        );
        assert_eq!(event.get_tag(), Some("//p:t"));
    }
}
