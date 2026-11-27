//! The modules of `Bazel.BAZEL_MODULES`, in that order, with their option suppliers and the
//! commands and info items they register in `serverInit`.
//!
//! Each module keeps the trait defaults for the Java hooks it overrides; a `PORT:` line on each
//! module says for those hooks where their behaviour is ported (most of it runs with the command
//! that needs it rather than through the module), which allowed skip covers them, or where a
//! command that needs them fails loudly.

use std::sync::Arc;

use blaze_options::OptionsClass;
use blaze_options::definition::OptionsClassRef;

use blaze_optdefs::command::bazel_rules_module::{
    BazelRulesModuleAllCommandGraveyardOptions, BazelRulesModuleBazelBuildGraveyardOptions,
};
use blaze_optdefs::command::bazel_startup_options_module::BazelStartupOptionsModuleOptions;
use blaze_optdefs::command::execution_graph_module::ExecutionGraphModuleExecutionGraphOptions;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::command::local_diff_awareness::LocalDiffAwarenessOptions;
use blaze_optdefs::command::memory_pressure_options::MemoryPressureOptions;
use blaze_optdefs::command::output_filtering_module::OutputFilteringModuleOptions;
use blaze_optdefs::command::workspace_status_action::WorkspaceStatusActionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::bazel_coverage_report_module::BazelCoverageReportModuleOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_optdefs::misc::build_event_stream_options::BuildEventStreamOptions;
use blaze_optdefs::misc::command_profiler_module::CommandProfilerModuleOptions;
use blaze_optdefs::misc::debugging_options::DebuggingOptions;
use blaze_optdefs::misc::dynamic_execution_options::DynamicExecutionOptions;
use blaze_optdefs::misc::include_scanning_options::IncludeScanningOptions;
use blaze_optdefs::misc::metrics_module::MetricsModuleOptions;
use blaze_optdefs::misc::package_metrics_module::PackageMetricsModuleOptions;
use blaze_optdefs::misc::post_gc_memory_use_recorder::PostGCMemoryUseRecorderGcAfterBuildModuleOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::remote_startup_options::RemoteStartupOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_optdefs::misc::starlark_debugger_options::StarlarkDebuggerOptions;
use blaze_optdefs::misc::worker_options::WorkerOptions;

use crate::command::BlazeCommand;
use crate::command_environment::CommandEnvironment;
use crate::command_registry::{
    AqueryCommand, BuildCommand, CanonicalizeCommand, ConfigCommand, CoverageCommand,
    CqueryCommand, DumpCommand, FetchCommand, LicenseCommand, MobileInstallCommand, ModCommand,
    PrintActionCommand, QueryCommand, RunCommand, TestCommand, VendorCommand,
};
use crate::info_item::InfoItem;
use crate::module::{AbruptExitException, BlazeModule, ServerBuilder};
use crate::options_result::OptionsParsingResult;

/// `Bazel.BAZEL_MODULES` instantiated (`BlazeRuntime.createBlazeModules`), as `BlazeRuntime`
/// sees them.
///
/// `BlazeRuntime.Builder.build` appends a `DummyMetricsModule` only when no module posts a
/// `BuildMetricsEvent`; `MetricsModule.postsBuildMetricsEvent` returns true, so Bazel's list is
/// used as is.
// DummyMetricsModule is added only when no module posts the BuildMetricsEvent (see above):
// PORT: skipped, not in Bazel's module list: DummyMetricsModule.beforeCommand, DummyMetricsModule.onBuildComplete
// PORT: skipped, not in Bazel's module list: DummyMetricsModule.postsBuildMetricsEvent
// Bazel's main registers no BlazeService (BlazeRuntime.Builder.addBlazeService):
// PORT: skipped, default only (Bazel registers no service): BlazeService.getStartupOptions
// PORT: skipped, default only (Bazel registers no service): BlazeService.getCommonCommandOptions
// PORT: skipped, default only (Bazel registers no service): BlazeService.getCommandOptions
// Java: com/google/devtools/build/lib/bazel/Bazel.java:41 Bazel.BAZEL_MODULES
pub fn bazel_modules() -> Vec<Arc<dyn BlazeModule>> {
    vec![
        Arc::new(BazelStartupOptionsModule),
        // This module is registered early so that profiles are as complete as possible.
        Arc::new(CommandProfilerModule),
        // This module needs to be registered before any module providing a SpawnCache
        // implementation.
        Arc::new(NoSpawnCacheModule),
        // This module needs to be registered before any module that uses the credential cache.
        Arc::new(CredentialModule),
        Arc::new(CommandLogModule::default()),
        Arc::new(MemoryPressureModule),
        Arc::new(ThreadDumpModule),
        Arc::new(SleepPreventionModule),
        Arc::new(SystemSuspensionModule),
        Arc::new(BazelFileSystemModule),
        Arc::new(MobileInstallModule),
        Arc::new(BazelWorkspaceStatusModule),
        Arc::new(BazelDiffAwarenessModule),
        Arc::new(RemoteModule),
        Arc::new(BazelRepositoryModule::default()),
        Arc::new(StarlarkRepositoryDebugModule),
        Arc::new(WorkspaceRuleModule),
        Arc::new(BazelCoverageReportModule),
        Arc::new(StarlarkDebuggerModule),
        Arc::new(CacheHitReportingModule),
        Arc::new(SpawnLogModule),
        Arc::new(BazelLockFileModule),
        Arc::new(OutputFilteringModule),
        Arc::new(WorkerModule),
        Arc::new(CacheFileDigestsModule),
        Arc::new(StandaloneModule),
        Arc::new(SandboxModule),
        Arc::new(BuildSummaryStatsModule),
        Arc::new(DynamicExecutionModule),
        Arc::new(BazelRulesModule),
        Arc::new(BazelStrategyModule),
        Arc::new(NoOpConnectivityModule),
        Arc::new(AllocationTrackerModule),
        Arc::new(PackageMetricsModule::default()),
        Arc::new(ExecutionGraphModule),
        Arc::new(BazelBuiltinCommandModule),
        Arc::new(IncludeScanningModule),
        Arc::new(SkymeldModule),
        Arc::new(SerializationModule),
        // This module needs to be registered after any module submitting tasks with its {@code
        // submit} method.
        Arc::new(BlockWaitingModule),
        // This module needs to come after BlockWaitingModule so that the BES isn't closed until
        // the background tasks maintained by the module have completed.
        Arc::new(BazelBuildEventServiceModule::default()),
        // Modules that are involved in the collection of heap-related metrics of a build. They
        // need to be last in the modules order, so when the GCs happen at the end of the build,
        // we mitigate the risk that objects are still held onto by the other modules.
        Arc::new(PostGCMemoryUseRecorderPostGCMemoryUseRecorderModule),
        Arc::new(PostGCMemoryUseRecorderGcAfterBuildModule),
        Arc::new(MetricsModule),
    ]
}

/// Module providing Bazel's startup options.
// Java: com/google/devtools/build/lib/bazel/BazelStartupOptionsModule.java:25 BazelStartupOptionsModule
pub struct BazelStartupOptionsModule;

impl BlazeModule for BazelStartupOptionsModule {
    fn name(&self) -> &'static str {
        "BazelStartupOptionsModule"
    }

    // Java: com/google/devtools/build/lib/bazel/BazelStartupOptionsModule.java:82 BazelStartupOptionsModule.getStartupOptions
    fn get_startup_options(&self) -> Vec<OptionsClassRef> {
        vec![BazelStartupOptionsModuleOptions::class()]
    }
}

/// Bazel module to record pprof-compatible profiles for single invocations.
/// PORT: skipped, profiling (beforeCommand, afterCommand: `--experimental_command_profile`).
// Java: com/google/devtools/build/lib/profiler/CommandProfilerModule.java:34 CommandProfilerModule
pub struct CommandProfilerModule;

