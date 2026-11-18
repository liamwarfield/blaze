//! Ports of `CommandException` and its subclasses `ExecFailedException`,
//! `AbnormalTerminationException` and `BadExitStatusException`.

use std::fmt;
use std::io;
use std::ops::Deref;
use std::sync::Arc;

use crate::command::Command;
use crate::command_result::CommandResult;
use crate::java_process::java_io_exception_to_string;
use crate::thread_interrupt::InterruptedException;

/// Thrown when a command could not even be executed; in particular, when the process could not
/// be started.
// Java: com/google/devtools/build/lib/shell/ExecFailedException.java:21 ExecFailedException
#[derive(Clone, Debug)]
pub struct ExecFailedException {
    command: Command,
    cause: Arc<io::Error>,
    message: String,
}

impl ExecFailedException {
    // Java: com/google/devtools/build/lib/shell/ExecFailedException.java:23 ExecFailedException.<init>
    pub fn new(command: Command, cause: io::Error) -> ExecFailedException {
        // Java's Exception(Throwable cause) uses cause.toString() as the message.
        let message = java_io_exception_to_string(&cause);
        ExecFailedException {
            command,
            cause: Arc::new(cause),
            message,
        }
    }

    pub fn get_command(&self) -> &Command {
        &self.command
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_cause(&self) -> &io::Error {
        &self.cause
    }
}

/// Thrown when a command's execution terminates abnormally -- for example, if it is killed, or if
/// it terminates with a non-zero exit status.
// Java: com/google/devtools/build/lib/shell/AbnormalTerminationException.java:21 AbnormalTerminationException
#[derive(Clone, Debug)]
pub struct AbnormalTerminationException {
    command: Command,
    result: Box<CommandResult>,
    message: String,
    cause: Option<Arc<io::Error>>,
}

impl AbnormalTerminationException {
    // Java: com/google/devtools/build/lib/shell/AbnormalTerminationException.java:25 AbnormalTerminationException.<init>
    pub fn new(command: Command, result: CommandResult, message: String) -> Self {
        AbnormalTerminationException {
            command,
            result: Box::new(result),
            message,
            cause: None,
        }
    }

    /// `AbnormalTerminationException(Command, CommandResult, Throwable)`: the message is the
    /// cause's `toString()`.
    // Java: com/google/devtools/build/lib/shell/AbnormalTerminationException.java:32 AbnormalTerminationException.<init>
    pub fn with_cause(command: Command, result: CommandResult, cause: io::Error) -> Self {
        AbnormalTerminationException {
            command,
            result: Box::new(result),
            message: java_io_exception_to_string(&cause),
            cause: Some(Arc::new(cause)),
        }
    }

    // Java: com/google/devtools/build/lib/shell/AbnormalTerminationException.java:39 AbnormalTerminationException.<init>
    pub fn with_message_and_cause(
        command: Command,
        result: CommandResult,
        message: String,
        cause: io::Error,
    ) -> Self {
        AbnormalTerminationException {
            command,
            result: Box::new(result),
            message,
            cause: Some(Arc::new(cause)),
        }
    }

    pub fn get_command(&self) -> &Command {
        &self.command
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/shell/AbnormalTerminationException.java:47 AbnormalTerminationException.getResult
    pub fn get_result(&self) -> &CommandResult {
        &self.result
    }

    pub fn get_cause(&self) -> Option<&io::Error> {
        self.cause.as_deref()
    }
}

/// Thrown when a command's execution terminates with a non-zero exit status. A subclass of
/// [`AbnormalTerminationException`] in Java; here it derefs to one.
// Java: com/google/devtools/build/lib/shell/BadExitStatusException.java:20 BadExitStatusException
#[derive(Clone, Debug)]
pub struct BadExitStatusException(AbnormalTerminationException);

impl BadExitStatusException {
    // Java: com/google/devtools/build/lib/shell/BadExitStatusException.java:22 BadExitStatusException.<init>
    pub fn new(command: Command, result: CommandResult, message: String) -> Self {
        BadExitStatusException(AbnormalTerminationException::new(command, result, message))
    }

