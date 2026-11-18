//! Port of `com.google.devtools.build.lib.util.CommandUtils`.

use crate::command::Command;
use crate::command_exception::CommandException;
use crate::util::command_failure_utils::CommandFailureUtils;

/// Utility methods relating to the [`Command`] class.
// Java: com/google/devtools/build/lib/util/CommandUtils.java:25 CommandUtils
pub struct CommandUtils;

impl CommandUtils {
    // Java: com/google/devtools/build/lib/util/CommandUtils.java:31 CommandUtils.cwd
    pub fn cwd(command: &Command) -> Option<String> {
        command
            .get_working_directory()
            .map(|wd| wd.get_path().to_string())
    }

    /// Construct an error message that describes a failed command invocation. Currently this
    /// returns a message of the form "foo failed: error executing command /dir/foo bar baz:
    /// exception message", with the command's stdout and stderr output appended if available.
    // Java: com/google/devtools/build/lib/util/CommandUtils.java:41 CommandUtils.describeCommandFailure
    pub fn describe_command_failure(verbose: bool, exception: &CommandException) -> String {
        let command = exception.get_command();
        let message = format!(
            "{}: {}",
            CommandFailureUtils::describe_command_failure(
                verbose,
                CommandUtils::cwd(command).as_deref(),
                command
            ),
            exception.get_message()
        );
        if let Some(abnormal_termination_exception) = exception.as_abnormal_termination() {
            let result = abnormal_termination_exception.get_result();
            // PORT: Java catches the IllegalStateException that getStdout() throws when the
            // command didn't save stdout/stderr; here the sentinel is checked first.
            if !result.stdout_stream().is_no_output_collected()
                && !result.stderr_stream().is_no_output_collected()
            {
                // `new String(bytes)` decodes with the platform charset (UTF-8).
                return format!(
                    "{}\n{}{}",
                    message,
                    String::from_utf8_lossy(&result.get_stdout()),
                    String::from_utf8_lossy(&result.get_stderr())
                );
            }
        }
        message
    }
}