impl BlazeModule for CommandProfilerModule {
    fn name(&self) -> &'static str {
        "CommandProfilerModule"
    }

    // Java: com/google/devtools/build/lib/profiler/CommandProfilerModule.java:81 CommandProfilerModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![CommandProfilerModuleOptions::class()]
    }
}

/// Module providing the "no spawn cache" action context.
/// PORT: registerActionContexts is ported where the executor is prepared: it registers
/// `SpawnCache.NO_CACHE` (`blaze_buildtool::buildtool::execution_tool`).
// Java: com/google/devtools/build/lib/runtime/NoSpawnCacheModule.java:23 NoSpawnCacheModule
pub struct NoSpawnCacheModule;

impl BlazeModule for NoSpawnCacheModule {
    fn name(&self) -> &'static str {
        "NoSpawnCacheModule"
    }
}

/// A module whose sole purpose is to hold the credential cache which is shared by other
/// modules.
/// PORT: the cache is `blaze_repo_fetch`'s process-wide
/// `credentialhelper::credential_module()` (its users sit below this crate); this module calls
/// its `beforeCommand`.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:31 CredentialModule
pub struct CredentialModule;

impl BlazeModule for CredentialModule {
    fn name(&self) -> &'static str {
        "CredentialModule"
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:56 CredentialModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![AuthAndTLSOptions::class()]
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:61 CredentialModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        // The options are common to every command (getCommonCommandOptions above).
        if let Some(auth_and_tls_options) = env.get_options().get_options::<AuthAndTLSOptions>() {
            blaze_repo_fetch::authandtls::credentialhelper::credential_module().before_command(
                env.get_command_name(),
                auth_and_tls_options.credential_helper_cache_timeout,
            );
        }
        Ok(())
    }
}

/// This module logs complete stdout / stderr output of Bazel to a local file.
// Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:34 CommandLogModule
#[derive(Default)]
pub struct CommandLogModule {
    /// `env`: what `commandComplete` reports to (its reporter), between `beforeCommand` and
    /// `commandComplete`.
    env: parking_lot::Mutex<Option<Arc<blaze_events::Reporter>>>,
    /// `logOutputStream`: the open `command.log` of the running command, if it writes one.
    log_output_stream: parking_lot::Mutex<Option<Arc<dyn blaze_events::OutputStream>>>,
}

impl BlazeModule for CommandLogModule {
    fn name(&self) -> &'static str {
        "CommandLogModule"
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:41 CommandLogModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        let item: Arc<dyn InfoItem> =
            Arc::new(crate::commands::info::module_items::CommandLogInfoItem::new());
        builder.add_info_items([item]);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:46 CommandLogModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        *self.env.lock() = Some(env.get_reporter().clone());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:51 CommandLogModule.getOutputListener
    fn get_output_listener(&self, env: &CommandEnvironment) -> Option<blaze_events::OutErr> {
        use blaze_events::EventHandler as _;
        let common_options = env
            .get_options()
            .get_options::<blaze_optdefs::command::common_command_options::CommonCommandOptions>()
            .expect("CommonCommandOptions");
        if !common_options.redirect_local_instrumentation_output_writes {
            // When instrumentation output are locally written, we need to unlink old local
            // command log from previous build, if present.
            let command_log = get_command_log_path(env.get_output_base());
            if let Err(io_exception) = command_log.delete() {
                env.get_reporter()
                    .handle(&blaze_events::Event::warn(format!(
                        "Unable to delete command log: {}",
                        java_io_message(&io_exception)
                    )));
            }
        }

        if common_options.write_command_log && env.get_command_name() != "clean" {
            // env.getRuntime().getInstrumentationOutputFactory().createInstrumentationOutput(
            //     "command_log", PathFragment.create("command.log"), OUTPUT_BASE, env,
            //     env.getReporter(), /* append= */ false, /* internal= */ true)
            // Java: com/google/devtools/build/lib/runtime/InstrumentationOutputFactory.java:126 InstrumentationOutputFactory.createInstrumentationOutput
            if common_options.redirect_local_instrumentation_output_writes {
                // Bazel has no redirecting InstrumentationOutputBuilder.
                env.get_reporter().handle(&blaze_events::Event::warn(
                    "Redirecting to write Instrumentation Output on a different machine is not \
                     supported. Defaulting to writing output locally.",
                ));
            }
            let command_log_output = crate::profiler_init::LocalInstrumentationOutput::new(
                "command_log",
                env.get_output_base().get_relative_str("command.log"),
                None,
                Some(false),
                Some(true),
                false,
            );
            match command_log_output.create_output_stream() {
                Ok(stream) => {
                    let log_output_stream: Arc<dyn blaze_events::OutputStream> =
                        Arc::new(blaze_events::io::WriteOutputStream::new(stream));
                    *self.log_output_stream.lock() = Some(log_output_stream.clone());
                    return Some(blaze_events::OutErr::create(
                        log_output_stream.clone(),
                        log_output_stream,
                    ));
                }
                Err(io_exception) => {
                    env.get_reporter()
                        .handle(&blaze_events::Event::warn(format!(
                            "Unable to open command log: {}",
                            java_io_message(&io_exception)
                        )));
                }
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:62 CommandLogModule.commandComplete
    fn command_complete(&self) {
        let local_env = self.env.lock().take();
        if let Some(log_output_stream) = self.log_output_stream.lock().take() {
            let closed = log_output_stream
                .flush()
                .and_then(|()| log_output_stream.close());
            if let Err(e) = closed {
                blaze_util::server_logger::warning(
                    "com.google.devtools.build.lib.runtime.CommandLogModule",
                    "commandComplete",
                    "I/O exception closing log",
                    Some(&format!("java.io.IOException: {}", java_io_message(&e))),
                );
                let msg = format!("I/O exception closing log: {}", java_io_message(&e));
                match local_env {
                    Some(reporter) => {
                        use blaze_events::EventHandler as _;
                        reporter.handle(&blaze_events::Event::error(msg));
                    }
                    None => blaze_events::io::system_err_println(&msg),
                }
            }
            // finally { logOutputStream = null; }: taken above.
        }
    }
}

/// `IOException.getMessage()`: Blaze's file system errors already carry Java's message; plain OS
/// errors lose Rust's " (os error N)" suffix.
fn java_io_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => text[..i].to_string(),
        _ => text,
    }
}

/// For a given output_base directory, returns the command log file path.
// Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:58 CommandLogModule.getCommandLogPath
fn get_command_log_path(output_base: &blaze_vfs::Path) -> blaze_vfs::Path {
    output_base.get_relative_str("command.log")
}

/// Module that tracks memory pressure.
/// PORT: skipped, memory instrumentation (beforeCommand, afterCommand).
// Java: com/google/devtools/build/lib/runtime/MemoryPressureModule.java:31 MemoryPressureModule
pub struct MemoryPressureModule;

impl BlazeModule for MemoryPressureModule {
    fn name(&self) -> &'static str {
        "MemoryPressureModule"
    }

    // Java: com/google/devtools/build/lib/runtime/MemoryPressureModule.java:39 MemoryPressureModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![MemoryPressureOptions::class()]
    }
}

/// A module that dumps thread stacks.
/// PORT: beforeCommand, afterCommand are not ported; `--experimental_enable_thread_dump`
/// fails loudly (`crate::unported_options`).
// Java: com/google/devtools/build/lib/runtime/ThreadDumpModule.java:56 ThreadDumpModule
pub struct ThreadDumpModule;

impl BlazeModule for ThreadDumpModule {
    fn name(&self) -> &'static str {
        "ThreadDumpModule"
    }
}

