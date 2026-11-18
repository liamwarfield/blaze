//! Execution of `lib/analysis/actions/BuildInfoFileWriteAction.java` (`TranslateBuildInfo`).

use blaze_actions::analysis::actions::build_info_file_write_action::BuildInfoFileWriteAction;

use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

impl ExecutableAction for BuildInfoFileWriteAction {
    /// Parses the workspace status file (`WorkspaceStatusAction.parseValues`), calls the Starlark
    /// translation callback in a fresh `StarlarkThread` created with the action's
    /// `StarlarkSemantics`, and expands the template with the returned dict
    /// (`TemplateExpansionAction.execute`, see
    /// [`super::template_expansion_action::execute_template_expansion`]).
    ///
    /// PORT: `blaze_actions` does not keep the `StarlarkSemantics` Java's constructor receives
    /// (only the callback), and the thread must be created with it.
    // Java: com/google/devtools/build/lib/analysis/actions/BuildInfoFileWriteAction.java:97 BuildInfoFileWriteAction.execute
    fn execute(
        &self,
        _ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        blaze_util::unported!(
            "BuildInfoFileWriteAction.execute",
            "blaze_actions does not expose the action's StarlarkSemantics (needed to call the translation callback)"
        )
    }
}
