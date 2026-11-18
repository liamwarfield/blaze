//! Port of `lib/exec/FileWriteStrategy.java`.

use std::io::{BufWriter, Write};

use blaze_actions::actions::action::Action;
use blaze_actions::actions::artifact::Artifact;
use blaze_actions::analysis::actions::abstract_file_write_action::DeterministicWriter;
use blaze_protos::failure_details::execution::Code;

use super::file_write_action_context::FileWriteActionContext;
use super::{environmental_exec_exception, get_input_path, post_running_action_event};
use crate::actions::{ExecException, SpawnResult};
use crate::exec::action_execution_context::ActionExecutionContext;

/// A strategy for executing an `AbstractFileWriteAction`.
// Java: com/google/devtools/build/lib/exec/FileWriteStrategy.java:37 FileWriteStrategy
#[derive(Debug, Default)]
pub struct FileWriteStrategy;

impl FileWriteStrategy {
    pub fn new() -> FileWriteStrategy {
        FileWriteStrategy
    }
}

impl FileWriteActionContext for FileWriteStrategy {
    // Java: com/google/devtools/build/lib/exec/FileWriteStrategy.java:41 FileWriteStrategy.writeOutputToFile
    fn write_output_to_file_with_output(
        &self,
        action: &dyn Action,
        action_execution_context: &dyn ActionExecutionContext,
        deterministic_writer: &dyn DeterministicWriter,
        make_executable: bool,
        _is_remotable: bool,
        output: &Artifact,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        post_running_action_event(action_execution_context, action, "local");
        // TODO(ulfjack): Consider acquiring local resources here before trying to write the file.
        // PORT: skipped, profiling (GoogleAutoProfilerUtils.logged).
        let output_path = get_input_path(action_execution_context, output);
        let write = || -> std::io::Result<()> {
            {
                let mut out = BufWriter::new(output_path.get_output_stream()?);
                deterministic_writer.write_to(&mut out)?;
                out.flush()?;
            }
            if make_executable {
                output_path.set_executable(true)?;
            }
            Ok(())
        };
        if let Err(e) = write() {
            return Err(environmental_exec_exception(&e, Code::FileWriteIoException));
        }
        Ok(Vec::new())
    }
}
