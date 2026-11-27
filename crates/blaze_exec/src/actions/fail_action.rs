//! Execution half of `lib/actions/FailAction.java` (`execute`); the action class is
//! `blaze_actions::actions::fail_action::FailAction`.

use blaze_actions::actions::fail_action::FailAction;

use crate::actions::action_execution::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

impl ExecutableAction for FailAction {
    // Java: com/google/devtools/build/lib/actions/FailAction.java:59 FailAction.execute
    fn execute(
        &self,
        _action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        Err(ActionExecutionException::new(
            self.get_error_message(),
            Some(self),
            false,
            Some(self.get_failure_detail().clone()),
        ))
    }
}
