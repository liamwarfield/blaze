//! Execution of `lib/analysis/starlark/UnresolvedSymlinkAction.java`.

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::starlark::unresolved_symlink_action::UnresolvedSymlinkAction;
use blaze_protos::failure_details::symlink_action::Code;
use blaze_vfs::PathFragment;

use super::get_input_path;
use super::symlink_action::create_detailed_exit_code;
use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

impl ExecutableAction for UnresolvedSymlinkAction {
    // Java: com/google/devtools/build/lib/analysis/starlark/UnresolvedSymlinkAction.java:77 UnresolvedSymlinkAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let output_path = get_input_path(ctx, &self.get_primary_output());
        // getTargetPathFragment(): PathFragment.create(target) (getTarget() is its string form).
        let target = self.get_target();
        // PORT: skipped, Windows only (the SymlinkTargetType argument of createSymbolicLink).
        if let Err(e) = output_path.create_symbolic_link_to_fragment(&PathFragment::create(&target))
        {
            // PORT: Java prints the raw `target` field; `blaze_actions` exposes only its
            // normalized form (`getTarget()`), which differs only for non-normalized targets.
            let message = format!(
                "failed to create symbolic link '{}' with target '{}' due to I/O error: {}",
                self.get_primary_output().get_exec_path_string(),
                target,
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

        Ok(ActionResult::empty())
    }
}
