//! Port of `com.google.devtools.build.lib.shell.Command`.

use std::fmt::Write as _;
use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;

use crate::command_exception::{CommandExecutionError, ExecFailedException};
use crate::consumers::{Consumers, OutErrConsumers};
use crate::future_command_result::FutureCommandResult;
use crate::input_stream_sink::OutputStream;
use crate::java_file::JavaFile;
use crate::subprocess::Subprocess;
use crate::subprocess_builder::SubprocessBuilder;
use crate::util::describable_execution_unit::DescribableExecutionUnit;

/// An executable command, including its arguments and runtime environment (environment
/// variables, working directory). It lets a caller execute a command, get its results, and
/// optionally forward interrupts to the subprocess. This class creates threads to ensure timely
/// reading of subprocess outputs.
///
/// This class is immutable and thread-safe. No shell is executed: in terms of the way its
/// arguments are interpreted, this class is closer to `execve(2)` than to `system(3)`.
// Java: com/google/devtools/build/lib/shell/Command.java:126 Command
#[derive(Clone, Debug)]
pub struct Command {
    subprocess_builder: Arc<SubprocessBuilder>,
}

/// Formats a map like Java's `AbstractMap.toString()`: `{k1=v1, k2=v2}`.
fn java_map_to_string(map: Option<&IndexMap<String, String>>) -> String {
    match map {
        None => "null".to_string(),
        Some(map) => {
            let mut s = String::from("{");
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                let _ = write!(s, "{k}={v}");
            }
            s.push('}');
            s
        }
    }
}

impl Command {
    /// Pass this value to `execute_async_with_input` to indicate that no input should be written
    /// to stdin (Java `Command.NO_INPUT`).
    pub fn no_input() -> io::Empty {
        io::empty()
    }

    pub const KILL_SUBPROCESS_ON_INTERRUPT: bool = true;
    pub const CONTINUE_SUBPROCESS_ON_INTERRUPT: bool = false;

    /// Creates a new command for the given command line. The environment is inherited from the
    /// client (`client_env`), as is the working directory. No timeout is enforced.
    // Java: com/google/devtools/build/lib/shell/Command.java:148 Command.<init>
    pub fn new(command_line_elements: Vec<String>, client_env: &IndexMap<String, String>) -> Self {
        Command::with_timeout(
            command_line_elements,
            None,
            None,
            Duration::ZERO,
            client_env,
        )
    }

    /// Just like [`Self::with_timeout`], but without a timeout.
    // Java: com/google/devtools/build/lib/shell/Command.java:153 Command.<init>
    pub fn with_env(
        command_line_elements: Vec<String>,
        environment_variables: Option<IndexMap<String, String>>,
        working_directory: Option<JavaFile>,
        client_env: &IndexMap<String, String>,
    ) -> Self {
        Command::with_timeout(
            command_line_elements,
            environment_variables,
            working_directory,
            Duration::ZERO,
            client_env,
        )
    }

    /// Creates a new command for the given command line elements. The command line is executed
    /// without a shell.
    ///
    /// The 0-th element (the executable) is treated specially: an absolute path is used as is; a
    /// single file name is looked up in PATH; any other relative path is resolved against
    /// `working_directory`.
    ///
    /// `timeout`: a zero value is treated as no timeout.
    // Java: com/google/devtools/build/lib/shell/Command.java:188 Command.<init>
    pub fn with_timeout(
        mut command_line_elements: Vec<String>,
        environment_variables: Option<IndexMap<String, String>>,
        working_directory: Option<JavaFile>,
        timeout: Duration,
        client_env: &IndexMap<String, String>,
    ) -> Self {
        blaze_util::check_argument!(
            !command_line_elements.is_empty(),
            "cannot run an empty command line"
        );

        let executable = JavaFile::new(&command_line_elements[0]);
        if !executable.is_absolute() && executable.get_parent().is_some() {
            command_line_elements[0] =
                JavaFile::new_child(working_directory.as_ref(), &command_line_elements[0])
                    .get_absolute_path();
        }

        let mut subprocess_builder = SubprocessBuilder::new(client_env);
        subprocess_builder.set_argv(command_line_elements);
        subprocess_builder.set_env(environment_variables);
        subprocess_builder.set_working_directory(working_directory);
        // PORT: java.time.Duration.toMillis truncates; a std Duration cannot be negative.
        subprocess_builder.set_timeout_millis(timeout.as_millis().min(i64::MAX as u128) as i64);
        Command {
            subprocess_builder: Arc::new(subprocess_builder),
        }
    }

