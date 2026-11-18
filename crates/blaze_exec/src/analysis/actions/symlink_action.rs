//! Execution of `lib/analysis/actions/SymlinkAction.java` (`Symlink`, `ExecutableSymlink`).

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::symlink_action::SymlinkAction;
use blaze_cmdline::Label;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail, symlink_action::Code};
use blaze_vfs::{NOW_SENTINEL_TIME, Path, PathFragment, Symlinks};

use super::get_input_path;
use crate::actions::action_adapters::artifact_input;
use crate::actions::{
    ActionExecutionException, ActionInputRef, ActionResult, ExecutableAction, FileStateType,
};
use crate::exec::action_execution_context::ActionExecutionContext;
use crate::exec::spawn_log_context::SpawnLogAction;

/// `UnixFileStatus.S_IXUSR`.
// Java: com/google/devtools/build/lib/unix/UnixFileStatus.java S_IXUSR
const S_IXUSR: u32 = 0o100;

/// `SymlinkAction.createDetailedExitCode(message, detailedCode)`.
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:476 SymlinkAction.createDetailedExitCode
pub(crate) fn create_detailed_exit_code(message: &str, detailed_code: Code) -> FailureDetail {
    FailureDetail {
        message: message.to_string(),
        category: Some(failure_detail::Category::SymlinkAction(
            failure_details::SymlinkAction {
                code: detailed_code as i32,
            },
        )),
    }
}

/// The view of an action that `SpawnLogContext.logSymlinkAction` reads.
pub(crate) struct SymlinkLogAction<'a>(pub &'a dyn Action);

impl SpawnLogAction for SymlinkLogAction<'_> {
    fn get_primary_input(&self) -> Option<ActionInputRef> {
        self.0.get_primary_input().map(|a| artifact_input(&a))
    }
    fn get_primary_output(&self) -> ActionInputRef {
        artifact_input(&self.0.get_primary_output())
    }
    fn get_owner_label(&self) -> Option<Label> {
        self.0.get_owner().get_label().cloned()
    }
    fn get_mnemonic(&self) -> String {
        self.0.get_mnemonic()
    }
}

/// `SymlinkAction.maybeInjectMetadata(symlinkAction, ctx)`.
///
/// PORT: Java injects the input's metadata into `ctx.getOutputMetadataStore()` as an
/// optimization; the execution context here has no output metadata store, so this does what
/// Java does when the metadata cannot be obtained: nothing (the output symlink is read back from
/// the file system after the action finishes executing).
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:396 SymlinkAction.maybeInjectMetadata
pub(crate) fn maybe_inject_metadata(
    _symlink_action: &dyn Action,
    _ctx: &dyn ActionExecutionContext,
) {
}

/// Whether the action's private `inputPath` field is null.
///
/// PORT: `blaze_actions` does not expose `SymlinkAction.inputPath` (only `getInputPath()`, which
/// falls back to the primary input's exec path); `describeKey()` prints the field verbatim
/// (`inputPath: null` when it is null), so it is read from there.
fn input_path_field(action: &SymlinkAction) -> Option<PathFragment> {
    let key = action
        .describe_key()
        .expect("SymlinkAction.describeKey is non-null");
    let line = key
        .lines()
        .find_map(|l| l.strip_prefix("inputPath: "))
        .expect("SymlinkAction.describeKey prints inputPath");
    if line == "null" {
        None
    } else {
        Some(action.get_input_path())
    }
}

fn is_executable_target(action: &SymlinkAction) -> bool {
    // targetType == TargetType.EXECUTABLE (getMnemonic returns "ExecutableSymlink" exactly then).
    action.get_mnemonic() == "ExecutableSymlink"
}

/// `printInputs()`.
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:417 SymlinkAction.printInputs
fn print_inputs(action: &SymlinkAction) -> String {
    let inputs = action.get_inputs();
    if inputs.is_empty() {
        match input_path_field(action) {
            Some(p) => p.get_path_string().to_string(),
            None => "null".to_string(),
        }
    } else if inputs.is_singleton() {
        action
            .get_primary_input()
            .expect("singleton inputs")
            .get_exec_path_string()
            .to_string()
    } else {
        panic!(
            "IllegalStateException: Inputs unexpectedly contains more than 1 element: {}",
            inputs
        )
    }
}

/// `getOutputPath(actionExecutionContext)`.
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:204 SymlinkAction.getOutputPath
fn get_output_path(action: &SymlinkAction, ctx: &dyn ActionExecutionContext) -> Path {
    get_input_path(ctx, &action.get_primary_output())
}