/// Blaze module to prevent the system from sleeping while Blaze is running.
/// PORT: skipped, macOS and Windows only (beforeCommand, afterCommand).
// Java: com/google/devtools/build/lib/platform/SleepPreventionModule.java:23 SleepPreventionModule
pub struct SleepPreventionModule;

impl BlazeModule for SleepPreventionModule {
    fn name(&self) -> &'static str {
        "SleepPreventionModule"
    }
}

/// Monitors system suspension events.
/// PORT: skipped, macOS only (beforeCommand, afterCommand hold the reporter for the
/// suspension callback; `portable_start_suspend_monitoring` is empty on Linux).
// Java: com/google/devtools/build/lib/platform/SystemSuspensionModule.java:26 SystemSuspensionModule
pub struct SystemSuspensionModule;

impl BlazeModule for SystemSuspensionModule {
    fn name(&self) -> &'static str {
        "SystemSuspensionModule"
    }
}

/// Module to provide a `FileSystem` instance that uses `SHA256` as the default hash function, or
/// else what's specified by `-Dbazel.DigestFunction`.
/// PORT: getFileSystem is in `BlazeRuntime::new_runtime` (the Unix file system with SHA-256;
/// `--unix_digest_hash_attribute_name` fails loudly there).
// Java: com/google/devtools/build/lib/bazel/BazelFileSystemModule.java:46 BazelFileSystemModule
pub struct BazelFileSystemModule;

impl BlazeModule for BazelFileSystemModule {
    fn name(&self) -> &'static str {
        "BazelFileSystemModule"
    }
}

/// Module for mobile_install.
// Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallModule.java:23 MobileInstallModule
pub struct MobileInstallModule;

impl BlazeModule for MobileInstallModule {
    fn name(&self) -> &'static str {
        "MobileInstallModule"
    }

    // Java: com/google/devtools/build/lib/runtime/mobileinstall/MobileInstallModule.java:25 MobileInstallModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        let commands: [Arc<dyn BlazeCommand>; 1] = [Arc::new(MobileInstallCommand)];
        builder.add_commands(commands);
        Ok(())
    }
}

/// Provides information about the workspace (e.g. source control context, current machine,
/// current user, etc).
/// PORT: workspaceInit (the `WorkspaceStatusAction` factory) and registerActionContexts are
/// ported with the analysis services and the command's execution setup
/// (`blaze_analysis::analysis_services`, `blaze_sky_top::execution_phase`).
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:73 BazelWorkspaceStatusModule
// PORT: skipped, the factory is made where it is used (blaze_analysis analysis_services): BazelWorkspaceStatusModule.workspaceInit
pub struct BazelWorkspaceStatusModule;

impl BlazeModule for BazelWorkspaceStatusModule {
    fn name(&self) -> &'static str {
        "BazelWorkspaceStatusModule"
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:324 BazelWorkspaceStatusModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![WorkspaceStatusActionOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Provides the `DiffAwareness` implementations available to Bazel.
/// PORT: skipped, incremental (workspaceInit registers the `--watchfs` diff awareness).
// Java: com/google/devtools/build/lib/bazel/BazelDiffAwarenessModule.java:28 BazelDiffAwarenessModule
pub struct BazelDiffAwarenessModule;

impl BlazeModule for BazelDiffAwarenessModule {
    fn name(&self) -> &'static str {
        "BazelDiffAwarenessModule"
    }

    // Java: com/google/devtools/build/lib/bazel/BazelDiffAwarenessModule.java:30 BazelDiffAwarenessModule.workspaceInit
    fn workspace_init(
        &self,
        _directories: &crate::directories::BlazeDirectories,
        builder: &mut crate::workspace::WorkspaceBuilder,
    ) {
        // Order here is important - LocalDiffAwareness creation always succeeds, so it must be
        // last.
        builder.add_diff_awareness_factory(Arc::new(
            blaze_sky_loading::diff_awareness::LocalDiffAwarenessFactory::new(Vec::new()),
        ));
    }

    // Java: com/google/devtools/build/lib/bazel/BazelDiffAwarenessModule.java:37 BazelDiffAwarenessModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![LocalDiffAwarenessOptions::class()]
    }
}

/// RemoteModule provides distributed cache and remote execution for Bazel.
/// PORT: not ported (serverInit's "remote" build event artifact uploader factory and repository
/// remote helpers factory; globalInit, getFileSystemForBuildArtifacts, workspaceInit,
/// beforeCommand, the afterAnalysis hooks, afterCommand, registerSpawnStrategies,
/// registerActionContexts, executorInit, getOutputService). The module only acts when a disk
/// cache, remote cache, remote executor, remote output service or remote downloader is
/// requested, and each of those fails loudly (`crate::unported_options` and
/// `blaze_exec::exec::executor_setup`, which gets the command's `RemoteOptions`).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:139 RemoteModule
pub struct RemoteModule;

impl BlazeModule for RemoteModule {
    fn name(&self) -> &'static str {
        "RemoteModule"
    }

    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:206 RemoteModule.getStartupOptions
    fn get_startup_options(&self) -> Vec<OptionsClassRef> {
        vec![RemoteStartupOptions::class()]
    }

    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1341 RemoteModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![RemoteOptions::class(), AuthAndTLSOptions::class()]
    }
}

/// Adds support for fetching external code.
/// PORT: workspaceInit, initializeRuleClasses, beforeCommand (but for the repo contents cache's
/// shared lock and garbage collection idle task, below) and getPrecomputedValues are ported with
/// the command's executor
/// (`blaze_repo_rules::setup::prepare_repository_functions_with`, called by
/// `blaze_sky_top::skyframe_executor::SkyframeExecutor::create`).
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:127 BazelRepositoryModule
#[derive(Default)]
pub struct BazelRepositoryModule {
    /// The repo contents cache whose shared lock this command holds, from `beforeCommand` to
    /// `afterCommand` (Java: `repositoryCache.getRepoContentsCache()` and its `sharedLock`).
    /// PORT: Java's module keeps one `RepositoryCache` for the server and sets its paths at each
    /// `beforeCommand`; here only the cache that holds the lock is kept, and only while it does.
    repo_contents_cache:
        parking_lot::Mutex<Option<blaze_repo_fetch::cache::LocalRepoContentsCache>>,
}

