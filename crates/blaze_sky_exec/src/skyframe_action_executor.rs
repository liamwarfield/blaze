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
use blaze_exec::actions::action_adapters::{
    ActionMetadataSnapshot, artifact_input, as_blaze_artifact,
};
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
    CacheCheckAction, FileArtifactValue, FileContentsProxy, FilesetOutputTree,
    InputMetadataProvider, LostArtifacts, OutputChecker, RunfilesArtifactValue, Spawn,
    TreeArtifactValue, as_executable,
};
use blaze_exec::analysis::actions::runfiles_tree_action::create_runfiles_artifact_value;
use blaze_exec::analysis::actions::to_action_input_set;
use blaze_exec::analysis::test::test_action_context::TestActionContext;
use blaze_exec::analysis::test::test_runner_action::TestRunnerAction;
use blaze_exec::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, ActionFinalizer, ActionInputPrefetcher,
    RemoteOutputTree, SpawnCache,
};
use blaze_exec::exec::evaluation_stack::on_evaluation_stack;
use blaze_exec::exec::executor_setup::LostInputsPolicy;
use blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem;
use blaze_exec::exec::single_build_file_cache::SingleBuildFileCache;
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::action_execution_context::ShowSubcommands;
use blaze_protos::failure_details::{FailureDetail, execution};
use blaze_skyframe::Environment;
use blaze_util::check_state;
use blaze_vfs::{DirentType, Path, PathFragment, Root, Symlinks};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Mutex, RwLock};

use crate::action_execution_value::ActionExecutionValue;
use crate::artifact_function::execution_failure;
use crate::events::{
    ActionCompletionEvent, ActionExecutedEvent, ActionResultReceivedEvent, ActionStartedEvent,
    CachingActionEvent, ErrorTiming, RunningActionEvent, ScanningActionEvent,
    SchedulingActionEvent, StoppedScanningActionEvent,
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
    /// `<output_base>/external` (Java `directories.getOutputBase().getRelative(
    /// LabelConstants.EXTERNAL_PATH_PREFIX)`), which `SkyframeExecutor.skyFunctions` hands
    /// `RecursiveFilesystemTraversalFunction`.
    pub external_directory: Path,
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
/// The build's implementation is
/// [`crate::action_execution_function::SkyframeContainingPackageRootLookup`], which ports the body
/// of `ActionExecutionFunction.PackageRootResolverWithEnvironment.findPackageRootsForFiles`
/// (ActionExecutionFunction.java:602-662) after its `checkState`. `None` (tests without loading
/// functions) fails loudly when an action cache entry has source paths to resolve.
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
    /// `outputService.getOutputChecker()`: `None` is `LocalOutputService`'s
    /// `OutputChecker.TRUST_LOCAL_ONLY`; the remote module's `RemoteOutputService` gives its
    /// `RemoteOutputChecker`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:782 SkyframeActionExecutor.checkActionCache
    pub output_checker: Option<Arc<dyn OutputChecker>>,
}

/// `ActionRewindStrategy.MAX_LOST_INPUTS_RECORDED`.
// Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindStrategy.java:96 ActionRewindStrategy.MAX_LOST_INPUTS_RECORDED
const MAX_LOST_INPUTS_RECORDED: usize = 5;

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

/// The (shared) execution of the action that generates one primary output: the
/// `ActionLookupData` of the Skyframe node that runs it, that action, and its result.
// Java: com/google/devtools/build/lib/skyframe/ActionExecutionState.java:60 ActionExecutionState
#[derive(Clone)]
struct ActionExecutionState {
    action_lookup_data: ActionLookupData,
    /// The action of `action_lookup_data` (Java reaches it through the state's `ActionRunner`).
    action: ActionRef,
    result: Arc<OnceLock<SharedResult>>,
}

/// Returns true iff the two actions can be shared: an equivalence relationship across actions,
/// based on the action class, the key, and the list of inputs and outputs.
///
/// PORT: `Actions` belongs to `blaze_actions` (which has no port of this method yet).
// Java: com/google/devtools/build/lib/actions/Actions.java:85 Actions.canBeShared
fn can_be_shared(
    action_key_context: &ActionKeyContext,
    a: &dyn Action,
    b: &dyn Action,
) -> Result<bool, ExecutionFailure> {
    // The action keys may run Starlark callbacks (map_each): an evaluation-sized stack.
    let (key_a, key_b) = on_evaluation_stack(|| {
        (
            a.get_key(action_key_context, None).ok(),
            b.get_key(action_key_context, None).ok(),
        )
    });
    let key_a = key_a.ok_or(ExecutionFailure::Interrupted)?;
    let key_b = key_b.ok_or(ExecutionFailure::Interrupted)?;
    if !(a.get_mnemonic() == b.get_mnemonic() && key_a == key_b) {
        return Ok(false);
    }
    // Uses a standard comparison technique for shareable actions.
    if a.is_shareable() && b.is_shareable() {
        return Ok(artifacts_equal_without_owner(
            &a.get_mandatory_inputs().to_list(),
            &b.get_mandatory_inputs().to_list(),
        ) && artifacts_equal_without_owner(&a.get_outputs(), &b.get_outputs()));
    }

    // If this is reached, at least one action is not shareable. If the actions differ on this,
    // they cannot be shared.
    if a.is_shareable() || b.is_shareable() {
        return Ok(false);
    }

    // If the artifacts are in fact equal (with owners), these are aliases of the same action and
    // not in conflict with each other.
    Ok(
        *a.get_mandatory_inputs().to_list() == *b.get_mandatory_inputs().to_list()
            && a.get_outputs() == b.get_outputs(),
    )
}

// Java: com/google/devtools/build/lib/actions/Actions.java:145 Actions.artifactsEqualWithoutOwner
fn artifacts_equal_without_owner(collection1: &[Artifact], collection2: &[Artifact]) -> bool {
    if collection1.len() != collection2.len() {
        return false;
    }
    collection1
        .iter()
        .zip(collection2)
        .all(|(artifact1, artifact2)| artifact1.equals_without_owner(artifact2))
}

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
    external_directory: Path,
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
    /// See [`ActionCacheSetup::output_checker`].
    output_checker: Option<Arc<dyn OutputChecker>>,
    /// Java: `outputDirectoryHelper` (`SkyframeExecutor`'s `ActionOutputDirectoryHelper`).
    output_directory_helper: ActionOutputDirectoryHelper,
    /// The reporter's `OutputFilter` (`--output_filter` / `--auto_output_filter`); `None` is
    /// `OutputFilter.OUTPUT_EVERYTHING`.
    output_filter: Option<Arc<dyn OutputFilter>>,
    /// `ExecutionOptions.showSubcommands` (`--subcommands`; `executor.reportsSubcommands()`).
    show_subcommands: ShowSubcommands,
    /// See [`SkyframeActionExecutor::with_lost_inputs_policy`].
    lost_inputs_policy: Option<LostInputsPolicy>,
    /// Whether the policy's `purge_before_unported_rewinding` purged this build's action cache.
    /// The purge holds the write lock; [`SkyframeActionExecutor::update_action_cache`] holds the
    /// read lock while it writes the cache and writes nothing once this is set.
    ///
    /// PORT: no Java counterpart (Java rewinds instead of failing loudly; see
    /// `handle_lost_inputs`). The build crashes after the purge, but actions in flight on other
    /// evaluator threads still finish while the evaluation is torn down; an entry they put would
    /// reach the cache's journal on disk and bring remote metadata back for the next command.
    action_cache_purged: RwLock<bool>,
    /// Java: `perBuildFileCache` (`getPerBuildFileCache()`), set by
    /// [`SkyframeActionExecutor::configure_file_cache`]. It also carries
    /// `directories.getRelativeOutputPath()`, which `ActionExecutionFunction` hands the
    /// `SkyframeInputMetadataProvider` of each action.
    per_build_file_cache: OnceLock<Arc<SingleBuildFileCache>>,
    /// Java: `buildActionMap`. Shared actions (same primary output) execute once; the entry
    /// records the `ActionLookupData` of the action that runs (`ActionExecutionState
    /// .actionLookupData`).
    build_action_map: Mutex<HashMap<OwnerlessArtifactWrapper, ActionExecutionState>>,
    had_execution_error: AtomicBool,
    /// Java: `ActionLogBufferPathGenerator.actionUniquifier`.
    action_uniquifier: AtomicU64,
    /// Java: `progressSupplier` and `completionReceiver`, set by
    /// [`SkyframeActionExecutor::set_action_execution_progress_reporting_objects`].
    progress_reporting: RwLock<ProgressReportingObjects>,
}

/// `progressSupplier` and `completionReceiver` (both `@Nullable`).
#[derive(Default)]
struct ProgressReportingObjects {
    progress_supplier: Option<Arc<dyn ProgressSupplier>>,
    completion_receiver: Option<Arc<dyn ActionCompletedReceiver>>,
}

/// An object supplying data for action execution progress reporting.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1988 SkyframeActionExecutor.ProgressSupplier
pub trait ProgressSupplier: Send + Sync {
    /// Returns the progress string to prefix action execution messages with.
    fn get_progress_string(&self) -> String;
}