    /// Returns the raw command line elements to be executed.
    // Java: com/google/devtools/build/lib/shell/Command.java:215 Command.getArguments
    pub fn get_arguments(&self) -> &[String] {
        self.subprocess_builder.get_argv()
    }

    /// Returns the command's environment variables, or `None` (inherit the client's).
    // Java: com/google/devtools/build/lib/shell/Command.java:222 Command.getEnvironment
    pub fn get_environment(&self) -> Option<&IndexMap<String, String>> {
        self.subprocess_builder.get_env()
    }

    /// Returns the working directory to be used for execution, or `None`.
    // Java: com/google/devtools/build/lib/shell/Command.java:228 Command.getWorkingDirectory
    pub fn get_working_directory(&self) -> Option<&JavaFile> {
        self.subprocess_builder.get_working_directory()
    }

    /// Execute this command with no input to stdin, and with the output captured in memory. If
    /// the current thread is interrupted, then the subprocess is also interrupted. This call
    /// blocks until the subprocess completes or an error occurs.
    // Java: com/google/devtools/build/lib/shell/Command.java:245 Command.execute
    pub fn execute(&self) -> Result<crate::CommandResult, CommandExecutionError> {
        self.execute_async()?.get()
    }

    /// Execute this command with no input to stdin, and with the output streamed to the given
    /// output streams. The streams are never closed by this class.
    // Java: com/google/devtools/build/lib/shell/Command.java:264 Command.execute
    pub fn execute_streaming(
        &self,
        std_out: OutputStream,
        std_err: OutputStream,
    ) -> Result<crate::CommandResult, CommandExecutionError> {
        self.do_execute(
            &mut io::empty(),
            Consumers::create_streaming_consumers(std_out, std_err),
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )?
        .get()
    }