impl BlazeModule for BazelRepositoryModule {
    fn name(&self) -> &'static str {
        "BazelRepositoryModule"
    }

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:206 BazelRepositoryModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        let fetch: [Arc<dyn BlazeCommand>; 1] = [Arc::new(FetchCommand)];
        builder.add_commands(fetch);
        let mod_: [Arc<dyn BlazeCommand>; 1] = [Arc::new(ModCommand)];
        builder.add_commands(mod_);
        let vendor: [Arc<dyn BlazeCommand>; 1] = [Arc::new(VendorCommand)];
        builder.add_commands(vendor);
        // PORT: Java passes the module's RepositoryCache; the info item reads the repository
        // cache path itself.
        let item: Arc<dyn InfoItem> =
            Arc::new(crate::commands::info::module_items::RepositoryCacheInfoItem::new());
        builder.add_info_items([item]);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:781 BazelRepositoryModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![RepositoryOptions::class()]
    }

    /// The repo contents cache part of `beforeCommand`: when the cache is enabled, its shared
    /// lock (`acquireSharedLock`, released by `afterCommand`) and
    /// `env.addIdleTask(repositoryCache.getRepoContentsCache().createGcIdleTask(maxAge,
    /// idleDelay))`.
    ///
    /// PORT: the rest of `beforeCommand` (download manager, repository cache paths, the checks
    /// of the repo contents cache path) runs with the command's executor
    /// (`blaze_repo_rules::setup::prepare_repository_functions_with`), which reports the path
    /// errors; the lock is not taken and the task is not registered in those cases, as Java
    /// throws before it takes the lock.
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:276 BazelRepositoryModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        use crate::commands::info::module_items::RepositoryCacheInfoItem;
        let Some(repo_options) = env.get_options().get_options::<RepositoryOptions>() else {
            return Ok(());
        };
        // Note that the repo contents cache stuff has to happen _after_ the repo cache stuff,
        // because the specific settings about the repo contents cache might overwrite the repo
        // cache settings.
        let mut repository_cache = blaze_repo_fetch::cache::RepositoryCache::new();
        repository_cache.set_path(RepositoryCacheInfoItem::get_repository_cache_path(env));
        if let Some(p) = &repo_options.repo_contents_cache {
            repository_cache
                .get_repo_contents_cache_mut()
                .set_path(RepositoryCacheInfoItem::to_path(p.get(), env));
        }
        let Some(repo_contents_cache_path) = repository_cache
            .get_repo_contents_cache()
            .get_path()
            .cloned()
        else {
            return Ok(());
        };
        let resolve = |p: &blaze_vfs::Path| match p.resolve_symbolic_links() {
            Ok(resolved) => Some(resolved),
            // Will be created later.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(p.clone()),
            // "could not resolve ...": reported by the executor's setup.
            Err(_) => None,
        };
        let (Some(resolved_output_base), Some(resolved_repo_contents_cache)) = (
            resolve(env.get_output_base()),
            resolve(&repo_contents_cache_path),
        ) else {
            return Ok(());
        };
        if resolved_output_base.starts_with(&resolved_repo_contents_cache) {
            // "The output base [...] is inside the repo contents cache [...]": the executor's
            // setup throws it.
            return Ok(());
        }
        if env
            .get_workspace()
            .is_some_and(|workspace| repo_contents_cache_path.starts_with(workspace))
        {
            // "The repo contents cache [...] is inside the main repo [...]": likewise.
            return Ok(());
        }
        // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:403 BazelRepositoryModule.beforeCommand
        {
            let _profile = blaze_util::profiler::Profiler::instance().profile(
                blaze_util::profiler::ProfilerTask::REPO_CACHE_GC_WAIT,
                "waiting to acquire repo cache lock",
            );
            let mut held = self.repo_contents_cache.lock();
            // Preconditions.checkState(sharedLock == null, "this process already has the shared
            // lock"): the previous command's afterCommand released it.
            blaze_util::check_state!(held.is_none(), "this process already has the shared lock");
            let mut repo_contents_cache = blaze_repo_fetch::cache::LocalRepoContentsCache::new();
            repo_contents_cache.set_path(Some(repo_contents_cache_path.clone()));
            if repo_contents_cache.acquire_shared_lock().is_err() {
                return Err(repo_contents_cache_exception(
                    "could not acquire lock on repo contents cache",
                ));
            }
            *held = Some(repo_contents_cache);
        }
        env.add_idle_task(Arc::new(
            crate::idle_tasks::RepoContentsCacheGcIdleTask::create(
                repo_contents_cache_path,
                repo_options.repo_contents_cache_gc_max_age,
                repo_options.repo_contents_cache_gc_idle_delay,
            ),
        ));
        Ok(())
    }

    /// PORT: Java releases the lock when the module's repo contents cache is enabled, which is
    /// when this command's `beforeCommand` took it; Blaze releases the lock it holds.
    // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:731 BazelRepositoryModule.afterCommand
    fn after_command(&self) -> Result<(), AbruptExitException> {
        if let Some(mut repo_contents_cache) = self.repo_contents_cache.lock().take()
            && repo_contents_cache.release_shared_lock().is_err()
        {
            return Err(repo_contents_cache_exception(
                "could not release lock on repo contents cache",
            ));
        }
        Ok(())
    }
}

/// `BazelRepositoryModule.detailedExitCode(message, Code.BAD_REPO_CONTENTS_CACHE)` as an
/// `AbruptExitException`.
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:181 BazelRepositoryModule.detailedExitCode
fn repo_contents_cache_exception(message: &str) -> AbruptExitException {
    use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
    AbruptExitException::new(crate::exit_code::DetailedExitCode::of(FailureDetail {
        message: message.to_string(),
        category: Some(Category::ExternalRepository(
            failure_details::ExternalRepository {
                code: failure_details::external_repository::Code::BadRepoContentsCache as i32,
            },
        )),
    }))
}

/// Module for the Starlark repository debugger: reports back the place an external repository
/// was defined, if requested by some error involving that repository
/// (`crate::bzlmod_cycle_reporters::StarlarkRepositoryDebugSubscriber`).
/// PORT: afterCommand only drops the module's per-command state, which lives in the subscriber
/// (dropped with the command's event bus).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:31 StarlarkRepositoryDebugModule
pub struct StarlarkRepositoryDebugModule;

impl BlazeModule for StarlarkRepositoryDebugModule {
    fn name(&self) -> &'static str {
        "StarlarkRepositoryDebugModule"
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:37 StarlarkRepositoryDebugModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        let subscriber = Arc::new(
            crate::bzlmod_cycle_reporters::StarlarkRepositoryDebugSubscriber::new(
                env.get_reporter().clone(),
            ),
        );
        env.get_event_bus().register(subscriber);
        Ok(())
    }
}

/// A module for logging workspace rule events.
/// PORT: beforeCommand, afterCommand are not ported;
/// `--experimental_workspace_rules_log_file` fails loudly (`crate::unported_options`).
// Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleModule.java:35 WorkspaceRuleModule
pub struct WorkspaceRuleModule;

impl BlazeModule for WorkspaceRuleModule {
    fn name(&self) -> &'static str {
        "WorkspaceRuleModule"
    }

    // Java: com/google/devtools/build/lib/bazel/debug/WorkspaceRuleModule.java:86 WorkspaceRuleModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![DebuggingOptions::class()]
    }
}

/// Adds support for coverage report generation.
/// PORT: getCoverageReportFactory is blaze_analysis'
/// (`bazel::coverage::bazel_coverage_report_module`); the build command asks it for the factory.
// Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:42 BazelCoverageReportModule
pub struct BazelCoverageReportModule;

impl BlazeModule for BazelCoverageReportModule {
    fn name(&self) -> &'static str {
        "BazelCoverageReportModule"
    }

    // Java: com/google/devtools/build/lib/bazel/coverage/BazelCoverageReportModule.java:73 BazelCoverageReportModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![BazelCoverageReportModuleOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Blaze module for setting up Starlark debugging.
/// PORT: beforeCommand, afterCommand, blazeShutdown, blazeShutdownOnCrash are not ported;
/// `--experimental_skylark_debug` fails loudly (`crate::unported_options`).
// Java: com/google/devtools/build/lib/starlarkdebug/module/StarlarkDebuggerModule.java:34 StarlarkDebuggerModule
pub struct StarlarkDebuggerModule;

impl BlazeModule for StarlarkDebuggerModule {
    fn name(&self) -> &'static str {
        "StarlarkDebuggerModule"
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/module/StarlarkDebuggerModule.java:55 StarlarkDebuggerModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![StarlarkDebuggerOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Module that reports cache hits.
/// PORT: beforeCommand, afterCommand are not ported: on a `RepositoryFailedEvent` the module
/// lists the download cache hits the repository used, and no ported code posts that event (the
/// repository crates post `DownloadCacheHitEvent` only).
// Java: com/google/devtools/build/lib/bazel/CacheHitReportingModule.java:31 CacheHitReportingModule
pub struct CacheHitReportingModule;

impl BlazeModule for CacheHitReportingModule {
    fn name(&self) -> &'static str {
        "CacheHitReportingModule"
    }
}

/// Module providing on-demand spawn logging.
/// PORT: registerActionContexts, executorInit, afterCommand are ported with the build command
/// (`crate::commands::build`, `blaze_exec::exec::spawn_log_module`).
// Java: com/google/devtools/build/lib/bazel/SpawnLogModule.java:48 SpawnLogModule
pub struct SpawnLogModule;

impl BlazeModule for SpawnLogModule {
    fn name(&self) -> &'static str {
        "SpawnLogModule"
    }
}