/// `getSymlinkTargetType(ctx)`: reads the primary input's metadata (the result only matters on
/// Windows, where it picks a file or directory link).
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:279 SymlinkAction.getSymlinkTargetType
fn check_symlink_target_type(
    action: &SymlinkAction,
    ctx: &dyn ActionExecutionContext,
) -> std::io::Result<()> {
    let Some(primary_input) = action.get_primary_input() else {
        return Ok(()); // SymlinkTargetType.UNSPECIFIED
    };
    let metadata = ctx
        .get_input_metadata_provider()
        .get_input_metadata(&artifact_input(&primary_input))?;
    let metadata = metadata.unwrap_or_else(|| {
        panic!(
            "NullPointerException: missing metadata for {}",
            primary_input
        )
    });
    // PORT: skipped, Windows only (DIRECTORY vs FILE symlink target type).
    let _ = metadata.get_type() == FileStateType::Directory;
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:295 SymlinkAction.maybeVerifyTargetIsExecutable
fn maybe_verify_target_is_executable(
    action: &SymlinkAction,
    ctx: &dyn ActionExecutionContext,
) -> Result<(), ActionExecutionException> {
    if !is_executable_target(action) {
        return Ok(());
    }

    let primary_input = action
        .get_primary_input()
        .expect("ExecutableSymlink has a primary input");
    let input_path = get_input_path(ctx, &primary_input);

    // Source artifacts are probably in the syscall cache. Generated artifacts are probably not.
    // PORT: the syscall cache is not ported; the stat goes to the file system.
    match input_path.stat_if_found(Symlinks::Follow) {
        Ok(stat) => {
            let stat = match stat {
                Some(s) if s.is_file() => s,
                _ => {
                    let message =
                        format!("'{}' is not a file", primary_input.get_exec_path_string());
                    return Err(ActionExecutionException::new(
                        message.clone(),
                        Some(action),
                        false,
                        Some(create_detailed_exit_code(
                            &message,
                            Code::ExecutableInputNotFile,
                        )),
                    ));
                }
            };
            // PORT: UnixFileStatus.getPermissions is never -1.
            let is_executable = (stat.get_permissions() & S_IXUSR) != 0;
            if !is_executable {
                let message = format!(
                    "failed to create symbolic link '{}': file '{}' is not executable",
                    action.get_primary_output().get_exec_path_string(),
                    primary_input.get_exec_path_string()
                );
                return Err(ActionExecutionException::new(
                    message.clone(),
                    Some(action),
                    false,
                    Some(create_detailed_exit_code(
                        &message,
                        Code::ExecutableInputIsNot,
                    )),
                ));
            }
            Ok(())
        }
        Err(e) => {
            let message = format!(
                "failed to create symbolic link '{}' to the '{}' due to I/O error: {}",
                action.get_primary_output().get_exec_path_string(),
                primary_input.get_exec_path_string(),
                e
            );
            Err(ActionExecutionException::with_cause(
                message.clone(),
                &e,
                Some(action),
                false,
                Some(create_detailed_exit_code(
                    &message,
                    Code::ExecutableInputCheckIoException,
                )),
            ))
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:341 SymlinkAction.updateInputMtimeIfNeeded
fn update_input_mtime_if_needed(
    action: &SymlinkAction,
    ctx: &dyn ActionExecutionContext,
) -> Result<(), ActionExecutionException> {
    if !action.is_fileset() {
        return Ok(());
    }

    let run = || -> std::io::Result<()> {
        // Update the mtime of the target of the symlink to force downstream re-execution of
        // actions. This is needed because dependency checking of Fileset output trees is unsound
        // (it's a directory). Note that utime() on a symlink actually changes the mtime of its
        // target.
        let link_path = get_output_path(action, ctx);
        if link_path.exists() {
            link_path.set_last_modified_time(NOW_SENTINEL_TIME)
        } else {
            // Should only happen if the Fileset included no links.
            ctx.get_exec_root()
                .get_relative(&action.get_input_path())
                .create_directory_and_parents()
        }
    };
    run().map_err(|e| {
        let message = format!(
            "failed to touch symbolic link '{}' to the '{}' due to I/O error: {}",
            action.get_primary_output().get_exec_path_string(),
            action
                .get_primary_input()
                .map(|i| i.get_exec_path_string().to_string())
                .unwrap_or_else(|| "null".to_string()),
            e
        );
        ActionExecutionException::with_cause(
            message.clone(),
            &e,
            Some(action),
            false,
            Some(create_detailed_exit_code(
                &message,
                Code::LinkTouchIoException,
            )),
        )
    })
}

impl ExecutableAction for SymlinkAction {
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkAction.java:217 SymlinkAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        maybe_verify_target_is_executable(self, ctx)?;

        let target_path = match input_path_field(self) {
            None => get_input_path(
                ctx,
                &self
                    .get_primary_input()
                    .expect("SymlinkAction without input path has a primary input"),
            ),
            Some(input_path) => ctx.get_exec_root().get_relative(&input_path),
        };

        let output_path = get_output_path(self, ctx);

        let link = || -> std::io::Result<()> {
            // Delete the empty output directory created prior to the action execution when the
            // output is a tree artifact. All other actions that produce tree artifacts expect it
            // to exist prior to their execution. It's not worth complicating
            // ActionOutputDirectoryHelper just to avoid this small amount of overhead.
            output_path.delete()?;

            check_symlink_target_type(self, ctx)?;
            output_path.create_symbolic_link(&target_path)
        };
        if let Err(e) = link() {
            let message = format!(
                "failed to create symbolic link '{}' to '{}' due to I/O error: {}",
                self.get_primary_output().get_exec_path_string(),
                print_inputs(self),
                e
            );
            return Err(ActionExecutionException::with_cause(
                message.clone(),
                &e,
                Some(self),
                false,
                Some(create_detailed_exit_code(
                    &message,
                    Code::LinkCreationIoException,
                )),
            ));
        }

        update_input_mtime_if_needed(self, ctx)?;

        if let Some(log_context) = ctx.get_spawn_log_context()
            && let Err(e) = log_context.log_symlink_action(&SymlinkLogAction(self))
        {
            let message = format!(
                "failed to log creation of symlink '{}' to '{}' due to I/O error: {}",
                self.get_primary_output().get_exec_path_string(),
                print_inputs(self),
                e
            );
            return Err(ActionExecutionException::with_cause(
                message.clone(),
                &e,
                Some(self),
                false,
                Some(create_detailed_exit_code(
                    &message,
                    Code::LinkLogIoException,
                )),
            ));
        }

        if self.is_fileset() {
            // Forward the Fileset metadata to the output artifact of this symlink.
            blaze_util::unported!(
                "SymlinkAction.execute (Fileset)",
                "ActionExecutionContext.setRichArtifactData(FilesetOutputTree.forward(...))"
            );
        } else {
            maybe_inject_metadata(self, ctx);
        }
        Ok(ActionResult::empty())
    }
}
