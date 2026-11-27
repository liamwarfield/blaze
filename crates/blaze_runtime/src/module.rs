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
// The hooks of Java's `BlazeModule` that are not methods of this trait. A hook that no module in
// Bazel's module list overrides keeps Java's default (null, empty or no-op), so it has nothing to
// port:
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getEventBusAndAsyncExceptionHandler
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getPackageSettings
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getPackageValidator
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getPackageOverheadEstimator
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getSlowThreadInterruptMessageSuffix
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.createProjectFileProvider
// PORT: skipped, default only (no Bazel module overrides it): BlazeModule.getFileFromWorkspace
// The hooks that Bazel's modules override are dispatched statically: Blaze runs the overriding
// module's code where Java calls the hook (file system: `BlazeRuntime::new_runtime`
// (BazelFileSystemModule, RemoteModule); rule classes: `BazelRuleClassProvider`; precomputed
// values: the repository setup of each executor; executor and strategy registration, analysis
// callbacks and coverage: `commands/build.rs`, `commands/build_remote.rs`, `bazel_modules.rs`;
// query helpers: `commands/query.rs`; the module name is `BlazeModule::name`).
// PORT: skipped, dispatched statically: BlazeModule.getFileSystem, BlazeModule.fileSystem
// PORT: skipped, dispatched statically: BlazeModule.virtualSourceRoot, BlazeModule.create
// PORT: skipped, dispatched statically: BlazeModule.createWithVirtualization
// PORT: skipped, dispatched statically: BlazeModule.getFileSystemForBuildArtifacts
// PORT: skipped, dispatched statically: BlazeModule.initializeRuleClasses, BlazeModule.getPrecomputedValues
// PORT: skipped, dispatched statically: BlazeModule.executorInit, BlazeModule.registerActionContexts
// PORT: skipped, dispatched statically: BlazeModule.registerSpawnStrategies
// PORT: skipped, dispatched statically: BlazeModule.afterAnalysis, BlazeModule.afterTopLevelTargetAnalysis
// PORT: skipped, dispatched statically: BlazeModule.coverageArtifactsKnown, BlazeModule.getCoverageReportFactory
// PORT: skipped, dispatched statically: BlazeModule.postsBuildMetricsEvent, BlazeModule.getQueryRuntimeHelperFactory
// PORT: skipped, dispatched statically: BlazeModule.toString
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

    /// Returns a `PackageLoadingListener` for observing successful package loading, or `None`.
    /// Called once during server startup some time after `server_init`.
    ///
    /// PORT: the `PackageSettings`, rule class provider and file system parameters are not
    /// passed (the only implementation, `PackageMetricsModule`, does not read them).
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:439 BlazeModule.getPackageLoadingListener
    fn get_package_loading_listener(
        &self,
    ) -> Option<
        std::sync::Arc<
            dyn blaze_packages::package::package_loading_listener::PackageLoadingListener,
        >,
    > {
        None
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

    /// Registers any spawn strategies this module provides with the execution phase. Invoked
    /// before actions are executed, after Bazel's built-in modules registered theirs.
    ///
    /// PORT: Java passes the `CommandEnvironment`; the registry is built by the execution tool
    /// (blaze_buildtool), which hands the command's execution environment
    /// (`ExecCommandEnvironment`: its options, exec root and reporter). The exception is the
    /// registry's `AbruptExitException` (Java has one class). Bazel's own modules register
    /// theirs in blaze_exec's `executor_setup`, not through this method.
    // Java: com/google/devtools/build/lib/runtime/BlazeModule.java:329 BlazeModule.registerSpawnStrategies
    fn register_spawn_strategies(
        &self,
        _registry_builder: &mut blaze_exec::exec::spawn_strategy_registry::SpawnStrategyRegistryBuilder,
        _env: &blaze_exec::exec::exec_command_environment::ExecCommandEnvironment,
    ) -> Result<(), blaze_exec::exec::spawn_strategy_registry::AbruptExitException> {
        Ok(())
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
// The rest of Java's ServerBuilder: what no Bazel module adds keeps its default, and what the
// runtime itself or the remote module adds is set where Blaze uses it (query output formatters:
// `blaze_query`; the BEP artifact uploaders and repository helpers: `commands/build_remote.rs`;
// the instrumentation output factory: `commands/bep.rs`).
// PORT: skipped, default only (no Bazel module calls it): ServerBuilder.addInvocationPolicy, ServerBuilder.getInvocationPolicy
// PORT: skipped, default only (no Bazel module calls it): ServerBuilder.setQueryEnvironmentFactory
// PORT: skipped, default only (no Bazel module calls it): ServerBuilder.getQueryEnvironmentFactory
// PORT: skipped, default only (no Bazel module calls it): ServerBuilder.addQueryFunctions, ServerBuilder.getQueryFunctions
// PORT: skipped, set where used: ServerBuilder.addQueryOutputFormatters, ServerBuilder.getQueryOutputFormatters
// PORT: skipped, set where used: ServerBuilder.addBuildEventArtifactUploaderFactory
// PORT: skipped, set where used: ServerBuilder.getBuildEventArtifactUploaderMap
// PORT: skipped, set where used: ServerBuilder.setRepositoryHelpersFactory, ServerBuilder.getRepositoryHelpersFactory
// PORT: skipped, set where used: ServerBuilder.getInstrumentationOutputFactoryBuilder
// PORT: skipped, set where used: ServerBuilder.createInstrumentationOutputFactory
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
    // Java: com/google/devtools/build/lib/runtime/ServerBuilder.java:122 ServerBuilder.addCommands
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