/// Module collecting Bazel module and module extensions resolution results and updating the
/// lockfile.
/// PORT: beforeCommand, afterCommand are ported with the command's executor
/// (`blaze_sky_top::skyframe_executor::SkyframeExecutor::after_command`, run by
/// `crate::commands::query::AfterCommand`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelLockFileModule.java:47 BazelLockFileModule
pub struct BazelLockFileModule;

impl BlazeModule for BazelLockFileModule {
    fn name(&self) -> &'static str {
        "BazelLockFileModule"
    }
}

/// Blaze module implementing output filtering.
/// PORT: beforeCommand, afterCommand (the reporter's output filter) are ported with the build
/// command (`crate::commands::build::output_filter`).
// Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:34 OutputFilteringModule
pub struct OutputFilteringModule;

impl BlazeModule for OutputFilteringModule {
    fn name(&self) -> &'static str {
        "OutputFilteringModule"
    }

    // Java: com/google/devtools/build/lib/outputfilter/OutputFilteringModule.java:60 OutputFilteringModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![OutputFilteringModuleOptions::class()]
        } else {
            vec![]
        }
    }
}

/// A module that adds the WorkerActionContextProvider to the available action context providers.
/// The module's state (worker factory, pool, lifecycle manager) is
/// `blaze_exec::worker::worker_module::WorkerModule`; `registerSpawnStrategies` runs in the
/// command's execution setup (`blaze_exec::exec::executor_setup`), `buildStarting` and
/// `buildComplete` in the build command (`commands/build.rs`).
// Java: com/google/devtools/build/lib/worker/WorkerModule.java:49 WorkerModule
pub struct WorkerModule;

impl BlazeModule for WorkerModule {
    fn name(&self) -> &'static str {
        "WorkerModule"
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:62 WorkerModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![WorkerOptions::class()]
        } else {
            vec![]
        }
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:67 WorkerModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        blaze_exec::worker::worker_module::WorkerModule::before_command(
            env.get_reporter().clone() as Arc<dyn blaze_events::EventHandler>,
            None,
        );
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/WorkerModule.java:278 WorkerModule.afterCommand
    fn after_command(&self) -> Result<(), AbruptExitException> {
        blaze_exec::worker::worker_module::WorkerModule::after_command();
        Ok(())
    }
}

/// Enables the caching of file digests in `DigestUtils`.
///
/// PORT: the module's fields live in a process static, because the build command reaches the
/// module's `executorInit` statically (`commands/build.rs`) rather than through the module list;
/// a server has one runtime, so one instance, as in Java.
// Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:26 CacheFileDigestsModule
pub struct CacheFileDigestsModule;

/// The fields of [`CacheFileDigestsModule`].
#[derive(Default)]
struct CacheFileDigestsModuleState {
    /// Stats gathered at the beginning of a command, to compute deltas on completion.
    stats: Option<blaze_exec::actions::cache::digest_utils::CacheStats>,
    /// Last known size of the cache. Changes to this value cause the cache to be reinitialized.
    /// `None` if we don't know anything about the last value yet (i.e. before any command has
    /// been run).
    last_known_cache_size: Option<i64>,
}

static CACHE_FILE_DIGESTS_MODULE: parking_lot::Mutex<Option<CacheFileDigestsModuleState>> =
    parking_lot::Mutex::new(None);

const CACHE_FILE_DIGESTS_MODULE_CLASS: &str =
    "com.google.devtools.build.lib.runtime.CacheFileDigestsModule";

impl CacheFileDigestsModule {
    /// Adds a line to the log with cache statistics.
    // Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:47 CacheFileDigestsModule.logStats
    fn log_stats(message: &str, stats: &blaze_exec::actions::cache::digest_utils::CacheStats) {
        blaze_util::server_logger::info(
            CACHE_FILE_DIGESTS_MODULE_CLASS,
            "logStats",
            &format!(
                "{}: hit count={}, miss count={}, hit rate={}, eviction count={}",
                message,
                stats.hit_count,
                stats.miss_count,
                java_format_g(stats.hit_rate()),
                stats.eviction_count
            ),
        );
    }

    // Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:53 CacheFileDigestsModule.executorInit
    pub fn executor_init(cache_size_for_computed_file_digests: i64) {
        use blaze_exec::actions::cache::digest_utils::DigestUtils;
        let mut guard = CACHE_FILE_DIGESTS_MODULE.lock();
        let state = guard.get_or_insert_with(Default::default);
        if state.last_known_cache_size != Some(cache_size_for_computed_file_digests) {
            blaze_util::server_logger::info(
                CACHE_FILE_DIGESTS_MODULE_CLASS,
                "executorInit",
                &format!(
                    "Reconfiguring cache with size={}",
                    cache_size_for_computed_file_digests
                ),
            );
            DigestUtils::configure_cache(cache_size_for_computed_file_digests);
            state.last_known_cache_size = Some(cache_size_for_computed_file_digests);
        }

        if cache_size_for_computed_file_digests == 0 {
            state.stats = None;
            blaze_util::server_logger::info(
                CACHE_FILE_DIGESTS_MODULE_CLASS,
                "executorInit",
                "Disabled cache",
            );
        } else {
            let stats = DigestUtils::get_cache_stats();
            Self::log_stats("Accumulated cache stats before command", &stats);
            state.stats = Some(stats);
        }
    }
}

/// Java `String.format("%g", x)` for `x >= 0` (a hit rate): the shortest decimal digits of `x`
/// rounded half-up to six significant digits (`FormattedFloatingDecimal`, GENERAL form); fixed
/// notation when the rounded value is in `[10^-4, 10^6)`, scientific notation otherwise.
// Java: java/util/Formatter.java FormatSpecifier.print(double) (Conversion.GENERAL)
fn java_format_g(x: f64) -> String {
    const PRECISION: i32 = 6;
    if x == 0.0 {
        return "0.00000".to_string();
    }
    // Rust's `{:e}` prints the shortest round-trip digits: "d.ddde-N".
    let shortest = format!("{x:e}");
    let (mantissa, exponent) = shortest.split_once('e').unwrap();
    let mut exponent: i32 = exponent.parse().unwrap();
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    if digits.len() > PRECISION as usize {
        let round_up = digits[PRECISION as usize] >= b'5';
        digits.truncate(PRECISION as usize);
        if round_up {
            let mut k = digits.len();
            loop {
                if k == 0 {
                    digits.insert(0, b'1');
                    digits.pop();
                    exponent += 1;
                    break;
                }
                k -= 1;
                if digits[k] == b'9' {
                    digits[k] = b'0';
                } else {
                    digits[k] += 1;
                    break;
                }
            }
        }
    }
    if (-4..PRECISION).contains(&exponent) {
        return blaze_events::java_format::java_format_fixed(
            x,
            (PRECISION - 1 - exponent) as usize,
        );
    }
    digits.resize(PRECISION as usize, b'0');
    format!(
        "{}.{}e{}{:02}",
        digits[0] as char,
        std::str::from_utf8(&digits[1..]).unwrap(),
        if exponent < 0 { '-' } else { '+' },
        exponent.abs()
    )
}

impl BlazeModule for CacheFileDigestsModule {
    fn name(&self) -> &'static str {
        "CacheFileDigestsModule"
    }

    // Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:73 CacheFileDigestsModule.commandComplete
    fn command_complete(&self) {
        use blaze_exec::actions::cache::digest_utils::DigestUtils;
        let mut guard = CACHE_FILE_DIGESTS_MODULE.lock();
        let Some(state) = guard.as_mut() else {
            return;
        };
        if let Some(stats) = state.stats.take() {
            let new_stats = DigestUtils::get_cache_stats();
            Self::log_stats("Accumulated cache stats after command", &new_stats);
            Self::log_stats("Cache stats for finished command", &new_stats.minus(&stats));
            // Silence stats until next command that uses the executor.
        }
    }
}

