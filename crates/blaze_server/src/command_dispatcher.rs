//! `CommandDispatcher` (from `lib/runtime`) and `CommandExtensionReporter` (from `lib/util/io`).
//!
//! PORT: Java's `com.google.devtools.build.lib.runtime.CommandDispatcher` lives in this crate
//! (decision D3: `blaze_server` sits below `blaze_runtime`); `blaze_runtime` implements it on
//! `BlazeCommandDispatcher` and re-exports `LockingMode` and `UiVerbosity`.

use std::sync::Arc;

use blaze_events::io::OutErr;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;
use blaze_protos::google::protobuf::Any;
use blaze_shell::thread_interrupt::InterruptedException;

use crate::blaze_command_result::BlazeCommandResult;
use crate::idle_task::IdleTaskResult;
use crate::terminal_size_monitor::TerminalSizeMonitor;

/// What to do if the command lock is not available.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:35 CommandDispatcher.LockingMode
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockingMode {
    /// Wait until it is available.
    WAIT,
    /// Return with an error.
    ERROR_OUT,
}

/// How much output to emit on the console.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:41 CommandDispatcher.UiVerbosity
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiVerbosity {
    /// Only errors.
    QUIET,
    /// Everything.
    NORMAL,
}

/// Reports command extensions back to the client while a command runs.
///
/// PORT: belongs to `lib/util/io`; lives here until that package has a crate.
// Java: com/google/devtools/build/lib/util/io/CommandExtensionReporter.java:23 CommandExtensionReporter
pub trait CommandExtensionReporter: Send + Sync {
    fn report(&self, command_extension: Any);
}

/// `CommandExtensionReporter.NO_OP_COMMAND_EXTENSION_REPORTER`.
// Java: com/google/devtools/build/lib/util/io/CommandExtensionReporter.java:26 CommandExtensionReporter.NO_OP_COMMAND_EXTENSION_REPORTER
pub struct NoOpCommandExtensionReporter;

impl CommandExtensionReporter for NoOpCommandExtensionReporter {
    fn report(&self, _command_extension: Any) {}
}

/// `Supplier<ImmutableList<IdleTask.Result>>`: the idle task results of the previous idle
/// period, or `None` (Java `null`) if there are none or they were taken already.
pub type IdleTaskResultsSupplier = Arc<dyn Fn() -> Option<Vec<IdleTaskResult>> + Send + Sync>;

/// Dispatches to the commands; that is, given a command line, this abstraction looks up the
/// appropriate command object, parses the options required by the object, and calls its exec
/// method.
///
/// PORT: Java's `exec` overloads are [`CommandDispatcher::exec`] (without a terminal size
/// monitor) and [`CommandDispatcher::exec_with_terminal_size_monitor`] (the one
/// `GrpcServerImpl` calls; it defaults to `exec`). `InterruptedException` is the `Err` value.
/// `startupOptionsTaggedWithBazelRc` pairs are `(source, option)`, as Java's `Pair`s.
// Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:32 CommandDispatcher
pub trait CommandDispatcher: Send + Sync {
    /// Executes a single command. Returns a [`BlazeCommandResult`] to indicate either an exit
    /// code, the desire to shut down the server, or that a given binary should be executed by
    /// the client.
    // Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:51 CommandDispatcher.exec
    fn exec(
        &self,
        invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: String,
        first_contact_time_millis: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        idle_task_results_supplier: IdleTaskResultsSupplier,
        command_extensions: Vec<Any>,
        command_extension_reporter: Arc<dyn CommandExtensionReporter>,
    ) -> Result<BlazeCommandResult, InterruptedException>;

    /// Executes a single command with a monitor carrying terminal size updates from the client.
    ///
    /// The default implementation ignores terminal size updates so tests and embedders that do
    /// not render Bazel's terminal UI do not need to implement this overload.
    // Java: com/google/devtools/build/lib/runtime/CommandDispatcher.java:71 CommandDispatcher.exec
    fn exec_with_terminal_size_monitor(
        &self,
        invocation_policy: InvocationPolicy,
        args: Vec<String>,
        out_err: OutErr,
        locking_mode: LockingMode,
        ui_verbosity: UiVerbosity,
        client_description: String,
        first_contact_time_millis: i64,
        startup_options_tagged_with_bazel_rc: Option<Vec<(String, String)>>,
        idle_task_results_supplier: IdleTaskResultsSupplier,
        command_extensions: Vec<Any>,
        command_extension_reporter: Arc<dyn CommandExtensionReporter>,
        _terminal_size_monitor: Arc<TerminalSizeMonitor>,
    ) -> Result<BlazeCommandResult, InterruptedException> {
        self.exec(
            invocation_policy,
            args,
            out_err,
            locking_mode,
            ui_verbosity,
            client_description,
            first_contact_time_millis,
            startup_options_tagged_with_bazel_rc,
            idle_task_results_supplier,
            command_extensions,
            command_extension_reporter,
        )
    }
}
