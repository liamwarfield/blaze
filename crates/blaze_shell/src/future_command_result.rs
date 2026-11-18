//! Port of `com.google.devtools.build.lib.shell.FutureCommandResult`.

use crate::command::Command;
use crate::command_exception::{
    AbnormalTerminationException, BadExitStatusException, CommandException, CommandExecutionError,
};
use crate::command_result::CommandResult;
use crate::consumers::OutErrConsumers;
use crate::subprocess::Subprocess;
use crate::termination_status::TerminationStatus;
use crate::thread_interrupt::{self, InterruptedException};

/// Supplier of the command result which additionally allows to check if the command already
/// terminated. Implementations of this interface may not be thread-safe.
// Java: com/google/devtools/build/lib/shell/FutureCommandResult.java:23 FutureCommandResult
pub struct FutureCommandResult {
    command: Command,
    process: Box<dyn Subprocess>,
    out_err_consumers: OutErrConsumers,
    kill_subprocess_on_interrupt: bool,
}

impl FutureCommandResult {
    // Java: com/google/devtools/build/lib/shell/FutureCommandResult.java:29 FutureCommandResult.<init>
    pub(crate) fn new(
        command: Command,
        process: Box<dyn Subprocess>,
        out_err_consumers: OutErrConsumers,
        kill_subprocess_on_interrupt: bool,
    ) -> FutureCommandResult {
        FutureCommandResult {
            command,
            process,
            out_err_consumers,
            kill_subprocess_on_interrupt,
        }
    }

    /// Returns the result of command execution. If the process is not finished yet (as reported
    /// by [`Self::is_done`]), the call will block until that process terminates.
    ///
    /// Fails with an abnormal-termination exception if command execution failed, or with
    /// `InterruptedException` if `kill_subprocess_on_interrupt` is true and the thread is
    /// interrupted before the subprocess completes.
    // Java: com/google/devtools/build/lib/shell/FutureCommandResult.java:49 FutureCommandResult.get
    pub fn get(&mut self) -> Result<CommandResult, CommandExecutionError> {
        let status = match wait_for_process(&*self.process, self.kill_subprocess_on_interrupt) {
            Ok(status) => status,
            Err(e) => {
                self.out_err_consumers.cancel();
                self.process.close();
                return Err(e.into());
            }
        };
        let completion = if thread_interrupt::is_current_interrupted() {
            // Can be interrupted if killSubprocessOnInterrupt is false, or interrupt raced with us.
            self.out_err_consumers.cancel();
            Ok(())
        } else {
            self.out_err_consumers.wait_for_completion()
        };
        // finally
        self.process.close();
        if let Err(ioe) = completion {
            let no_output_result = CommandResult::builder()
                .set_stdout_stream(CommandResult::EMPTY_OUTPUT)
                .set_stderr_stream(CommandResult::EMPTY_OUTPUT)
                .set_termination_status(status.clone())
                .build();
            let e: CommandException = if status.success() {
                // If command was otherwise successful, throw an exception about this
                AbnormalTerminationException::with_cause(
                    self.command.clone(),
                    no_output_result,
                    ioe,
                )
                .into()
            } else {
                // Otherwise, throw the more important exception -- command was not successful
                let message = format!(
                    "{status}; also encountered an error while attempting to retrieve output"
                );
                if status.exited() {
                    BadExitStatusException::with_message_and_cause(
                        self.command.clone(),
                        no_output_result,
                        message,
                        ioe,
                    )
                    .into()
                } else {
                    AbnormalTerminationException::with_message_and_cause(
                        self.command.clone(),
                        no_output_result,
                        message,
                        ioe,
                    )
                    .into()
                }
            };
            return Err(e.into());
        }

        let command_result = CommandResult::builder()
            .set_stdout_stream(self.out_err_consumers.get_accumulated_out())
            .set_stderr_stream(self.out_err_consumers.get_accumulated_err())
            .set_termination_status(status.clone())
            .build();
        command_result.log_this();
        if status.success() {
            Ok(command_result)
        } else if status.exited() {
            Err(CommandException::from(BadExitStatusException::new(
                self.command.clone(),
                command_result,
                status.to_string(),
            ))
            .into())
        } else {
            Err(CommandException::from(AbnormalTerminationException::new(
                self.command.clone(),
                command_result,
                status.to_string(),
            ))
            .into())
        }
    }

    // Java: com/google/devtools/build/lib/shell/FutureCommandResult.java:133 FutureCommandResult.isDone
    pub fn is_done(&self) -> bool {
        self.process.finished()
    }
}

// Java: com/google/devtools/build/lib/shell/FutureCommandResult.java:103 FutureCommandResult.waitForProcess
fn wait_for_process(
    process: &dyn Subprocess,
    kill_subprocess_on_interrupt: bool,
) -> Result<TerminationStatus, InterruptedException> {
    let mut was_interrupted = false;
    loop {
        match process.wait_for() {
            Ok(()) => break,
            Err(InterruptedException) => {
                was_interrupted = true;
                if kill_subprocess_on_interrupt {
                    process.destroy();
                }
            }
        }
    }
    if was_interrupted && kill_subprocess_on_interrupt {
        // Don't need to do any clean-up below.
        return Err(InterruptedException);
    }
    let result = TerminationStatus::new(process.exit_value(), process.timedout());
    if was_interrupted {
        thread_interrupt::interrupt_current(); // preserve interrupted status
    }
    Ok(result)
}
