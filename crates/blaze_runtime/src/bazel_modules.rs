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
        Arc::new(CommandLogModule),
        Arc::new(MemoryPressureModule),
        Arc::new(ThreadDumpModule),
        Arc::new(SleepPreventionModule),
        Arc::new(SystemSuspensionModule),
        Arc::new(BazelFileSystemModule),
        Arc::new(MobileInstallModule),
        Arc::new(BazelWorkspaceStatusModule),
        Arc::new(BazelDiffAwarenessModule),
        Arc::new(RemoteModule),
        Arc::new(BazelRepositoryModule),
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
        Arc::new(PackageMetricsModule),
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
        Arc::new(BazelBuildEventServiceModule),
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

/// A module to manage credentials.
/// PORT: skipped, incremental (beforeCommand resets the credential cache the server keeps
/// between commands). The credential helpers of a command are set up with its repository
/// functions (`blaze_repo_rules::setup`).
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
}

/// This module logs complete stdout / stderr output of Blaze to a file.
/// PORT: getOutputListener is in the dispatcher (`BlazeCommandDispatcher::exec_exclusively`):
/// the command log of a previous command is unlinked; writing one (`--write_command_log`,
/// commandComplete) is not ported and fails loudly (`crate::unported_options`).
// Java: com/google/devtools/build/lib/runtime/CommandLogModule.java:34 CommandLogModule
pub struct CommandLogModule;

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
/// PORT: workspaceInit, initializeRuleClasses, beforeCommand, afterCommand and
/// getPrecomputedValues are ported with the command's executor
/// (`blaze_repo_rules::setup::prepare_repository_functions_with`, called by
/// `blaze_sky_top::skyframe_executor::SkyframeExecutor::create`).
// Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:127 BazelRepositoryModule
pub struct BazelRepositoryModule;

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
}

/// Module for the Starlark repository debugger.
/// PORT: beforeCommand, afterCommand are not ported: the module prints a repository's
/// definition when a `RequestRepositoryInformationEvent` asks for it, and no ported code posts
/// that event.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:31 StarlarkRepositoryDebugModule
pub struct StarlarkRepositoryDebugModule;

impl BlazeModule for StarlarkRepositoryDebugModule {
    fn name(&self) -> &'static str {
        "StarlarkRepositoryDebugModule"
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
/// PORT: getCoverageReportFactory is not ported; a `test` that collects coverage with a
/// combined report fails loudly (`crate::commands::build`).
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
/// PORT: registerSpawnStrategies is ported with the command's execution setup
/// (`blaze_exec::exec::executor_setup` registers the `worker` strategy with the command's
/// `WorkerOptions`; running a spawn with a persistent worker fails loudly there). PORT: skipped,
/// incremental: beforeCommand, afterCommand manage the worker pool the server keeps between
/// commands.
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
}

/// Enables the caching of file digests in `DigestUtils`.
/// PORT: skipped, incremental (executorInit, commandComplete size the digest cache the server
/// keeps between commands and log its statistics).
// Java: com/google/devtools/build/lib/runtime/CacheFileDigestsModule.java:26 CacheFileDigestsModule
pub struct CacheFileDigestsModule;

impl BlazeModule for CacheFileDigestsModule {
    fn name(&self) -> &'static str {
        "CacheFileDigestsModule"
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
/// PORT: getPackageLoadingListener, beforeCommand, afterCommand are not ported: the package
/// metrics only reach the server log and, with
/// `--experimental_publish_package_metrics_in_bep`, the BEP, where they fail loudly
/// (`crate::commands::bep`).
// Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:31 PackageMetricsModule
pub struct PackageMetricsModule;

impl BlazeModule for PackageMetricsModule {
    fn name(&self) -> &'static str {
        "PackageMetricsModule"
    }

    // Java: com/google/devtools/build/lib/packages/metrics/PackageMetricsModule.java:82 PackageMetricsModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![PackageMetricsModuleOptions::class()]
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
            Arc::new(RunCommand),
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
/// PORT: the inherited beforeCommand, afterCommand and commandComplete are ported for the BEP
/// files of build and test (`crate::commands::bep`); a BEP file for another command and the
/// upload to a Build Event Service (`--bes_backend`: workspaceInit, getBesKeywords,
/// getBesClient, clearBesClient, allowedCommands, getInvocationIdPrefix,
/// getBuildRequestIdPrefix) fail loudly (`crate::unported_options`). getOutputListener (the
/// command's stdout and stderr in the BEP's progress events) is not ported: progress events
/// carry no output. blazeShutdown, blazeShutdownOnCrash close a streamer the server left open.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:52 BazelBuildEventServiceModule
pub struct BazelBuildEventServiceModule;

impl BlazeModule for BazelBuildEventServiceModule {
    fn name(&self) -> &'static str {
        "BazelBuildEventServiceModule"
    }

    /// `optionsClass()` is `BuildEventServiceOptions` for Bazel.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:195 BuildEventServiceModule.getCommonCommandOptions
    fn get_common_command_options(&self) -> Vec<OptionsClassRef> {
        vec![
            BuildEventServiceOptions::class(),
            AuthAndTLSOptions::class(),
            BuildEventStreamOptions::class(),
            BuildEventProtocolOptions::class(),
        ]
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
