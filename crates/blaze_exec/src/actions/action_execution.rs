//! The execution half of `lib/actions/Action.java` (`Action.execute`, `Action.prepare`),
//! `ActionResult`, `ActionExecutionException` and the `ActionFailed` root cause.
//!
//! # Seam with the analysis crates
//!
//! `blaze_actions::Action` carries only analysis-time behaviour (keys, inputs, outputs). Java's
//! `Action.execute(ActionExecutionContext)` lives in each action class; here it is the
//! [`ExecutableAction`] trait, implemented next to the code that executes each action class (this
//! crate for the `blaze_actions` classes, the analysis crates for theirs, such as
//! `CppCompileAction`). Each implementation registers itself once with
//! [`register_executable_action`]; [`as_executable`] then finds the implementation for a
//! `&dyn Action` by its concrete type (Java: virtual dispatch on `Action.execute`).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::action::Action;
use blaze_cmdline::Label;
use blaze_protos::failure_details::FailureDetail;
use blaze_vfs::PathFragment;
use parking_lot::RwLock;

use super::exec_exception::ExecException;
use super::spawn_result::SpawnResult;
use crate::exec::action_execution_context::ActionExecutionContext;

/// Holds the result(s) of an action's execution.
// Java: com/google/devtools/build/lib/actions/ActionResult.java:31 ActionResult
#[derive(Clone, Debug, Default)]
pub struct ActionResult {
    spawn_results: Vec<SpawnResult>,
}

impl ActionResult {
    /// `ActionResult.EMPTY`.
    // Java: com/google/devtools/build/lib/actions/ActionResult.java:37 ActionResult.EMPTY
    pub fn empty() -> ActionResult {
        ActionResult {
            spawn_results: Vec::new(),
        }
    }

    /// `ActionResult.create(spawnResults)`.
    // Java: com/google/devtools/build/lib/actions/ActionResult.java:45 ActionResult.create
    pub fn create(spawn_results: Vec<SpawnResult>) -> ActionResult {
        ActionResult { spawn_results }
    }

    /// `spawnResults()`.
    pub fn spawn_results(&self) -> &[SpawnResult] {
        &self.spawn_results
    }

    /// `this == ActionResult.EMPTY` (an action that ran no spawns).
    pub fn is_empty(&self) -> bool {
        self.spawn_results.is_empty()
    }

    /// Returns the cumulative command execution wall time for the action.
    // Java: com/google/devtools/build/lib/actions/ActionResult.java:87 ActionResult.cumulativeCommandExecutionWallTimeInMs
    pub fn cumulative_command_execution_wall_time_in_ms(&self) -> i32 {
        self.spawn_results
            .iter()
            .map(|r| r.get_wall_time_in_ms())
            .fold(0i32, |a, b| a.wrapping_add(b))
    }
}

/// `ActionFailed` (lib/causes): the root cause recorded for an action failure.
// Java: com/google/devtools/build/lib/causes/ActionFailed.java:30 ActionFailed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionFailed {
    pub exec_path: PathFragment,
    pub label: Label,
    pub configuration_checksum: Option<String>,
}

/// `LabelCause` (lib/causes): a failure attributed to a label, such as a missing source input.
// Java: com/google/devtools/build/lib/causes/LabelCause.java:23 LabelCause
#[derive(Clone, Debug, PartialEq)]
pub struct LabelCause {
    pub label: Label,
    /// The `DetailedExitCode`'s failure detail.
    pub failure_detail: FailureDetail,
}

impl LabelCause {
    /// `getMessage()`: the message of the detailed exit code's failure detail.
    // Java: com/google/devtools/build/lib/causes/LabelCause.java:42 LabelCause.getMessage
    pub fn get_message(&self) -> &str {
        &self.failure_detail.message
    }
}

/// A root cause of an `ActionExecutionException` (Java's `Cause` interface; the two
/// implementations execution produces).
// Java: com/google/devtools/build/lib/causes/Cause.java:22 Cause
#[derive(Clone, Debug, PartialEq)]
pub enum RootCause {
    ActionFailed(ActionFailed),
    Label(LabelCause),
}

