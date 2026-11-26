//! `BlazeModule` (with `OptionsSupplier`) and `ServerBuilder`.

use std::sync::Arc;

use blaze_options::definition::OptionsClassRef;
use indexmap::IndexMap;

use crate::command::BlazeCommand;
use crate::command_environment::CommandEnvironment;
use crate::exit_code::DetailedExitCode;
use crate::info_item::InfoItem;
use crate::options_result::OptionsParsingResult;

/// An `AbruptExitException`: a message and the exit code to exit with.
// Java: com/google/devtools/build/lib/util/AbruptExitException.java:28 AbruptExitException
#[derive(Clone, Debug, PartialEq)]
pub struct AbruptExitException {
    pub message: String,
    pub detailed_exit_code: DetailedExitCode,
}

impl AbruptExitException {
    pub fn new(detailed_exit_code: DetailedExitCode) -> Self {
        let message = detailed_exit_code
            .get_failure_detail()
            .map(|f| f.message.clone())
            .unwrap_or_default();
        AbruptExitException {
            message,
            detailed_exit_code,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

/// A module Blaze can load at the beginning of its execution. Modules supply startup and command
/// options, commands, info items and hooks around commands.
///
/// PORT: the hooks a server runs around commands are present (`globalInit`, `blazeStartup`,
/// `serverInit`, `workspaceInit`, `beforeCommand`, `getOutputListener`, `afterCommand`,
/// `commandComplete`, `blazeShutdown`, `blazeShutdownOnCrash`). The others (file systems, rule
/// classes, `executorInit`, action context and strategy registration, package settings, ...) run
/// where the command that needs them builds its executor (see `crate::bazel_modules`); they are
/// added here as callers outside the command appear.
// Java: com/google/devtools/build/lib/runtime/BlazeModule.java:69 BlazeModule
pub trait BlazeModule: Send + Sync {
    /// The Java class name (used in `toString()` and logs).
    fn name(&self) -> &'static str;

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:72 BlazeModule.getStartupOptions
    fn get_startup_options(&self) -> Vec<OptionsClassRef> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:243 BlazeModule.getCommandOptions
    fn get_command_options(&self, _command_name: &str) -> Vec<OptionsClassRef> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:248 BlazeModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        Vec::new()
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:83 BlazeModule.globalInit
    fn global_init(
        &self,
        _startup_options: &OptionsParsingResult,
    ) -> Result<(), AbruptExitException> {
        Ok(())
    }

    /// Called when Bazel starts up after `get_startup_options`, `global_init`, and
    /// `getFileSystem`.
    ///
    /// PORT: Java also passes `BlazeVersionInfo.instance()` (here
    /// `crate::version_info::BlazeVersionInfo::instance()`), the server's random instance id
    /// (`instance_id`) and the runtime's clock (always `BlazeClock`, here
    /// `blaze_events::JavaClock`).
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:165 BlazeModule.blazeStartup
    fn blaze_startup(
        &self,
        _startup_options: &OptionsParsingResult,
        _instance_id: &str,
        _file_system: &blaze_vfs::FileSystemRef,
        _directories: &crate::directories::ServerDirectories,
    ) -> Result<(), AbruptExitException> {
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:183 BlazeModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        _builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        Ok(())
    }

    /// Called when Bazel initializes a new workspace; this is only called after `server_init`,
    /// and only if the server initialization was successful.
    ///
    /// PORT: Java also passes the runtime; the `WorkspaceBuilder` holds what Blaze's workspace
    /// takes from modules (the diff awareness factories); the server's Skyframe executor is
    /// created per command.
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:207 BlazeModule.workspaceInit
    fn workspace_init(
        &self,
        _directories: &crate::directories::BlazeDirectories,
        _builder: &mut crate::workspace::WorkspaceBuilder,
    ) {
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:218 BlazeModule.beforeCommand
    fn before_command(&self, _env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        Ok(())
    }

    /// Returns additional listeners to the console output stream. Called at the beginning of
    /// each command (after `before_command`).
    ///
    /// PORT: Java's modules keep the `CommandEnvironment` of `beforeCommand` in a field; the
    /// Rust hooks take `&self`, so the environment is passed.
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:225 BlazeModule.getOutputListener
    fn get_output_listener(&self, _env: &CommandEnvironment) -> Option<blaze_events::OutErr> {
        None
    }

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:338 BlazeModule.afterCommand
    fn after_command(&self) -> Result<(), AbruptExitException> {
        Ok(())
    }

    /// Called after `after_command`. This method can be used to close and cleanup resources
    /// specific to the command.
    ///
    /// This method must not throw any exceptions, report any errors or generate any
    /// stdout/stderr.
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:348 BlazeModule.commandComplete
    fn command_complete(&self) {}

    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:360 BlazeModule.blazeShutdown
    fn blaze_shutdown(&self) {}

    /// Called when Blaze shuts down due to a crash.
    ///
    /// Modules may use this to flush pending state, but they must be careful to only do a
    /// minimal number of things. Keep in mind that we are crashing so who knows what state we
    /// are in. Modules rarely need to implement this.
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:369 BlazeModule.blazeShutdownOnCrash
    fn blaze_shutdown_on_crash(&self, _exit_code: &DetailedExitCode) {}
}

/// Builder class to create a [`crate::runtime::BlazeRuntime`] instance: the parts modules
/// contribute in `serverInit`.
/// PORT: invocation policy, query functions/output formatters, BEP uploaders and
/// instrumentation outputs are not ported yet.
// Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:30 ServerBuilder
#[derive(Default)]
pub struct ServerBuilder {
    commands: Vec<Arc<dyn BlazeCommand>>,
    info_items: IndexMap<String, Arc<dyn InfoItem>>,
}

impl ServerBuilder {
    pub fn new() -> Self {
        ServerBuilder::default()
    }

    // Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:70 ServerBuilder.getCommands
    pub fn get_commands(&self) -> &[Arc<dyn BlazeCommand>] {
        &self.commands
    }

    // Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:57 ServerBuilder.getInfoItems
    pub fn get_info_items(&self) -> &IndexMap<String, Arc<dyn InfoItem>> {
        &self.info_items
    }

    /// Adds the given command to the server. This method may be called multiple times.
    // Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:116 ServerBuilder.addCommands
    pub fn add_commands(&mut self, commands: impl IntoIterator<Item = Arc<dyn BlazeCommand>>) {
        self.commands.extend(commands);
    }

    /// Adds the given info items. `ImmutableMap.Builder.put` with a duplicate key fails at
    /// build time; Blaze checks at insertion.
    // Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:134 ServerBuilder.addInfoItems
    pub fn add_info_items(&mut self, items: impl IntoIterator<Item = Arc<dyn InfoItem>>) {
        for item in items {
            let name = item.get_name().to_string();
            if self.info_items.insert(name.clone(), item).is_some() {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    name
                );
            }
        }
    }
}
