//! Port of Bazel's `lib/shell` (running subprocesses) and the command-description parts of
//! `lib/util` (`CommandBuilder`, `CommandFailureUtils`, `CommandUtils`, `ScriptUtil`,
//! `ShellEscaper`, `OS`, `CPU`, `OsUtils`).
#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod command;
pub mod command_exception;
pub mod command_result;
pub mod consumers;
pub mod execution_statistics;
pub mod future_command_result;
pub mod input_stream_sink;
pub mod java_file;
pub mod java_process;
pub mod java_subprocess_factory;
pub mod log_util;
pub mod shell_utils;
pub mod subprocess;
pub mod subprocess_builder;
pub mod termination_status;
pub mod thread_interrupt;
pub mod util;

pub use command::Command;
pub use command_exception::{
    AbnormalTerminationException, BadExitStatusException, CommandException, CommandExecutionError,
    ExecFailedException,
};
pub use command_result::{ByteArrayOutputStream, CommandResult, CommandResultBuilder};
pub use consumers::{Consumers, OutErrConsumers};
pub use execution_statistics::{ExecutionStatistics, ExecutionStatisticsResourceUsage};
pub use future_command_result::FutureCommandResult;
pub use input_stream_sink::{InputStream, InputStreamSink, OutputStream};
pub use java_file::JavaFile;
pub use java_subprocess_factory::JavaSubprocessFactory;
pub use log_util::LogUtil;
pub use shell_utils::{ShellUtils, ShellUtilsTokenizationException};
pub use subprocess::{Subprocess, SubprocessFactory};
pub use subprocess_builder::{SubprocessBuilder, SubprocessBuilderStreamAction};
pub use termination_status::{TerminationStatus, TerminationStatusBuilder};
pub use thread_interrupt::{InterruptedException, ThreadInterrupt};
pub use util::command_builder::CommandBuilder;
pub use util::command_description_form::CommandDescriptionForm;
pub use util::command_failure_utils::CommandFailureUtils;
pub use util::command_utils::CommandUtils;
pub use util::cpu::CPU;
pub use util::describable_execution_unit::DescribableExecutionUnit;
pub use util::os::OS;
pub use util::os_utils::OsUtils;
pub use util::script_util::ScriptUtil;
pub use util::shell_escaper::ShellEscaper;
