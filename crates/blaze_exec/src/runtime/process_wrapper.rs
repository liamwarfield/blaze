//! Port of `lib/runtime/ProcessWrapper.java`: the command line of the `process-wrapper` tool.

use blaze_util::java_duration::JavaDuration;

use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::ActionInputRef;
use crate::actions::execution_requirements;
use crate::exec::bin_tools::BinTools;

/// Tracks the `process-wrapper` tool and builds command lines that use it.
// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:36 ProcessWrapper
#[derive(Debug, Clone)]
pub struct ProcessWrapper {
    bin_path: PathFragment,
    action_input: ActionInputRef,
    kill_delay: Option<JavaDuration>,
    graceful_sigterm: bool,
}

impl ProcessWrapper {
    /// `BIN_BASENAME`.
    pub const BIN_BASENAME: &'static str = "process-wrapper";

    /// `new ProcessWrapper(binPath, actionInput, killDelay, gracefulSigterm)`.
    pub fn new(
        bin_path: PathFragment,
        action_input: ActionInputRef,
        kill_delay: Option<JavaDuration>,
        graceful_sigterm: bool,
    ) -> ProcessWrapper {
        ProcessWrapper {
            bin_path,
            action_input,
            kill_delay,
            graceful_sigterm,
        }
    }

    /// `fromCommandEnvironment(cmdEnv)`: `None` if the tool is not embedded. `kill_delay` is
    /// `LocalExecutionOptions.getLocalSigkillGraceSeconds()`, `graceful_sigterm`
    /// `processWrapperGracefulSigterm`.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:66 ProcessWrapper.fromCommandEnvironment
    pub fn from_bin_tools(
        bin_tools: &BinTools,
        kill_delay: Option<JavaDuration>,
        graceful_sigterm: bool,
    ) -> Option<ProcessWrapper> {
        let action_input = bin_tools.get_action_input(ProcessWrapper::BIN_BASENAME)?;
        Some(ProcessWrapper::new(
            bin_tools
                .get_embedded_path(ProcessWrapper::BIN_BASENAME)?
                .as_fragment()
                .clone(),
            action_input,
            kill_delay,
            graceful_sigterm,
        ))
    }

    /// `commandLineBuilder(commandArguments)`.
    pub fn command_line_builder(&self, command_arguments: Vec<String>) -> CommandLineBuilder {
        CommandLineBuilder {
            process_wrapper_path: self.bin_path.clone(),
            command_arguments,
            kill_delay: self.kill_delay,
            graceful_sigterm: self.graceful_sigterm,
            stdout_path: None,
            stderr_path: None,
            timeout: None,
            statistics_path: None,
        }
    }

    /// `asActionInput()`.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:97 ProcessWrapper.asActionInput
    pub fn as_action_input(&self) -> &ActionInputRef {
        &self.action_input
    }
}

/// `ProcessWrapper.CommandLineBuilder`.
// Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:93 ProcessWrapper.CommandLineBuilder
#[derive(Debug, Clone)]
pub struct CommandLineBuilder {
    process_wrapper_path: PathFragment,
    command_arguments: Vec<String>,
    kill_delay: Option<JavaDuration>,
    graceful_sigterm: bool,
    stdout_path: Option<PathFragment>,
    stderr_path: Option<PathFragment>,
    timeout: Option<JavaDuration>,
    statistics_path: Option<PathFragment>,
}

impl CommandLineBuilder {
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:133 ProcessWrapper.CommandLineBuilder.overrideProcessWrapperPath
    pub fn override_process_wrapper_path(&mut self, p: PathFragment) -> &mut Self {
        self.process_wrapper_path = p;
        self
    }
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:140 ProcessWrapper.CommandLineBuilder.setStdoutPath
    pub fn set_stdout_path(&mut self, p: PathFragment) -> &mut Self {
        self.stdout_path = Some(p);
        self
    }
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:147 ProcessWrapper.CommandLineBuilder.setStderrPath
    pub fn set_stderr_path(&mut self, p: PathFragment) -> &mut Self {
        self.stderr_path = Some(p);
        self
    }
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:155 ProcessWrapper.CommandLineBuilder.setTimeout
    pub fn set_timeout(&mut self, timeout: impl Into<JavaDuration>) -> &mut Self {
        self.timeout = Some(timeout.into());
        self
    }
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:161 ProcessWrapper.CommandLineBuilder.setStatisticsPath
    pub fn set_statistics_path(&mut self, p: PathFragment) -> &mut Self {
        self.statistics_path = Some(p);
        self
    }
    /// `addExecutionInfo(executionInfo)`.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:168 ProcessWrapper.CommandLineBuilder.addExecutionInfo
    pub fn add_execution_info(&mut self, execution_info: &IndexMap<String, String>) -> &mut Self {
        if execution_info.contains_key(execution_requirements::GRACEFUL_TERMINATION) {
            self.graceful_sigterm = true;
        }
        self
    }

    /// `build()`.
    // Java: com/google/devtools/build/lib/runtime/ProcessWrapper.java:165 ProcessWrapper.CommandLineBuilder.build
    pub fn build(&self) -> Vec<String> {
        let mut full = vec![self.process_wrapper_path.get_path_string().to_string()];
        if let Some(t) = self.timeout {
            // Duration.toSeconds(): signed.
            full.push(format!("--timeout={}", t.to_seconds()));
        }
        if let Some(k) = self.kill_delay {
            full.push(format!("--kill_delay={}", k.to_seconds()));
        }
        if let Some(p) = &self.stdout_path {
            full.push(format!("--stdout={}", p.get_path_string()));
        }
        if let Some(p) = &self.stderr_path {
            full.push(format!("--stderr={}", p.get_path_string()));
        }
        if let Some(p) = &self.statistics_path {
            full.push(format!("--stats={}", p.get_path_string()));
        }
        if self.graceful_sigterm {
            full.push("--graceful_sigterm".to_string());
        }
        full.extend(self.command_arguments.iter().cloned());
        full
    }
}
