//! `exec_result`: the result of `repository_ctx.execute()` / `module_ctx.execute()`, and the
//! builder that runs the command.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java

use std::fmt;
use std::hash::Hash;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use allocative::Allocative;
use blaze_events::OutErr;
use blaze_shell::ByteArrayOutputStream;
use blaze_shell::Command;
use blaze_shell::CommandException;
use blaze_shell::CommandExecutionError;
use blaze_shell::InterruptedException;
use blaze_shell::JavaFile;
use blaze_shell::OutputStream;
use blaze_starlark::structs::DeeplyImmutable;
use blaze_util::string_encoding::latin1_decode;
use indexmap::IndexMap;
use indexmap::IndexSet;
use parking_lot::Mutex;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

/// A structure callable from Starlark that stores the result of `repository_ctx.execute()`. It
/// contains the standard output stream content, the standard error stream content and the
/// execution return code.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:59 StarlarkExecutionResult
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkExecutionResult {
    return_code: i32,
    stdout: String,
    stderr: String,
}

starlark_simple_value!(StarlarkExecutionResult);

impl StarlarkExecutionResult {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:64 StarlarkExecutionResult.<init>
    pub fn new(return_code: i32, stdout: String, stderr: String) -> Self {
        StarlarkExecutionResult {
            return_code,
            stdout,
            stderr,
        }
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:84 StarlarkExecutionResult.getReturnCode
    pub fn get_return_code(&self) -> i32 {
        self.return_code
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:92 StarlarkExecutionResult.getStdout
    pub fn get_stdout(&self) -> &str {
        &self.stdout
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:100 StarlarkExecutionResult.getStderr
    pub fn get_stderr(&self) -> &str {
        &self.stderr
    }

    /// Returns a builder that can be used to execute a command and build an execution result.
    ///
    /// `environment`: the environment variables from the client to be passed to the execution
    /// environment.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:110 StarlarkExecutionResult.builder
    pub fn builder(environment: &IndexMap<String, String>) -> StarlarkExecutionResultBuilder {
        StarlarkExecutionResultBuilder::new(environment)
    }
}

impl fmt::Display for StarlarkExecutionResult {
    // Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr (default)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.bazel.repository.starlark.StarlarkExecutionResult>",
        )
    }
}

#[starlark_value(type = "exec_result")]
impl<'v> StarlarkValue<'v> for StarlarkExecutionResult {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("exec_result", exec_result_methods);
        Some(RES.methods())
    }

    // Java: isImmutable() is true, so the object is hashable (by identity, Object.hashCode).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (self as *const Self as usize).hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:71 StarlarkExecutionResult.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn exec_result_methods(builder: &mut MethodsBuilder) {
    /// The return code returned after the execution of the program. 256 if the process was
    /// terminated by a time out; values larger than 128 indicate termination by a signal.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:84 StarlarkExecutionResult.getReturnCode
    #[starlark(attribute)]
    fn return_code(this: &StarlarkExecutionResult) -> starlark::Result<i32> {
        Ok(this.return_code)
    }

    /// The content of the standard output returned by the execution.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:92 StarlarkExecutionResult.getStdout
    #[starlark(attribute)]
    fn stdout(this: &StarlarkExecutionResult) -> starlark::Result<String> {
        Ok(this.stdout.clone())
    }

    /// The content of the standard error output returned by the execution.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:100 StarlarkExecutionResult.getStderr
    #[starlark(attribute)]
    fn stderr(this: &StarlarkExecutionResult) -> starlark::Result<String> {
        Ok(this.stderr.clone())
    }
}

/// One output stream of Java's `DelegatingOutErr` with the sinks `RecordingOutErr` and, when not
/// quiet, `OutErr.create(System.err, System.err)`.
struct DelegatingStream {
    recorder: Arc<Mutex<Vec<u8>>>,
    to_system_err: bool,
}

impl Write for DelegatingStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.recorder.lock().extend_from_slice(buf);
        if self.to_system_err {
            // PORT: Java writes to System.err, which BlazeCommandDispatcher patches to the
            // command's reporter output for the duration of the command; Blaze writes to the
            // process-wide error stream.
            OutErr::system_out_err().get_error_stream().write(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.to_system_err {
            OutErr::system_out_err().get_error_stream().flush()?;
        }
        Ok(())
    }
}

/// A builder that executes a command and builds a [`StarlarkExecutionResult`].
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:115 StarlarkExecutionResult.Builder
pub struct StarlarkExecutionResultBuilder {
    args: Vec<String>,
    directory: Option<JavaFile>,
    env_builder: IndexMap<String, String>,
    client_env: IndexMap<String, String>,
    timeout: i64,
    executed: bool,
    quiet: bool,
}

impl StarlarkExecutionResultBuilder {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:125 Builder.<init>
    fn new(environment: &IndexMap<String, String>) -> Self {
        StarlarkExecutionResultBuilder {
            args: Vec::new(),
            directory: None,
            env_builder: environment.clone(),
            client_env: environment.clone(),
            timeout: -1,
            executed: false,
            quiet: false,
        }
    }

    /// Adds arguments to the list of arguments to pass to the command. The first argument is
    /// expected to be the binary to execute.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:136 Builder.addArguments
    pub fn add_arguments(mut self, args: Vec<String>) -> Self {
        self.args.extend(args);
        self
    }

    /// Sets the directory to execute the process in. Must be called before `execute`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:146 Builder.setDirectory
    pub fn set_directory(mut self, path: JavaFile) -> Self {
        self.directory = Some(path);
        self
    }

