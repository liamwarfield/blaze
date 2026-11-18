//! Execution of `lib/analysis/actions/SymlinkTreeAction.java`, and
//! `lib/analysis/actions/SymlinkTreeActionContext.java`.

use blaze_actions::actions::action::Action;
use blaze_actions::analysis::actions::symlink_tree_action::SymlinkTreeAction;

use crate::actions::{ActionExecutionException, ActionResult, ExecutableAction};
use crate::exec::action_execution_context::ActionExecutionContext;

/// Action context for symlink tree actions (an action that creates a tree of symlinks).
///
/// Register as `Arc<dyn SymlinkTreeActionContext>` (see [`super::default_action_contexts`]).
// Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeActionContext.java:22 SymlinkTreeActionContext
pub trait SymlinkTreeActionContext: Send + Sync {
    /// Creates the symlink tree.
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeActionContext.java:25 SymlinkTreeActionContext.createSymlinks
    fn create_symlinks(
        &self,
        action: &SymlinkTreeAction,
        action_execution_context: &dyn ActionExecutionContext,
    ) -> Result<(), ActionExecutionException>;
}

impl ExecutableAction for SymlinkTreeAction {
    // Java: com/google/devtools/build/lib/analysis/actions/SymlinkTreeAction.java:237 SymlinkTreeAction.execute
    fn execute(
        &self,
        ctx: &dyn ActionExecutionContext,
    ) -> Result<ActionResult, ActionExecutionException> {
        super::get_required_context::<dyn SymlinkTreeActionContext>(
            ctx,
            "SymlinkTreeActionContext",
        )
        .create_symlinks(self, ctx)?;
        if self.get_primary_output().is_fileset() {
            blaze_util::unported!(
                "SymlinkTreeAction.execute (Fileset)",
                "ActionExecutionContext.setRichArtifactData(FilesetOutputTree.forward(...))"
            );
        }
        Ok(ActionResult::empty())
    }
}