    /// Execute this command with no input to stdin, and with the output captured in memory. This
    /// call blocks until the subprocess is started, but does not wait for it to exit.
    // Java: com/google/devtools/build/lib/shell/Command.java:285 Command.executeAsync
    pub fn execute_async(&self) -> Result<FutureCommandResult, ExecFailedException> {
        self.do_execute(
            &mut io::empty(),
            Consumers::create_accumulating_consumers(),
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
    }

    /// Like [`Self::execute_async`], with the output streamed to the given streams.
    // Java: com/google/devtools/build/lib/shell/Command.java:304 Command.executeAsync
    pub fn execute_async_streaming(
        &self,
        std_out: OutputStream,
        std_err: OutputStream,
    ) -> Result<FutureCommandResult, ExecFailedException> {
        self.do_execute(
            &mut io::empty(),
            Consumers::create_streaming_consumers(std_out, std_err),
            Command::KILL_SUBPROCESS_ON_INTERRUPT,
        )
    }

    /// Execute this command with `stdin_input` written to its stdin and the output captured in
    /// memory. Blocks until the input is written, but does not wait for the subprocess to exit.
    // Java: com/google/devtools/build/lib/shell/Command.java:325 Command.executeAsync
    pub fn execute_async_with_input(
        &self,
        stdin_input: &mut dyn Read,
        kill_subprocess_on_interrupt: bool,
    ) -> Result<FutureCommandResult, ExecFailedException> {
        self.do_execute(
            stdin_input,
            Consumers::create_accumulating_consumers(),
            kill_subprocess_on_interrupt,
        )
    }

    /// Like [`Self::execute_async_with_input`], with the output streamed to the given streams.
    // Java: com/google/devtools/build/lib/shell/Command.java:346 Command.executeAsync
    pub fn execute_async_with_input_streaming(
        &self,
        stdin_input: &mut dyn Read,
        std_out: OutputStream,
        std_err: OutputStream,
        kill_subprocess_on_interrupt: bool,
    ) -> Result<FutureCommandResult, ExecFailedException> {
        self.do_execute(
            stdin_input,
            Consumers::create_streaming_consumers(std_out, std_err),
            kill_subprocess_on_interrupt,
        )
    }

    /// A string representation of this command object which includes the arguments, the
    /// environment, and the working directory.
    // Java: com/google/devtools/build/lib/shell/Command.java:361 Command.toDebugString
    pub fn to_debug_string(&self) -> String {
        let mut message = String::with_capacity(128);
        message.push_str("Executing (without brackets):");
        for arg in self.subprocess_builder.get_argv() {
            message.push_str(" [");
            message.push_str(arg);
            message.push(']');
        }
        message.push_str("; environment: ");
        message.push_str(&java_map_to_string(self.subprocess_builder.get_env()));
        message.push_str("; working dir: ");
        match self.subprocess_builder.get_working_directory() {
            None => message.push_str("(current)"),
            Some(wd) => message.push_str(wd.get_path()),
        }
        message
    }

    // Java: com/google/devtools/build/lib/shell/Command.java:377 Command.doExecute
    fn do_execute(
        &self,
        stdin_input: &mut dyn Read,
        mut out_err_consumers: OutErrConsumers,
        kill_subprocess_on_interrupt: bool,
    ) -> Result<FutureCommandResult, ExecFailedException> {
        self.log_command();

        let process = self.start_process()?;

        out_err_consumers.log_consumption_strategy();
        out_err_consumers.register_inputs(
            process
                .get_input_stream()
                .expect("NullPointerException: in"),
            process
                .get_error_stream()
                .expect("NullPointerException: in"),
            /* close_streams= */ false,
        );

        // TODO(ulfjack): This call blocks until all input is written. If stdinInput is large (or
        // unbounded), then the async calls can block for a long time, and the timeout is not
        // properly enforced.
        process_input(stdin_input, &*process);

        Ok(FutureCommandResult::new(
            self.clone(),
            process,
            out_err_consumers,
            kill_subprocess_on_interrupt,
        ))
    }

    // Java: com/google/devtools/build/lib/shell/Command.java:397 Command.startProcess
    fn start_process(&self) -> Result<Box<dyn Subprocess>, ExecFailedException> {
        self.subprocess_builder
            .start()
            .map_err(|ioe| ExecFailedException::new(self.clone(), ioe))
    }

    // Java: com/google/devtools/build/lib/shell/Command.java:433 Command.logCommand
    fn log_command(&self) {
        // PORT: skipped, FINE-level java.log logging.
    }
}

// Java: com/google/devtools/build/lib/shell/Command.java:417 Command.processInput
fn process_input(stdin_input: &mut dyn Read, process: &dyn Subprocess) {
    if let Some(mut out) = process.get_output_stream() {
        // Note: an error is not an error! Perhaps the command just isn't hungry for our input and
        // exited with success. Process.waitFor (later) will tell us.
        let _ = io::copy(stdin_input, &mut out);
        // `out` is dropped (closed) here.
    }
}

impl DescribableExecutionUnit for Command {
    fn get_arguments(&self) -> Vec<String> {
        Command::get_arguments(self).to_vec()
    }

    fn get_environment(&self) -> Option<IndexMap<String, String>> {
        Command::get_environment(self).cloned()
    }

    // Java: com/google/devtools/build/lib/shell/Command.java:438 Command.getMnemonic
    fn get_mnemonic(&self) -> String {
        "<shell command>".to_string()
    }
}