    /// For each key `k` of `variables`, the process will have `k=variables[k]` defined.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:157 Builder.addEnvironmentVariables
    pub fn add_environment_variables(mut self, variables: &IndexMap<String, String>) -> Self {
        for (k, v) in variables {
            self.env_builder.insert(k.clone(), v.clone());
        }
        self
    }

    /// Ensures that the given environment variables are not passed to the process.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:164 Builder.removeEnvironmentVariables
    pub fn remove_environment_variables(mut self, remove_env_variables: &IndexSet<String>) -> Self {
        for k in remove_env_variables {
            // Java LinkedHashMap.remove keeps the order of the remaining entries.
            self.env_builder.shift_remove(k);
        }
        self
    }

    /// Sets the timeout, in milliseconds, after which the command will be terminated.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:171 Builder.setTimeout
    pub fn set_timeout(mut self, timeout: i64) -> Self {
        blaze_util::check_argument!(timeout > 0, "Timeout must be a positive number.");
        self.timeout = timeout;
        self
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:178 Builder.setQuiet
    pub fn set_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:183 Builder.toString
    fn to_string(stream: &ByteArrayOutputStream) -> String {
        // Java: the streaming consumers' result streams throw IllegalStateException.
        if stream.is_no_output_collected() {
            return String::new();
        }
        // Java: stream.toString(ISO_8859_1)
        latin1_decode(&stream.to_byte_array()).into_owned()
    }

    /// Executes the command given by `add_arguments`.
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkExecutionResult.java:192 Builder.execute
    pub fn execute(mut self) -> Result<StarlarkExecutionResult, InterruptedException> {
        blaze_util::check_argument!(
            self.timeout > 0,
            "Timeout must be set prior to calling execute()."
        );
        blaze_util::check_argument!(!self.args.is_empty(), "No command specified.");
        blaze_util::check_state!(
            !self.executed,
            "Command was already executed, cannot re-use builder."
        );
        let directory = self
            .directory
            .clone()
            .expect("Directory must be set before calling execute().");
        self.executed = true;

        // TODO(dmarting): if a lot of data is sent to stdout, this will use all the memory and
        // Bazel will crash. Maybe we should use custom output streams that throw an appropriate
        // exception when reaching a specific size.
        let recorded_out = Arc::new(Mutex::new(Vec::new()));
        let recorded_err = Arc::new(Mutex::new(Vec::new()));
        let out: OutputStream = Arc::new(Mutex::new(DelegatingStream {
            recorder: recorded_out.clone(),
            to_system_err: !self.quiet,
        }));
        let err: OutputStream = Arc::new(Mutex::new(DelegatingStream {
            recorder: recorded_err.clone(),
            to_system_err: !self.quiet,
        }));
        // Java: RecordingOutErr.outAsLatin1() / errAsLatin1()
        let out_as_latin1 = || latin1_decode(&recorded_out.lock()).into_owned();
        let err_as_latin1 = || latin1_decode(&recorded_err.lock()).into_owned();

        let command = Command::with_timeout(
            self.args.clone(),
            Some(self.env_builder.clone()),
            Some(directory),
            Duration::from_millis(self.timeout as u64),
            &self.client_env,
        );
        match command.execute_streaming(out, err) {
            Ok(result) => Ok(StarlarkExecutionResult::new(
                result.termination_status().get_exit_code(),
                out_as_latin1(),
                err_as_latin1(),
            )),
            Err(CommandExecutionError::Interrupted(e)) => Err(e),
            Err(CommandExecutionError::Command(CommandException::BadExitStatus(e))) => {
                Ok(StarlarkExecutionResult::new(
                    e.get_result().termination_status().get_exit_code(),
                    out_as_latin1(),
                    err_as_latin1(),
                ))
            }
            Err(CommandExecutionError::Command(CommandException::AbnormalTermination(e))) => {
                let status = e.get_result().termination_status();
                if status.timed_out() {
                    // Signal a timeout by an exit code outside the normal range
                    Ok(StarlarkExecutionResult::new(
                        256,
                        String::new(),
                        e.get_message().to_owned(),
                    ))
                } else if status.exited() {
                    Ok(StarlarkExecutionResult::new(
                        status.get_exit_code(),
                        Self::to_string(e.get_result().stdout_stream()),
                        Self::to_string(e.get_result().stderr_stream()),
                    ))
                } else if status.get_terminating_signal() == 15 {
                    // We have a bit of a problem here: we cannot distingusih between the case
                    // where the SIGTERM was sent by something that the calling rule wants to
                    // legitimately handle, and the case where it was sent by bazel to abort the
                    // build, e.g., because something else failed.
                    //
                    // We just assume the latter to correctly handle aborts, accepting that rule
                    // authors have to write their rules without relying on the ability to handle
                    // termination by signal 15.
                    Err(InterruptedException)
                } else {
                    Ok(StarlarkExecutionResult::new(
                        status.get_raw_exit_code(),
                        Self::to_string(e.get_result().stdout_stream()),
                        Self::to_string(e.get_result().stderr_stream()),
                    ))
                }
            }
            Err(CommandExecutionError::Command(e)) => {
                // 256 is outside of the standard range for exit code on Unixes. We are not
                // guaranteed that on all system it would be outside of the standard range.
                Ok(StarlarkExecutionResult::new(
                    256,
                    String::new(),
                    e.get_message().to_owned(),
                ))
            }
        }
    }
}