/// StandaloneModule provides pluggable functionality for blaze.
/// PORT: registerActionContexts, registerSpawnStrategies are ported with the command's
/// execution setup (`blaze_buildtool::runtime::standalone_module`,
/// `blaze_exec::exec::executor_setup`).
// Java: com/google/devtools/build/lib/standalone/StandaloneModule.java:47 StandaloneModule
pub struct StandaloneModule;

impl BlazeModule for StandaloneModule {
    fn name(&self) -> &'static str {
        "StandaloneModule"
    }
}

/// This module provides the Sandbox spawn strategy.
/// PORT: beforeCommand, registerSpawnStrategies, afterCommand are ported with the command's
/// execution setup (`blaze_exec::sandbox::sandbox_module`, run by
/// `blaze_buildtool::buildtool::execution_tool` and
/// `blaze_sky_top::execution_phase::ExecutionState::after_command`). PORT: skipped,
/// incremental: blazeShutdown, blazeShutdownOnCrash clean up what a server keeps between
/// commands.
// Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:58 SandboxModule
pub struct SandboxModule;

impl BlazeModule for SandboxModule {
    fn name(&self) -> &'static str {
        "SandboxModule"
    }

    // Java: com/google/devtools/build/lib/sandbox/SandboxModule.java:106 SandboxModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![SandboxOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Blaze module for the build summary message that reports various stats to the user.
/// PORT: beforeCommand, afterCommand, executorInit and the subscribers are ported with the
/// build command (`blaze_buildtool::buildtool::execution_tool::ExecutionEventCollector`,
/// `report_build_summary_stats`). PORT: skipped, profiling: the critical path computer.
// Java: com/google/devtools/build/lib/runtime/BuildSummaryStatsModule.java:52 BuildSummaryStatsModule
pub struct BuildSummaryStatsModule;

impl BlazeModule for BuildSummaryStatsModule {
    fn name(&self) -> &'static str {
        "BuildSummaryStatsModule"
    }
}

/// `BlazeModule` providing support for dynamic spawn execution and scheduling.
/// PORT: registerSpawnStrategies is ported with the command's execution setup
/// (`blaze_exec::exec::executor_setup`, with the command's `DynamicExecutionOptions`);
/// beforeCommand, afterCommand manage the executor of the dynamic strategies.
// Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:54 DynamicExecutionModule
pub struct DynamicExecutionModule;

impl BlazeModule for DynamicExecutionModule {
    fn name(&self) -> &'static str {
        "DynamicExecutionModule"
    }

    // Java: com/google/devtools/build/lib/dynamic/DynamicExecutionModule.java:71 DynamicExecutionModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![DynamicExecutionOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Module implementing the rule set of Bazel.
/// PORT: initializeRuleClasses (`BazelRuleClassProvider.setup`; its fragment options are in
/// `crate::fragment_options`) and registerActionContexts are ported with the rule class
/// provider and the command's execution setup (`blaze_build_config`,
/// `blaze_sky_top::execution_phase`).
// Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:34 BazelRulesModule
pub struct BazelRulesModule;

impl BlazeModule for BazelRulesModule {
    fn name(&self) -> &'static str {
        "BazelRulesModule"
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelRulesModule.java:968 BazelRulesModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![
                BazelRulesModuleBazelBuildGraveyardOptions::class(),
                BazelRulesModuleAllCommandGraveyardOptions::class(),
            ]
        } else {
            vec![BazelRulesModuleAllCommandGraveyardOptions::class()]
        }
    }
}

/// Module which registers the strategy options for Bazel.
/// PORT: registerActionContexts, registerSpawnStrategies are ported with the command's
/// execution setup (`blaze_exec::exec::executor_setup`).
// Java: com/google/devtools/build/lib/bazel/rules/BazelStrategyModule.java:40 BazelStrategyModule
pub struct BazelStrategyModule;

impl BlazeModule for BazelStrategyModule {
    fn name(&self) -> &'static str {
        "BazelStrategyModule"
    }

    // Java: com/google/devtools/build/lib/bazel/rules/BazelStrategyModule.java:42 BazelStrategyModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![ExecutionOptions::class(), RemoteOptions::class()]
        } else {
            vec![]
        }
    }
}

/// A `ConnectivityStatusProvider` that always reports the network as available.
/// PORT: getStatus (ConnectivityStatusProvider) always reports the network as available; its
/// readers (the Build Event Service and remote modules) are not ported.
// Java: com/google/devtools/build/lib/network/NoOpConnectivityModule.java:23 NoOpConnectivityModule
pub struct NoOpConnectivityModule;

impl BlazeModule for NoOpConnectivityModule {
    fn name(&self) -> &'static str {
        "NoOpConnectivityModule"
    }
}

/// A Blaze module that installs the allocation tracker.
/// PORT: skipped, memory instrumentation (blazeStartup, workspaceInit, beforeCommand).
// Java: com/google/devtools/build/lib/profiler/memory/AllocationTrackerModule.java:49 AllocationTrackerModule
pub struct AllocationTrackerModule;

impl BlazeModule for AllocationTrackerModule {
    fn name(&self) -> &'static str {
        "AllocationTrackerModule"
    }
}

/// Module providing metrics on package loading.
// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:31 PackageMetricsModule
pub struct PackageMetricsModule {
    package_loading_listener: Arc<crate::package_metrics::PackageMetricsPackageLoadingListener>,
}

impl Default for PackageMetricsModule {
    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:63 PackageMetricsModule.<init>
    fn default() -> Self {
        PackageMetricsModule {
            package_loading_listener:
                crate::package_metrics::PackageMetricsPackageLoadingListener::get_instance(),
        }
    }
}

impl BlazeModule for PackageMetricsModule {
    fn name(&self) -> &'static str {
        "PackageMetricsModule"
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:74 PackageMetricsModule.getPackageLoadingListener
    fn get_package_loading_listener(
        &self,
    ) -> Option<Arc<dyn blaze_packages::package::package_loading_listener::PackageLoadingListener>>
    {
        Some(self.package_loading_listener.clone())
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:82 PackageMetricsModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![PackageMetricsModuleOptions::class()]
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:87 PackageMetricsModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        let options = env
            .get_options()
            .get_options::<PackageMetricsModuleOptions>()
            .expect("PackageMetricsModule.Options");
        let recorder: Arc<dyn crate::package_metrics::PackageMetricsRecorder> =
            if options.enable_all_metrics {
                Arc::new(crate::package_metrics::CompletePackageMetricsRecorder::new())
            } else {
                Arc::new(crate::package_metrics::ExtremaPackageMetricsRecorder::new(
                    options.number_of_packages_to_track.max(0),
                ))
            };
        self.package_loading_listener
            .set_package_metrics_recorder(Some(recorder));
        self.package_loading_listener
            .set_publish_package_metrics_in_bep(options.publish_package_metrics_in_bep);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:98 PackageMetricsModule.afterCommand
    fn after_command(&self) -> Result<(), AbruptExitException> {
        if let Some(recorder) = self.package_loading_listener.get_package_metrics_recorder() {
            recorder.loading_finished();
        }
        Ok(())
    }
}

/// Blaze module that writes a partial execution graph with performance data.
/// PORT: beforeCommand, afterCommand are not ported;
/// `--experimental_enable_execution_graph_log` fails loudly (`crate::unported_options`).
// Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:99 ExecutionGraphModule
pub struct ExecutionGraphModule;

impl BlazeModule for ExecutionGraphModule {
    fn name(&self) -> &'static str {
        "ExecutionGraphModule"
    }

