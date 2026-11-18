//! `SkyframeActionExecutor`: action execution shared across `ActionExecutionFunction` calls
//! (contexts, output preparation, `Action.execute`, output checks and permissions, error
//! reporting, shared actions).

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use blaze_actions::actions::action::{Action, ActionRef};
use blaze_actions::actions::artifact::OwnerlessArtifactWrapper;
use blaze_actions::{ActionKeyContext, ActionLookupData, Artifact};
use blaze_events::{Clock, Event, ExtendedEventHandler, JavaClock};
use blaze_exec::actions::action_adapters::{artifact_input, as_blaze_artifact};
use blaze_exec::actions::action_execution::ActionContextRef;
use blaze_exec::actions::{
    ActionExecutionException, ActionExecutionMetadata, ActionInputRef, ActionResult,
    FileArtifactValue, FilesetOutputTree, InputMetadataProvider, RunfilesArtifactValue, Spawn,
    TreeArtifactValue, as_executable,
};
use blaze_exec::exec::action_execution_context::{
    ActionContextRegistry, ActionExecutionContext, SpawnCache,
};
use blaze_exec::exec::spawn_log_context::SpawnLogContext;
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistry;
use blaze_exec::util::FileOutErr;
use blaze_protos::failure_details::{FailureDetail, execution};
use blaze_vfs::{Path, PathFragment, Symlinks};
use indexmap::IndexMap;
use parking_lot::Mutex;

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

type SharedResult = Result<ActionExecutionValue, ActionExecutionException>;

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
    /// Java: `buildActionMap`. Shared actions (same primary output) execute once.
    build_action_map: Mutex<HashMap<OwnerlessArtifactWrapper, Arc<OnceLock<SharedResult>>>>,
    had_execution_error: AtomicBool,
    /// Java: `ActionLogBufferPathGenerator.actionUniquifier`.
    action_uniquifier: AtomicU64,
}

impl SkyframeActionExecutor {
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
            build_action_map: Mutex::new(HashMap::new()),
            had_execution_error: AtomicBool::new(false),
            action_uniquifier: AtomicU64::new(0),
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

    /// Executes `action` (shared actions run once), returning its value.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:622 SkyframeActionExecutor.executeAction
    pub fn execute_action(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        input_metadata: Arc<ActionInputMetadata>,
        action_start_time: Instant,
        action_lookup_data: &ActionLookupData,
    ) -> Result<ActionExecutionValue, ActionExecutionException> {
        // PORT: actionCacheChecker.isActionExecutionProhibited (--check_up_to_date) is not wired.
        let cell = {
            let mut map = self.build_action_map.lock();
            map.entry(OwnerlessArtifactWrapper(action.get_primary_output()))
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        // Use the first execution of a shared action; later ones reuse its result.
        // PORT: Java's ActionExecutionState futures; a OnceLock blocks the second caller until the
        // first finishes (no Skyframe restarts here).
        cell.get_or_init(|| {
            self.run_action(
                listener,
                action,
                input_metadata,
                action_start_time,
                action_lookup_data,
            )
        })
        .clone()
    }

    /// `ActionRunner.run`.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1087 ActionRunner.run
    fn run_action(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        input_metadata: Arc<ActionInputMetadata>,
        action_start_time: Instant,
        action_lookup_data: &ActionLookupData,
    ) -> SharedResult {
        let context = ActionExecutionContextImpl {
            executor: self,
            input_metadata,
            file_out_err: self.generate_file_out_err(),
            rich_artifact_data: Mutex::new(None),
            clock: JavaClock,
        };
        // PORT: reporter.startTask/finishTask (progress UI) are not ported.
        listener.post(Arc::new(ActionStartedEvent {
            action: action.clone(),
            nano_time_start: action_start_time,
        }));
        let result = (|| {
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
                return Err(self.to_action_execution_exception(
                    "failed to delete output files before executing action",
                    Some(&e),
                    action.as_ref(),
                    None,
                    execution::Code::ActionOutputsDeletionFailure,
                ));
            }
            self.create_output_directories(action.as_ref())?;
            self.execute_action_inner(listener, action, &context)
        })();
        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1199 ActionRunner.notifyActionCompletion
        listener.post(Arc::new(ActionCompletionEvent {
            related_time_start: action_start_time,
            finish_time: Instant::now(),
            action: action.clone(),
            action_lookup_data: action_lookup_data.clone(),
        }));
        result
    }