/// A snapshot of the parts of the failed action that consumers of an
/// `ActionExecutionException` read (`getAction()`): owner label, configuration, primary output,
/// mnemonic, and the owner's location.
#[derive(Clone, Debug)]
pub struct FailedActionInfo {
    pub owner_label: Option<Label>,
    pub configuration_checksum: Option<String>,
    pub primary_output: PathFragment,
    pub mnemonic: String,
    pub location: String,
    pub pretty_print: String,
    /// `action.describe()`.
    pub describe: String,
}

impl FailedActionInfo {
    pub fn of(action: &dyn Action) -> FailedActionInfo {
        let owner = action.get_owner();
        FailedActionInfo {
            owner_label: owner.get_label().cloned(),
            configuration_checksum: Some(owner.get_configuration_checksum().to_string()),
            primary_output: action.get_primary_output().get_exec_path().clone(),
            mnemonic: action.get_mnemonic(),
            location: owner.get_location().to_string(),
            pretty_print: action.pretty_print(),
            describe: action.describe(),
        }
    }
}

/// This exception gets thrown if there were errors during the execution of an action.
// Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:35 ActionExecutionException
// PORT: `DetailedExitCode` is `blaze_runtime`'s; this keeps its `FailureDetail` (the exit code
// follows from it through `DetailedExitCode.of`). `getAction()` is the [`FailedActionInfo`]
// snapshot; `getCause()` is kept as the cause's message.
#[derive(Clone, Debug)]
pub struct ActionExecutionException {
    message: String,
    cause: Option<String>,
    action: Option<FailedActionInfo>,
    root_causes: Vec<RootCause>,
    catastrophe: bool,
    failure_detail: Option<FailureDetail>,
    /// This is an `AlreadyReportedActionExecutionException`: the error was already shown.
    already_reported: bool,
    /// `getCause() instanceof TestExecException`.
    cause_is_test_exec_exception: bool,
    /// Made from [`ExecException::Interrupted`]: Java's `InterruptedException`, which Java's
    /// `Action.execute` / `discoverInputs` throw apart from `ActionExecutionException` (see
    /// [`ActionExecutionException::is_interrupted`]).
    interrupted: bool,
    /// This is a `LostInputsActionExecutionException`: its `getLostInputs()`.
    lost_inputs: Option<std::sync::Arc<super::LostArtifacts>>,
    /// This is a `SpawnActionExecutionException`: its `getSpawnResult()` (the failed spawn's).
    spawn_result: Option<std::sync::Arc<super::spawn_result::SpawnResult>>,
}

impl ActionExecutionException {
    /// `new AlreadyReportedActionExecutionException(cause)`: this wrapper exception is used
    /// when the error message has already been reported; it gets translated into a
    /// BuildFailedException without message.
    // Java: com/google/devtools/build/lib/actions/AlreadyReportedActionExecutionException.java:31 AlreadyReportedActionExecutionException.<init>
    pub fn already_reported(cause: ActionExecutionException) -> ActionExecutionException {
        // A new exception (not a SpawnActionExecutionException).
        ActionExecutionException {
            already_reported: true,
            spawn_result: None,
            ..cause
        }
    }

    /// `new ActionExecutionException(message, action, catastrophe, detailedExitCode)`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:67 ActionExecutionException.<init>
    pub fn new(
        message: impl Into<String>,
        action: Option<&dyn Action>,
        catastrophe: bool,
        failure_detail: Option<FailureDetail>,
    ) -> ActionExecutionException {
        let action = action.map(FailedActionInfo::of);
        let root_causes = Self::root_causes_from_action(action.as_ref());
        ActionExecutionException {
            message: message.into(),
            cause: None,
            action,
            root_causes,
            catastrophe,
            failure_detail,
            already_reported: false,
            cause_is_test_exec_exception: false,
            interrupted: false,
            lost_inputs: None,
            spawn_result: None,
        }
    }