    // Java: com/google/devtools/build/lib/runtime/ExecutionGraphModule.java:195 ExecutionGraphModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![ExecutionGraphModuleExecutionGraphOptions::class()]
        } else {
            vec![]
        }
    }
}

/// A module that contains the built-in commands (`BuiltinCommandModule` constructed with a
/// `RunCommand` that uses `StandaloneTestStrategy.DEFAULT_LOCAL_POLICY`).
// Java: com/google/devtools/build/lib/bazel/BazelBuiltinCommandModule.java:21 BazelBuiltinCommandModule
pub struct BazelBuiltinCommandModule;

impl BlazeModule for BazelBuiltinCommandModule {
    fn name(&self) -> &'static str {
        "BazelBuiltinCommandModule"
    }

    // Java: com/google/devtools/build/lib/runtime/commands/BuiltinCommandModule.java:30 BuiltinCommandModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        let commands: Vec<Arc<dyn BlazeCommand>> = vec![
            Arc::new(BuildCommand),
            Arc::new(CanonicalizeCommand),
            Arc::new(crate::commands::clean::CleanCommand::new()),
            Arc::new(CoverageCommand),
            Arc::new(DumpCommand),
            Arc::new(crate::commands::help::HelpCommand::new()),
            Arc::new(crate::commands::info::InfoCommand::new()),
            Arc::new(PrintActionCommand),
            Arc::new(QueryCommand),
            Arc::new(RunCommand::new(
                blaze_exec::exec::standalone_test_strategy::default_local_policy(),
            )),
            Arc::new(crate::commands::shutdown::ShutdownCommand::new()),
            Arc::new(TestCommand),
            Arc::new(crate::commands::version::VersionCommand::new()),
            Arc::new(AqueryCommand),
            Arc::new(CqueryCommand),
            Arc::new(ConfigCommand),
        ];
        builder.add_commands(commands);
        // Only enable the "license" command when this binary has an embedded LICENSE file.
        if LicenseCommand::is_supported() {
            let license: [Arc<dyn BlazeCommand>; 1] = [Arc::new(LicenseCommand)];
            builder.add_commands(license);
        }
        Ok(())
    }
}

/// Module that provides implementations of the include scanning action context.
/// PORT: registerActionContexts, executorInit, beforeCommand, afterCommand, workspaceInit are
/// not ported here: include scanning belongs to the C++ compile action (`blaze_rules::cpp`).
// Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:81 IncludeScanningModule
pub struct IncludeScanningModule;

impl BlazeModule for IncludeScanningModule {
    fn name(&self) -> &'static str {
        "IncludeScanningModule"
    }

    // Java: com/google/devtools/build/lib/includescanning/IncludeScanningModule.java:120 IncludeScanningModule.getCommandOptions
    fn get_command_options(&self, command_name: &str) -> Vec<OptionsClassRef> {
        if command_name == "build" {
            vec![IncludeScanningOptions::class()]
        } else {
            vec![]
        }
    }
}

/// Sets up Skymeld (merged analysis and execution).
/// PORT: beforeCommand (whether the request merges analysis and execution) is ported with the
/// build command (`crate::commands::build::determine_if_merging_analysis_execution`).
// Java: com/google/devtools/build/lib/skyframe/SkymeldModule.java:25 SkymeldModule
pub struct SkymeldModule;

impl BlazeModule for SkymeldModule {
    fn name(&self) -> &'static str {
        "SkymeldModule"
    }
}

/// A module for Skyframe serialization.
/// PORT: skipped, incremental (workspaceInit, commandComplete).
// Java: com/google/devtools/build/lib/skyframe/serialization/SerializationModule.java:29 SerializationModule
pub struct SerializationModule;

impl BlazeModule for SerializationModule {
    fn name(&self) -> &'static str {
        "SerializationModule"
    }
}

/// A module that waits for background tasks submitted with `submit` at the end of a command.
/// PORT: beforeCommand, afterCommand are not ported: they run and await the background
/// tasks other modules submit (the remote module's uploads), and no ported module submits any.
// Java: com/google/devtools/build/lib/runtime/BlockWaitingModule.java:30 BlockWaitingModule
pub struct BlockWaitingModule;

impl BlazeModule for BlockWaitingModule {
    fn name(&self) -> &'static str {
        "BlockWaitingModule"
    }
}

/// Bazel's BES module (`BuildEventServiceModule<BuildEventServiceOptions>`).
/// beforeCommand and afterCommand set up and close the Build Event Protocol of a command
/// (`crate::commands::bep::BepCommandSupport`); the upload to a Build Event Service
/// (`--bes_backend`: getBesKeywords, getBesClient, clearBesClient, getInvocationIdPrefix,
/// getBuildRequestIdPrefix) is `blaze_bes::module`. getOutputListener returns the command's BEP output listener (stdout and stderr in the BEP's
/// progress events). blazeShutdownOnCrash closes the command's streamer with an abort reason
/// (`BepCommandSupport::blaze_shutdown_on_crash`).
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:52 BazelBuildEventServiceModule
#[derive(Default)]
pub struct BazelBuildEventServiceModule {
    /// The command's streamer support, between beforeCommand and afterCommand (Java: the
    /// module's `streamer`, `bepTransports`, ... fields, reset by `commandComplete`).
    current: parking_lot::Mutex<Option<Arc<crate::commands::bep::BepCommandSupport>>>,
    /// `besStreamOptions.keepBackendConnections` of the command (beforeCommand reads the
    /// options for every command, before the allowed-commands check).
    keep_backend_connections: std::sync::atomic::AtomicBool,
}

impl BlazeModule for BazelBuildEventServiceModule {
    fn name(&self) -> &'static str {
        "BazelBuildEventServiceModule"
    }

    /// `optionsClass()` is `BuildEventServiceOptions` for Bazel.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:195 BuildEventServiceModule.getCommonCommandOptions
    // Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:92 BazelBuildEventServiceModule.optionsClass
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:945 BuildEventServiceModule.optionsClass
    // workspaceInit looks the CredentialModule up for getBesClient; the port's credential cache
    // is a process global (`credential_module()`).
    // PORT: skipped, a process global replaces the lookup: BazelBuildEventServiceModule.workspaceInit
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![
            BuildEventServiceOptions::class(),
            AuthAndTLSOptions::class(),
            BuildEventStreamOptions::class(),
            BuildEventProtocolOptions::class(),
        ]
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:337 BuildEventServiceModule.beforeCommand
    fn before_command(&self, env: &mut CommandEnvironment) -> Result<(), AbruptExitException> {
        // this.besStreamOptions = parsingResult.getOptions(BuildEventStreamOptions.class)
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:353 BuildEventServiceModule.beforeCommand
        self.keep_backend_connections.store(
            env.get_options()
                .get_options::<BuildEventStreamOptions>()
                .is_some_and(|o| o.keep_backend_connections),
            std::sync::atomic::Ordering::SeqCst,
        );
        let bep = crate::commands::bep::BepCommandSupport::module_before_command(env);
        *self.current.lock() = bep.clone();
        env.set_bep(bep);
        Ok(())
    }

    /// `outErr`: the stream pair the command's streamer turns into the BEP's progress events
    /// (null when the command streams no build events).
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:471 BuildEventServiceModule.getOutputListener
    fn get_output_listener(&self, env: &CommandEnvironment) -> Option<blaze_events::OutErr> {
        env.get_bep().map(|bep| bep.get_output_listener().clone())
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:662 BuildEventServiceModule.afterCommand
    fn after_command(&self) -> Result<(), AbruptExitException> {
        // commandComplete: the module forgets the command's streamer.
        // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:690 BuildEventServiceModule.commandComplete
        let Some(bep) = self.current.lock().take() else {
            // Without a streamer: `if (!besStreamOptions.keepBackendConnections)
            // clearBesClient()` (the streaming commands do it at the end of their own
            // afterCommand, `BepCommandSupport::after_command`).
            // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:682 BuildEventServiceModule.afterCommand
            if !self
                .keep_backend_connections
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                blaze_bes::module::clear_bes_client();
            }
            return Ok(());
        };
        bep.after_command()
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:512 BuildEventServiceModule.blazeShutdown
    fn blaze_shutdown(&self) {
        crate::commands::bep::blaze_shutdown_pending_uploads();
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:500 BuildEventServiceModule.blazeShutdownOnCrash
    fn blaze_shutdown_on_crash(&self, exit_code: &crate::exit_code::DetailedExitCode) {
        // `if (streamer != null)`: the command's streamer, kept until afterCommand (which a crash
        // skips).
        let bep = self.current.lock().clone();
        if let Some(bep) = bep {
            bep.blaze_shutdown_on_crash(exit_code);
        }
    }
}

/// Module to support "blaze info peak-heap-size".
/// PORT: skipped, memory instrumentation (beforeCommand resets the recorder's peak heap).
// Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:231 PostGCMemoryUseRecorder.PostGCMemoryUseRecorderModule
pub struct PostGCMemoryUseRecorderPostGCMemoryUseRecorderModule;

impl BlazeModule for PostGCMemoryUseRecorderPostGCMemoryUseRecorderModule {
    fn name(&self) -> &'static str {
        "PostGCMemoryUseRecorderModule"
    }

    // Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:233 PostGCMemoryUseRecorder.PostGCMemoryUseRecorderModule.serverInit
    fn server_init(
        &self,
        _startup_options: &OptionsParsingResult,
        builder: &mut ServerBuilder,
    ) -> Result<(), AbruptExitException> {
        let item: Arc<dyn InfoItem> =
            Arc::new(crate::commands::info::module_items::PeakMemInfoItem::new());
        builder.add_info_items([item]);
        Ok(())
    }
}