    // Java: com/google/devtools/build/lib/shell/BadExitStatusException.java:28 BadExitStatusException.<init>
    pub fn with_message_and_cause(
        command: Command,
        result: CommandResult,
        message: String,
        cause: io::Error,
    ) -> Self {
        BadExitStatusException(AbnormalTerminationException::with_message_and_cause(
            command, result, message, cause,
        ))
    }

    /// The `AbnormalTerminationException` view of this exception.
    pub fn as_abnormal_termination(&self) -> &AbnormalTerminationException {
        &self.0
    }
}

impl Deref for BadExitStatusException {
    type Target = AbnormalTerminationException;

    fn deref(&self) -> &AbnormalTerminationException {
        &self.0
    }
}

/// Superclass of all exceptions that may be thrown during command execution. It exists to unify
/// them. It also provides access to the command name and arguments for the failing command.
// Java: com/google/devtools/build/lib/shell/CommandException.java:22 CommandException
#[derive(Clone, Debug)]
pub enum CommandException {
    ExecFailed(ExecFailedException),
    AbnormalTermination(AbnormalTerminationException),
    BadExitStatus(BadExitStatusException),
}

impl CommandException {
    /// Returns the command that failed.
    // Java: com/google/devtools/build/lib/shell/CommandException.java:27 CommandException.getCommand
    pub fn get_command(&self) -> &Command {
        match self {
            CommandException::ExecFailed(e) => e.get_command(),
            CommandException::AbnormalTermination(e) => e.get_command(),
            CommandException::BadExitStatus(e) => e.get_command(),
        }
    }

    /// `Throwable.getMessage()`.
    pub fn get_message(&self) -> &str {
        match self {
            CommandException::ExecFailed(e) => e.get_message(),
            CommandException::AbnormalTermination(e) => e.get_message(),
            CommandException::BadExitStatus(e) => e.get_message(),
        }
    }

    /// `this instanceof AbnormalTerminationException` (true for `BadExitStatusException` too).
    pub fn as_abnormal_termination(&self) -> Option<&AbnormalTerminationException> {
        match self {
            CommandException::ExecFailed(_) => None,
            CommandException::AbnormalTermination(e) => Some(e),
            CommandException::BadExitStatus(e) => Some(e.as_abnormal_termination()),
        }
    }
}

impl fmt::Display for CommandException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_message())
    }
}

impl std::error::Error for CommandException {}

impl From<ExecFailedException> for CommandException {
    fn from(e: ExecFailedException) -> Self {
        CommandException::ExecFailed(e)
    }
}

impl From<AbnormalTerminationException> for CommandException {
    fn from(e: AbnormalTerminationException) -> Self {
        CommandException::AbnormalTermination(e)
    }
}

impl From<BadExitStatusException> for CommandException {
    fn from(e: BadExitStatusException) -> Self {
        CommandException::BadExitStatus(e)
    }
}

/// What `Command.execute` and `FutureCommandResult.get` throw: a [`CommandException`] or an
/// `InterruptedException`.
// PORT: Java declares `throws CommandException, InterruptedException`; Rust needs one error type.
#[derive(Clone, Debug)]
pub enum CommandExecutionError {
    Command(CommandException),
    Interrupted(InterruptedException),
}

impl CommandExecutionError {
    /// The `CommandException`, if this is one.
    pub fn as_command_exception(&self) -> Option<&CommandException> {
        match self {
            CommandExecutionError::Command(e) => Some(e),
            CommandExecutionError::Interrupted(_) => None,
        }
    }
}

impl fmt::Display for CommandExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandExecutionError::Command(e) => e.fmt(f),
            CommandExecutionError::Interrupted(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CommandExecutionError {}

impl From<CommandException> for CommandExecutionError {
    fn from(e: CommandException) -> Self {
        CommandExecutionError::Command(e)
    }
}

impl From<InterruptedException> for CommandExecutionError {
    fn from(e: InterruptedException) -> Self {
        CommandExecutionError::Interrupted(e)
    }
}

impl From<ExecFailedException> for CommandExecutionError {
    fn from(e: ExecFailedException) -> Self {
        CommandExecutionError::Command(CommandException::ExecFailed(e))
    }
}
