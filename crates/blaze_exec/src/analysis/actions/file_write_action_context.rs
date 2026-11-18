//! Port of `lib/analysis/actions/FileWriteActionContext.java`.

use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter;

use crate::actions::{ExecException, SpawnResult};
use crate::exec::action_execution_context::ActionExecutionContext;

/// The action context for `AbstractFileWriteAction` instances (technically instances of
/// subclasses).
///
/// Register as `Arc<dyn FileWriteActionContext>` (see [`super::default_action_contexts`]).
// Java: com/google/devtools/build/lib/analysis/actions/FileWriteActionContext.java:29 FileWriteActionContext
pub trait FileWriteActionContext: Send + Sync {
    /// Writes the output created by the [`DeterministicWriter`] to `output`.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteActionContext.java:31 FileWriteActionContext.writeOutputToFile
    fn write_output_to_file_with_output(
        &self,
        action: &dyn Action,
        action_execution_context: &dyn ActionExecutionContext,
        deterministic_writer: &dyn DeterministicWriter,
        make_executable: bool,
        is_remotable: bool,
        output: &Artifact,
    ) -> Result<Vec<SpawnResult>, ExecException>;

    /// Writes the output created by the [`DeterministicWriter`] to the sole output of the given
    /// action.
    // Java: com/google/devtools/build/lib/analysis/actions/FileWriteActionContext.java:44 FileWriteActionContext.writeOutputToFile
    fn write_output_to_file(
        &self,
        action: &dyn Action,
        action_execution_context: &dyn ActionExecutionContext,
        deterministic_writer: &dyn DeterministicWriter,
        make_executable: bool,
        is_remotable: bool,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        let outputs = action.get_outputs();
        // Iterables.getOnlyElement
        blaze_util::check_argument!(
            outputs.len() == 1,
            "expected one element but was: {:?}",
            outputs
        );
        self.write_output_to_file_with_output(
            action,
            action_execution_context,
            deterministic_writer,
            make_executable,
            is_remotable,
            &outputs[0],
        )
    }
}