    /// Executes the given action.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1253 ActionRunner.executeAction
    fn execute_action_inner(
        &self,
        listener: &dyn ExtendedEventHandler,
        action: &ActionRef,
        context: &ActionExecutionContextImpl<'_>,
    ) -> SharedResult {
        let Some(executable) = as_executable(action.as_ref()) else {
            blaze_util::unported!(&format!(
                "{}.execute (no ExecutableAction registered for mnemonic {})",
                std::any::type_name_of_val(action.as_any()),
                action.get_mnemonic()
            ));
        };
        let result = match executable.execute(context) {
            Ok(r) => r,
            Err(e) => {
                let primary_output_path = context.get_input_path(&action.get_primary_output());
                return Err(self.process_and_get_exception_to_throw(
                    listener,
                    &primary_output_path,
                    action,
                    e,
                    context.get_file_out_err(),
                    ErrorTiming::AFTER_EXECUTION,
                ));
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
    ) -> SharedResult {
        let mut output_already_dumped = false;
        if !action_result.is_empty() {
            listener.post(Arc::new(ActionResultReceivedEvent {
                action: action.clone(),
                action_result: action_result.clone(),
            }));
        }
        // Action terminated fine, now report the output.
        let out_err_buffer = context.get_file_out_err();
        if out_err_buffer.has_recorded_output() {
            // PORT: reporter.showOutput(label) (--output_filter) is always true without a filter.
            self.dump_recorded_out_err_for_action(action.as_ref(), out_err_buffer);
            output_already_dumped = true;
        }
        let primary_output = action.get_primary_output();
        let primary_output_path = context.get_input_path(&primary_output);
        blaze_util::check_state!(
            action.inputs_known(),
            "Action {} successfully executed, but inputs still not known",
            action.to_string_java()
        );
        let outputs = match self.check_outputs(action.as_ref()) {
            Some(o) => o,
            None => {
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
        };
        self.report_action_execution(
            listener,
            &primary_output_path,
            action,
            None,
            out_err_buffer,
            ErrorTiming::NO_ERROR,
        );
        let (artifact_data, tree_artifact_data) = outputs;
        Ok(ActionExecutionValue::create(
            artifact_data,
            tree_artifact_data,
            context.rich_artifact_data.lock().clone(),
        ))
    }

    /// Stats every output (`ActionOutputMetadataStore.getOutputMetadata`), setting output
    /// permissions; `None` (after reporting) if an output is missing.
    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1627 SkyframeActionExecutor.checkOutputs
    #[allow(clippy::type_complexity)]
    fn check_outputs(
        &self,
        action: &dyn Action,
    ) -> Option<(
        IndexMap<Artifact, FileArtifactValue>,
        IndexMap<Artifact, Arc<TreeArtifactValue>>,
    )> {
        let mut success = true;
        let mut files = IndexMap::new();
        let mut trees = IndexMap::new();
        for output in action.get_outputs() {
            if output.is_runfiles_tree() {
                // A runfiles tree has no file of its own: its value is the action's rich data.
                files.insert(output.clone(), FileArtifactValue::RunfilesTreeMarker);
                continue;
            }
            let path = output.get_path();
            match self.get_output_metadata(&output, &path) {
                Ok(OutputMetadata::File(v)) => {
                    files.insert(output.clone(), v);
                }
                Ok(OutputMetadata::Tree(t)) => {
                    trees.insert(output.clone(), t);
                }
                Err(e) => {
                    success = false;
                    if output.is_tree_artifact() {
                        // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1586 SkyframeActionExecutor.reportOutputTreeArtifactErrors
                        let error_message = if e.kind() == std::io::ErrorKind::NotFound {
                            format!("TreeArtifact {} was not created", output.pretty_print())
                        } else {
                            format!(
                                "Error while validating output TreeArtifact {} : {}",
                                output, e
                            )
                        };
                        self.reporter.handle(&Event::error_at(
                            Some(action.get_owner().get_location().clone()),
                            error_message,
                        ));
                    } else {
                        // Are all other exceptions caught due to missing files?
                        report_missing_output_file(
                            action,
                            &output,
                            &*self.reporter,
                            path.is_symbolic_link(),
                            &e,
                        );
                    }
                }
            }
        }
        success.then_some((files, trees))
    }

    /// `ActionOutputMetadataStore.getOutputMetadata` for a freshly executed action: stats the
    /// output, sets its permissions (`setPathPermissionsIfFile`) and computes its digest.
    // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:282 ActionOutputMetadataStore.getOutputMetadata
    fn get_output_metadata(
        &self,
        output: &Artifact,
        path: &Path,
    ) -> std::io::Result<OutputMetadata> {
        let mode = self.output_permissions.get_permissions_mode();
        if output.is_tree_artifact() {
            // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:373 ActionOutputMetadataStore.constructTreeArtifactValueFromFilesystem
            let stat = path.stat_with(Symlinks::Nofollow)?;
            if !stat.is_directory() {
                return Err(std::io::Error::other(format!(
                    "{} is not a directory",
                    path.get_path_string()
                )));
            }
            set_tree_read_only_and_executable(path, mode)?;
            let mut children = Vec::new();
            visit_tree(path, &PathFragment::empty_fragment(), &mut |rel, child| {
                let value = FileArtifactValue::create_for_testing(child)?;
                let child_artifact = Artifact::create_tree_output(output, rel.clone());
                children.push((artifact_input(&child_artifact), value));
                Ok(())
            })?;
            return Ok(OutputMetadata::Tree(Arc::new(TreeArtifactValue {
                child_values: children,
                archived_representation: None,
                metadata: None,
                resolved_path: None,
            })));
        }
        let stat_no_follow = path.stat_with(Symlinks::Nofollow)?;
        if output.is_symlink() {
            // An unresolved symlink output: its value is its target.
            let _ = path.read_symbolic_link()?;
            blaze_util::unported!("FileArtifactValue.createForUnresolvedSymlink");
        }
        // Java: com/google/devtools/build/lib/skyframe/ActionOutputMetadataStore.java:509 ActionOutputMetadataStore.setPathPermissionsIfFile
        if stat_no_follow.is_file() && (stat_no_follow.get_permissions() & 0o7777) != mode {
            path.chmod(mode)?;
        }
        Ok(OutputMetadata::File(FileArtifactValue::create_for_testing(
            path,
        )?))
    }

    // Java: com/google/devtools/build/lib/skyframe/SkyframeActionExecutor.java:1469 SkyframeActionExecutor.createOutputDirectories
    fn create_output_directories(
        &self,
        action: &dyn Action,
    ) -> Result<(), ActionExecutionException> {
        // Java: com/google/devtools/build/lib/skyframe/OutputDirectoryHelper.java:117 OutputDirectoryHelper.createOutputDirectories
        for output in action.get_outputs() {
            let dir = if output.is_tree_artifact() {
                Some(output.get_path())
            } else {
                output.get_path().get_parent_directory()
            };
            let Some(dir) = dir else { continue };
            if let Err(e) = dir.create_directory_and_parents() {
                return Err(self.to_action_execution_exception(
                    &format!(
                        "failed to create output directory '{}': {}",
                        dir.get_path_string(),
                        e
                    ),
                    Some(&e),
                    action,
                    None,
                    execution::Code::ActionOutputDirectoryCreationFailure,
                ));
            }
        }
        Ok(())
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

enum OutputMetadata {
    File(FileArtifactValue),
    Tree(Arc<TreeArtifactValue>),
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
    let prefix = if is_symlink {
        "dangling symbolic link"
    } else {
        "missing"
    };
    let mut error_message = format!("output '{}' was not created", output.pretty_print());
    if is_symlink {
        error_message = format!(
            "declared output '{}' is a {}",
            output.pretty_print(),
            prefix
        );
    }
    let _ = exception;
    reporter.handle(&Event::error_at(
        Some(action.get_owner().get_location().clone()),
        error_message,
    ));
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

/// `ActionInputMap` + `ActionInputMetadataProvider`: the metadata of an action's inputs, from
/// their Skyframe values.
// Java: com/google/devtools/build/lib/actions/ActionInputMap.java:53 ActionInputMap
// Java: com/google/devtools/build/lib/skyframe/ActionInputMetadataProvider.java:45 ActionInputMetadataProvider
pub struct ActionInputMetadata {
    exec_root: Path,
    files: HashMap<PathFragment, (ActionInputRef, FileArtifactValue)>,
    trees: HashMap<PathFragment, Arc<TreeArtifactValue>>,
    runfiles: HashMap<PathFragment, Arc<RunfilesArtifactValue>>,
    /// PORT: metadata of files the map does not hold (runfiles members, files written by an
    /// earlier spawn of the same action), stat'ed on first use; see `get_input_metadata`.
    stat_cache: Mutex<HashMap<PathFragment, Option<FileArtifactValue>>>,
}

impl ActionInputMetadata {
    pub fn new(exec_root: Path) -> ActionInputMetadata {
        ActionInputMetadata {
            exec_root,
            files: HashMap::new(),
            trees: HashMap::new(),
            runfiles: HashMap::new(),
            stat_cache: Mutex::new(HashMap::new()),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:236 ActionInputMap.put
    pub fn put(&mut self, input: &Artifact, value: FileArtifactValue) {
        self.files.insert(
            input.get_exec_path().clone(),
            (artifact_input(input), value),
        );
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:264 ActionInputMap.putTreeArtifact
    pub fn put_tree_artifact(&mut self, tree: &Artifact, value: Arc<TreeArtifactValue>) {
        self.trees.insert(tree.get_exec_path().clone(), value);
    }

    // Java: com/google/devtools/build/lib/actions/ActionInputMap.java:285 ActionInputMap.putRunfilesMetadata
    pub fn put_runfiles_metadata(&mut self, input: &Artifact, value: Arc<RunfilesArtifactValue>) {
        self.runfiles.insert(input.get_exec_path().clone(), value);
    }
}

impl InputMetadataProvider for ActionInputMetadata {
    fn get_input_metadata(
        &self,
        input: &ActionInputRef,
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
        if let Some((_, v)) = self.files.get(exec_path) {
            return Ok(Some(v.clone()));
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

    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees.get(input.get_exec_path()).cloned()
    }

    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }

    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles.get(input.get_exec_path()).cloned()
    }

    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.files.get(exec_path).map(|(i, _)| i.clone())
    }
}

/// The `ActionExecutionContext` for one action.
// Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:59 ActionExecutionContext
struct ActionExecutionContextImpl<'a> {
    executor: &'a SkyframeActionExecutor,
    input_metadata: Arc<ActionInputMetadata>,
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
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {
        // PORT: --subcommands is not ported.
    }
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {
        // PORT: progress events (SchedulingActionEvent, RunningActionEvent) feed the UI only.
    }
    fn get_verbose_failures(&self) -> bool {
        self.executor.verbose_failures
    }
    // Java: com/google/devtools/build/lib/actions/ActionExecutionContext.java:401 ActionExecutionContext.getInputPath
    fn get_input_path(&self, input: &dyn blaze_exec::actions::ActionInput) -> Path {
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
}

/// The `blaze_actions` artifact behind an execution-layer input, if any.
pub fn input_artifact(input: &ActionInputRef) -> Option<&Artifact> {
    as_blaze_artifact(input)
}