    /// `new ActionExecutionException(message, cause, action, catastrophe, detailedExitCode)`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:54 ActionExecutionException.<init>
    pub fn with_cause(
        message: impl Into<String>,
        cause: &dyn fmt::Display,
        action: Option<&dyn Action>,
        catastrophe: bool,
        failure_detail: Option<FailureDetail>,
    ) -> ActionExecutionException {
        let mut e = Self::new(message, action, catastrophe, failure_detail);
        e.cause = Some(cause.to_string());
        e
    }

    /// `new ActionExecutionException(message, action, rootCauses, catastrophe, detailedExitCode)`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:79 ActionExecutionException.<init>
    pub fn with_root_causes(
        message: impl Into<String>,
        action: Option<FailedActionInfo>,
        root_causes: Vec<RootCause>,
        catastrophe: bool,
        failure_detail: Option<FailureDetail>,
    ) -> ActionExecutionException {
        ActionExecutionException {
            message: message.into(),
            cause: None,
            action,
            root_causes,
            catastrophe,
            failure_detail,
            already_reported: false,
            cause_is_test_exec_exception: false,
            interrupted: false,
            lost_inputs: None,
            spawn_result: None,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:106 ActionExecutionException.rootCausesFromAction
    fn root_causes_from_action(action: Option<&FailedActionInfo>) -> Vec<RootCause> {
        match action {
            Some(a) => match &a.owner_label {
                Some(label) => vec![RootCause::ActionFailed(ActionFailed {
                    exec_path: a.primary_output.clone(),
                    label: label.clone(),
                    configuration_checksum: a.configuration_checksum.clone(),
                })],
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    /// Returns a new ActionExecutionException given an optional action subtask describing which
    /// part of the action failed (should be null for standard action failures).
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:134 ActionExecutionException.fromExecException
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:119 ActionExecutionException.fromExecException
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:153 ActionExecutionException.fromExecException
    pub fn from_exec_exception(
        exception: &ExecException,
        action_subtask: Option<&str>,
        action: &dyn Action,
    ) -> ActionExecutionException {
        // Message from ActionExecutionException will be prepended with action.describe() where
        // necessary: because not all ActionExecutionExceptions come from this codepath, it is
        // safer for consumers to manually prepend. We still put action.describe() in the failure
        // detail message argument.
        let message = format!(
            "{}{}",
            action_subtask.map(|s| format!("{s}: ")).unwrap_or_default(),
            exception.get_message_for_action_execution_exception()
        );
        let code =
            exception.get_failure_detail(&format!("{} failed: {}", action.describe(), message));
        // LostInputsExecException.fromExecException: a LostInputsActionExecutionException.
        let mut e = Self::with_cause(
            message,
            &exception.get_message(),
            Some(action),
            exception.is_catastrophic(),
            code,
        );
        e.cause_is_test_exec_exception = exception.is_test_exec_exception();
        e.interrupted = matches!(exception, ExecException::Interrupted);
        // Java: com/google/devtools/build/lib/actions/LostInputsExecException.java:54 LostInputsExecException.fromExecException
        e.lost_inputs = exception.get_lost_inputs().cloned();
        e
    }

    /// `new SpawnActionExecutionException(..., spawnResult)`: this exception with the failed
    /// spawn's result.
    // Java: com/google/devtools/build/lib/actions/SpawnActionExecutionException.java:24 SpawnActionExecutionException.<init>
    pub fn with_spawn_result(
        mut self,
        spawn_result: super::spawn_result::SpawnResult,
    ) -> ActionExecutionException {
        self.spawn_result = Some(std::sync::Arc::new(spawn_result));
        self
    }

    /// `this instanceof SpawnActionExecutionException`: its `getSpawnResult()`.
    // Java: com/google/devtools/build/lib/actions/SpawnActionExecutionException.java:35 SpawnActionExecutionException.getSpawnResult
    pub fn get_spawn_result(&self) -> Option<&super::spawn_result::SpawnResult> {
        self.spawn_result.as_deref()
    }

    /// `this instanceof LostInputsActionExecutionException ? getLostInputs() : null`.
    // Java: com/google/devtools/build/lib/actions/LostInputsActionExecutionException.java LostInputsActionExecutionException.getLostInputs
    pub fn get_lost_inputs(&self) -> Option<&std::sync::Arc<super::LostArtifacts>> {
        self.lost_inputs.as_ref()
    }

    /// `LostInputsActionExecutionException.toExecException()`: converts to the "lost inputs"
    /// subtype of the other exception type ([`ExecException`]) used during action execution.
    /// Panics (`ClassCastException`) when this is not a `LostInputsActionExecutionException`.
    ///
    /// PORT: the `checkState`s on the decorations (primary action, output path, out/err, input
    /// discovery) have no counterpart: this port does not decorate the exception (see
    /// `SkyframeActionExecutor.enrichLostInputsException` in `blaze_sky_exec`).
    // Java: com/google/devtools/build/lib/actions/LostInputsActionExecutionException.java:133 LostInputsActionExecutionException.toExecException
    pub fn to_exec_exception(&self) -> ExecException {
        let lost_inputs = self.lost_inputs.as_ref().unwrap_or_else(|| {
            panic!(
                "ClassCastException: ActionExecutionException cannot be cast to LostInputsActionExecutionException"
            )
        });
        super::LostInputsExecException::create((**lost_inputs).clone())
    }

    /// The action was interrupted: this stands for the `InterruptedException` Java's
    /// `Action.execute` / `discoverInputs` throw, not for a failure. The caller
    /// (`SkyframeActionExecutor`) rethrows it as `InterruptedException`; nothing is reported.
    ///
    /// PORT: `ExecutableAction::execute` returns one `Result`, so an action that converts every
    /// `ExecException` with [`ActionExecutionException::from_exec_exception`] (SpawnAction)
    /// passes Java's `InterruptedException` through here.
    pub fn is_interrupted(&self) -> bool {
        self.interrupted
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// `getCause().getMessage()`, if there is a cause.
    pub fn get_cause_message(&self) -> Option<&str> {
        self.cause.as_deref()
    }

    /// `getCause() instanceof TestExecException`: the action is a test that failed without
    /// `--test_keep_going` (`SkyframeErrorProcessor.rethrow` rethrows the cause).
    pub fn is_caused_by_test_exec_exception(&self) -> bool {
        self.cause_is_test_exec_exception
    }

    /// This exception with a `TestExecException` as its cause, as
    /// [`ActionExecutionException::from_exec_exception`] makes it for a test that failed without
    /// `--test_keep_going`; for tests of the code that handles such exceptions.
    pub fn with_test_exec_exception_cause_for_testing(self) -> ActionExecutionException {
        ActionExecutionException {
            cause_is_test_exec_exception: true,
            ..self
        }
    }

    /// `getAction()`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:160 ActionExecutionException.getAction
    pub fn get_action(&self) -> Option<&FailedActionInfo> {
        self.action.as_ref()
    }

    /// `getRootCauses()`.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:168 ActionExecutionException.getRootCauses
    pub fn get_root_causes(&self) -> &[RootCause] {
        &self.root_causes
    }

    /// Catastrophic exceptions should stop builds, even if --keep_going.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:182 ActionExecutionException.isCatastrophe
    pub fn is_catastrophe(&self) -> bool {
        self.catastrophe
    }

    /// `getDetailedExitCode().getFailureDetail()`.
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }

    /// Returns true if the error should be shown.
    // Java: com/google/devtools/build/lib/actions/ActionExecutionException.java:202 ActionExecutionException.showError
    // Java: com/google/devtools/build/lib/actions/AlreadyReportedActionExecutionException.java:42 AlreadyReportedActionExecutionException.showError
    pub fn show_error(&self) -> bool {
        // PORT: Java's getMessage() is null only for exceptions built from a null message.
        !self.already_reported
    }
}

impl fmt::Display for ActionExecutionException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ActionExecutionException {}

/// The execution side of `Action`: `Action.execute(ActionExecutionContext)`.
///
/// Implemented for every action class that can be executed; see the module docs for the
/// registration protocol.
// Java: com/google/devtools/build/lib/actions/Action.java:170 Action.execute
#[allow(clippy::result_large_err)]
pub trait ExecutableAction: Action {
    /// Executes this action. This method <i>unconditionally does the work of the Action</i>,
    /// although it may delegate some of that work to `ActionContext` instances.
    ///
    /// The caller (`SkyframeActionExecutor`) has deleted the outputs and created their parent
    /// directories (`Action.prepare`).
    // Java: com/google/devtools/build/lib/actions/Action.java:170 Action.execute
    fn execute(
        &self,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException>;

    /// Method used to find inputs before execution for an action that
    /// [`Action::discovers_inputs`]. Returns the set of discovered inputs (`None`: none) and may
    /// call [`Action::update_inputs`]; called by `ActionExecutionFunction` before `execute` when
    /// `!inputs_known()`.
    // Java: com/google/devtools/build/lib/actions/Action.java:172 Action.discoverInputs
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:145 AbstractAction.discoverInputs
    fn discover_inputs(
        &self,
        _action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<Option<blaze_collect::NestedSet<blaze_actions::Artifact>>, ActionExecutionException>
    {
        panic!(
            "IllegalStateException: Not an input-discovering action: {}",
            self.to_string_java()
        )
    }

    /// Prepares for executing this action; called by the Builder prior to executing the Action
    /// itself. This method should prepare the file system, so that the execution of the Action can
    /// write the output files. At a minimum that means removing any files or directories with the
    /// same name as any of the outputs. Override (`RunfilesTreeAction`, `TestRunnerAction`,
    /// `BazelWorkspaceStatusAction`) where Java overrides `prepare`.
    // Java: com/google/devtools/build/lib/actions/AbstractAction.java:547 AbstractAction.prepare
    fn prepare(&self, exec_root: &blaze_vfs::Path) -> std::io::Result<()> {
        delete_outputs(self, exec_root)
    }

    /// `NotifyOnActionCacheHit.actionCacheHit(context)` for the actions that implement it (the
    /// `ActionCachedContext` is its event handler): `Some(recorded)` where Java's action is a
    /// `NotifyOnActionCacheHit`, `None` (the default) otherwise. `TestRunnerAction` is handled
    /// by the executor itself.
    // Java: com/google/devtools/build/lib/actions/NotifyOnActionCacheHit.java:53 NotifyOnActionCacheHit.actionCacheHit
    fn action_cache_hit(&self, _event_handler: &dyn blaze_events::EventHandler) -> Option<bool> {
        None
    }

    /// `action instanceof NotifyOnActionCacheHit` for the actions that answer
    /// [`Self::action_cache_hit`] (`TestRunnerAction` is recognized by the executor itself).
    fn is_notify_on_action_cache_hit(&self) -> bool {
        false
    }
}

/// `AbstractAction.deleteOutputs` without a bulk deleter or archived tree artifacts.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:438 AbstractAction.deleteOutputs
pub fn delete_outputs(
    action: &(impl Action + ?Sized),
    _exec_root: &blaze_vfs::Path,
) -> std::io::Result<()> {
    for output in action.get_outputs() {
        // Java: com/google/devtools/build/lib/actions/AbstractAction.java:499 AbstractAction.deleteOutput
        delete_output(&output.get_path(), Some(output.get_root().get_root()))?;
    }
    Ok(())
}

/// Helper method to remove an output file. If the path refers to a directory, recursively removes
/// the contents of the directory.
// Java: com/google/devtools/build/lib/actions/AbstractAction.java:514 AbstractAction.deleteOutput
pub fn delete_output(
    path: &blaze_vfs::Path,
    root: Option<&blaze_vfs::Root>,
) -> std::io::Result<()> {
    // Optimize for the common case: output artifacts are files.
    match path.delete() {
        Ok(_) => Ok(()),
        Err(e) => {
            // Handle a couple of scenarios where the output can still be deleted, but make sure
            // we're not deleting random files on the filesystem.
            let Some(root) = root else {
                return Err(std::io::Error::new(e.kind(), format!("null root: {e}")));
            };
            if !root.contains(path) {
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("{} not under {}", path.get_path_string(), root),
                ));
            }
            if let Some(parent_dir) = path.get_parent_directory()
                && root.contains(&parent_dir)
            {
                // Intentionally ignored because we will fail below anyway.
                let _ = parent_dir.set_writable(true);
            }
            // Retry deleting after making the parent writable.
            if path.is_directory_with(blaze_vfs::Symlinks::Nofollow) {
                path.delete_tree()
            } else {
                path.delete().map(|_| ())
            }
        }
    }
}

type Caster = fn(&dyn Any) -> Option<&dyn ExecutableAction>;

fn registry() -> &'static RwLock<HashMap<TypeId, Caster>> {
    static REGISTRY: std::sync::OnceLock<RwLock<HashMap<TypeId, Caster>>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn cast<T: ExecutableAction>(a: &dyn Any) -> Option<&dyn ExecutableAction> {
    a.downcast_ref::<T>().map(|t| t as &dyn ExecutableAction)
}

/// Registers `T`'s [`ExecutableAction`] implementation (idempotent). Call once at startup for
/// each executable action class.
pub fn register_executable_action<T: ExecutableAction>() {
    registry().write().insert(TypeId::of::<T>(), cast::<T>);
}

/// The [`ExecutableAction`] view of `action`, or `None` if its class registered no
/// implementation (Java would always have one; the executor reports `unported!`).
pub fn as_executable(action: &dyn Action) -> Option<&dyn ExecutableAction> {
    let any = action.as_any();
    let caster = *registry().read().get(&any.type_id())?;
    caster(any)
}

/// Shared handle for registering contexts in `ActionContextRegistry::get_context_any`.
pub type ActionContextRef = Arc<dyn Any + Send + Sync>;

/// `ActionExecutionContext.maybeReportSubcommand`: reports the command line of `spawn` as a
/// `SUBCOMMAND` event when `--subcommands` is set.
///
/// PORT: Java reads `executor.reportsSubcommands()`; the executor passes its
/// `ExecutionOptions.showSubcommands`. The resource owner's `ActionOwner` is never null here.
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:459 ActionExecutionContext.maybeReportSubcommand
pub fn maybe_report_subcommand(
    show_subcommands: blaze_optdefs::command::action_execution_context::ShowSubcommands,
    spawn: &dyn super::Spawn,
    spawn_runner: Option<&str>,
    exec_root: &blaze_vfs::Path,
    event_handler: &dyn blaze_events::EventHandler,
) {
    if !show_subcommands.should_show_subcommands() {
        return;
    }

    let mut reason = String::new();
    let resource_owner = spawn.get_resource_owner();
    let owner = resource_owner.get_owner();
    reason.push_str(&java_string(owner.description.as_deref()));
    reason.push_str(" [");
    reason.push_str(&resource_owner.pretty_print());
    reason.push_str(", configuration: ");
    reason.push_str(&java_string(owner.configuration_checksum.as_deref()));
    if let Some(execution_platform) = resource_owner.get_execution_platform() {
        reason.push_str(", execution platform: ");
        reason.push_str(&execution_platform.label.to_string());
    }
    reason.push_str(", mnemonic: ");
    reason.push_str(&spawn.get_mnemonic());
    reason.push(']');

    // We print this command out in such a way that it can safely be
    // copied+pasted as a Bourne shell command.  This is extremely valuable for
    // debugging.
    let configuration_checksum = spawn.get_configuration_checksum();
    let execution_platform_label = spawn.get_execution_platform_label();
    let message = blaze_shell::CommandFailureUtils::describe_command(
        blaze_shell::CommandDescriptionForm::Complete,
        show_subcommands.pretty_print_args(),
        spawn.get_arguments(),
        Some(spawn.get_environment()),
        /* environment_variables_to_clear= */ None,
        Some(exec_root.get_path_string()),
        configuration_checksum.as_deref(),
        execution_platform_label.as_ref(),
        spawn_runner,
    );
    event_handler.handle(&blaze_events::Event::of(
        blaze_events::EventKind::Subcommand,
        format!("# {reason}\n{message}"),
    ));
}

/// Java string concatenation of a nullable `String`.
fn java_string(s: Option<&str>) -> String {
    s.unwrap_or("null").to_string()
}