/// An object that can be notified about action completion.
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1994 SkyframeActionExecutor.ActionCompletedReceiver
pub trait ActionCompletedReceiver: Send + Sync {
    /// Receives a completed action.
    fn action_completed(&self, action_lookup_data: &ActionLookupData);
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
            external_directory: config.external_directory,
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
            output_checker: None,
            output_directory_helper: ActionOutputDirectoryHelper::new(),
            output_filter: None,
            show_subcommands: ShowSubcommands::FALSE,
            lost_inputs_policy: None,
            action_cache_purged: RwLock::new(false),
            per_build_file_cache: OnceLock::new(),
            build_action_map: Mutex::new(HashMap::new()),
            had_execution_error: AtomicBool::new(false),
            action_uniquifier: AtomicU64::new(0),
            progress_reporting: RwLock::new(ProgressReportingObjects::default()),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1999 SkyframeActionExecutor.setActionExecutionProgressReportingObjects
    pub fn set_action_execution_progress_reporting_objects(
        &self,
        progress_supplier: Option<Arc<dyn ProgressSupplier>>,
        completion_receiver: Option<Arc<dyn ActionCompletedReceiver>>,
    ) {
        *self.progress_reporting.write() = ProgressReportingObjects {
            progress_supplier,
            completion_receiver,
        };
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1488 SkyframeActionExecutor.prependExecPhaseStats
    fn prepend_exec_phase_stats(&self, message: &str) -> String {
        let progress_supplier = self.progress_reporting.read().progress_supplier.clone();
        match progress_supplier {
            None => String::new(),
            Some(progress_supplier) => {
                format!("{} {}", progress_supplier.get_progress_string(), message)
            }
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
        self.output_checker = setup.output_checker;
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

    /// Sets `rewindingEnabled` / `invocationRetriesEnabled` and the `LostInputsEvent` handler
    /// (`None`: a build without a remote cache, where no input can be lost).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:355 SkyframeActionExecutor.prepareForExecution
    pub fn with_lost_inputs_policy(
        mut self,
        lost_inputs_policy: Option<LostInputsPolicy>,
    ) -> SkyframeActionExecutor {
        self.lost_inputs_policy = lost_inputs_policy;
        self
    }

    /// `ActionExecutionFunction.handleLostInputs` →
    /// `ActionRewindStrategy.prepareRewindPlanForLostInputs` for an action whose execution lost
    /// inputs, with the `ActionRewindException` it throws turned into the action's failure
    /// (`new ActionExecutionException(rewindingFailedException, action, false,
    /// rewindingFailedException.getDetailedExitCode())`).
    ///
    /// PORT: Java's `SkyframeActionExecutor` rethrows a `LostInputsActionExecutionException`
    /// unreported and `ActionExecutionFunction` handles it; here the executor handles it where
    /// it would report the failure, with the same result. Action rewinding itself
    /// (`--rewind_lost_inputs`, Bazel's default) is not ported and fails loudly.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:460 ActionExecutionFunction.handleLostInputs
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindStrategy.java:229 ActionRewindStrategy.prepareRewindPlanForLostInputs
    // Java: com/google/devtools/build/lib/skyframe/rewinding/ActionRewindStrategy.java:307 ActionRewindStrategy.checkRewindingEnabled
    fn handle_lost_inputs(
        &self,
        action: &dyn Action,
        lost_inputs_exception: &ActionExecutionException,
    ) -> ActionExecutionException {
        let lost_artifacts: &LostArtifacts = lost_inputs_exception
            .get_lost_inputs()
            .expect("a LostInputsActionExecutionException");
        // LostType.INPUT
        let (description, code_when_disabled) = (
            "inputs",
            blaze_protos::failure_details::action_rewinding::Code::LostInputRewindingDisabled,
        );
        let Some(policy) = &self.lost_inputs_policy else {
            panic!(
                "IllegalStateException: {} lost inputs in a build without a remote cache",
                action.pretty_print()
            );
        };
        if policy.rewinding_enabled {
            // PORT: Java rewinds here. Before failing loudly, forget the remote metadata of this
            // build's action cache (what LeaseService.handleMissingInputs does without
            // rewinding) and save it, so that a rerun does not trust the same stale entry again.
            // No action writes the cache while the purge runs, and none after it (see
            // action_cache_purged).
            if let Some(purge) = &policy.purge_before_unported_rewinding {
                let mut purged = self.action_cache_purged.write();
                purge(self.action_cache_checker.get_action_cache());
                *purged = true;
            }
            blaze_util::unported!(
                "ActionRewindStrategy.prepareRewindPlanForLostInputs",
                "action rewinding (--rewind_lost_inputs, the default) for the lost inputs of {}; \
                 --norewind_lost_inputs retries the build instead",
                action.pretty_print()
            );
        }
        let pretty_print = |input: &ActionInputRef| match as_blaze_artifact(input) {
            Some(a) => a.pretty_print(),
            None => input.get_exec_path().get_path_string().to_string(),
        };
        let (message, failure_detail) = if policy.invocation_retries_enabled {
            // Bazel's (but not Blaze's) remote implementation needs to learn about lost digests
            // so that the retried invocation doesn't accept the same stale action result.
            let missing_digests: Vec<String> = lost_artifacts.by_digest.keys().cloned().collect();
            (policy.on_lost_inputs)(&missing_digests);
            // When action rewinding is disabled, recover by retrying the invocation in
            // BlazeCommandDispatcher instead.
            // FallbackToBuildRewindingException
            let message = format!(
                "Lost {} no longer available remotely: {}",
                description,
                lost_artifacts
                    .entries()
                    .into_iter()
                    .take(MAX_LOST_INPUTS_RECORDED)
                    .map(|(digest, input)| format!("{} ({})", pretty_print(input), digest))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let failure_detail = blaze_protos::failure_details::FailureDetail {
                message: message.clone(),
                category: Some(
                    blaze_protos::failure_details::failure_detail::Category::Spawn(
                        blaze_protos::failure_details::Spawn {
                            code: blaze_protos::failure_details::spawn::Code::RemoteCacheEvicted
                                as i32,
                            ..Default::default()
                        },
                    ),
                ),
            };
            (message, failure_detail)
        } else {
            // GenericActionRewindException
            let message = format!(
                "Unexpected lost {} (pass --rewind_lost_inputs to enable recovery): {}",
                description,
                lost_artifacts
                    .entries()
                    .into_iter()
                    .map(|(_, input)| pretty_print(input))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let failure_detail = blaze_protos::failure_details::FailureDetail {
                message: message.clone(),
                category: Some(
                    blaze_protos::failure_details::failure_detail::Category::ActionRewinding(
                        blaze_protos::failure_details::ActionRewinding {
                            code: code_when_disabled as i32,
                        },
                    ),
                ),
            };
            (message, failure_detail)
        };
        ActionExecutionException::new(message, Some(action), false, Some(failure_detail))
    }

    /// A `LostInputsActionExecutionException` that leaves `ActionRunner.actuallyCompleteAction`
    /// (outputs or inputs lost while completing the action) and reaches
    /// `ActionExecutionFunction.handleLostInputs`: the rewinding failure, reported as
    /// `processAndGetExceptionToThrow(env.getListener(), e.getPrimaryOutputPath(), action, ...,
    /// e.getFileOutErr(), ErrorTiming.AFTER_EXECUTION)` reports it.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:509 ActionExecutionFunction.handleLostInputs
    fn handle_lost_inputs_of_completion(
        &self,
        listener: &dyn ExtendedEventHandler,
        primary_output_path: &Path,
        action: &ActionRef,
        lost_inputs_exception: &ActionExecutionException,
        out_err_buffer: &FileOutErr,
    ) -> ActionExecutionException {
        let e = self.handle_lost_inputs(action.as_ref(), lost_inputs_exception);
        self.process_and_get_exception_to_throw(
            listener,
            primary_output_path,
            action,
            e,
            out_err_buffer,
            ErrorTiming::AFTER_EXECUTION,
        )
    }

    /// `maybeSignalLostInputs(e, primaryOutputPath)`: action failures may be caused by lost
    /// inputs. Lost input failures have higher priority because rewinding may be able to restore
    /// what was lost and allow the action to complete without error. `Err` is the
    /// `LostInputsActionExecutionException` Java throws (then the failure is not reported here);
    /// `Ok` gives `e` back when no input was lost.
    ///
    /// PORT: `lostInputsException.setActionStartedEventAlreadyEmitted()` and
    /// `enrichLostInputsException` (primary action, primary output path, out/err) decorate the
    /// exception for `ActionExecutionFunction.handleLostInputs`; the callers here pass the
    /// primary output path and out/err to [`Self::handle_lost_inputs`]'s report directly, and
    /// the `ActionCompletionEvent` is posted by `run_action` either way (Java's `handleLostInputs`
    /// posts it when rewinding fails). The primary-action check and the started-event flag only
    /// matter to action rewinding, which is not ported.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1220 ActionRunner.maybeSignalLostInputs
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1557 SkyframeActionExecutor.enrichLostInputsException
    fn maybe_signal_lost_inputs(
        e: ActionExecutionException,
        action: &dyn Action,
        context: &ActionExecutionContextImpl<'_>,
    ) -> Result<ActionExecutionException, ActionExecutionException> {
        if e.get_lost_inputs().is_some() {
            return Err(e);
        }
        match Self::check_action_file_system_for_lost_inputs(
            context.get_remote_action_file_system().as_ref(),
            action,
        ) {
            Ok(()) => Ok(e),
            Err(lost_inputs_exception) => Err(lost_inputs_exception),
        }
    }

    /// Validates that all action input contents were not lost if they were read, and if an
    /// action file system was used. Fails with a `LostInputsActionExecutionException` describing
    /// the lost inputs if any were (`outputService.checkActionFileSystemForLostInputs`:
    /// `RemoteOutputService` asks a `RemoteActionFileSystem`, the only action file system here).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1606 SkyframeActionExecutor.checkActionFileSystemForLostInputs
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:233 RemoteOutputService.checkActionFileSystemForLostInputs
    fn check_action_file_system_for_lost_inputs(
        action_file_system: Option<&Arc<RemoteActionFileSystem>>,
        action: &dyn Action,
    ) -> Result<(), ActionExecutionException> {
        if let Some(action_file_system) = action_file_system
            && let Some(lost_inputs) = Self::action_file_system_lost_inputs(action_file_system)
        {
            // throw (LostInputsActionExecutionException)
            //     ActionExecutionException.fromExecException(mergedException, action)
            // Java: com/google/devtools/build/lib/remote/RemoteActionFileSystem.java:914 RemoteActionFileSystem.checkForLostInputs
            return Err(ActionExecutionException::from_exec_exception(
                &blaze_exec::actions::LostInputsExecException::create(lost_inputs),
                None,
                action,
            ));
        }
        Ok(())
    }

    /// The merged record of the inputs the action file system found lost
    /// (`RemoteActionFileSystem.checkForLostInputs`), `None` when nothing was lost. The one place
    /// that asks the overlay.
    fn action_file_system_lost_inputs(
        action_file_system: &RemoteActionFileSystem,
    ) -> Option<LostArtifacts> {
        action_file_system.check_for_lost_inputs()
    }

    /// Sets `--subcommands` (`BlazeExecutor.reportsSubcommands`).
    pub fn with_show_subcommands(
        mut self,
        show_subcommands: ShowSubcommands,
    ) -> SkyframeActionExecutor {
        self.show_subcommands = show_subcommands;
        self
    }

    /// The `fileCache` argument of Java's `configure`: the per-build file cache
    /// (`CommandEnvironment.getFileCache`: `new SingleBuildFileCache(execRoot,
    /// PathFragment.create(directories.getRelativeOutputPath()), fs, syscallCache)`), which
    /// `ExecutionTool` hands the executor once per build.
    // Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:1044 CommandEnvironment.getFileCache
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1035 SkyframeActionExecutor.configure
    pub fn configure_file_cache(&self, file_cache: Arc<SingleBuildFileCache>) {
        if self.per_build_file_cache.set(file_cache).is_err() {
            // One build per executor: Java replaces the cache of the previous build.
            panic!("IllegalStateException: the per-build file cache is already configured");
        }
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

    /// See [`SkyframeActionExecutorConfig::external_directory`].
    pub fn get_external_directory(&self) -> &Path {
        &self.external_directory
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
    ///
    /// `action_file_system` is the resolver (`createPathResolver(actionFileSystem, execRoot)`):
    /// the paths are converted to it when there is one.
    // Java: com/google/devtools/build/lib/actions/ActionLogBufferPathGenerator.java:36 ActionLogBufferPathGenerator.generate
    fn generate_file_out_err(
        &self,
        action_file_system: Option<&Arc<RemoteActionFileSystem>>,
    ) -> FileOutErr {
        let action_id = self.action_uniquifier.fetch_add(1, Ordering::SeqCst) + 1;
        let convert_path = |path: Path| match action_file_system {
            Some(file_system) => file_system.get_path(path.as_fragment()),
            None => path,
        };
        FileOutErr::new(
            convert_path(
                self.action_out_dir
                    .get_child(&format!("stdout-{action_id}")),
            ),
            convert_path(
                self.action_out_dir
                    .get_child(&format!("stderr-{action_id}")),
            ),
        )
    }

    /// `ActionOutputMetadataStore.create(useArchivedTreeArtifacts(action), getOutputPermissions(),
    /// action.getOutputs(), ...)`, as `ActionExecutionFunction.checkCacheAndExecuteIfNeeded`
    /// creates it.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:713 ActionExecutionFunction.checkCacheAndExecuteIfNeeded
    pub fn create_output_metadata_store(
        &self,
        action: &dyn Action,
        input_metadata: &Arc<ActionInputMetadata>,
    ) -> ActionOutputMetadataStore {
        let mut store = ActionOutputMetadataStore::create(
            self.use_archived_tree_artifacts(action),
            self.output_permissions,
            action.get_outputs(),
        );
        store.action_file_system = self.create_action_file_system(input_metadata);
        store
    }

    /// `actionFileSystemType().inMemoryFileSystem()`: the output service's file system type is
    /// not `DISABLED`. The only ported output service with an action file system is the remote
    /// module's (`RemoteOutputService.actionFileSystemType()` is `REMOTE_FILE_SYSTEM` when it has
    /// an input fetcher), registered as the context `RemoteOutputTree`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:401 SkyframeActionExecutor.actionFileSystemType
    // Java: com/google/devtools/build/lib/vfs/OutputService.java:82 ActionFileSystemType.inMemoryFileSystem
    pub fn action_file_system_type_in_memory_file_system(&self) -> bool {
        self.contexts
            .get_any(TypeId::of::<RemoteOutputTree>())
            .is_some()
    }

    /// `createActionFileSystem(relativeOutputPath, inputArtifactData, outputArtifacts)` when the
    /// output service has an action file system (`actionFileSystemType().isEnabled()`): the
    /// remote module registered the context `RemoteOutputTree` (`RemoteOutputService`), which
    /// gives every action a `RemoteActionFileSystem` over the local file system.
    ///
    /// PORT: Java creates it in `ActionExecutionFunction` once the inputs are known and keeps it
    /// in the action's state; here it is created with the action's output metadata store, from
    /// the same input metadata, and the store holds it.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:331 ActionExecutionFunction.computeInternal
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:463 SkyframeActionExecutor.createActionFileSystem
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:94 RemoteOutputService.createActionFileSystem
    fn create_action_file_system(
        &self,
        input_metadata: &Arc<ActionInputMetadata>,
    ) -> Option<Arc<RemoteActionFileSystem>> {
        self.contexts.get_any(TypeId::of::<RemoteOutputTree>())?;
        let input_fetcher = self
            .contexts
            .get_any(TypeId::of::<Arc<dyn ActionInputPrefetcher>>())
            .and_then(|c| c.downcast::<Arc<dyn ActionInputPrefetcher>>().ok())
            .map(|f| (*f).clone());
        // checkNotNull(actionInputFetcher, "actionInputFetcher")
        let input_fetcher = Some(input_fetcher.expect("NullPointerException: actionInputFetcher"));
        Some(RemoteActionFileSystem::new(
            self.exec_root.get_file_system().clone(),
            self.exec_root.as_fragment().clone(),
            // directories.getRelativeOutputPath(): `<product name>-out`.
            // PORT: Blaze's only product name is "bazel" (BlazeDirectories of the runtime).
            // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:232 BlazeDirectories.getRelativeOutputPath
            "bazel-out",
            input_metadata.clone() as Arc<dyn InputMetadataProvider>,
            input_fetcher,
        ))
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
        let ActionExecutionState {
            action_lookup_data: owner,
            action: owning_action,
            result: cell,
        } = {
            let mut map = self.build_action_map.lock();
            map.entry(OwnerlessArtifactWrapper(action.get_primary_output()))
                .or_insert_with(|| ActionExecutionState {
                    action_lookup_data: action_lookup_data.clone(),
                    action: action.clone(),
                    result: Arc::new(OnceLock::new()),
                })
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
        // PORT: Java relies on the analysis-time ArtifactConflictFinder ("file '<x>' is generated
        // by these conflicting actions") for the two actions of one primary output to be
        // shareable, and would hand the second one the first one's outputs. Until that guarantee
        // is established before execution, a second action that cannot be shared with the one
        // that runs is an illegal state here.
        if !can_be_shared(
            &self.action_key_context,
            owning_action.as_ref(),
            action.as_ref(),
        )? {
            panic!(
                "IllegalStateException: Cannot share {} and {}: file '{}' is generated by these \
                 conflicting actions: {} and {}",
                owner,
                action_lookup_data,
                action.get_primary_output().pretty_print(),
                owning_action.pretty_print(),
                action.pretty_print()
            );
        }
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
        // if (actionFileSystem != null) updateActionFileSystemContext(action, actionFileSystem,
        // outputMetadataStore): RemoteOutputService.updateActionFileSystemContext.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:634 SkyframeActionExecutor.executeAction
        // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:112 RemoteOutputService.updateActionFileSystemContext
        if let Some(file_system) = output_metadata_store.get_action_file_system() {
            file_system.update_context(ActionMetadataSnapshot::of(action.as_ref()));
        }
        let context = ActionExecutionContextImpl {
            action: action.clone(),
            exec_root: action_exec_root(self, output_metadata_store.get_action_file_system()),
            executor: self,
            input_metadata: input_metadata.clone(),
            output_metadata_store: Some(output_metadata_store.clone()),
            action_file_system: output_metadata_store.get_action_file_system().cloned(),
            file_out_err: self
                .generate_file_out_err(output_metadata_store.get_action_file_system()),
            rich_artifact_data: Mutex::new(None),
            clock: JavaClock,
        };
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1111 ActionRunner.run
        if let Some(message) = action.get_progress_message(None) {
            // reporter.startTask(null, prependExecPhaseStats(message))
            // Java: com/google/devtools/build/lib/events/Reporter.java:159 Reporter.startTask
            self.reporter.handle(&Event::of_at(
                EventKind::Start,
                None,
                self.prepend_exec_phase_stats(&message),
            ));
        }
        env.listener().post(Arc::new(ActionStartedEvent {
            action: action.clone(),
            nano_time_start: action_start_time,
        }));
        let result: Result<ActionExecutionValue, ExecutionFailure> = (|| {
            let Some(executable) = as_executable(action.as_ref()) else {
                blaze_util::unported!(&format!(
                    "{}.execute (no ExecutableAction registered for mnemonic {})",
                    std::any::type_name_of_val(action.as_any()),
                    action.get_mnemonic()
                ));
            };
            // actionFileSystemType(): the remote output service answers REMOTE_FILE_SYSTEM when
            // it gives actions a file system (RemoteOutputService.java:86), and an output service
            // without one DISABLED; the action's output metadata store holds that file system.
            // Both answer true to shouldDoEagerActionPrep() (OutputService.java:93); only
            // REMOTE_FILE_SYSTEM answers true to inMemoryFileSystem() (OutputService.java:82).
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1128 ActionRunner.run
            let action_file_system = output_metadata_store.get_action_file_system();
            // if (actionFileSystemType().shouldDoEagerActionPrep()):
            // This call generally deletes any files at locations that are declared outputs of the
            // action, although some actions perform additional work, while others intentionally
            // keep previous outputs in place.
            // PORT: `prepare` takes only the exec root (`actionExecutionContext.getExecRoot()`);
            // Java also passes the path resolver (to delete the outputs on the action file system,
            // whose remote output tree is still empty here), the bulk deleter and
            // cleanupArchivedArtifacts.
            if let Err(e) = executable.prepare(context.get_exec_root()) {
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
            if let Some(action_file_system) = action_file_system {
                // There's nothing to delete when the action file system is used, but we must
                // ensure that the output directories for stdout and stderr exist.
                setup_action_fs_file_out_err(context.get_file_out_err(), action.as_ref())?;
                self.create_action_fs_output_directories(
                    action.as_ref(),
                    &action_file_system.as_file_system(),
                )?;
            } else {
                self.create_output_directories(action.as_ref())?;
            }
            self.execute_action_inner(env.listener(), action, &context)
        })();
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1199 ActionRunner.notifyActionCompletion
        env.listener().post(Arc::new(ActionCompletionEvent {
            related_time_start: action_start_time,
            finish_time: Instant::now(),
            action: action.clone(),
            action_lookup_data: action_lookup_data.clone(),
        }));
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1211 ActionRunner.notifyActionCompletion
        if let Some(message) = action.get_progress_message(None) {
            let completion_receiver = self.progress_reporting.read().completion_receiver.clone();
            if let Some(completion_receiver) = completion_receiver {
                completion_receiver.action_completed(action_lookup_data);
            }
            // reporter.finishTask(null, prependExecPhaseStats(message))
            // Java: com/google/devtools/build/lib/events/Reporter.java:170 Reporter.finishTask
            self.reporter.handle(&Event::of_at(
                EventKind::Finish,
                None,
                self.prepend_exec_phase_stats(&message),
            ));
        }
        let value = result?;

        // A closure to post-process the executed action, doing work like updating cached state
        // with any newly discovered inputs, and writing the result to the action cache.
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1424 ActionPostprocessingStep.run
        postprocessing(
            env,
            action,
            &input_metadata,
            &output_metadata_store,
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
            // Starlark callbacks (map_each) run on an evaluation-sized stack.
            .and_then(|()| on_evaluation_stack(|| executable.execute(context)))
            // An action's result (or intermediate state) may have been affected by lost inputs.
            // If an action filesystem is used, it may know whether inputs were lost. We should
            // fail fast if any were; rewinding may be able to fix it.
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1265 ActionRunner.executeAction
            .and_then(|r| {
                Self::check_action_file_system_for_lost_inputs(
                    context.get_remote_action_file_system().as_ref(),
                    action.as_ref(),
                )?;
                Ok(r)
            }) {
            Ok(r) => r,
            // Java: InterruptedException is not caught here; it leaves Action.execute and this
            // method unreported.
            Err(e) if e.is_interrupted() => return Err(ExecutionFailure::Interrupted),
            Err(e) => {
                let primary_output_path = context.get_input_path(&action.get_primary_output());
                // maybeSignalLostInputs(e, primaryOutputPath): a LostInputsActionExecutionException
                // leaves this method unreported and ActionExecutionFunction.handleLostInputs
                // handles it (see handle_lost_inputs).
                let e = match Self::maybe_signal_lost_inputs(e, action.as_ref(), context) {
                    Ok(e) => e,
                    Err(lost_inputs_exception) => {
                        self.handle_lost_inputs(action.as_ref(), &lost_inputs_exception)
                    }
                };
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
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1302 ActionRunner.actuallyCompleteAction
    fn actually_complete_action(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        context: &ActionExecutionContextImpl<'_>,
        action_result: ActionResult,
    ) -> Result<ActionExecutionValue, ExecutionFailure> {
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
            // catch (ActionExecutionException actionException): maybeSignalLostInputs, then
            // report.
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1356 ActionRunner.actuallyCompleteAction
            let e = match Self::maybe_signal_lost_inputs(e, action.as_ref(), context) {
                Ok(e) => e,
                Err(lost_inputs_exception) => {
                    return Err(self
                        .handle_lost_inputs_of_completion(
                            listener,
                            &primary_output_path,
                            action,
                            &lost_inputs_exception,
                            out_err_buffer,
                        )
                        .into());
                }
            };
            // Success in execution but failure in completion.
            self.report_action_execution(
                listener,
                &primary_output_path,
                /* primaryOutputMetadata= */ None,
                action,
                Some(&action_result),
                Some(e.clone()),
                out_err_buffer,
                ErrorTiming::AFTER_EXECUTION,
            );
            return Err(e.into());
        }
        // if (finalizeActions) outputService.finalizeAction(action, outputMetadataStore): the
        // context is registered only for an output service that does something there (the remote
        // one) and only with finalizeActions; LocalOutputService.finalizeAction is a no-op.
        if let Some(finalizer) = self
            .contexts
            .get_any(TypeId::of::<Arc<dyn ActionFinalizer>>())
            .and_then(|c| c.downcast::<Arc<dyn ActionFinalizer>>().ok())
        {
            let outputs: Vec<ActionInputRef> =
                action.get_outputs().iter().map(artifact_input).collect();
            let metadata = ActionMetadataSnapshot::of(action.as_ref());
            if let Err(e) =
                finalizer.finalize_action(&*metadata, &outputs, &**output_metadata_store)
            {
                use blaze_exec::exec::action_execution_context::SpawnCacheError;
                let e: std::io::Error = match e {
                    // catch (LostInputsExecException e): the remote cache has lost an output of the
                    // action before it could be downloaded. Like a lost input, this is recovered
                    // from by rewinding rather than reported: `throw
                    // ActionExecutionException.fromExecException(e, action)`, which the catch of
                    // ActionExecutionException below hands to maybeSignalLostInputs.
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1344 ActionRunner.actuallyCompleteAction
                    SpawnCacheError::Exec(lost) if lost.get_lost_inputs().is_some() => {
                        let lost_inputs_exception = ActionExecutionException::from_exec_exception(
                            &lost,
                            None,
                            action.as_ref(),
                        );
                        return Err(self
                            .handle_lost_inputs_of_completion(
                                listener,
                                &primary_output_path,
                                action,
                                &lost_inputs_exception,
                                out_err_buffer,
                            )
                            .into());
                    }
                    // InterruptedException (see below).
                    SpawnCacheError::Exec(ExecException::Interrupted) => {
                        std::io::Error::from(std::io::ErrorKind::Interrupted)
                    }
                    // PORT: `finalizeAction` throws no other ExecException; one would be reported
                    // as the failure to finalize, with its message as the cause.
                    SpawnCacheError::Exec(other) => std::io::Error::other(other.get_message()),
                    SpawnCacheError::Io(e) => e,
                };
                // InterruptedException is not caught: it leaves finalizeAction and this method
                // unreported (`finalizeAction(...) throws ... InterruptedException`).
                // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1340 ActionRunner.actuallyCompleteAction
                if e.kind() == std::io::ErrorKind::Interrupted {
                    return Err(ExecutionFailure::Interrupted);
                }
                // PORT: skipped, logging (logger.atWarning "unable to finalize action").
                let e = self.to_action_execution_exception(
                    "unable to finalize action",
                    Some(&e),
                    action.as_ref(),
                    Some(out_err_buffer),
                    execution::Code::ActionFinalizationFailure,
                );
                // Success in execution but failure in completion.
                self.report_action_execution(
                    listener,
                    &primary_output_path,
                    /* primaryOutputMetadata= */ None,
                    action,
                    Some(&action_result),
                    Some(e.clone()),
                    out_err_buffer,
                    ErrorTiming::AFTER_EXECUTION,
                );
                return Err(e.into());
            }
        }
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1389 ActionRunner.actuallyCompleteAction
        let primary_output_metadata =
            match output_metadata_store.get_output_metadata(&artifact_input(&primary_output)) {
                Ok(Some(metadata)) => metadata,
                Ok(None) => panic!(
                    "IllegalStateException: Metadata already obtained for {}",
                    primary_output
                ),
                Err(e) => panic!(
                    "IllegalStateException: Metadata already obtained for {}: {:?}",
                    primary_output, e
                ),
            };
        self.report_action_execution(
            listener,
            &primary_output_path,
            Some(primary_output_metadata),
            action,
            Some(&action_result),
            None,
            out_err_buffer,
            ErrorTiming::NO_ERROR,
        );
        let rich_artifact_data = context.rich_artifact_data.lock().clone();
        Ok(self.create_action_execution_value(output_metadata_store, rich_artifact_data))
    }

    /// `ActionExecutionValue.create(outputMetadataStore, richArtifactData, action)`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionValue.java:145 ActionExecutionValue.create
    fn create_action_execution_value(
        &self,
        output_metadata_store: &ActionOutputMetadataStore,
        rich_artifact_data: Option<Arc<RunfilesArtifactValue>>,
    ) -> ActionExecutionValue {
        ActionExecutionValue::create_with_rich_metadata(
            output_metadata_store.get_all_artifact_data(),
            output_metadata_store.get_all_tree_artifact_data(),
            rich_artifact_data.map(|r| {
                let metadata = r.get_metadata().clone();
                (r, metadata)
            }),
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
                    } else if output.is_symlink() && blaze_vfs::NotASymlinkException::is(&e) {
                        self.reporter.handle(&Event::error_at(
                            Some(action.get_owner().get_location().clone()),
                            format!(
                                "declared output '{}' is not a symlink",
                                output.pretty_print()
                            ),
                        ));
                    } else {
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
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1453 SkyframeActionExecutor.createActionFsOutputDirectories
    fn create_action_fs_output_directories(
        &self,
        action: &dyn Action,
        artifact_path_resolver: &blaze_vfs::FileSystemRef,
    ) -> Result<(), ActionExecutionException> {
        if let Err(e) = self
            .output_directory_helper
            .create_action_fs_output_directories(
                &action.get_outputs(),
                Some(artifact_path_resolver),
            )
        {
            return Err(self.to_action_execution_exception(
                &format!(
                    "failed to create output directory '{}': {}",
                    e.get_directory_path(),
                    e.get_cause()
                ),
                Some(e.get_cause()),
                action,
                None,
                execution::Code::ActionFsOutputDirectoryCreationFailure,
            ));
        }
        Ok(())
    }

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
        if crate::recursive_filesystem_traversal_function::TrackSourceDirectoriesFlag::track_source_directories() {
            return Ok(());
        }

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
                        lost_inputs: None,
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
        // outputChecker = outputService.getOutputChecker(): LocalOutputService's
        // TRUST_LOCAL_ONLY, or RemoteOutputService's RemoteOutputChecker.
        let output_checker: &dyn OutputChecker = match &self.output_checker {
            Some(checker) => &**checker,
            None => &TrustLocalOnly,
        };
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
            Some(output_checker),
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
                        Some(output_checker),
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
    /// PORT: `TestRunnerAction`'s `ActionCachedContext` is its arguments (event handler, exec
    /// root, `getContext(TestActionContext.class)`); the other implementations
    /// (`CoverageReportAction`) answer through `ExecutableAction::action_cache_hit`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:801 SkyframeActionExecutor.checkActionCache
    fn notify_on_action_cache_hit(&self, action: &ActionRef) -> Option<bool> {
        // Java's `instanceof`: the analysis-side TestRunnerAction is viewed as the execution one.
        let Some(test_action): Option<Arc<TestRunnerAction>> =
            blaze_exec::analysis::test::as_test_runner_action(action.as_ref())
        else {
            // The other NotifyOnActionCacheHit actions (CoverageReportAction).
            return blaze_exec::actions::action_execution::as_executable(action.as_ref()).and_then(
                |executable| executable.action_cache_hit(&*self.reporter as &dyn EventHandler),
            );
        };
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
            // Java passes state.actionInputMetadataProvider: the input map alone.
            let value = match create_runfiles_artifact_value(
                runfiles_tree_action,
                &input_metadata_provider.action_input_map_only(),
            ) {
                Ok(value) => Arc::new(value),
                // On action cache hits, all input metadata should already be in RAM
                Err(e) => panic!("IllegalStateException: {e}"),
            };
            let metadata = value.get_metadata().clone();
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
        // PORT: no Java counterpart: after the purge before the unported action rewinding, this
        // build's action cache takes no new entry (see action_cache_purged).
        let purged = self.action_cache_purged.read();
        if *purged {
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
                Err(ExecutionFailure::Exception(
                    ActionExecutionException::already_reported(ActionExecutionException::with_cause(
                        e.message.clone(),
                        &e,
                        Some(action),
                        /* catastrophe= */ false,
                        Some(blaze_protos::failure_details::FailureDetail {
                            message: e.message.clone(),
                            category: Some(
                                blaze_protos::failure_details::failure_detail::Category::IncludeScanning(
                                    *e.get_error(),
                                ),
                            ),
                        }),
                    )),
                ))
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
        action_file_system: Option<Arc<RemoteActionFileSystem>>,
    ) -> Result<Option<NestedSet<Artifact>>, ExecutionFailure> {
        // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:315 ActionExecutionContext.forInputDiscovery
        let context = ActionExecutionContextImpl {
            action: action.clone(),
            exec_root: action_exec_root(self, action_file_system.as_ref()),
            executor: self,
            input_metadata,
            // ActionExecutionContext.forInputDiscovery has no output metadata store.
            output_metadata_store: None,
            file_out_err: self.generate_file_out_err(action_file_system.as_ref()),
            action_file_system,
            rich_artifact_data: Mutex::new(None),
            clock: JavaClock,
        };
        if let Some(action_file_system) = &context.action_file_system {
            // updateActionFileSystemContext(action, actionFileSystem,
            // THROWING_OUTPUT_METADATA_STORE_FOR_ACTIONFS): RemoteOutputService ignores the store.
            // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:112 RemoteOutputService.updateActionFileSystemContext
            action_file_system.update_context(ActionMetadataSnapshot::of(action.as_ref()));
            // Note that when not using ActionFS, a global setup of the parent directories of the
            // OutErr streams is sufficient.
            setup_action_fs_file_out_err(context.get_file_out_err(), action.as_ref())?;
        }
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:958 SkyframeActionExecutor.discoverInputs
        listener.post(Arc::new(ScanningActionEvent {
            action: action.clone(),
        }));
        let Some(executable) = as_executable(action.as_ref()) else {
            blaze_util::unported!(&format!(
                "{}.discoverInputs (no ExecutableAction registered for mnemonic {})",
                std::any::type_name_of_val(action.as_any()),
                action.get_mnemonic()
            ));
        };
        let result = executable.discover_inputs(&context).and_then(|artifacts| {
            // Input discovery may have been affected by lost inputs. If an action filesystem is
            // used, it may know whether inputs were lost. We should fail fast if any were;
            // rewinding may be able to fix it.
            // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:967 SkyframeActionExecutor.discoverInputs
            Self::check_action_file_system_for_lost_inputs(
                context.action_file_system.as_ref(),
                action.as_ref(),
            )?;
            Ok(artifacts)
        });
        let outcome = (|| match result {
            Ok(artifacts) => Ok(artifacts),
            // Java: InterruptedException propagates unreported.
            Err(e) if e.is_interrupted() => Err(ExecutionFailure::Interrupted),
            Err(e) => {
                // Input discovery failures may be caused by lost inputs. Lost input failures have
                // higher priority because rewinding may be able to restore what was lost and allow
                // the action to complete without error.
                // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:976 SkyframeActionExecutor.discoverInputs
                let e = if e.get_lost_inputs().is_none() {
                    match Self::check_action_file_system_for_lost_inputs(
                        context.action_file_system.as_ref(),
                        action.as_ref(),
                    ) {
                        Ok(()) => e,
                        Err(lost_inputs_exception) => lost_inputs_exception,
                    }
                } else {
                    e
                };
                let primary_output_path = context.get_input_path(&action.get_primary_output());
                if e.get_lost_inputs().is_some() {
                    // If inputs were lost during input discovery, then enrich the exception,
                    // informing action rewinding machinery that these lost inputs are now Skyframe
                    // deps of the action.
                    // PORT: setFromInputDiscovery() and enrichLostInputsException decorate the
                    // exception for ActionExecutionFunction.handleLostInputs; the flag only
                    // chooses the failed action's deps for the rewind plan, which is prepared
                    // after checkRewindingEnabled (ActionExecutionFunction.java:471,
                    // ActionRewindStrategy.java:239), and rewinding is not ported. The exception
                    // goes to handleLostInputs here, as the execution path does (see
                    // handle_lost_inputs), with the primary output path and out/err it would carry.
                    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:983 SkyframeActionExecutor.discoverInputs
                    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:509 ActionExecutionFunction.handleLostInputs
                    return Err(self
                        .handle_lost_inputs_of_completion(
                            listener,
                            &primary_output_path,
                            action,
                            &e,
                            context.get_file_out_err(),
                        )
                        .into());
                }
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
        })();
        // finally
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1002 SkyframeActionExecutor.discoverInputs
        listener.post(Arc::new(StoppedScanningActionEvent {
            action: action.clone(),
        }));
        outcome
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
            /* primaryOutputMetadata= */ None,
            action,
            None,
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
    #[allow(clippy::too_many_arguments)]
    fn report_action_execution(
        &self,
        listener: &dyn ExtendedEventHandler,
        primary_output_path: &Path,
        primary_output_metadata: Option<FileArtifactValue>,
        action: &ActionRef,
        action_result: Option<&ActionResult>,
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
        // Collect MetadataLogs and spawn start times/end times from the Action's SpawnResults.
        // PORT: `findSpawnResultsInActionResultAndException`'s second case (the SpawnResult of a
        // SpawnActionExecutionException) has no counterpart: blaze_exec's
        // ActionExecutionException does not keep the failed spawn's result, so a failed spawn
        // action reports no times (Java reports the failed spawn's).
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1976 SkyframeActionExecutor.findSpawnResultsInActionResultAndException
        let spawn_results: &[blaze_exec::actions::spawn_result::SpawnResult] = match action_result {
            Some(action_result) => action_result.spawn_results(),
            None => &[],
        };
        let mut first_start_time: Option<std::time::SystemTime> = None;
        let mut last_end_time: Option<std::time::SystemTime> = None;
        for spawn_result in spawn_results {
            // Not all SpawnResults have a start time, and some use Instant.MIN/MAX instead of
            // null.
            // PORT: Blaze's SpawnResult keeps no Instant.MIN/MAX sentinel (`None` instead).
            if let Some(start_time) = spawn_result.get_start_time() {
                // startTime.plusMillis(wallTimeInMs)
                let wall_time_in_ms = spawn_result.get_wall_time_in_ms();
                let wall_time =
                    std::time::Duration::from_millis(wall_time_in_ms.unsigned_abs() as u64);
                let end_time = if wall_time_in_ms >= 0 {
                    start_time + wall_time
                } else {
                    start_time - wall_time
                };
                first_start_time = Some(match first_start_time {
                    Some(t) => t.min(start_time),
                    None => start_time,
                });
                last_end_time = Some(match last_end_time {
                    Some(t) => t.max(end_time),
                    None => end_time,
                });
            }
        }
        listener.post(Arc::new(ActionExecutedEvent {
            action_id: action.get_primary_output().get_exec_path().clone(),
            action: action.clone(),
            exception,
            primary_output_path: primary_output_path.clone(),
            output_artifact: action.get_primary_output(),
            stdout,
            stderr,
            error_timing,
            primary_output_metadata,
            start_time: first_start_time,
            end_time: last_end_time,
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

// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1823 SkyframeActionExecutor.createDetailedExitCode
/// Creates the parent directories of the action's stdout and stderr files (on the action file
/// system).
// Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1495 SkyframeActionExecutor.setupActionFsFileOutErr
fn setup_action_fs_file_out_err(
    file_out_err: &FileOutErr,
    action: &dyn Action,
) -> Result<(), ActionExecutionException> {
    let result = (|| -> std::io::Result<()> {
        file_out_err
            .get_output_path()
            .get_parent_directory()
            .expect("stdout file has a parent directory")
            .create_directory_and_parents()?;
        file_out_err
            .get_error_path()
            .get_parent_directory()
            .expect("stderr file has a parent directory")
            .create_directory_and_parents()
    })();
    if let Err(e) = result {
        let message = format!(
            "failed to create output directory for output streams '{}': {}",
            file_out_err.get_error_path(),
            e
        );
        let code = create_detailed_exit_code(
            &message,
            execution::Code::ActionFsOutErrDirectoryCreationFailure,
        );
        return Err(ActionExecutionException::with_cause(
            message,
            &e,
            Some(action),
            false,
            Some(code),
        ));
    }
    Ok(())
}

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
    // PORT: skipped, tracing (logger.atWarning: "Error creating ..." goes to the Java log).
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
    /// The action file system (`RemoteOutputService.createActionFileSystem`) the outputs are
    /// stat'ed through: Java's `artifactPathResolver` of the store.
    action_file_system: Option<Arc<RemoteActionFileSystem>>,
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
            action_file_system: None,
        }
    }

    /// The action file system of this action, if any (`ActionExecutionContext
    /// .getActionFileSystem()` of its execution).
    pub fn get_action_file_system(&self) -> Option<&Arc<RemoteActionFileSystem>> {
        self.action_file_system.as_ref()
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

    /// `getAllTreeArtifactData()`: data for tree artifacts that was computed during execution.
    /// May contain copies of `TreeArtifactValue.MISSING_TREE_ARTIFACT`
    /// ([`crate::action_execution_value::MISSING_TREE_ARTIFACT`], the store's `None`).
    ///
    /// PORT: Java copies a `ConcurrentHashMap`; the entries are returned in the action's output
    /// order here, then any other key, as `get_all_artifact_data`.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:144 ActionOutputMetadataStore.getAllTreeArtifactData
    pub fn get_all_tree_artifact_data(&self) -> IndexMap<Artifact, Arc<TreeArtifactValue>> {
        let data = self.tree_artifact_data.lock();
        let value_of = |v: &Option<Arc<TreeArtifactValue>>| match v {
            Some(v) => v.clone(),
            None => crate::action_execution_value::MISSING_TREE_ARTIFACT.clone(),
        };
        let mut result = IndexMap::new();
        for output in &self.outputs {
            if let Some(v) = data.get(output) {
                result.insert(output.clone(), value_of(v));
            }
        }
        for (k, v) in data.iter() {
            if !result.contains_key(k) {
                result.insert(k.clone(), value_of(v));
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

    /// `artifactPathResolver.toPath(artifact)`: the artifact's path on the action file system
    /// when there is one (`TransformResolver`), its own path otherwise (`IdentityResolver`).
    // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:75 IdentityResolver.toPath
    // Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:107 TransformResolver.toPath
    fn to_path(&self, artifact: &Artifact) -> Path {
        artifact_path_resolver_to_path(self.action_file_system.as_ref(), artifact)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:236 ActionOutputMetadataStore.constructTreeArtifactValueFromFilesystem
    fn construct_tree_artifact_value_from_filesystem(
        &self,
        parent: &Artifact,
    ) -> std::io::Result<Option<Arc<TreeArtifactValue>>> {
        let tree_dir = self.to_path(parent);
        let chmod = self.execution_mode.load(Ordering::SeqCst);

        let stat = stat_if_found_with_metadata(&tree_dir, Symlinks::Follow)?;

        // Make sure the tree artifact root exists and is a regular directory. Note that this is
        // how the action is initialized, so this should hold unless the action itself has deleted
        // the root.
        let Some(stat) = stat.filter(|s| s.status.is_directory()) else {
            if chmod {
                self.set_path_permissions_if_file(&tree_dir)?;
            }
            return Ok(None);
        };

        // TreeArtifactValue.newBuilder(parent)
        let mut tree: Vec<(ActionInputRef, FileArtifactValue)> = Vec::new();

        TreeArtifactValue::visit_tree(
            &tree_dir,
            &mut |parent_relative_path, type_, traversed_symlink| {
                check_state!(type_ == DirentType::File || type_ == DirentType::Directory);
                // Set the output permissions when the execution mode requires it, unless at least
                // one symlink was traversed on the way to this entry, as it might have led outside
                // of the root directory.
                if chmod && !traversed_symlink {
                    self.set_path_permissions(&tree_dir.get_relative(parent_relative_path))?;
                }
                if type_ == DirentType::Directory {
                    return Ok(()); // The final TreeArtifactValue does not contain child directories.
                }
                let child = Artifact::create_tree_output(parent, parent_relative_path.clone());
                let metadata = self.construct_file_artifact_value_from_filesystem(&child)?;
                tree.push((artifact_input(&child), metadata));
                Ok(())
            },
        )?;

        if self.archived_tree_artifacts_enabled {
            blaze_util::unported!("ArchivedTreeArtifact.createForTree");
        }

        // If the artifact was materialized in the filesystem as as symlink to another artifact,
        // record the real path in the metadata so that it can be recreated as such later.
        // See {@link FileArtifactValue#getResolvedPath} for why this is useful.
        // TODO(tjgq): Actually check whether the path matches one of the action inputs. The
        // presence of a FileStatusWithMetadata happens to coincide, but seems a little brittle.
        let mut resolved_path: Option<PathFragment> = None;
        if let Some(metadata) = stat.metadata
            && tree_dir.is_symbolic_link()
        {
            match metadata.get_resolved_path() {
                Some(path) => resolved_path = Some(path.clone()),
                None => {
                    resolved_path = Some(tree_dir.resolve_symbolic_links()?.as_fragment().clone())
                }
            }
            // Builder.setResolvedPath: checkArgument(resolvedPath.isAbsolute(), resolvedPath).
            let path = resolved_path.as_ref().unwrap();
            blaze_util::check_argument!(path.is_absolute(), "{}", path);
        }

        Ok(Some(Arc::new(TreeArtifactValue::build(
            tree,
            None,
            resolved_path,
        ))))
    }

    /// Sets the output permissions of a freshly created output file.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:590 ActionOutputMetadataStore.setPathPermissionsIfFile
    fn set_path_permissions_if_file(&self, path: &Path) -> std::io::Result<()> {
        let stat = path.stat_if_found(Symlinks::Nofollow)?;
        if let Some(stat) = stat
            && stat.is_file()
            && stat.get_permissions() != self.output_permissions.get_permissions_mode()
        {
            self.set_path_permissions(path)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:599 ActionOutputMetadataStore.setPathPermissions
    fn set_path_permissions(&self, path: &Path) -> std::io::Result<()> {
        path.chmod(self.output_permissions.get_permissions_mode())
    }

    /// Constructs a new `FileArtifactValue` by reading from the file system.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:381 ActionOutputMetadataStore.constructFileArtifactValueFromFilesystem
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:387 ActionOutputMetadataStore.constructFileArtifactValue
    fn construct_file_artifact_value_from_filesystem(
        &self,
        artifact: &Artifact,
    ) -> std::io::Result<FileArtifactValue> {
        check_state!(
            !artifact.is_tree_artifact(),
            "{} is a tree artifact",
            artifact
        );

        let stat_and_value =
            file_artifact_value_from_artifact(artifact, self.action_file_system.as_ref())?;
        let value = stat_and_value.file_artifact_value;

        let type_ = value.get_type();

        if !type_.exists() {
            // Nonexistent files should only occur before executing an action.
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{} does not exist", artifact.pretty_print()),
            ));
        }

        if type_.is_symlink() {
            // We always create a FileArtifactValue for an unresolved symlink with a digest
            // (calling readlink() is easy, unlike checksumming a potentially huge file).
            if value.get_digest().is_none() {
                panic!("NullPointerException: {value} missing digest");
            }
            return Ok(value);
        }

        if type_.is_file() && value.get_digest().is_some() {
            // The digest is in the file value and that is all that is needed for this file's
            // metadata.
            return Ok(value);
        }

        if type_.is_directory() {
            // This branch is taken when the output of an action is a directory:
            //   - A Fileset (in this case, Blaze is correct)
            //   - A directory someone created in a local action (in this case, changes under the
            //     directory may not be detected since we use the mtime of the directory for
            //     up-to-dateness checks)
            //   - A symlink to a source directory due to Filesets
            return Ok(FileArtifactValue::create_for_directory_with_mtime(
                self.to_path(artifact).get_last_modified_time()?,
            ));
        }

        if type_.is_special_file() {
            // FileArtifactValue.createFromInjectedDigest(value, /* digest= */ null)
            return Ok(FileArtifactValue::create_for_normal_file(
                None,
                value.get_contents_proxy(),
                value.get_size(),
            ));
        }

        check_state!(type_.is_file(), "{:?}", type_);
        // We don't have an injected digest and there is no digest in the file value (which
        // attempts a fast digest). Manually compute the digest instead.
        let mut path = stat_and_value.path_no_follow;
        // The file exists and is not an unresolved symlink, so it must have been stat'ed.
        if stat_and_value
            .stat_no_follow
            .unwrap_or_else(|| panic!("NullPointerException: statNoFollow of {artifact}"))
            .is_symbolic_link()
        {
            // If the file is a symlink, we compute the digest using the target path so that it's
            // possible to hit the digest cache - we probably already computed the digest for the
            // target during previous action execution.
            // The case of an unresolved symlink has been handled above, so realPath() is never
            // null.
            path = stat_and_value
                .real_path
                .unwrap_or_else(|| panic!("NullPointerException: realPath of {artifact}"));
        }
        // FileArtifactValue.createFromInjectedDigest(value, DigestUtils.manuallyComputeDigest(path))
        Ok(FileArtifactValue::create_for_normal_file(
            Some(path.get_digest()?),
            value.get_contents_proxy(),
            value.get_size(),
        ))
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

// Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:548 ActionOutputMetadataStore.FileArtifactStatAndValue
struct FileArtifactStatAndValue {
    path_no_follow: Path,
    real_path: Option<Path>,
    stat_no_follow: Option<blaze_vfs::FileStatus>,
    file_artifact_value: FileArtifactValue,
}

/// `artifactPathResolver.toPath(artifact)` for the output metadata store's resolver
/// (`ArtifactPathResolver.createPathResolver(actionFileSystem, execRoot)`).
// Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:75 IdentityResolver.toPath
// Java: com/google/devtools/build/lib/actions/ArtifactPathResolver.java:107 TransformResolver.toPath
fn artifact_path_resolver_to_path(
    action_file_system: Option<&Arc<RemoteActionFileSystem>>,
    artifact: &Artifact,
) -> Path {
    match action_file_system {
        // fileSystem.getPath(artifact.getPath().asFragment())
        Some(file_system) => file_system.get_path(artifact.get_path().as_fragment()),
        None => artifact.get_path(),
    }
}

/// A `FileStatus` as Java's `Path.statIfFound` returns it, with the metadata of a
/// `FileStatusWithMetadata` (what the action file system returns for a path its in-memory
/// sources know: an input, or an output in its remote output tree).
///
/// PORT: `blaze_vfs::FileStatus` carries no metadata, so the `instanceof FileStatusWithMetadata`
/// test is `RemoteActionFileSystem::stat_metadata` on the same path and the same `Symlinks`.
struct StatWithMetadata {
    status: blaze_vfs::FileStatus,
    metadata: Option<FileArtifactValue>,
}

/// `path.statIfFound(symlinks)`, with the metadata of a `FileStatusWithMetadata`.
fn stat_if_found_with_metadata(
    path: &Path,
    symlinks: Symlinks,
) -> std::io::Result<Option<StatWithMetadata>> {
    let Some(status) = path.stat_if_found(symlinks)? else {
        return Ok(None);
    };
    let metadata = match path
        .get_file_system()
        .as_any()
        .downcast_ref::<RemoteActionFileSystem>()
    {
        Some(file_system) => {
            file_system.stat_metadata(path.as_fragment(), symlinks == Symlinks::Follow)?
        }
        None => None,
    };
    Ok(Some(StatWithMetadata { status, metadata }))
}

/// Constructs a `FileArtifactValue` for a regular (non-tree, non-runfiles tree) artifact.
///
/// PORT: Java's `statNoFollow` parameter is always null here (`constructFileArtifactValue` is
/// only reached through `constructFileArtifactValueFromFilesystem`; the archived tree artifact
/// caller is not ported), so it is not a parameter. `action_file_system` is the
/// `artifactPathResolver` (`None`: `IDENTITY`).
// Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:478 ActionOutputMetadataStore.fileArtifactValueFromArtifact
fn file_artifact_value_from_artifact(
    artifact: &Artifact,
    action_file_system: Option<&Arc<RemoteActionFileSystem>>,
) -> std::io::Result<FileArtifactStatAndValue> {
    check_state!(
        !artifact.is_tree_artifact() && !artifact.is_runfiles_tree(),
        "{}",
        artifact
    );

    let path_no_follow = artifact_path_resolver_to_path(action_file_system, artifact);
    // If we expect a symlink, we can readlink it directly and handle errors appropriately - there
    // is no need for the stat below.
    if artifact.is_symlink() {
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:394 FileArtifactValue.createForUnresolvedSymlink
        let file_artifact_value =
            FileArtifactValue::create_for_unresolved_symlink(&path_no_follow)?;
        return Ok(FileArtifactStatAndValue {
            path_no_follow,
            real_path: None,
            stat_no_follow: None,
            file_artifact_value,
        });
    }

    // RootedPath.toRootedPath(artifactPathResolver.transformRoot(root), rootRelativePath): its
    // `asPath()` is `pathNoFollow`.
    // Stat the file. All output artifacts of an action are deleted before execution, so if a file
    // exists, it was most likely created by the current action. There is a race condition here if
    // an external process creates (or modifies) the file between the deletion and this stat,
    // which we cannot solve.
    let stat_no_follow = stat_if_found_with_metadata(&path_no_follow, Symlinks::Nofollow)?;

    if !stat_no_follow
        .as_ref()
        .is_some_and(|s| s.status.is_symbolic_link())
    {
        let file_artifact_value =
            file_artifact_value_from_stat(&path_no_follow, stat_no_follow.as_ref())?;
        return Ok(FileArtifactStatAndValue {
            path_no_follow,
            real_path: None,
            stat_no_follow: stat_no_follow.map(|s| s.status),
            file_artifact_value,
        });
    }

    // We use FileStatus#isSymbolicLink over Path#isSymbolicLink to avoid the unnecessary stat
    // done by the latter.  We need to protect against symlink cycles since
    // ArtifactFileMetadata#value assumes it's dealing with a file that's not in a symlink cycle.
    let real_path = path_no_follow.resolve_symbolic_links()?;
    if real_path == path_no_follow {
        return Err(std::io::Error::other("symlink cycle"));
    }

    // RootedPath.toRootedPathMaybeUnderRoot(realPath, [transformRoot(root)]): its `asPath()` is
    // `realPath`.
    // TODO(bazel-team): consider avoiding a 'stat' here when the symlink target hasn't changed
    // and is a source file (since changes to those are checked separately).
    let real_stat = stat_if_found_with_metadata(&real_path, Symlinks::Nofollow)?;
    let mut file_artifact_value = file_artifact_value_from_stat(&real_path, real_stat.as_ref())?;

    // If the artifact was materialized in the filesystem as as symlink to another artifact, record
    // the real path in the metadata so that it can be recreated as such later.
    // See {@link FileArtifactValue#getResolvedPath} for why this is useful.
    // TODO(tjgq): Actually check whether the path matches one of the action inputs. The presence
    // of a FileStatusWithMetadata happens to coincide, but seems a little brittle.
    if real_stat.as_ref().is_some_and(|s| s.metadata.is_some())
        && file_artifact_value.get_resolved_path().is_none()
    {
        file_artifact_value = FileArtifactValue::create_from_existing_with_resolved_path(
            file_artifact_value,
            real_path.as_fragment().clone(),
        );
    }

    Ok(FileArtifactStatAndValue {
        path_no_follow,
        real_path: Some(real_path),
        stat_no_follow: stat_no_follow.map(|s| s.status),
        file_artifact_value,
    })
}

// Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:566 ActionOutputMetadataStore.fileArtifactValueFromStat
fn file_artifact_value_from_stat(
    path: &Path,
    stat: Option<&StatWithMetadata>,
) -> std::io::Result<FileArtifactValue> {
    let Some(stat_with_metadata) = stat else {
        return Ok(FileArtifactValue::MissingFileMarker);
    };
    let stat = &stat_with_metadata.status;

    if stat.is_directory() {
        return Ok(FileArtifactValue::create_for_directory_with_mtime(
            stat.get_last_modified_time(),
        ));
    }

    if let Some(metadata) = &stat_with_metadata.metadata {
        return Ok(metadata.clone());
    }

    // FileStateValue.createWithStatNoFollow(rootedPath, stat, xattrProvider, tsgm), then
    // FileArtifactValue.createForNormalFile(fileStateValue.getDigest(),
    // fileStateValue.getContentsProxy(), stat.getSize()).
    // Java: com/google/devtools/build/lib/actions/FileStateValue.java:86 FileStateValue.createWithStatNoFollow
    if stat.is_file() {
        // A special file (SpecialFileStateValue.fromStat) and a regular file without a fast
        // digest (RegularFileStateValueWithContentsProxy: the stat carries no digest, and
        // `xattrProvider.getFastDigest` is null for a file system without digest xattrs) both
        // have no digest and the contents proxy of the stat.
        // PORT: skipped, incremental (TimestampGranularityMonitor.notifyDependenceOnFileTime).
        return Ok(FileArtifactValue::create_for_normal_file(
            None,
            Some(FileContentsProxy::create(stat)),
            stat.get_size(),
        ));
    }
    if stat.is_symbolic_link() {
        // SymlinkFileStateValue: getDigest() and getContentsProxy() throw.
        panic!(
            "IllegalStateException: {path} is a symlink (FileStateValue of a symlink has no digest)"
        );
    }
    // InconsistentFilesystemException
    Err(std::io::Error::other(format!(
        "according to stat, existing path {path} is neither a file nor directory nor symlink."
    )))
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
            self.set_path_permissions_if_file(&self.to_path(artifact))?;
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
        // The key may run Starlark callbacks (map_each): an evaluation-sized stack.
        on_evaluation_stack(|| {
            self.action.get_key(
                self.action_key_context,
                Some(&KeyMetadataAdapter(input_metadata_provider)),
            )
        })
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
            // ArtifactFactory.resolveSourceArtifacts throws only what the resolver throws.
            Err(message) => Err(bridge.error.borrow_mut().take().unwrap_or_else(|| {
                panic!("IllegalStateException: resolver error not recorded: {message}")
            })),
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

/// One value of an `ActionInputMap` (`values[index]`).
enum ActionInputMapValue {
    File(FileArtifactValue),
    Tree(Arc<TreeArtifactValue>),
    Runfiles(Arc<RunfilesArtifactValue>),
}

/// The metadata of an action's inputs: Java's `ActionInputMap` (filled from the inputs' Skyframe
/// values; inputs discovered by the action are added to the same map) and the providers
/// `ActionExecutionFunction` builds over it: `ActionInputMetadataProvider` (the map alone, see
/// [`ActionInputMetadata::action_input_map_only`]) and the
/// `DelegatingPairInputMetadataProvider` of it and a `SkyframeInputMetadataProvider` (this type's
/// own `InputMetadataProvider` implementation, used while the action discovers inputs and
/// executes).
///
/// PORT: Java's `ActionInputMap` is an open-addressed table plus a trie of the tree artifacts'
/// exec paths; here it is one hash map from exec path to key and value, and the trie lookups walk
/// the ancestors of the path.
// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:53 ActionInputMap
// Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:45 ActionInputMetadataProvider
// Java: com/google/devtools/build/lib/skyframe/SkyframeInputMetadataProvider.java:52 SkyframeInputMetadataProvider
// Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:27 DelegatingPairInputMetadataProvider
pub struct ActionInputMetadata {
    /// `ActionInputMap`: `paths[index]` to `(keys[index], values[index])`.
    map: RwLock<HashMap<PathFragment, (ActionInputRef, ActionInputMapValue)>>,
    /// `SkyframeInputMetadataProvider.perBuild` (`skyframeActionExecutor.getPerBuildFileCache()`),
    /// which also holds `relativeOutputPath`; `None` when the executor was not configured with
    /// one ([`SkyframeActionExecutor::configure_file_cache`]).
    per_build: Option<Arc<SingleBuildFileCache>>,
    /// `SkyframeInputMetadataProvider.allowSkyframe`.
    allow_skyframe: AtomicBool,
    /// `SkyframeInputMetadataProvider.seen`.
    seen: Mutex<HashMap<PathFragment, ActionInputRef>>,
}

impl ActionInputMetadata {
    /// An empty `ActionInputMap` with the providers of an action executed by `executor`.
    pub fn new(executor: &SkyframeActionExecutor) -> ActionInputMetadata {
        ActionInputMetadata::with_providers(executor.per_build_file_cache.get().cloned())
    }

    fn with_providers(per_build: Option<Arc<SingleBuildFileCache>>) -> ActionInputMetadata {
        ActionInputMetadata {
            map: RwLock::new(HashMap::new()),
            per_build,
            allow_skyframe: AtomicBool::new(false),
            seen: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:424 ActionInputMap.putIfAbsent
    fn put_if_absent(
        &self,
        input: ActionInputRef,
        value: ActionInputMapValue,
    ) -> Option<ActionInputRef> {
        let mut map = self.map.write();
        match map.get(input.get_exec_path()) {
            Some((old_key, _)) => Some(old_key.clone()),
            None => {
                map.insert(input.get_exec_path().clone(), (input, value));
                None
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:371 ActionInputMap.put
    pub fn put(&self, input: &Artifact, metadata: FileArtifactValue) {
        self.put_input(artifact_input(input), metadata);
    }

    /// [`ActionInputMetadata::put`] for an input the execution layer holds.
    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:371 ActionInputMap.put
    pub fn put_input(&self, input: ActionInputRef, metadata: FileArtifactValue) {
        blaze_util::check_argument!(
            !is_tree_artifact_input(&input),
            "Can't add tree artifact: {} using put -- please use putTreeArtifact for that",
            input
        );
        blaze_util::check_argument!(
            !is_runfiles_tree_input(&input),
            "Can't add runfiles tree: {} using put -- please use putRunfilesMetadata for that",
            input
        );

        let display = input.clone();
        let old_key = self.put_if_absent(input, ActionInputMapValue::File(metadata));
        blaze_util::check_argument!(
            old_key.as_ref().is_none_or(|k| !is_tree_artifact_input(k)),
            "Tried to overwrite tree artifact with a file: '{}' with the same exec path",
            display
        );
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:393 ActionInputMap.putRunfilesMetadata
    pub fn put_runfiles_metadata(&self, input: &Artifact, metadata: Arc<RunfilesArtifactValue>) {
        blaze_util::check_argument!(input.is_runfiles_tree(), "{}", input);

        let old_key = self.put_if_absent(
            artifact_input(input),
            ActionInputMapValue::Runfiles(metadata),
        );
        check_state!(old_key.is_none());
        // runfilesTrees.add(metadata.getRunfilesTree()): getRunfilesTrees() is not part of the
        // ported InputMetadataProvider.
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:402 ActionInputMap.putTreeArtifact
    pub fn put_tree_artifact(&self, tree: &Artifact, tree_artifact_value: Arc<TreeArtifactValue>) {
        self.put_tree_artifact_input(artifact_input(tree), tree_artifact_value);
    }

    /// [`ActionInputMetadata::put_tree_artifact`] for an input the execution layer holds.
    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:402 ActionInputMap.putTreeArtifact
    pub fn put_tree_artifact_input(
        &self,
        tree: ActionInputRef,
        tree_artifact_value: Arc<TreeArtifactValue>,
    ) {
        blaze_util::check_argument!(is_tree_artifact_input(&tree), "{}", tree);
        let display = tree.clone();
        let old_key = self.put_if_absent(tree, ActionInputMapValue::Tree(tree_artifact_value));
        if let Some(old_key) = old_key {
            blaze_util::check_argument!(
                is_tree_artifact_input(&old_key),
                "Tried to overwrite file with a tree artifact: '{}' with the same exec path",
                display
            );
        }
    }

    /// `ActionInputMap.getInputMetadata(input) != null`: whether the map (not the Skyframe
    /// provider) knows `input`.
    // Java: com/google/devtools/build/lib/skyframe/ActionExecutionFunction.java:1256 InputDiscoveryState.getKnownDiscoveredInputMetadata
    pub fn is_known(&self, input: &Artifact) -> bool {
        self.action_input_map_get_input_metadata_checked(&artifact_input(input))
            .is_some()
    }

    /// The tree artifact of the map whose exec path is a prefix of `exec_path`
    /// (`treeArtifactsRoot.findTreeArtifactNodeAtPrefix(execPath)`).
    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:118 TrieArtifact.findTreeArtifactNodeAtPrefix
    fn find_tree_artifact_node_at_prefix(
        &self,
        exec_path: &PathFragment,
    ) -> Option<Arc<TreeArtifactValue>> {
        // The trie is walked from the root: the shortest prefix that is a tree artifact wins.
        let mut prefixes = Vec::new();
        let mut prefix = Some(exec_path.clone());
        while let Some(p) = prefix {
            prefix = p.get_parent_directory();
            prefixes.push(p);
        }
        let map = self.map.read();
        for p in prefixes.iter().rev() {
            if let Some((_, ActionInputMapValue::Tree(tree))) = map.get(p) {
                return Some(tree.clone());
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:212 ActionInputMap.getInputMetadataChecked
    fn action_input_map_get_input_metadata_checked(
        &self,
        input: &ActionInputRef,
    ) -> Option<FileArtifactValue> {
        if is_runfiles_tree_input(input) {
            return self
                .action_input_map_get_runfiles_metadata(input)
                .map(|runfiles_metadata| runfiles_metadata.get_metadata().clone());
        }

        let artifact = input.as_artifact();
        // `input instanceof TreeFileArtifact`: every tree file artifact (`getParent()` is set),
        // also one owned by an action template expansion, not only `isChildOfDeclaredDirectory()`.
        if let Some(parent) = artifact.and_then(|a| a.get_parent()) {
            if let Some((_, value)) = self.map.read().get(parent.get_exec_path()) {
                let ActionInputMapValue::Tree(tree) = value else {
                    panic!(
                        "IllegalArgumentException: Requested tree file artifact under \
                         non-tree/omitted tree artifact: {input}"
                    );
                };
                return find_child_entry_by_exec_path(tree, input.get_exec_path())
                    .map(|(_, child_value)| child_value.clone());
            }
        }
        if let Some((_, value)) = self.map.read().get(input.get_exec_path()) {
            return match value {
                ActionInputMapValue::Tree(tree_value) => {
                    Some(tree_value.metadata.clone().unwrap_or_else(|| {
                        panic!("NullPointerException: TreeArtifactValue.getMetadata() of {input}")
                    }))
                }
                ActionInputMapValue::File(value) => Some(value.clone()),
                ActionInputMapValue::Runfiles(_) => {
                    panic!(
                        "ClassCastException: RunfilesArtifactValue cannot be cast to FileArtifactValue: {input}"
                    )
                }
            };
        }
        if artifact.is_some() {
            // Non tree artifacts cannot overlap with tree files, therefore we can skip searching
            // the parents.
            return None;
        }

        // Check the trees in case input is a non-Artifact ActionInput pointing to a tree artifact
        // file. This can happen if both a TreeArtifact and a Fileset containing the TreeArtifact
        // are inputs to the same action.
        self.get_metadata_from_tree_artifacts(input.get_exec_path())
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:307 ActionInputMap.getMetadataFromTreeArtifacts
    fn get_metadata_from_tree_artifacts(
        &self,
        exec_path: &PathFragment,
    ) -> Option<FileArtifactValue> {
        let tree = self.find_tree_artifact_node_at_prefix(exec_path)?;
        find_child_entry_by_exec_path(&tree, exec_path).map(|(_, value)| value.clone())
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:258 ActionInputMap.getRunfilesMetadata
    fn action_input_map_get_runfiles_metadata(
        &self,
        input: &ActionInputRef,
    ) -> Option<Arc<RunfilesArtifactValue>> {
        blaze_util::check_argument!(is_runfiles_tree_input(input), "{}", input);

        match self.map.read().get(input.get_exec_path()) {
            None => None,
            Some((_, ActionInputMapValue::Runfiles(value))) => Some(value.clone()),
            Some(_) => {
                panic!("ClassCastException: the value of {input} is not a RunfilesArtifactValue")
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:320 ActionInputMap.getTreeMetadata
    fn action_input_map_get_tree_metadata(
        &self,
        input: &ActionInputRef,
    ) -> Option<Arc<TreeArtifactValue>> {
        blaze_util::check_argument!(is_tree_artifact_input(input), "{}", input);
        match self.map.read().get(input.get_exec_path()) {
            Some((_, ActionInputMapValue::Tree(tree_value))) => Some(tree_value.clone()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:342 ActionInputMap.getInput
    fn action_input_map_get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        if let Some((key, _)) = self.map.read().get(exec_path) {
            return Some(key.clone());
        }

        // Search ancestor paths since execPathString may point to a TreeFileArtifact within one of
        // the tree artifacts.
        let tree = self.find_tree_artifact_node_at_prefix(exec_path)?;

        // We must return an entry from the map since a duplicate would not have the generating
        // action key set.
        find_child_entry_by_exec_path(&tree, exec_path).map(|(key, _)| key.clone())
    }

    /// The `ActionInputMetadataProvider` over the map alone (Java's
    /// `state.actionInputMetadataProvider`, used for the action cache check and to reconstruct
    /// rich data on an action cache hit).
    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:45 ActionInputMetadataProvider
    pub fn action_input_map_only(&self) -> ActionInputMapOnly<'_> {
        ActionInputMapOnly(self)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:76 ActionInputMetadataProvider.getInputMetadataChecked
    fn action_input_metadata_provider_get_input_metadata_checked(
        &self,
        action_input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        if action_input.as_artifact().is_none() {
            return Ok(None);
        }
        let value = self.action_input_map_get_input_metadata_checked(action_input);
        if let Some(value) = value {
            // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:150 ActionInputMetadataProvider.checkExists
            if value == FileArtifactValue::MissingFileMarker {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{action_input} does not exist"),
                ));
            }
            return Ok(Some(value));
        }
        // filesetMapping: no fileset is ever put in the map (ActionInputMapHelper.addToMap fails
        // with unported! for a fileset input).
        Ok(None)
    }

    /// Allow Skyframe access while the returned guard is alive.
    ///
    /// This should only happen during input discovery, so we disallow it everywhere else.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeInputMetadataProvider.java:153 SkyframeInputMetadataProvider.withSkyframeAllowed
    pub fn with_skyframe_allowed(&self) -> SkyframeAllowed<'_> {
        self.allow_skyframe.store(true, Ordering::SeqCst);
        SkyframeAllowed(self)
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeInputMetadataProvider.java:167 SkyframeInputMetadataProvider.getInputMetadataChecked
    fn skyframe_input_metadata_provider_get_input_metadata_checked(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        let Some(artifact) = input.as_artifact() else {
            let per_build = self.per_build();
            if !input
                .get_exec_path()
                .starts_with(per_build.get_relative_output_path())
            {
                return per_build.get_input_metadata(input);
            } else {
                return Ok(None);
            }
        };

        if artifact.is_source_artifact() {
            return self.per_build().get_input_metadata(input);
        }

        if !self.allow_skyframe.load(Ordering::SeqCst) {
            return Ok(None);
        }

        // artifact instanceof SpecialArtifact
        let blaze_artifact = as_blaze_artifact(input)
            .unwrap_or_else(|| panic!("ClassCastException: {input} is not an Artifact"));
        if blaze_artifact.get_special_artifact_type().is_some() {
            return Ok(None);
        }

        // PORT: Java reads the ActionExecutionValue of the input's generating action from the
        // graph (`evaluator.getExistingValue(Artifact.key(artifact))`; these are transitive
        // dependencies of the action) and returns `getExistingFileArtifactValue(artifact)`. The
        // engine gives no handle to read a done node from an InputMetadataProvider (the
        // Environment is not shared with the action), so the metadata is rebuilt from the output
        // file, which is what that value was built from
        // (ActionOutputMetadataStore.constructFileArtifactValueFromFilesystem). A file that does
        // not exist means the generating action is not done: Java requests it
        // (`env.getValue(key)`) and throws MissingDepExecException.
        let path = blaze_artifact.get_path();
        let Some(stat) = path.stat_if_found(Symlinks::Follow)? else {
            blaze_util::unported!(
                "SkyframeInputMetadataProvider.SkyframeLookup.tryLookup",
                "the generating action of {} is not done (MissingDepExecException)",
                input
            );
        };
        let value = FileArtifactValue::create_from_stat(&path, &stat)?;
        self.seen
            .lock()
            .insert(input.get_exec_path().clone(), input.clone());
        Ok(Some(value))
    }

    /// `SkyframeInputMetadataProvider.perBuild`.
    fn per_build(&self) -> &SingleBuildFileCache {
        match &self.per_build {
            Some(per_build) => per_build.as_ref(),
            None => blaze_util::unported!(
                "SkyframeActionExecutor.getPerBuildFileCache",
                "the executor was not given the per-build file cache \
                 (SkyframeActionExecutor::configure_file_cache)"
            ),
        }
    }
}

/// See [`ActionInputMetadata::with_skyframe_allowed`].
pub struct SkyframeAllowed<'a>(&'a ActionInputMetadata);

impl Drop for SkyframeAllowed<'_> {
    fn drop(&mut self) {
        self.0.allow_skyframe.store(false, Ordering::SeqCst);
    }
}

// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:480 ActionInputMap.isTreeArtifact
fn is_tree_artifact_input(input: &ActionInputRef) -> bool {
    input.as_artifact().is_some_and(|a| a.is_tree_artifact())
}

// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:488 ActionInputMap.isRunfilesTree
fn is_runfiles_tree_input(input: &ActionInputRef) -> bool {
    input.as_artifact().is_some_and(|a| a.is_runfiles_tree())
}

/// The child of `tree` at `exec_path`, with its metadata.
// Java: com/google/devtools/build/lib/skyframe/TreeArtifactValue.java:372 TreeArtifactValue.findChildEntryByExecPath
fn find_child_entry_by_exec_path<'a>(
    tree: &'a TreeArtifactValue,
    exec_path: &PathFragment,
) -> Option<&'a (ActionInputRef, FileArtifactValue)> {
    // The children are sorted by exec path (TreeArtifactValue::build).
    tree.child_values
        .binary_search_by(|(child, _)| child.get_exec_path().cmp(exec_path))
        .ok()
        .map(|index| &tree.child_values[index])
}

/// See [`ActionInputMetadata::action_input_map_only`].
pub struct ActionInputMapOnly<'a>(&'a ActionInputMetadata);

impl InputMetadataProvider for ActionInputMapOnly<'_> {
    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:76 ActionInputMetadataProvider.getInputMetadataChecked
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        self.0
            .action_input_metadata_provider_get_input_metadata_checked(input)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:93 ActionInputMetadataProvider.getTreeMetadata
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.0.action_input_map_get_tree_metadata(input)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:105 ActionInputMetadataProvider.getFileset
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        // No fileset is ever put in the map (see ActionInputMapHelper.addToMap).
        None
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:117 ActionInputMetadataProvider.getRunfilesMetadata
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.0.action_input_map_get_runfiles_metadata(input)
    }

    // Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:128 ActionInputMetadataProvider.getInput
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.0.action_input_map_get_input(exec_path)
    }
}

/// `DelegatingPairInputMetadataProvider(actionInputMetadataProvider,
/// skyframeInputMetadataProvider)`: `state.compositeInputMetadataProvider`.
impl InputMetadataProvider for ActionInputMetadata {
    // Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:38 DelegatingPairInputMetadataProvider.getInputMetadataChecked
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
        let metadata = self.action_input_metadata_provider_get_input_metadata_checked(input)?;
        match metadata {
            Some(metadata) if metadata != FileArtifactValue::MissingFileMarker => {
                Ok(Some(metadata))
            }
            _ => self.skyframe_input_metadata_provider_get_input_metadata_checked(input),
        }
    }

    // Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:48 DelegatingPairInputMetadataProvider.getTreeMetadata
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        // SkyframeInputMetadataProvider.getTreeMetadata returns null.
        self.action_input_map_get_tree_metadata(input)
    }

    // Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:62 DelegatingPairInputMetadataProvider.getFileset
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    // Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:88 DelegatingPairInputMetadataProvider.getRunfilesMetadata
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        // SkyframeInputMetadataProvider.getRunfilesMetadata returns null.
        self.action_input_map_get_runfiles_metadata(input)
    }

    // Java: com/google/devtools/build/lib/actions/DelegatingPairInputMetadataProvider.java:103 DelegatingPairInputMetadataProvider.getInput
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        if let Some(input) = self.action_input_map_get_input(exec_path) {
            return Some(input);
        }
        // Java: com/google/devtools/build/lib/skyframe/SkyframeInputMetadataProvider.java:236 SkyframeInputMetadataProvider.getInput
        if let Some(result) = self.seen.lock().get(exec_path) {
            return Some(result.clone());
        }
        self.per_build.as_ref().and_then(|p| p.get_input(exec_path))
    }
}

/// The `ActionExecutionContext` for one action (or, without an output metadata store, the
/// `ActionExecutionContext.forInputDiscovery` of one action).
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:59 ActionExecutionContext
struct ActionExecutionContextImpl<'a> {
    /// The action the context was created for: the action of the progress events
    /// ([`ActionExecutionContext::post_progress`]).
    action: ActionRef,
    /// `getExecRoot()`: the executor's exec root, on the action file system when there is one.
    exec_root: Path,
    executor: &'a SkyframeActionExecutor,
    input_metadata: Arc<ActionInputMetadata>,
    output_metadata_store: Option<Arc<ActionOutputMetadataStore>>,
    /// `actionFileSystem`: the action's `RemoteActionFileSystem` when the output service has one
    /// (for execution, the one of the output metadata store; for input discovery, the same one).
    action_file_system: Option<Arc<RemoteActionFileSystem>>,
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

/// `ActionExecutionContext.getExecRoot()`: `actionFileSystem != null ?
/// actionFileSystem.getPath(executor.getExecRoot().asFragment()) : executor.getExecRoot()`.
///
/// PORT: Java computes it on every call; the context returns a borrowed `Path`, so it is computed
/// once when the context is created (the action file system of a context never changes). The
/// action file system is the one of the action's output metadata store, which input discovery
/// shares (`get_remote_action_file_system`).
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:369 ActionExecutionContext.getExecRoot
fn action_exec_root(
    executor: &SkyframeActionExecutor,
    action_file_system: Option<&Arc<RemoteActionFileSystem>>,
) -> Path {
    match action_file_system {
        Some(action_file_system) => action_file_system.get_path(executor.exec_root.as_fragment()),
        None => executor.exec_root.clone(),
    }
}

impl ActionExecutionContext for ActionExecutionContextImpl<'_> {
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:369 ActionExecutionContext.getExecRoot
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
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
    /// `progress.postTo(getEventHandler(), action)`.
    ///
    /// PORT: the posted events hold the action (`ActionRef`); the metadata the strategies pass
    /// is the action this context was created for (a snapshot of it), so the context's own
    /// action is posted.
    // Java: com/google/devtools/build/lib/exec/SpawnRunner.java:116 SpawnRunner.ProgressStatus.postTo
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, progress: &ProgressStatus) {
        let event_handler = self.get_event_handler();
        let action = self.action.clone();
        match progress {
            // Java: com/google/devtools/build/lib/exec/SpawnSchedulingEvent.java:36 SpawnSchedulingEvent.postTo
            ProgressStatus::Scheduling(name) => {
                event_handler.post(Arc::new(SchedulingActionEvent {
                    action,
                    strategy: name.clone(),
                }))
            }
            // Java: com/google/devtools/build/lib/exec/SpawnExecutingEvent.java:41 SpawnExecutingEvent.postTo
            ProgressStatus::Executing(name) => event_handler.post(Arc::new(RunningActionEvent {
                action,
                strategy: name.clone(),
            })),
            // Java: com/google/devtools/build/lib/exec/SpawnCheckingCacheEvent.java:34 SpawnCheckingCacheEvent.postTo
            ProgressStatus::CheckingCache(name) => {
                event_handler.post(Arc::new(CachingActionEvent {
                    action,
                    strategy: name.clone(),
                }))
            }
        }
    }
    fn get_verbose_failures(&self) -> bool {
        self.executor.verbose_failures
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:376 ActionExecutionContext.getActionFileSystem
    fn get_remote_action_file_system(&self) -> Option<Arc<RemoteActionFileSystem>> {
        self.action_file_system.clone()
    }
    /// `lostInputsCheck.checkForLostInputs()` with the check the executor gives the context:
    /// `actionFileSystem == null ? LostInputsCheck.NONE : () ->
    /// outputService.checkActionFileSystemForLostInputs(actionFileSystem, action)`, in the
    /// `toExecException()` form the trait returns (see
    /// `ActionExecutionContext::check_for_lost_inputs`).
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1614 SkyframeActionExecutor.lostInputsCheck
    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        match self.get_remote_action_file_system() {
            Some(action_file_system) => {
                match SkyframeActionExecutor::action_file_system_lost_inputs(&action_file_system) {
                    Some(lost_inputs) => Err(blaze_exec::actions::LostInputsExecException::create(
                        lost_inputs,
                    )),
                    None => Ok(()),
                }
            }
            None => Ok(()),
        }
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
    fn get_input_path(&self, input: &dyn ActionInput) -> Path {
        // ArtifactPathResolver.IDENTITY: an artifact resolves to `artifact.getPath()` (source
        // artifacts live under their source root); other inputs under the execroot.
        // TransformResolver (an action file system): the same paths on the action file system.
        let path = match input.as_any().downcast_ref::<Artifact>() {
            Some(a) => a.get_path(),
            None => self.executor.exec_root.get_relative(input.get_exec_path()),
        };
        self.convert_path(&path)
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
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:358 ActionExecutionContext.getOutputMetadataStore
    fn get_tree_artifact_value(
        &self,
        tree_artifact: &ActionInputRef,
    ) -> std::io::Result<TreeArtifactValue> {
        let Some(store) = &self.output_metadata_store else {
            panic!("NullPointerException: no OutputMetadataStore during input discovery");
        };
        match store.get_tree_artifact_value(tree_artifact) {
            Ok(v) => Ok(v),
            Err(IoOrInterruptedException::Io(e)) => Err(e),
            Err(IoOrInterruptedException::Interrupted(_)) => {
                Err(std::io::Error::other("interrupted"))
            }
        }
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:358 ActionExecutionContext.getOutputMetadataStore
    fn reset_outputs(&self, outputs: &[ActionInputRef]) {
        let Some(store) = &self.output_metadata_store else {
            panic!("NullPointerException: no OutputMetadataStore during input discovery");
        };
        store.reset_outputs(outputs);
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

/// Port of the tree artifact tests of `ActionOutputMetadataStoreTest`.
///
/// PORT: Java uses an `InMemoryFileSystem` that records `chmod` calls; these tests use the real
/// file system under a fresh temporary directory and read the resulting modes.
#[cfg(test)]
mod action_output_metadata_store_tests {
    use std::any::Any;
    use std::fmt;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::os::unix::fs::PermissionsExt;

    use blaze_actions::actions::action_lookup::ActionLookupKey;
    use blaze_actions::actions::artifact::SpecialArtifactType;
    use blaze_actions::{ActionLookupData, ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
    use blaze_cmdline::Label;
    use blaze_exec::actions::action_adapters::artifact_input;
    use blaze_exec::actions::cache::{IoOrInterruptedException, OutputMetadataStore};
    use std::sync::Arc;

    use blaze_exec::exec::remote_action_file_system::RemoteActionFileSystem;
    use blaze_vfs::{DigestHashFunction, FileSystemExt, UnixFileSystem};

    use super::{ActionInputMetadata, ActionOutputMetadataStore, OutputPermissions};

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

    /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(outputRoot, rootRelativePath)`
    /// for the output root `out` of the execroot `exec_root`.
    pub(super) fn create_tree_artifact(exec_root: &str, root_relative_path: &str) -> Artifact {
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = fs.get_path(exec_root);
        let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
        let exec_path = output_root
            .get_exec_path()
            .get_relative_str(root_relative_path);
        let owner = ActionLookupKeyRef::new(Key("//foo:bar".to_string()));
        let tree_artifact = Artifact::create_special_artifact(
            output_root,
            exec_path,
            owner.clone(),
            SpecialArtifactType::TREE,
        );
        tree_artifact.set_generating_action_key(ActionLookupData::create(owner, 0));
        tree_artifact
    }

    /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(outputRoot, "foo/bar")` under a
    /// fresh execroot.
    fn create_tree_artifact_with_generating_action() -> (tempfile::TempDir, Artifact) {
        let dir = tempfile::tempdir().unwrap();
        let tree_artifact = create_tree_artifact(dir.path().to_str().unwrap(), "foo/bar");
        (dir, tree_artifact)
    }

    /// `scratch.file(child.getPath().getPathString(), contents)`.
    fn scratch_file(child: &Artifact, contents: &str) -> String {
        let path = child.get_path().get_path_string().to_string();
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn create_store(tree_artifact: &Artifact) -> ActionOutputMetadataStore {
        ActionOutputMetadataStore::create(
            /* archived_tree_artifacts_enabled= */ false,
            OutputPermissions::READONLY,
            vec![tree_artifact.clone()],
        )
    }

    fn mode(path: &str) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn chmod(path: &str, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn get_output_metadata(
        store: &ActionOutputMetadataStore,
        artifact: &Artifact,
    ) -> Result<blaze_exec::actions::FileArtifactValue, std::io::Error> {
        match store.get_output_metadata(&artifact_input(artifact)) {
            Ok(value) => Ok(value.expect("a known output")),
            Err(IoOrInterruptedException::Io(e)) => Err(e),
            Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
        }
    }

    #[test]
    fn creates_tree_artifact_value_from_filesystem() {
        let (_dir, tree_artifact) = create_tree_artifact_with_generating_action();
        let child1 = Artifact::create_tree_output_str(&tree_artifact, "child1");
        let child2 = Artifact::create_tree_output_str(&tree_artifact, "child2");
        let child1_path = scratch_file(&child1, "child1");
        let child2_path = scratch_file(&child2, "child2");
        chmod(&child1_path, 0o644);
        chmod(&child2_path, 0o644);

        let store = create_store(&tree_artifact);

        let tree_metadata = get_output_metadata(&store, &tree_artifact).unwrap();
        let child1_metadata = get_output_metadata(&store, &child1).unwrap();
        let child2_metadata = get_output_metadata(&store, &child2).unwrap();
        let all_tree_artifact_data = store.get_all_tree_artifact_data();
        let tree = all_tree_artifact_data.get(&tree_artifact).unwrap();

        assert_eq!(tree.metadata.as_ref(), Some(&tree_metadata));
        assert_eq!(
            tree.child_values,
            vec![
                (artifact_input(&child1), child1_metadata),
                (artifact_input(&child2), child2_metadata),
            ]
        );
        assert!(store.get_all_artifact_data().is_empty());
        // assertThat(chmodCalls).isEmpty()
        assert_eq!(mode(&child1_path), 0o644);
        assert_eq!(mode(&child2_path), 0o644);
    }

    #[test]
    fn with_dangling_symlink_in_tree_artifact_fails_with_exception() {
        let (_dir, tree_artifact) = create_tree_artifact_with_generating_action();
        let child = Artifact::create_tree_output_str(&tree_artifact, "child");
        tree_artifact
            .get_path()
            .create_directory_and_parents()
            .unwrap();
        std::os::unix::fs::symlink("/does_not_exist", child.get_path().get_path_string()).unwrap();

        let store = create_store(&tree_artifact);

        let e = get_output_metadata(&store, &tree_artifact).unwrap_err();
        assert!(e.to_string().contains("dangling symbolic link"), "{e}");
        // The message SkyframeActionExecutor.reportOutputTreeArtifactErrors prints.
        assert_eq!(e.to_string(), "child child is a dangling symbolic link");
    }

    #[test]
    fn output_tree_artifact_not_previously_injected_in_execution_mode() {
        let (_dir, tree_artifact) = create_tree_artifact_with_generating_action();
        let child1 = Artifact::create_tree_output_str(&tree_artifact, "child1");
        let child2 = Artifact::create_tree_output_str(&tree_artifact, "subdir/child2");
        let child1_path = scratch_file(&child1, "contents1");
        let child2_path = scratch_file(&child2, "contents2");
        let tree_path = tree_artifact.get_path().get_path_string().to_string();
        let subdir_path = std::path::Path::new(&child2_path)
            .parent()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let store = create_store(&tree_artifact);
        store.prepare_for_action_execution();

        let tree_metadata = get_output_metadata(&store, &tree_artifact).unwrap();
        let child1_metadata = get_output_metadata(&store, &child1).unwrap();
        let child2_metadata = get_output_metadata(&store, &child2).unwrap();
        let all_tree_artifact_data = store.get_all_tree_artifact_data();
        let tree = all_tree_artifact_data.get(&tree_artifact).unwrap();

        assert_eq!(tree.metadata.as_ref(), Some(&tree_metadata));
        assert_eq!(
            tree.child_values,
            vec![
                (artifact_input(&child1), child1_metadata),
                (artifact_input(&child2), child2_metadata),
            ]
        );
        assert!(store.get_all_artifact_data().is_empty());
        // assertThat(chmodCalls).containsExactly(treeArtifact.getPath(), 0555, child1Path, 0555,
        // child2Path, 0555, child2Path.getParentDirectory(), 0555)
        assert_eq!(mode(&tree_path), 0o555);
        assert_eq!(mode(&child1_path), 0o555);
        assert_eq!(mode(&child2_path), 0o555);
        assert_eq!(mode(&subdir_path), 0o555);

        // Let the temporary directory be deleted.
        chmod(&tree_path, 0o755);
        chmod(&subdir_path, 0o755);
    }

    /// Symlinks inside the tree are followed (`TreeArtifactValue.visitTree`): the files below a
    /// symlink to a directory are children, and entries reached through a symlink are not
    /// chmod'ed. Expected children: the oracle's `exec-tree` probe scenario.
    #[test]
    fn tree_artifact_value_follows_symlinks_inside_the_tree() {
        let (_dir, tree_artifact) = create_tree_artifact_with_generating_action();
        let top = Artifact::create_tree_output_str(&tree_artifact, "top.txt");
        let nested = Artifact::create_tree_output_str(&tree_artifact, "sub/n.txt");
        scratch_file(&top, "top");
        scratch_file(&nested, "nested");
        let tree_path = tree_artifact.get_path().get_path_string().to_string();
        std::os::unix::fs::symlink("sub", format!("{tree_path}/link_to_sub")).unwrap();
        std::os::unix::fs::symlink("top.txt", format!("{tree_path}/link_to_file")).unwrap();
        let store = create_store(&tree_artifact);
        store.prepare_for_action_execution();

        get_output_metadata(&store, &tree_artifact).unwrap();
        let all_tree_artifact_data = store.get_all_tree_artifact_data();
        let tree = all_tree_artifact_data.get(&tree_artifact).unwrap();

        let children: Vec<String> = tree
            .child_values
            .iter()
            .map(|(child, _)| {
                child
                    .get_exec_path()
                    .relative_to(tree_artifact.get_exec_path())
                    .get_path_string()
                    .to_string()
            })
            .collect();
        assert_eq!(
            children,
            vec!["link_to_file", "link_to_sub/n.txt", "sub/n.txt", "top.txt"]
        );
        // The file a symlink points to has the metadata of the file itself.
        assert_eq!(
            tree.child_values[0].1.get_digest(),
            tree.child_values[3].1.get_digest()
        );
        assert_eq!(mode(&format!("{tree_path}/sub")), 0o555);

        // Let the temporary directory be deleted.
        chmod(&tree_path, 0o755);
        chmod(&format!("{tree_path}/sub"), 0o755);
    }

    /// A declared symlink output (`ctx.actions.declare_symlink`) of a fresh execroot.
    fn create_symlink_artifact() -> (tempfile::TempDir, Artifact) {
        let dir = tempfile::tempdir().unwrap();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = fs.get_path(dir.path().to_str().unwrap());
        let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
        let exec_path = output_root.get_exec_path().get_relative_str("foo/link");
        let owner = ActionLookupKeyRef::new(Key("//foo:bar".to_string()));
        let artifact = Artifact::create_special_artifact(
            output_root,
            exec_path,
            owner.clone(),
            SpecialArtifactType::UNRESOLVED_SYMLINK,
        );
        artifact.set_generating_action_key(ActionLookupData::create(owner, 0));
        artifact
            .get_path()
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        (dir, artifact)
    }

    /// The metadata of a declared symlink output is read with `readlink`, never by following the
    /// link: a dangling target is fine.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:487 ActionOutputMetadataStore.fileArtifactValueFromArtifact
    #[test]
    fn declared_symlink_output_has_unresolved_symlink_metadata() {
        let (_dir, symlink) = create_symlink_artifact();
        std::os::unix::fs::symlink(
            "../nonexistent/dangling",
            symlink.get_path().get_path_string(),
        )
        .unwrap();
        let store = create_store(&symlink);

        let metadata = get_output_metadata(&store, &symlink).unwrap();

        assert_eq!(
            metadata.get_type(),
            blaze_exec::actions::FileStateType::Symlink
        );
        assert_eq!(
            metadata.get_unresolved_symlink_target(),
            Some("../nonexistent/dangling")
        );
        assert_eq!(
            metadata,
            blaze_exec::actions::FileArtifactValue::create_for_unresolved_symlink(
                &symlink.get_path()
            )
            .unwrap()
        );
        assert_eq!(store.get_all_artifact_data().get(&symlink), Some(&metadata));
    }

    /// A declared symlink output that the action wrote as a regular file fails with
    /// `NotASymlinkException` ("declared output '...' is not a symlink" in `checkOutputs`).
    #[test]
    fn declared_symlink_output_written_as_file_is_not_a_symlink() {
        let (_dir, symlink) = create_symlink_artifact();
        let path = scratch_file(&symlink, "text");
        let store = create_store(&symlink);

        let e = get_output_metadata(&store, &symlink).unwrap_err();

        assert!(blaze_vfs::NotASymlinkException::is(&e), "{e}");
        assert_eq!(e.to_string(), format!("{path} is not a symlink"));
    }

    /// A declared symlink output that the action did not create fails with a missing-file error.
    #[test]
    fn declared_symlink_output_not_created_is_missing() {
        let (_dir, symlink) = create_symlink_artifact();
        let store = create_store(&symlink);

        let e = get_output_metadata(&store, &symlink).unwrap_err();

        assert!(!blaze_vfs::NotASymlinkException::is(&e), "{e}");
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
    }

    /// The fixture of the `ActionOutputMetadataStoreTest` methods that use an action file
    /// system: one file system, the execroot `<tmp>/workspace`, its source root and the output
    /// root `out` (both created).
    struct ActionFsFixture {
        _dir: tempfile::TempDir,
        fs: blaze_vfs::FileSystemRef,
        exec_root: blaze_vfs::Path,
        source_root: ArtifactRoot,
        output_root: ArtifactRoot,
    }

    #[derive(Clone, Copy, Debug)]
    enum ArtifactType {
        Source,
        Output,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum ResolvedPathDepth {
        Shallow,
        Deep,
    }

    impl ActionFsFixture {
        // Java: ActionOutputMetadataStoreTest.createRootDirs
        fn new() -> ActionFsFixture {
            let dir = tempfile::tempdir().unwrap();
            let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
            let exec_root = fs.get_path(&format!("{}/workspace", dir.path().to_str().unwrap()));
            let source_root =
                ArtifactRoot::as_source_root(blaze_vfs::Root::from_path(exec_root.clone()));
            let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
            exec_root.create_directory_and_parents().unwrap();
            output_root
                .get_root()
                .as_path()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
            ActionFsFixture {
                _dir: dir,
                fs,
                exec_root,
                source_root,
                output_root,
            }
        }

        fn owner() -> ActionLookupKeyRef {
            ActionLookupKeyRef::new(Key("//foo:bar".to_string()))
        }

        /// `ActionsTestUtil.createArtifactWithRootRelativePath(root, rootRelativePath)`.
        fn create_artifact_with_root_relative_path(
            &self,
            root: &ArtifactRoot,
            root_relative_path: &str,
        ) -> Artifact {
            let exec_path = root.get_exec_path().get_relative_str(root_relative_path);
            if root.is_source_root() {
                Artifact::new_source_artifact(
                    root.clone(),
                    exec_path,
                    blaze_actions::actions::action_lookup::ArtifactOwner::NullOwner,
                )
            } else {
                Artifact::create_derived_artifact(root.clone(), exec_path, Self::owner())
            }
        }

        /// `ActionsTestUtil.createTreeArtifactWithGeneratingAction(outputRoot, rootRelativePath)`.
        fn create_tree_artifact_with_generating_action(
            &self,
            root_relative_path: &str,
        ) -> Artifact {
            let exec_path = self
                .output_root
                .get_exec_path()
                .get_relative_str(root_relative_path);
            let owner = Self::owner();
            let tree_artifact = Artifact::create_special_artifact(
                self.output_root.clone(),
                exec_path,
                owner.clone(),
                SpecialArtifactType::TREE,
            );
            tree_artifact.set_generating_action_key(ActionLookupData::create(owner, 0));
            tree_artifact
        }

        // Java: ActionOutputMetadataStoreTest.createRemoteActionFileSystem
        fn create_remote_action_file_system(
            &self,
            input_map: ActionInputMetadata,
        ) -> Arc<RemoteActionFileSystem> {
            RemoteActionFileSystem::new(
                self.fs.clone(),
                self.exec_root.as_fragment().clone(),
                self.output_root.get_exec_path().get_path_string(),
                Arc::new(input_map) as Arc<dyn blaze_exec::actions::InputMetadataProvider>,
                // mock(RemoteActionInputFetcher.class): nothing is fetched.
                None,
            )
        }

        // Java: ActionOutputMetadataStoreTest.createStore
        fn create_store(
            &self,
            output: &Artifact,
            action_fs: Option<Arc<RemoteActionFileSystem>>,
        ) -> ActionOutputMetadataStore {
            let mut store = ActionOutputMetadataStore::create(
                /* archived_tree_artifacts_enabled= */ false,
                OutputPermissions::READONLY,
                vec![output.clone()],
            );
            store.action_file_system = action_fs;
            store
        }
    }

    /// `TreeArtifactValue.equals`: the digest (in the aggregate metadata), the children, the
    /// archived representation and the resolved path.
    fn assert_tree_eq(
        actual: &blaze_exec::actions::TreeArtifactValue,
        expected: &blaze_exec::actions::TreeArtifactValue,
        what: &str,
    ) {
        assert_eq!(actual.metadata, expected.metadata, "{what}");
        assert_eq!(actual.child_values, expected.child_values, "{what}");
        assert_eq!(
            actual.archived_representation, expected.archived_representation,
            "{what}"
        );
        assert_eq!(actual.resolved_path, expected.resolved_path, "{what}");
    }

    /// `actionFs.getPath(artifact.getPath().asFragment())`.
    fn on(action_fs: &Arc<RemoteActionFileSystem>, path: &blaze_vfs::Path) -> blaze_vfs::Path {
        action_fs.get_path(path.as_fragment())
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:376 ActionOutputMetadataStoreTest.fileArtifactMaterializedAsSymlinkToNonArtifact
    #[test]
    fn file_artifact_materialized_as_symlink_to_non_artifact() {
        let f = ActionFsFixture::new();
        let resolved_path = f
            .output_root
            .get_root()
            .as_path()
            .unwrap()
            .get_relative_str("resolved");
        let output_artifact = f.create_artifact_with_root_relative_path(&f.output_root, "output");

        let action_fs =
            f.create_remote_action_file_system(ActionInputMetadata::with_providers(None));

        let store = f.create_store(&output_artifact, Some(action_fs.clone()));
        store.prepare_for_action_execution();

        on(&action_fs, &output_artifact.get_path())
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        on(&action_fs, &output_artifact.get_path())
            .create_symbolic_link_to_fragment(resolved_path.as_fragment())
            .unwrap();
        std::fs::write(resolved_path.get_path_string(), "foo").unwrap();

        assert_eq!(
            get_output_metadata(&store, &output_artifact).unwrap(),
            blaze_exec::actions::FileArtifactValue::create_for_testing(&resolved_path).unwrap()
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:400 ActionOutputMetadataStoreTest.fileArtifactMaterializedAsSymlinkToFileArtifact
    #[test]
    fn file_artifact_materialized_as_symlink_to_file_artifact() {
        // @TestParameter ArtifactType artifactType, @TestParameter ResolvedPathDepth depth
        for artifact_type in [ArtifactType::Source, ArtifactType::Output] {
            for depth in [ResolvedPathDepth::Shallow, ResolvedPathDepth::Deep] {
                let f = ActionFsFixture::new();
                let root = match artifact_type {
                    ArtifactType::Source => f.source_root.clone(),
                    ArtifactType::Output => f.output_root.clone(),
                };
                let resolved_artifact =
                    f.create_artifact_with_root_relative_path(&root, "resolved");
                let input_artifact = match depth {
                    ResolvedPathDepth::Shallow => resolved_artifact.clone(),
                    ResolvedPathDepth::Deep => {
                        f.create_artifact_with_root_relative_path(&f.output_root, "input")
                    }
                };
                let output_artifact =
                    f.create_artifact_with_root_relative_path(&f.output_root, "output");

                let mut input_metadata =
                    blaze_exec::actions::FileArtifactValue::create_for_normal_file(
                        Some(vec![1, 2, 3]),
                        None,
                        123,
                    );
                if resolved_artifact != input_artifact {
                    input_metadata =
                        blaze_exec::actions::FileArtifactValue::create_from_existing_with_resolved_path(
                            input_metadata,
                            resolved_artifact.get_path().as_fragment().clone(),
                        );
                }

                let input_map = ActionInputMetadata::with_providers(None);
                input_map.put(&input_artifact, input_metadata.clone());

                let action_fs = f.create_remote_action_file_system(input_map);

                let store = f.create_store(&output_artifact, Some(action_fs.clone()));
                store.prepare_for_action_execution();

                on(&action_fs, &output_artifact.get_path())
                    .get_parent_directory()
                    .unwrap()
                    .create_directory_and_parents()
                    .unwrap();
                on(&action_fs, &output_artifact.get_path())
                    .create_symbolic_link_to_fragment(input_artifact.get_path().as_fragment())
                    .unwrap();

                let expected =
                    blaze_exec::actions::FileArtifactValue::create_from_existing_with_resolved_path(
                        input_metadata,
                        resolved_artifact.get_path().as_fragment().clone(),
                    );
                assert_eq!(
                    get_output_metadata(&store, &output_artifact).unwrap(),
                    expected,
                    "{artifact_type:?} {depth:?}"
                );
            }
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:452 ActionOutputMetadataStoreTest.treeArtifactMaterializedAsSymlinkToNonArtifact
    #[test]
    fn tree_artifact_materialized_as_symlink_to_non_artifact() {
        let f = ActionFsFixture::new();
        let resolved_path = f
            .output_root
            .get_root()
            .as_path()
            .unwrap()
            .get_relative_str("resolved");
        let output_artifact = f.create_tree_artifact_with_generating_action("output");

        let action_fs =
            f.create_remote_action_file_system(ActionInputMetadata::with_providers(None));

        let store = f.create_store(&output_artifact, Some(action_fs.clone()));
        store.prepare_for_action_execution();

        on(&action_fs, &output_artifact.get_path())
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        on(&action_fs, &output_artifact.get_path())
            .create_symbolic_link_to_fragment(resolved_path.as_fragment())
            .unwrap();
        on(&action_fs, &resolved_path)
            .create_directory_and_parents()
            .unwrap();
        std::fs::write(
            resolved_path.get_relative_str("child").get_path_string(),
            "foo",
        )
        .unwrap();

        // Java evaluates `store.getTreeArtifactValue(outputArtifact)` (which chmods the child and
        // so changes its ctime) before `createForTesting` stats the child for the expected value.
        let tree = match store.get_tree_artifact_value(&artifact_input(&output_artifact)) {
            Ok(tree) => tree,
            Err(IoOrInterruptedException::Io(e)) => panic!("{e}"),
            Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
        };
        let child = Artifact::create_tree_output_str(&output_artifact, "child");
        let expected = blaze_exec::actions::TreeArtifactValue::build(
            vec![(
                artifact_input(&child),
                blaze_exec::actions::FileArtifactValue::create_for_testing(
                    &resolved_path.get_relative_str("child"),
                )
                .unwrap(),
            )],
            None,
            None,
        );
        assert_tree_eq(&tree, &expected, "");

        // Let the temporary directory be deleted (the store chmod'ed the tree to 0555).
        chmod(resolved_path.get_path_string(), 0o755);
    }

    // Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:482 ActionOutputMetadataStoreTest.treeArtifactMaterializedAsSymlinkToAnotherTreeArtifact
    #[test]
    fn tree_artifact_materialized_as_symlink_to_another_tree_artifact() {
        // @TestParameter ResolvedPathDepth depth
        for depth in [ResolvedPathDepth::Shallow, ResolvedPathDepth::Deep] {
            let f = ActionFsFixture::new();
            let resolved_artifact = f.create_tree_artifact_with_generating_action("resolved");
            let input_artifact = match depth {
                ResolvedPathDepth::Shallow => resolved_artifact.clone(),
                ResolvedPathDepth::Deep => f.create_tree_artifact_with_generating_action("input"),
            };
            let output_artifact = f.create_tree_artifact_with_generating_action("output");

            let input_metadata = blaze_exec::actions::TreeArtifactValue::build(
                vec![(
                    artifact_input(&Artifact::create_tree_output_str(&input_artifact, "child")),
                    blaze_exec::actions::FileArtifactValue::create_for_normal_file(
                        Some(vec![1, 2, 3]),
                        /* proxy= */ None,
                        123,
                    ),
                )],
                None,
                (depth == ResolvedPathDepth::Deep)
                    .then(|| resolved_artifact.get_path().as_fragment().clone()),
            );

            let input_map = ActionInputMetadata::with_providers(None);
            input_map.put_tree_artifact(&input_artifact, Arc::new(input_metadata));

            let action_fs = f.create_remote_action_file_system(input_map);

            let store = f.create_store(&output_artifact, Some(action_fs.clone()));
            store.prepare_for_action_execution();

            on(&action_fs, &output_artifact.get_path())
                .get_parent_directory()
                .unwrap()
                .create_directory_and_parents()
                .unwrap();
            on(&action_fs, &output_artifact.get_path())
                .create_symbolic_link_to_fragment(input_artifact.get_path().as_fragment())
                .unwrap();

            let expected = blaze_exec::actions::TreeArtifactValue::build(
                vec![(
                    artifact_input(&Artifact::create_tree_output_str(&output_artifact, "child")),
                    blaze_exec::actions::FileArtifactValue::create_for_normal_file(
                        Some(vec![1, 2, 3]),
                        /* proxy= */ None,
                        123,
                    ),
                )],
                None,
                Some(resolved_artifact.get_path().as_fragment().clone()),
            );
            let tree = match store.get_tree_artifact_value(&artifact_input(&output_artifact)) {
                Ok(tree) => tree,
                Err(IoOrInterruptedException::Io(e)) => panic!("{depth:?}: {e}"),
                Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
            };
            assert_tree_eq(&tree, &expected, &format!("{depth:?}"));
        }
    }

    /// Review round 2, N1 (i): a tree output whose files exist only in the action's remote
    /// output tree (build without the bytes) and whose relative symlink
    /// `lib/libfoo.so -> libfoo.so.1` exists only locally is visited on the action file system:
    /// the link resolves to the remote file and is a child with the remote file's metadata and
    /// its resolved path. Expected children: the oracle (`scripts/remote/probe-trees2`, run1:
    /// the consumer lists both `lib/libfoo.so` and `lib/libfoo.so.1`); expected metadata:
    /// `ActionOutputMetadataStore.java:536-545` (the real stat is a `FileStatusWithMetadata`).
    #[test]
    fn tree_artifact_with_relative_symlink_to_remote_child() {
        let f = ActionFsFixture::new();
        let tree_artifact = f.create_tree_artifact_with_generating_action("libtree");
        let action_fs =
            f.create_remote_action_file_system(ActionInputMetadata::with_providers(None));
        let store = f.create_store(&tree_artifact, Some(action_fs.clone()));
        store.prepare_for_action_execution();

        let lib = tree_artifact.get_path().get_relative_str("lib");
        lib.create_directory_and_parents().unwrap();
        let remote_file = lib.get_relative_str("libfoo.so.1");
        action_fs
            .inject_remote_file(remote_file.as_fragment(), vec![1, 2, 3], 11, None, false)
            .unwrap();
        on(&action_fs, &lib.get_relative_str("libfoo.so"))
            .create_symbolic_link_to_fragment(&blaze_vfs::PathFragment::create("libfoo.so.1"))
            .unwrap();

        let tree = match store.get_tree_artifact_value(&artifact_input(&tree_artifact)) {
            Ok(tree) => tree,
            Err(IoOrInterruptedException::Io(e)) => panic!("{e}"),
            Err(IoOrInterruptedException::Interrupted(_)) => panic!("interrupted"),
        };
        let remote = action_fs
            .stat_metadata(remote_file.as_fragment(), false)
            .unwrap()
            .expect("the injected file");
        let expected = blaze_exec::actions::TreeArtifactValue::build(
            vec![
                (
                    artifact_input(&Artifact::create_tree_output_str(
                        &tree_artifact,
                        "lib/libfoo.so",
                    )),
                    blaze_exec::actions::FileArtifactValue::create_from_existing_with_resolved_path(
                        remote.clone(),
                        remote_file.as_fragment().clone(),
                    ),
                ),
                (
                    artifact_input(&Artifact::create_tree_output_str(
                        &tree_artifact,
                        "lib/libfoo.so.1",
                    )),
                    remote,
                ),
            ],
            None,
            None,
        );
        assert_tree_eq(&tree, &expected, "");

        // Let the temporary directory be deleted.
        chmod(tree_artifact.get_path().get_path_string(), 0o755);
        chmod(lib.get_path_string(), 0o755);
    }
}

/// The other 23 methods of `ActionOutputMetadataStoreTest`, with the table of all 30.
#[cfg(test)]
#[path = "action_output_metadata_store_tests.rs"]
mod action_output_metadata_store_java_tests;

/// Port of `ActionInputMapTest`, for the `ActionInputMap` inside [`ActionInputMetadata`].
///
/// PORT: Java's `TestMetadata.create(id)` is a regular file value whose digest and size are `id`;
/// `isSameInstanceAs` assertions compare values that differ between the candidates.
#[cfg(test)]
mod action_input_map_tests {
    use std::sync::Arc;

    use blaze_actions::Artifact;
    use blaze_exec::actions::action_adapters::artifact_input;
    use blaze_exec::actions::{
        ActionInputRef, BasicActionInput, FileArtifactValue, TreeArtifactValue,
    };
    use blaze_vfs::PathFragment;

    use super::ActionInputMetadata;
    use super::action_output_metadata_store_tests::create_tree_artifact;

    fn new_map() -> ActionInputMetadata {
        ActionInputMetadata::with_providers(None)
    }

    /// `TestMetadata.create(id)`.
    fn test_metadata(id: u8) -> FileArtifactValue {
        FileArtifactValue::create_for_normal_file(Some(vec![id]), None, id as i64)
    }

    /// `new TestInput(path)` / `ActionInputHelper.fromPath(path)`.
    fn from_path(path: &str) -> ActionInputRef {
        ActionInputRef::new(BasicActionInput::from_path(path))
    }

    fn from_path_fragment(path: &PathFragment) -> ActionInputRef {
        ActionInputRef::new(BasicActionInput::from_path_fragment(path.clone()))
    }

    /// `createTreeArtifact(rootRelativePath)`.
    fn tree_artifact(root_relative_path: &str) -> Artifact {
        create_tree_artifact("/execroot", root_relative_path)
    }

    /// `TreeArtifactValue.newBuilder(tree).putChild(child, metadata)...build()`.
    fn tree_value(children: &[(&Artifact, FileArtifactValue)]) -> Arc<TreeArtifactValue> {
        Arc::new(TreeArtifactValue::build(
            children
                .iter()
                .map(|(child, metadata)| (artifact_input(child), metadata.clone()))
                .collect(),
            None,
            None,
        ))
    }

    // assertContains / assertContainsFile
    fn assert_contains_file(map: &ActionInputMetadata, input: &ActionInputRef, id: u8) {
        assert_eq!(
            map.action_input_map_get_input_metadata_checked(input),
            Some(test_metadata(id))
        );
        assert_eq!(
            map.action_input_map_get_input(input.get_exec_path()),
            Some(input.clone())
        );
    }

    // assertContainsTree
    fn assert_contains_tree(
        map: &ActionInputMetadata,
        tree: &Artifact,
        value: &Arc<TreeArtifactValue>,
    ) {
        let tree_input = artifact_input(tree);
        let got = map.action_input_map_get_tree_metadata(&tree_input).unwrap();
        assert!(Arc::ptr_eq(&got, value));
        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&tree_input),
            value.metadata.clone()
        );
        assert_eq!(
            map.action_input_map_get_input(tree.get_exec_path()),
            Some(tree_input)
        );
    }

    // assertDoesNotContain
    fn assert_does_not_contain(map: &ActionInputMetadata, input: &ActionInputRef) {
        assert_eq!(map.action_input_map_get_input_metadata_checked(input), None);
        assert_eq!(map.action_input_map_get_input(input.get_exec_path()), None);
    }

    #[test]
    fn basic_put_and_lookup() {
        let map = new_map();
        let input = from_path("/abc/def");
        map.put_input(input.clone(), test_metadata(5));
        assert_contains_file(&map, &input, 5);
        assert_eq!(
            map.get_metadata_from_tree_artifacts(&PathFragment::create("blah")),
            None
        );
        assert_eq!(
            map.action_input_map_get_input(&PathFragment::create("blah")),
            None
        );
    }

    #[test]
    fn put_ignores_subsequent_puts() {
        let map = new_map();
        let abc = from_path("/abc/def");
        let ghi = from_path("/ghi/jkl");
        map.put_input(abc.clone(), test_metadata(5));
        map.put_input(abc.clone(), test_metadata(6));
        map.put_input(ghi.clone(), test_metadata(7));
        map.put_input(ghi.clone(), test_metadata(8));
        assert_eq!(map.map.read().len(), 2);
        assert_contains_file(&map, &abc, 5);
        assert_contains_file(&map, &ghi, 7);
    }

    #[test]
    #[ignore = "ActionInputMap.clear is not ported (Java calls it from tests only)"]
    fn clear_removes_all_elements() {
        unimplemented!("not ported: ActionInputMapTest.clear_removesAllElements")
    }

    #[test]
    fn put_tree_artifact_adds_empty_tree_artifact() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let empty = tree_value(&[]);

        map.put_tree_artifact(&tree, empty.clone());

        assert_eq!(map.map.read().len(), 1);
        assert_contains_tree(&map, &tree, &empty);
    }

    #[test]
    fn put_tree_artifact_adds_tree_artifact_and_all_children() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let child1 = Artifact::create_tree_output_str(&tree, "child1");
        let child2 = Artifact::create_tree_output_str(&tree, "child2");
        let value = tree_value(&[(&child1, test_metadata(1)), (&child2, test_metadata(2))]);

        map.put_tree_artifact(&tree, value.clone());

        assert_eq!(map.map.read().len(), 1);
        assert_contains_tree(&map, &tree, &value);
        assert_contains_file(&map, &artifact_input(&child1), 1);
        assert_contains_file(&map, &artifact_input(&child2), 2);
    }

    #[test]
    fn put_tree_artifact_multiple_trees_adds_all_trees_and_children() {
        let map = new_map();
        let tree1 = tree_artifact("tree1");
        let tree1_file = Artifact::create_tree_output_str(&tree1, "file");
        let tree1_value = tree_value(&[(&tree1_file, test_metadata(1))]);
        let tree2 = tree_artifact("tree2");
        let tree2_file = Artifact::create_tree_output_str(&tree2, "file");
        let tree2_value = tree_value(&[(&tree2_file, test_metadata(2))]);

        map.put_tree_artifact(&tree1, tree1_value.clone());
        map.put_tree_artifact(&tree2, tree2_value.clone());

        assert_eq!(map.map.read().len(), 2);
        assert_contains_tree(&map, &tree1, &tree1_value);
        assert_contains_file(&map, &artifact_input(&tree1_file), 1);
        assert_contains_tree(&map, &tree2, &tree2_value);
        assert_contains_file(&map, &artifact_input(&tree2_file), 2);
    }

    #[test]
    fn put_tree_artifact_after_put_tree_artifact_with_same_exec_path_does_nothing() {
        let map = new_map();
        let tree1 = tree_artifact("tree");
        let tree2 = tree_artifact("tree");
        let tree2_file = Artifact::create_tree_output_str(&tree2, "file");
        let tree1_value = tree_value(&[]);
        let tree2_value = tree_value(&[(&tree2_file, test_metadata(1))]);
        map.put_tree_artifact(&tree1, tree1_value.clone());

        map.put_tree_artifact(&tree2, tree2_value);

        assert_contains_tree(&map, &tree1, &tree1_value);
        assert_does_not_contain(&map, &artifact_input(&tree2_file));
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn put_tree_artifact_same_exec_path_as_a_regular_file_fails() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let file = from_path_fragment(tree.get_exec_path());
        map.put_input(file, test_metadata(1));

        map.put_tree_artifact(&tree, tree_value(&[]));
    }

    // Java: @TestParameter PutOrder (DECLARED, REVERSE).
    fn put_tree_artifact_nested_file_returns_nested_file_from_exec_path(reverse: bool) {
        let map = new_map();
        let tree = tree_artifact("tree");
        let tree_file = Artifact::create_tree_output_str(&tree, "file");
        let file = from_path_fragment(tree_file.get_exec_path());
        let value = tree_value(&[(&tree_file, test_metadata(1))]);

        if reverse {
            map.put_tree_artifact(&tree, value);
            map.put_input(file.clone(), test_metadata(2));
        } else {
            map.put_input(file.clone(), test_metadata(2));
            map.put_tree_artifact(&tree, value);
        }

        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&file),
            Some(test_metadata(2))
        );
        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&artifact_input(&tree_file)),
            Some(test_metadata(1))
        );
        let input = map
            .action_input_map_get_input(tree_file.get_exec_path())
            .unwrap();
        // isSameInstanceAs(file): the plain input, not the tree file artifact.
        assert!(input.as_artifact().is_none());
    }

    #[test]
    fn put_tree_artifact_nested_file_returns_nested_file_from_exec_path_declared() {
        put_tree_artifact_nested_file_returns_nested_file_from_exec_path(false);
    }

    #[test]
    fn put_tree_artifact_nested_file_returns_nested_file_from_exec_path_reverse() {
        put_tree_artifact_nested_file_returns_nested_file_from_exec_path(true);
    }

    #[test]
    fn put_tree_file_artifact_adds_entry() {
        let map = new_map();
        let tree_file = Artifact::create_tree_output_str(&tree_artifact("tree"), "file");

        map.put(&tree_file, test_metadata(1));

        // PORT: Java's assertContainsFile also looks the tree file up as an input
        // (getInputMetadata), which finds it by its own exec path because its parent is not in
        // the map.
        assert_contains_file(&map, &artifact_input(&tree_file), 1);
    }

    /// Not a Java test method: `getInputMetadataChecked` finds a child of a tree artifact that an
    /// action template expansion produced (`TreeFileArtifact.createTemplateExpansionOutput`, the
    /// C++ objects of a tree of sources) through its parent's `TreeArtifactValue`, as for a child
    /// of a declared directory (`input instanceof TreeFileArtifact`,
    /// `ActionInputMap.java:218`). Upstream `build_without_the_bytes_test`
    /// `test_cc_tree_*` crashed in the Merkle tree on this lookup.
    #[test]
    fn get_input_metadata_template_expansion_output_from_parent_tree() {
        use crate::action_template_expansion_value::ActionTemplateExpansionKey;
        let map = new_map();
        let tree = tree_artifact("tree");
        let child = Artifact::create_template_expansion_output(
            &tree,
            PathFragment::create("child.o"),
            blaze_actions::ActionLookupKeyRef::new(ActionTemplateExpansionKey::of(
                tree.get_action_lookup_key().clone(),
                0,
            )),
        );
        assert!(!child.is_child_of_declared_directory());
        map.put_tree_artifact(&tree, tree_value(&[(&child, test_metadata(1))]));

        assert_contains_file(&map, &artifact_input(&child), 1);
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn put_same_exec_path_as_a_tree_fails() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let file = from_path_fragment(tree.get_exec_path());
        map.put_tree_artifact(&tree, tree_value(&[]));

        map.put_input(file, test_metadata(1));
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn put_tree_artifact_fails() {
        let map = new_map();
        let tree = tree_artifact("tree");

        map.put(&tree, test_metadata(1));
    }

    #[test]
    fn get_metadata_action_input_with_tree_exec_path_returns_tree_artifact_entries() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let empty = tree_value(&[]);
        map.put_tree_artifact(&tree, empty.clone());
        let input = from_path_fragment(tree.get_exec_path());

        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&input),
            empty.metadata.clone()
        );
    }

    #[test]
    fn get_metadata_action_input_with_tree_file_exec_path_returns_tree_artifact_entries() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let tree_file = Artifact::create_tree_output_str(&tree, "file");
        map.put_tree_artifact(&tree, tree_value(&[(&tree_file, test_metadata(1))]));
        let input = from_path_fragment(tree_file.get_exec_path());

        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&input),
            Some(test_metadata(1))
        );
    }

    #[test]
    #[ignore = "needs ActionsTestUtil.createArtifactWithExecPath (a derived artifact at a tree file's exec path)"]
    fn get_metadata_artifact_with_tree_file_exec_path_returns_null() {
        unimplemented!(
            "not ported: ActionInputMapTest.getMetadata_artifactWithTreeFileExecPath_returnsNull"
        )
    }

    #[test]
    fn get_metadata_missing_file_within_tree_returns_null() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let file = Artifact::create_tree_output_str(&tree, "file");
        map.put_tree_artifact(&tree, tree_value(&[(&file, test_metadata(1))]));
        let nonexistent_tree_file = Artifact::create_tree_output_str(&tree, "nonexistent");

        assert_does_not_contain(&map, &artifact_input(&nonexistent_tree_file));
    }

    #[test]
    #[should_panic(expected = "IllegalArgumentException")]
    fn get_input_metadata_tree_file_under_file_fails() {
        let map = new_map();
        let tree = tree_artifact("tree");
        let child = Artifact::create_tree_output_str(&tree, "file");
        let file = from_path_fragment(tree.get_exec_path());
        map.put_input(file, test_metadata(1));

        map.action_input_map_get_input_metadata_checked(&artifact_input(&child));
    }

    #[test]
    fn get_tree_metadata_for_prefix_non_tree() {
        let map = new_map();
        let file = from_path("some/file");
        map.put_input(file.clone(), test_metadata(1));

        let path = file.get_exec_path();
        assert!(map.find_tree_artifact_node_at_prefix(path).is_none());
        assert!(
            map.find_tree_artifact_node_at_prefix(&path.get_parent_directory().unwrap())
                .is_none()
        );
        assert!(
            map.find_tree_artifact_node_at_prefix(&path.get_child("under"))
                .is_none()
        );
    }

    #[test]
    fn get_tree_metadata_for_prefix_empty_tree() {
        let map = new_map();
        let tree = tree_artifact("a/tree");
        let value = tree_value(&[]);
        map.put_tree_artifact(&tree, value.clone());

        let path = tree.get_exec_path();
        assert!(
            map.find_tree_artifact_node_at_prefix(&path.get_parent_directory().unwrap())
                .is_none()
        );
        assert!(Arc::ptr_eq(
            &map.find_tree_artifact_node_at_prefix(path).unwrap(),
            &value
        ));
        assert!(Arc::ptr_eq(
            &map.find_tree_artifact_node_at_prefix(&path.get_child("under"))
                .unwrap(),
            &value
        ));
    }

    #[test]
    fn get_tree_metadata_for_prefix_non_empty_tree() {
        let map = new_map();
        let tree = tree_artifact("a/tree");
        let child = Artifact::create_tree_output_str(&tree, "some/child");
        let value = tree_value(&[(&child, test_metadata(1))]);
        map.put_tree_artifact(&tree, value.clone());

        let tree_path = tree.get_exec_path();
        let child_path = child.get_exec_path();
        assert!(
            map.find_tree_artifact_node_at_prefix(&tree_path.get_parent_directory().unwrap())
                .is_none()
        );
        for path in [
            tree_path.clone(),
            child_path.clone(),
            child_path.get_parent_directory().unwrap(),
            child_path.get_child("under"),
        ] {
            assert!(Arc::ptr_eq(
                &map.find_tree_artifact_node_at_prefix(&path).unwrap(),
                &value
            ));
        }
    }

    #[test]
    fn getters_missing_tree_return_null() {
        let map = new_map();
        map.put_tree_artifact(&tree_artifact("tree"), tree_value(&[]));
        let other_tree = tree_artifact("other");

        let other_tree_input = artifact_input(&other_tree);
        assert_eq!(
            map.action_input_map_get_input_metadata_checked(&other_tree_input),
            None
        );
        assert!(
            map.action_input_map_get_tree_metadata(&other_tree_input)
                .is_none()
        );
        assert_does_not_contain(
            &map,
            &artifact_input(&Artifact::create_tree_output_str(&other_tree, "child")),
        );
    }

    #[test]
    #[ignore = "randomized stress test of the open-addressed table, which the port replaces by a HashMap"]
    fn stress() {
        unimplemented!("not ported: ActionInputMapTest.stress")
    }
}
