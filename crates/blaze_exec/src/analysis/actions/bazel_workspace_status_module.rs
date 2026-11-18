//! Execution of `lib/bazel/BazelWorkspaceStatusModule.java`'s `BazelWorkspaceStatusAction`, its
//! `BazelWorkspaceStatusActionContext`, the `WorkspaceStatusAction.Context` interface and
//! `lib/analysis/BuildInfoEvent.java`.

use std::any::TypeId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use blaze_actions::analysis::workspace_status_action::{
    WorkspaceStatusAction, execution_exception_message,
};
use blaze_actions::bazel::bazel_workspace_status_module::{
    BazelWorkspaceStatusAction, WorkspaceStatusInputs,
};
use blaze_events::{Event, Postable};
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_shell::util::command_builder::CommandBuilder;
use blaze_shell::{ByteArrayOutputStream, Command, CommandException, CommandExecutionError};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
use parking_lot::Mutex;

use super::file_system_utils;
use super::get_input_path;
use crate::actions::action_execution::ActionContextRef;
use crate::actions::{ActionExecutionException, ActionResult, ExecException, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

use failure_details::workspace_status::Code;

/// `WorkspaceStatusAction.Options` (the fields the action reads).
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:75 WorkspaceStatusAction.Options
#[derive(Clone, Debug)]
pub struct WorkspaceStatusOptions {
    /// `--embed_label`.
    pub embed_label: String,
    /// `--workspace_status_command` (`PathFragment.EMPTY_FRAGMENT` when unset).
    pub workspace_status_command: PathFragment,
}

/// The context for the workspace status action.
///
/// Register as `Arc<dyn WorkspaceStatusActionContext>` (see
/// [`workspace_status_context_entry`]).
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:103 WorkspaceStatusAction.Context
pub trait WorkspaceStatusActionContext: Send + Sync {
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:106 WorkspaceStatusAction.Context.getOptions
    fn get_options(&self) -> &WorkspaceStatusOptions;

    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:108 WorkspaceStatusAction.Context.getClientEnv
    fn get_client_env(&self) -> &IndexMap<String, String>;

    /// `None` where Java returns null (no `--workspace_status_command`).
    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:110 WorkspaceStatusAction.Context.getCommand
    fn get_command(&self) -> Option<Command>;
}

/// Bazel's `WorkspaceStatusAction.Context`, built from the command environment.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:288 BazelWorkspaceStatusModule.BazelWorkspaceStatusActionContext
pub struct BazelWorkspaceStatusActionContext {
    options: WorkspaceStatusOptions,
    client_env: IndexMap<String, String>,
    workspace: Path,
}

impl BazelWorkspaceStatusActionContext {
    /// `new BazelWorkspaceStatusActionContext(env)`: `options` are the command's
    /// `WorkspaceStatusAction.Options`, `client_env` is `env.getClientEnv()` and `workspace` is
    /// `env.getWorkspace()`.
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:292 BazelWorkspaceStatusActionContext.<init>
    pub fn new(
        options: WorkspaceStatusOptions,
        client_env: IndexMap<String, String>,
        workspace: Path,
    ) -> BazelWorkspaceStatusActionContext {
        BazelWorkspaceStatusActionContext {
            options,
            client_env,
            workspace,
        }
    }
}

impl WorkspaceStatusActionContext for BazelWorkspaceStatusActionContext {
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:297 BazelWorkspaceStatusActionContext.getOptions
    fn get_options(&self) -> &WorkspaceStatusOptions {
        &self.options
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:302 BazelWorkspaceStatusActionContext.getClientEnv
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:308 BazelWorkspaceStatusActionContext.getCommand
    fn get_command(&self) -> Option<Command> {
        if self.options.workspace_status_command == PathFragment::empty_fragment() {
            return None;
        }
        Some(
            CommandBuilder::new(&self.client_env)
                .add_args(&[self.options.workspace_status_command.get_path_string()])
                // Pass client env to allow SCM clients (like git) relying on environment
                // variables to work correctly.
                .set_env(&self.client_env)
                .set_working_dir(&self.workspace)
                .use_shell(true)
                .build(),
        )
    }
}

/// The `(TypeId, context)` pair under which the executor registers a workspace status context.
pub fn workspace_status_context_entry(
    context: Arc<dyn WorkspaceStatusActionContext>,
) -> (TypeId, ActionContextRef) {
    (
        TypeId::of::<Arc<dyn WorkspaceStatusActionContext>>(),
        Arc::new(context) as ActionContextRef,
    )
}

/// This event is fired once build info data is available.
// Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:22 BuildInfoEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfoEvent {
    build_info_map: BTreeMap<String, String>,
}

impl BuildInfoEvent {
    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:26 BuildInfoEvent.<init>
    pub fn new(build_info_map: BTreeMap<String, String>) -> BuildInfoEvent {
        BuildInfoEvent { build_info_map }
    }

    // Java: com/google/devtools/build/lib/analysis/BuildInfoEvent.java:30 BuildInfoEvent.getBuildInfoMap
    pub fn get_build_info_map(&self) -> &BTreeMap<String, String> {
        &self.build_info_map
    }
}

impl Postable for BuildInfoEvent {}

// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:261 BazelWorkspaceStatusModule.createDetailedCode
fn create_detailed_code(message: &str, detailed_code: Code) -> FailureDetail {
    super::failure_detail(
        message,
        Category::WorkspaceStatus(failure_details::WorkspaceStatus {
            code: detailed_code as i32,
        }),
    )
}

/// `WorkspaceStatusAction.createExecutionException(e, detailedCode)`.
// Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:205 WorkspaceStatusAction.createExecutionException
fn create_execution_exception(
    action: &BazelWorkspaceStatusAction,
    e: &str,
    detailed_code: Code,
) -> ActionExecutionException {
    let message = execution_exception_message(action.get_workspace_status_description(), e);
    ActionExecutionException::with_cause(
        message.clone(),
        &e,
        Some(action),
        false,
        Some(create_detailed_code(&message, detailed_code)),
    )
}

// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:99 BazelWorkspaceStatusAction.getAdditionalWorkspaceStatus
fn get_additional_workspace_status(
    action: &BazelWorkspaceStatusAction,
    options: &WorkspaceStatusOptions,
    context: &dyn WorkspaceStatusActionContext,
    action_execution_context: &dyn ActionExecutionContext,
) -> Result<String, ActionExecutionException> {
    let Some(get_workspace_status_command) = context.get_command() else {
        return Ok(String::new());
    };
    action_execution_context
        .get_event_handler()
        .handle(&Event::progress(format!(
            "Getting additional workspace status by running {}",
            options.workspace_status_command
        )));
    let stdout_stream = Arc::new(Mutex::new(ByteArrayOutputStream::new()));
    let err_stream = match action_execution_context
        .get_file_out_err()
        .get_error_path()
        .get_output_stream()
    {
        Ok(s) => s,
        Err(e) => {
            return Err(create_execution_exception(
                action,
                &e.to_string(),
                Code::StderrIoException,
            ));
        }
    };
    let err_stream: blaze_shell::OutputStream = Arc::new(Mutex::new(err_stream));
    match get_workspace_status_command.execute_streaming(stdout_stream.clone(), err_stream) {
        Ok(_) => {}
        Err(CommandExecutionError::Command(e)) => {
            let code = match &e {
                CommandException::BadExitStatus(_) => Code::NonZeroExit,
                CommandException::AbnormalTermination(_) => Code::AbnormalTermination,
                CommandException::ExecFailed(_) => Code::ExecFailed,
            };
            return Err(create_execution_exception(action, e.get_message(), code));
        }
        Err(CommandExecutionError::Interrupted(_)) => {
            return Err(ActionExecutionException::from_exec_exception(
                &ExecException::Interrupted,
                None,
                action,
            ));
        }
    }
    let bytes = stdout_stream.lock().to_byte_array();
    // stdoutStream.toString(UTF_8)
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl ExecutableAction for BazelWorkspaceStatusAction {
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:161 BazelWorkspaceStatusAction.prepare
    fn prepare(&self, _exec_root: &blaze_vfs::Path) -> std::io::Result<()> {
        // The default implementation of this method deletes all output files; override it to
        // keep the old stableStatus around. This way we can reuse the existing file (preserving
        // its mtime) if the contents haven't changed.
        let volatile_status = self.get_volatile_status();
        crate::actions::action_execution::delete_output(
            &volatile_status.get_path(),
            Some(volatile_status.get_root().get_root()),
        )
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:174 BazelWorkspaceStatusAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let context = super::get_required_context::<dyn WorkspaceStatusActionContext>(
            ctx,
            "WorkspaceStatusAction.Context",
        );
        let options = context.get_options().clone();
        let current_time_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let workspace_status_command_output =
            get_additional_workspace_status(self, &options, &*context, ctx)?;
        let files = self.compute_status_files(&WorkspaceStatusInputs {
            embed_label: options.embed_label.clone(),
            client_env: context.get_client_env().clone(),
            current_time_millis,
            workspace_status_command_output,
        });
        ctx.get_event_handler()
            .post(Arc::new(BuildInfoEvent::new(files.overall_map.clone())));

        let write = || -> std::io::Result<()> {
            // Only update the stableStatus contents if they are different than what we have on
            // disk. This is to preserve the old file's mtime so that we do not generate an
            // unnecessary dirty file on each incremental build.
            file_system_utils::maybe_update_content(
                &get_input_path(ctx, &self.get_stable_status()),
                &files.stable_status,
            )?;

            // Contrary to the stableStatus, write the contents of volatileStatus unconditionally
            // because we know it will be different. This output file is marked as "constant
            // metadata" so its dirtiness will be ignored anyway.
            file_system_utils::write_content(
                &get_input_path(ctx, &self.get_volatile_status()),
                &files.volatile_status,
            )
        };
        if let Err(e) = write() {
            let message = format!(
                "Failed to run workspace status command {}: {}",
                options.workspace_status_command, e
            );
            return Err(ActionExecutionException::with_cause(
                message.clone(),
                &e,
                Some(self),
                true,
                Some(create_detailed_code(
                    &message,
                    Code::ContentUpdateIoException,
                )),
            ));
        }
        Ok(ActionResult::empty())
    }
}
