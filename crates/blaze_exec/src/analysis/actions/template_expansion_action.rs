//! Execution of `lib/analysis/actions/TemplateExpansionAction.java`.

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::template_expansion_action::{
    TemplateExpansionAction, TemplateExpansionContextTemplateMetadata,
};
use blaze_protos::failure_details::execution::Code;

use super::local_template_expansion_strategy::{ExpandTemplateError, TemplateExpansionContext};
use super::{execution_category, failure_detail};
use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

/// `TemplateExpansionAction.execute(actionExecutionContext, action, templateMetadata)` (also used
/// by `BuildInfoFileWriteAction`).
// Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:131 TemplateExpansionAction.execute
pub fn execute_template_expansion(
    action_execution_context: &dyn ActionExecutionContext,
    action: &dyn Action,
    template_metadata: &TemplateExpansionContextTemplateMetadata,
) -> Result<ActionResult, ActionExecutionException> {
    let context = super::get_required_context::<dyn TemplateExpansionContext>(
        action_execution_context,
        "TemplateExpansionContext",
    );
    match context.expand_template(action, action_execution_context, template_metadata) {
        Ok(result) => Ok(ActionResult::create(result)),
        Err(ExpandTemplateError::Eval(message)) => {
            // new ActionExecutionException(e, action, false, DetailedExitCode(Execution.LOCAL_TEMPLATE_EXPANSION_FAILURE))
            Err(ActionExecutionException::with_cause(
                message.clone(),
                &message,
                Some(action),
                /* catastrophe= */ false,
                Some(failure_detail(
                    "",
                    execution_category(Code::LocalTemplateExpansionFailure),
                )),
            ))
        }
        Err(ExpandTemplateError::Exec(e)) => Err(ActionExecutionException::from_exec_exception(
            &e, None, action,
        )),
    }
}

impl ExecutableAction for TemplateExpansionAction {
    // Java: com/google/devtools/build/lib/analysis/actions/TemplateExpansionAction.java:158 TemplateExpansionAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        execute_template_expansion(ctx, self, &self.get_template_metadata())
    }
}
