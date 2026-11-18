//! Execution of `lib/rules/cpp/SolibSymlinkAction.java`.

use blaze_actions::actions::action::Action;
use blaze_actions::rules::cpp::solib_symlink_action::SolibSymlinkAction;
use blaze_protos::failure_details::symlink_action::Code;

use super::get_input_path;
use super::symlink_action::{SymlinkLogAction, create_detailed_exit_code, maybe_inject_metadata};
use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

impl ExecutableAction for SolibSymlinkAction {
    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:76 SolibSymlinkAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        let symlink = self.get_symlink();
        let primary_input = self
            .get_primary_input()
            .expect("SolibSymlinkAction has a primary input");
        let mangled_path = get_input_path(ctx, symlink);
        if let Err(e) = mangled_path.create_symbolic_link(&get_input_path(ctx, &primary_input)) {
            let message = format!(
                "failed to create _solib symbolic link '{}' to target '{}': {}",
                symlink.pretty_print(),
                primary_input,
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

        if let Some(log_context) = ctx.get_spawn_log_context()
            && let Err(e) = log_context.log_symlink_action(&SymlinkLogAction(self))
        {
            let message = format!(
                "failed to log creation of _solib symbolic link '{}' to target '{}': {}",
                symlink.pretty_print(),
                primary_input,
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
        maybe_inject_metadata(self, ctx);
        Ok(ActionResult::empty())
    }
}