/// Module to run a full GC after a build is complete on a Blaze server.
/// PORT: skipped, memory instrumentation (beforeCommand, afterCommand force a GC).
// Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:265 PostGCMemoryUseRecorder.GcAfterBuildModule
pub struct PostGCMemoryUseRecorderGcAfterBuildModule;

impl BlazeModule for PostGCMemoryUseRecorderGcAfterBuildModule {
    fn name(&self) -> &'static str {
        "GcAfterBuildModule"
    }

    // Java: com/google/devtools/build/lib/metrics/PostGCMemoryUseRecorder.java:283 PostGCMemoryUseRecorder.GcAfterBuildModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![PostGCMemoryUseRecorderGcAfterBuildModuleOptions::class()]
    }
}

/// A blaze module that installs metrics instrumentation and posts the `BuildMetricsEvent`
/// (`postsBuildMetricsEvent` is true).
/// PORT: beforeCommand (`MetricsCollector.installInEnv`) is ported with the BEP of build and
/// test (`crate::commands::bep::BepCommandSupport::before_command`).
// Java: com/google/devtools/build/lib/metrics/MetricsModule.java:29 MetricsModule
pub struct MetricsModule;

impl MetricsModule {
    /// The module's `numAnalyses` and `numBuilds` (`AtomicInteger`s that live as long as the
    /// module, that is the server), handed to each command's `MetricsCollector`.
    /// PORT: the module object is not reachable from the command's BEP support, which installs
    /// the collector; the counters are the process's, and there is one runtime (one
    /// `MetricsModule`) per server process.
    // Java: com/google/devtools/build/lib/metrics/MetricsModule.java:60 MetricsModule.numAnalyses
    pub(crate) fn counters() -> (
        Arc<std::sync::atomic::AtomicI32>,
        Arc<std::sync::atomic::AtomicI32>,
    ) {
        static COUNTERS: std::sync::OnceLock<(
            Arc<std::sync::atomic::AtomicI32>,
            Arc<std::sync::atomic::AtomicI32>,
        )> = std::sync::OnceLock::new();
        let (num_analyses, num_builds) = COUNTERS.get_or_init(|| {
            (
                Arc::new(std::sync::atomic::AtomicI32::new(0)),
                Arc::new(std::sync::atomic::AtomicI32::new(0)),
            )
        });
        (num_analyses.clone(), num_builds.clone())
    }
}

impl BlazeModule for MetricsModule {
    fn name(&self) -> &'static str {
        "MetricsModule"
    }

    // Java: com/google/devtools/build/lib/metrics/MetricsModule.java:64 MetricsModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![MetricsModuleOptions::class()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a Java test: `CacheFileDigestsModule`'s hit rate (`%g`); the expected strings are the
    /// oracle JDK's `String.format("%g", d)`.
    #[test]
    fn java_format_g_matches_java() {
        for (d, expected) in [
            (0.0, "0.00000"),
            (1.0, "1.00000"),
            (0.5, "0.500000"),
            (1.0 / 3.0, "0.333333"),
            (0.9999995, "1.00000"),
            (1.0 / 1024.0, "0.000976563"),
            (5e-05, "5.00000e-05"),
            (0.000123456789, "0.000123457"),
            (0.1234565, "0.123457"),
            (2.0 / 3.0, "0.666667"),
        ] {
            assert_eq!(java_format_g(d), expected, "{d}");
        }
    }

    #[test]
    fn module_names_in_bazel_modules_order() {
        let names: Vec<&str> = bazel_modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "BazelStartupOptionsModule",
                "CommandProfilerModule",
                "NoSpawnCacheModule",
                "CredentialModule",
                "CommandLogModule",
                "MemoryPressureModule",
                "ThreadDumpModule",
                "SleepPreventionModule",
                "SystemSuspensionModule",
                "BazelFileSystemModule",
                "MobileInstallModule",
                "BazelWorkspaceStatusModule",
                "BazelDiffAwarenessModule",
                "RemoteModule",
                "BazelRepositoryModule",
                "StarlarkRepositoryDebugModule",
                "WorkspaceRuleModule",
                "BazelCoverageReportModule",
                "StarlarkDebuggerModule",
                "CacheHitReportingModule",
                "SpawnLogModule",
                "BazelLockFileModule",
                "OutputFilteringModule",
                "WorkerModule",
                "CacheFileDigestsModule",
                "StandaloneModule",
                "SandboxModule",
                "BuildSummaryStatsModule",
                "DynamicExecutionModule",
                "BazelRulesModule",
                "BazelStrategyModule",
                "NoOpConnectivityModule",
                "AllocationTrackerModule",
                "PackageMetricsModule",
                "ExecutionGraphModule",
                "BazelBuiltinCommandModule",
                "IncludeScanningModule",
                "SkymeldModule",
                "SerializationModule",
                "BlockWaitingModule",
                "BazelBuildEventServiceModule",
                "PostGCMemoryUseRecorderModule",
                "GcAfterBuildModule",
                "MetricsModule",
            ]
        );
    }

    #[test]
    fn startup_options_parser_builds() {
        let classes = crate::runtime::BlazeRuntime::get_startup_options(&bazel_modules());
        blaze_options::OptionsParser::builder()
            .options_classes(&classes)
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    #[test]
    fn info_items_registered_by_modules() {
        let startup_options = blaze_options::OptionsParser::builder()
            .build()
            .unwrap_or_else(|e| e.throw());
        let mut builder = ServerBuilder::new();
        for module in bazel_modules() {
            module
                .server_init(&startup_options, &mut builder)
                .unwrap_or_else(|e| panic!("{}", e.get_message()));
        }
        let names: Vec<&str> = builder
            .get_info_items()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(names, ["command_log", "repository_cache", "peak-heap-size"]);
    }
}
