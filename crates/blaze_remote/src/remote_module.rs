//! Port of the per-command setup of `lib/remote/RemoteModule.java` (`beforeCommand` / `setup`,
//! `createChannel`, `initHttpAndDiskCache`, `afterCommand`) and of
//! `lib/remote/RemoteActionContextProvider.java`.
//!
//! PORT: `RemoteModule` is a `BlazeModule` that reads a `CommandEnvironment`; `blaze_runtime`
//! is above this crate, so the values Java reads from the environment are the fields of
//! [`RemoteModuleEnvironment`] and the integrator calls [`RemoteModule::before_command`] where
//! Bazel calls `beforeCommand`, [`RemoteActionContextProvider::get_spawn_cache`] /
//! [`RemoteActionContextProvider::get_remote_spawn_strategy`] where it calls
//! `registerSpawnCache` / `registerRemoteSpawnStrategy`, and [`RemoteModule::after_command`] at
//! the end. Not ported from the module (each fails loudly or is a documented no-op): the remote
//! output service (`--experimental_remote_output_service`), the remote downloader
//! (`--experimental_remote_downloader`), the BEP artifact uploader, the repository-rule remote
//! helpers, action rewinding, and the HTTP cache.

use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use blaze_events::ui::Clock;
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_exec::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use blaze_exec::exec::action_execution_context::{RemoteOutputTree, SpawnCache};
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_util::check_state;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path};
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::authandtls::call_credentials_provider::CallCredentialsProvider;
use crate::authandtls::google_auth_utils;
use crate::authandtls::headers;
use crate::byte_stream_build_event_artifact_uploader::ByteStreamBuildEventArtifactUploaderFactory;
use crate::circuitbreaker::circuit_breaker_factory;
use crate::combined_cache::CombinedCache;
use crate::combined_cache_client_factory;
use crate::common::errors::{self, StatusRuntimeException, is_interrupted};
use crate::common::remote_path_resolver::{
    DefaultRemotePathResolver, RemotePathResolver, SiblingRepositoryLayoutResolver,
};
use crate::common::{
    RemoteCacheClient, RemoteExecutionCapabilitiesException, RemoteExecutionClient,
};
use crate::disk::DiskCacheClient;
use crate::grpc::channel::CapabilitiesFetcher;
use crate::grpc::{ClientInterceptor, ReferenceCountedChannel};
use crate::grpc_cache_client::GrpcCacheClient;
use crate::grpc_remote_executor::GrpcRemoteExecutor;
use crate::hooks::RemoteHooks;
use crate::logging::logging_interceptor::LoggingInterceptor;
use crate::logging::{FileLogEntrySink, LogEntrySink};
use crate::reapi::{Digest, digest_function};
use crate::remote_action_input_fetcher::RemoteActionInputFetcher;
use crate::remote_execution_cache::RemoteExecutionCache;
use crate::remote_execution_service::RemoteExecutionService;
use crate::remote_grpc_service_config;
use crate::remote_output_checker::RemoteOutputChecker;
use crate::remote_retrier::{self, RemoteRetrier};
use crate::remote_server_capabilities::{
    RemoteServerCapabilities, ServerCapabilitiesRequirement, check_client_server_compatibility,
};
use crate::remote_spawn_cache::RemoteSpawnCache;
use crate::remote_spawn_runner::RemoteSpawnRunner;
use crate::remote_spawn_strategy::new_remote_spawn_strategy;
use crate::retrier::{RetryScheduler, TokioRetryScheduler};
use crate::scrubber::Scrubber;
use crate::util::DigestUtil;
use crate::util::temp_path_generator::TempPathGenerator;
use crate::util::tracing_metadata_utils;
use blaze_repo_fetch::authandtls::credentialhelper::{
    CommandLinePathFactory, CredentialCache, CredentialHelperEnvironment, credential_module,
};

/// What `RemoteModule` reads from the `CommandEnvironment` of a command.
pub struct RemoteModuleEnvironment {
    /// `env.getCommandName()`.
    pub command_name: String,
    /// `env.getOptions().getOptions(RemoteOptions.class)`.
    pub remote_options: RemoteOptions,
    /// `env.getOptions().getOptions(AuthAndTLSOptions.class)`.
    pub auth_and_tls_options: AuthAndTLSOptions,
    /// `env.getOptions().getOptions(ExecutionOptions.class)`.
    pub execution_options: Arc<ExecutionOptions>,
    /// `BuildRequestOptions.jobs` (unused: the runtime is shared) and
    /// `BuildRequestOptions.rewindLostInputs`; `None` when the command has no
    /// `BuildRequestOptions`.
    pub rewind_lost_inputs: Option<bool>,
    /// `BuildLanguageOptions.experimentalSiblingRepositoryLayout`.
    pub experimental_sibling_repository_layout: bool,
    /// `env.getReporter()`.
    pub reporter: Arc<dyn ExtendedEventHandler>,
    /// `env.getRuntime().getFileSystem()`.
    pub file_system: FileSystemRef,
    /// `env.getOutputBase()`.
    pub output_base: Path,
    /// `env.getExecRoot()`.
    pub exec_root: Path,
    /// `env.getWorkingDirectory()`.
    pub working_directory: Path,
    /// `env.getWorkspace()`.
    pub workspace: Option<Path>,
    /// `env.getDirectories().getInstallBase()` (a root of `env.getCommandLinePathFactory()`).
    pub install_base: Option<Path>,
    /// `env.getWorkspaceName()`.
    pub workspace_name: String,
    /// `env.getActionTempsDirectory()`.
    pub action_temps_directory: Path,
    /// `env.getBuildRequestId()`.
    pub build_request_id: String,
    /// `env.getCommandId().toString()`.
    pub command_id: String,
    /// `env.getRuntime().getProductName()`.
    pub product_name: String,
    /// `BlazeVersionInfo.instance().getVersion()`.
    pub tool_version: String,
    /// `env.getClientEnv()`.
    pub client_env: IndexMap<String, String>,
    /// `env.getRuntime().getClock()`.
    pub clock: Arc<dyn Clock>,
    /// `remoteOptions.scrubber` (the parsed `--experimental_remote_scrubbing_config`).
    pub scrubber: Option<Arc<Scrubber>>,
    /// The integration seams of this build (`crate::hooks`).
    pub hooks: RemoteHooks,
}

/// `RemoteModule.lastRemoteOutputChecker`: the checker of the previous command of the server
/// (`None` after a command without remote options).
///
/// PORT: Java's module lives with the server and keeps it in a field; Blaze builds the module
/// per command, so the field is this process-wide cell.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:159 RemoteModule.lastRemoteOutputChecker
static LAST_REMOTE_OUTPUT_CHECKER: parking_lot::Mutex<LastRemoteOutputChecker> =
    parking_lot::Mutex::new(LastRemoteOutputChecker {
        checker: None,
        set_this_command: false,
    });

struct LastRemoteOutputChecker {
    checker: Option<Arc<RemoteOutputChecker>>,
    /// The module's `afterCommand` of the current command recorded its checker.
    set_this_command: bool,
}

/// The end of every command (`RemoteModule.afterCommand` runs for every command in Java): a
/// command whose module recorded no checker leaves none (`lastRemoteOutputChecker =
/// remoteOutputChecker`, which is null without remote options).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1120 RemoteModule.afterCommand
pub fn end_command_for_last_remote_output_checker() {
    let mut last = LAST_REMOTE_OUTPUT_CHECKER.lock();
    if !last.set_this_command {
        last.checker = None;
    }
    last.set_this_command = false;
}

/// `RemoteModule`, as far as one command goes.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:139 RemoteModule
pub struct RemoteModule {
    action_context_provider: Option<RemoteActionContextProvider>,
    remote_output_checker: Option<Arc<RemoteOutputChecker>>,
    temp_path_generator: Option<Arc<TempPathGenerator>>,
    rpc_log_file: Option<Arc<dyn LogEntrySink>>,
    cache_available: bool,
    /// The remote output tree of the build (see [`RemoteModule::get_remote_output_tree`]).
    remote_output_tree: Option<Arc<RemoteOutputTree>>,
    /// `actionInputFetcher` (see [`RemoteModule::create_action_input_fetcher`]).
    action_input_fetcher: Option<Arc<RemoteActionInputFetcher>>,
    /// `RemoteOutputService.rewindLostInputs` (`buildRequestOptions != null &&
    /// buildRequestOptions.rewindLostInputs`).
    rewind_lost_inputs: bool,
    /// `RemoteOutputService.rewoundActionSynchronizer` when it is a
    /// `RemoteRewoundActionSynchronizer` (see [`RemoteModule::create_action_input_fetcher`]).
    rewound_action_synchronizer:
        Option<Arc<crate::remote_rewound_action_synchronizer::RemoteRewoundActionSynchronizer>>,
    /// `env.getExecRoot()`, for `createActionInputFetcher`.
    exec_root: Option<Path>,
    /// `env.getBuildRequestId()` / `env.getCommandId()`, for `createActionInputFetcher`.
    build_request_id: String,
    command_id: String,
    /// `buildEventArtifactUploaderFactoryDelegate`'s factory: the ByteStream build event artifact
    /// uploader factory `setup` installed (a gRPC cache), until `afterCommand` resets it.
    build_event_artifact_uploader_factory: Option<Arc<ByteStreamBuildEventArtifactUploaderFactory>>,
    /// The disk cache garbage collection task `beforeCommand` registers with `env.addIdleTask`
    /// (see [`RemoteModule::take_disk_cache_gc_idle_task`]).
    disk_cache_gc_idle_task: Option<crate::disk::DiskCacheGarbageCollectorIdleTask>,
}

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1370 RemoteModule.createOptionsExitException
fn create_options_exit_exception(
    message: impl Into<String>,
    remote_execution_code: failure_details::remote_options::Code,
) -> AbruptExitException {
    AbruptExitException::new(FailureDetail {
        message: message.into(),
        category: Some(failure_detail::Category::RemoteOptions(
            failure_details::RemoteOptions {
                code: remote_execution_code as i32,
            },
        )),
    })
}

/// The `ExitCode`s `RemoteModule` passes explicitly (`DetailedExitCode.of(exitCode,
/// failureDetail)`).
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitCode {
    /// `ExitCode.COMMAND_LINE_ERROR` (2).
    COMMAND_LINE_ERROR,
    /// `ExitCode.LOCAL_ENVIRONMENTAL_ERROR` (36).
    LOCAL_ENVIRONMENTAL_ERROR,
}

impl ExitCode {
    /// `getNumericExitCode()`.
    fn get_numeric_exit_code(self) -> i32 {
        match self {
            ExitCode::COMMAND_LINE_ERROR => 2,
            ExitCode::LOCAL_ENVIRONMENTAL_ERROR => 36,
        }
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1381 RemoteModule.createExitException
fn create_exit_exception(
    message: impl Into<String>,
    exit_code: ExitCode,
    remote_execution_code: failure_details::remote_execution::Code,
) -> AbruptExitException {
    AbruptExitException::with_exit_code(
        exit_code.get_numeric_exit_code(),
        FailureDetail {
            message: message.into(),
            category: Some(failure_detail::Category::RemoteExecution(
                failure_details::RemoteExecution {
                    code: remote_execution_code as i32,
                },
            )),
        },
    )
}

/// `handleInitFailure(env, e, code)`: reports the error and makes the command exit with
/// "Error initializing RemoteModule" (`ExitCode.COMMAND_LINE_ERROR`).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1048 RemoteModule.handleInitFailure
fn handle_init_failure(
    reporter: &dyn EventHandler,
    e: &io::Error,
    remote_execution_code: failure_details::remote_execution::Code,
) -> AbruptExitException {
    reporter.handle(&Event::error(e.to_string()));
    create_exit_exception(
        "Error initializing RemoteModule",
        ExitCode::COMMAND_LINE_ERROR,
        remote_execution_code,
    )
}

fn is_null_or_empty(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(str::is_empty)
}

/// `Strings.emptyToNull`.
fn empty_to_null(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

impl RemoteModule {
    /// Returns whether remote execution should be available.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:235 RemoteModule.shouldEnableRemoteExecution
    pub fn should_enable_remote_execution(options: &RemoteOptions) -> bool {
        !is_null_or_empty(&options.remote_executor)
    }

    /// `beforeCommand(env)`: sets up all requested remote functionality.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:413 RemoteModule.beforeCommand
    pub fn before_command(
        env: RemoteModuleEnvironment,
    ) -> Result<RemoteModule, AbruptExitException> {
        let mut module = RemoteModule {
            action_context_provider: None,
            remote_output_checker: None,
            temp_path_generator: None,
            rpc_log_file: None,
            cache_available: false,
            remote_output_tree: None,
            action_input_fetcher: None,
            rewind_lost_inputs: env.rewind_lost_inputs.unwrap_or(false),
            rewound_action_synchronizer: None,
            exec_root: Some(env.exec_root.clone()),
            build_request_id: env.build_request_id.clone(),
            command_id: env.command_id.clone(),
            build_event_artifact_uploader_factory: None,
            disk_cache_gc_idle_task: None,
        };
        // PORT: `knownMissingCasDigests` survives across commands in the Bazel server (cleared
        // by `clean`); one command per process here.
        module.cache_available = module.setup(env)?;
        Ok(module)
    }

    /// The disk cache garbage collection idle task `beforeCommand` created (Java registers it
    /// with `env.addIdleTask`), once.
    pub fn take_disk_cache_gc_idle_task(
        &mut self,
    ) -> Option<crate::disk::DiskCacheGarbageCollectorIdleTask> {
        self.disk_cache_gc_idle_task.take()
    }

    /// Whether any cache (disk or remote) is enabled (the result of `setup`).
    pub fn cache_available(&self) -> bool {
        self.cache_available
    }

    /// The `--experimental_remote_cache_lease_extension` part of `executorInit`, which Java runs
    /// for the commands that execute actions, after `beforeCommand`: with a combined cache and
    /// `--rewind_lost_inputs` (`rewind_lost_inputs`: `buildRequestOptions != null &&
    /// buildRequestOptions.rewindLostInputs`) the option has no effect and a warning says so;
    /// without rewinding it creates a `RemoteLeaseExtension`, which is not ported (it needs the
    /// Skyframe evaluator and the persistent action cache). The `LeaseService` of the same
    /// branch is the caller's (`blaze_runtime` `build_remote.rs`).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1241 RemoteModule.executorInit
    pub fn executor_init(&self, reporter: &dyn EventHandler, rewind_lost_inputs: bool) {
        let Some(action_context_provider) = &self.action_context_provider else {
            return;
        };
        if action_context_provider.get_combined_cache().is_none() {
            return;
        }
        // checkNotNull(actionInputFetcher); actionContextProvider.setActionInputFetcher(..)
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1257 RemoteModule.executorInit
        let action_input_fetcher = self
            .action_input_fetcher
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: actionInputFetcher must not be null"));
        action_context_provider.set_action_input_fetcher(action_input_fetcher);
        let remote_options = &action_context_provider.remote_options;
        if rewind_lost_inputs {
            // Action rewinding regenerates lost inputs within the build, so there is no need for
            // the lease service to extend leases or to discard all remote metadata after a build
            // that encountered lost inputs.
            if remote_options.remote_cache_lease_extension {
                reporter.handle(&Event::warn(
                    "--experimental_remote_cache_lease_extension has no effect since --rewind_lost_inputs is enabled, which recovers from lost remote cache entries as they are encountered.",
                ));
            }
        } else if remote_options.remote_cache_lease_extension {
            blaze_util::unported!(
                "RemoteLeaseExtension",
                "--experimental_remote_cache_lease_extension"
            );
        }
    }

    /// `actionContextProvider`.
    pub fn get_action_context_provider(&self) -> Option<&RemoteActionContextProvider> {
        self.action_context_provider.as_ref()
    }

    /// `remoteOutputChecker`.
    pub fn get_remote_output_checker(&self) -> Option<&Arc<RemoteOutputChecker>> {
        self.remote_output_checker.as_ref()
    }

    /// The remote output tree into which `RemoteExecutionService.downloadOutputs` injects the
    /// outputs it does not download, when `setup` enabled a cache and the integrator gave no
    /// `RemoteHooks.remote_output_injector` of its own. Register it as an action context so the
    /// output metadata stores consult it (`blaze_exec::exec::action_execution_context
    /// ::RemoteOutputTree`).
    ///
    /// PORT: stands for the remote output trees of the `RemoteActionFileSystem`s that
    /// `RemoteOutputService` creates per action.
    pub fn get_remote_output_tree(&self) -> Option<&Arc<RemoteOutputTree>> {
        self.remote_output_tree.as_ref()
    }

    /// `createActionInputFetcher(combinedCache)`: the `RemoteActionInputFetcher` of the command
    /// (`null` without a combined cache), kept as `actionInputFetcher`. `reporter` is
    /// `env.getReporter()`; `output_permissions` is `OutputPermissions.WRITABLE` with
    /// `--experimental_writable_outputs`, else `READONLY`; `output_directory_helper` is
    /// `env.getOutputDirectoryHelper()` (null without `BuildRequestOptions`).
    ///
    /// PORT: Java calls it at the end of `setup` (`beforeCommand`); the reporter of
    /// `blaze_runtime` is not part of [`RemoteModuleEnvironment`], so the integrator calls it
    /// right after [`RemoteModule::before_command`].
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:331 RemoteModule.createActionInputFetcher
    pub fn create_action_input_fetcher(
        &mut self,
        reporter: Arc<blaze_events::Reporter>,
        output_permissions: blaze_exec::actions::cache::OutputPermissions,
        output_directory_helper: Option<
            Arc<blaze_exec::actions::action_output_directory_helper::ActionOutputDirectoryHelper>,
        >,
    ) -> Option<Arc<RemoteActionInputFetcher>> {
        let combined_cache = self
            .action_context_provider
            .as_ref()
            .and_then(|p| p.get_combined_cache().cloned())?;
        let fetcher = crate::abstract_action_input_prefetcher::register_prefetcher(
            RemoteActionInputFetcher::new(
                reporter,
                &self.build_request_id,
                &self.command_id,
                combined_cache,
                self.exec_root.clone().expect("env.getExecRoot()"),
                self.temp_path_generator
                    .clone()
                    .expect("NullPointerException: tempPathGenerator"),
                self.remote_output_checker
                    .clone()
                    .expect("NullPointerException: remoteOutputChecker"),
                output_directory_helper,
                output_permissions,
            ),
        );
        self.action_input_fetcher = Some(fetcher.clone());
        // RemoteOutputService.setActionInputFetcher(actionInputFetcher, graph).
        // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:73 RemoteOutputService.setActionInputFetcher
        if self.rewind_lost_inputs {
            let synchronizer = Arc::new(
                crate::remote_rewound_action_synchronizer::RemoteRewoundActionSynchronizer::new(
                    fetcher.clone(),
                ),
            );
            if let Some(provider) = &self.action_context_provider {
                provider
                    .get_remote_execution_service()
                    .set_rewound_action_synchronizer(synchronizer.clone());
            }
            self.rewound_action_synchronizer = Some(synchronizer);
        }
        Some(fetcher)
    }

    /// `BuildEventArtifactUploaderFactoryDelegate.create(env)`: the uploader of the files the
    /// build event protocol refers to: the ByteStream uploader when `setup` found a gRPC cache,
    /// else a `LocalFilesArtifactUploader`. The build event module calls it through the
    /// "remote" factory it registers (`serverInit`), once per command for the first transport
    /// with path conversion.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1361 RemoteModule.BuildEventArtifactUploaderFactoryDelegate.create
    pub fn create_build_event_artifact_uploader(
        &self,
    ) -> Arc<dyn blaze_bep::buildeventstream::BuildEventArtifactUploader> {
        use blaze_bep::runtime::BuildEventArtifactUploaderFactory;
        match &self.build_event_artifact_uploader_factory {
            None => Arc::new(blaze_bep::buildeventstream::LocalFilesArtifactUploader),
            Some(uploader_factory) => uploader_factory.create(),
        }
    }

    /// The ByteStream build event artifact uploader factory of this command, if `setup`
    /// installed one (`buildEventArtifactUploaderFactoryDelegate.uploaderFactory`).
    pub fn get_build_event_artifact_uploader_factory(
        &self,
    ) -> Option<&Arc<ByteStreamBuildEventArtifactUploaderFactory>> {
        self.build_event_artifact_uploader_factory.as_ref()
    }

    /// `actionInputFetcher`.
    pub fn get_action_input_fetcher(&self) -> Option<&Arc<RemoteActionInputFetcher>> {
        self.action_input_fetcher.as_ref()
    }

    /// `outputService.getRewoundActionSynchronizer()` when it is the remote one (with
    /// `--rewind_lost_inputs` and an action input fetcher).
    // Java: com/google/devtools/build/lib/remote/RemoteOutputService.java:241 RemoteOutputService.getRewoundActionSynchronizer
    pub fn get_rewound_action_synchronizer(
        &self,
    ) -> Option<&Arc<crate::remote_rewound_action_synchronizer::RemoteRewoundActionSynchronizer>>
    {
        self.rewound_action_synchronizer.as_ref()
    }

    /// The salt of every action key (`ExecutorBuilder.setActionExecutionSalt`): whether a remote
    /// cache is enabled and the default exec properties.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1312 RemoteModule.computeActionExecutionSalt
    pub fn compute_action_execution_salt(remote_options: Option<&RemoteOptions>) -> String {
        let mut fp = blaze_util::fingerprint::Fingerprint::new();

        // When building without a remote cache following a build with one, cached actions may
        // reference remotely stored files which cannot be downloaded. For simplicity we also
        // invalidate in the reverse situation (building with a remote cache after building
        // without one) even though cached local actions don't need it.
        fp.add_boolean(remote_options.is_some_and(|o| o.is_remote_cache_enabled()));

        // The default exec properties may affect how a spawn is remotely executed without
        // affecting the action key. In practice, only spawns with no execution platform or whose
        // execution platform has no exec properties are affected, but we don't have access to
        // this information at the time we're computing the action key, so we unconditionally
        // include the defaults. This shouldn't be too bad, as we don't expect the defaults to
        // change very often.
        let defaults = remote_options
            .map(|o| o.get_remote_default_exec_properties())
            .unwrap_or_default();
        fp.add_string_map(defaults.iter().map(|(k, v)| (k.as_str(), v.as_str())));

        fp.hex_digest_and_reset()
    }

    /// Sets up all requested remote functionality (caching, execution, downloader, ...) and
    /// returns whether any cache (disk or remote) is enabled.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:437 RemoteModule.setup
    fn setup(&mut self, env: RemoteModuleEnvironment) -> Result<bool, AbruptExitException> {
        let RemoteModuleEnvironment {
            command_name,
            mut remote_options,
            auth_and_tls_options,
            execution_options,
            rewind_lost_inputs,
            experimental_sibling_repository_layout,
            reporter,
            file_system,
            output_base,
            exec_root,
            working_directory,
            workspace,
            install_base,
            workspace_name,
            action_temps_directory,
            build_request_id,
            command_id,
            product_name,
            tool_version,
            client_env,
            clock,
            scrubber,
            hooks,
        } = env;
        tracing_metadata_utils::set_tool_version(&tool_version);

        // Java: `env.getRuntime().getFileSystem().getDigestFunction()`.
        let hash_fn = DigestHashFunction::Sha256;
        let digest_util = DigestUtil::new(hash_fn);

        // If --remote_cache is empty but --remote_executor is not, reuse the latter for the
        // former.
        if !is_null_or_empty(&remote_options.remote_executor)
            && is_null_or_empty(&remote_options.remote_cache)
        {
            remote_options.remote_cache = remote_options.remote_executor.clone();
        }

        // Java: RemoteModule.shouldEnableRemoteOutputService
        if !is_null_or_empty(&remote_options.remote_output_service) {
            if combined_cache_client_factory::is_disk_cache(&remote_options) {
                remote_options.disk_cache = None;
                reporter.handle(&Event::warn(
                    "--disk_cache is ignored when --experimental_remote_output_service is set.",
                ));
            }

            if is_null_or_empty(&remote_options.remote_cache) {
                return Err(create_options_exit_exception(
                    "--experimental_remote_output_service must be used in combination with one of --remote_cache or --remote_executor.",
                    failure_details::remote_options::Code::ExecutionWithInvalidCache,
                ));
            }
        }

        let enable_disk_cache = combined_cache_client_factory::is_disk_cache(&remote_options);
        let enable_http_cache = combined_cache_client_factory::is_http_cache(&remote_options);
        let enable_remote_execution = RemoteModule::should_enable_remote_execution(&remote_options);
        let enable_grpc_cache = GrpcCacheClient::is_remote_cache_options(&remote_options);
        let enable_remote_downloader = !is_null_or_empty(&remote_options.remote_downloader);

        if enable_disk_cache {
            // Check that the disk cache directory, which is managed by a garbage collecting idle
            // task, does not contain the output base. Since the specified output base path may
            // be a symlink, we resolve it fully. Intermediate symlinks do not have to be checked
            // as the garbage collector ignores symlinks. We also resolve the disk cache
            // directory, where intermediate symlinks also don't matter since deletion only
            // occurs under the fully resolved path.
            let resolved_output_base = match output_base.resolve_symbolic_links() {
                Ok(p) => p,
                // Will be created later.
                Err(e) if e.kind() == io::ErrorKind::NotFound => output_base.clone(),
                Err(e) => {
                    return Err(create_options_exit_exception(
                        format!("Failed to resolve output base: {e}"),
                        failure_details::remote_options::Code::ExecutionWithInvalidCache,
                    ));
                }
            };
            let disk_cache = remote_options
                .disk_cache
                .as_ref()
                .expect("isDiskCache checked it");
            let unresolved_disk_cache = working_directory.get_relative(disk_cache.get());
            let resolved_disk_cache = match unresolved_disk_cache.resolve_symbolic_links() {
                Ok(p) => p,
                // Will be created later.
                Err(e) if e.kind() == io::ErrorKind::NotFound => unresolved_disk_cache,
                Err(e) => {
                    return Err(create_options_exit_exception(
                        format!("Failed to resolve disk cache directory: {e}"),
                        failure_details::remote_options::Code::ExecutionWithInvalidCache,
                    ));
                }
            };
            if resolved_output_base.starts_with(&resolved_disk_cache) {
                // This is dangerous as the disk cache GC may delete files in the output base.
                return Err(create_options_exit_exception(
                    format!(
                        "The output base [{}] cannot be a subdirectory of the --disk_cache directory [{}]",
                        resolved_output_base.get_path_string(),
                        resolved_disk_cache.get_path_string()
                    ),
                    failure_details::remote_options::Code::ExecutionWithInvalidCache,
                ));
            }
            // PORT: `env.addIdleTask(gcIdleTask)`: the module keeps the task, and the runtime,
            // which owns the command environment, registers it
            // (`take_disk_cache_gc_idle_task`).
            self.disk_cache_gc_idle_task = crate::disk::DiskCacheGarbageCollectorIdleTask::create(
                &remote_options,
                &working_directory,
            );
        }

        if enable_remote_downloader && !enable_grpc_cache {
            return Err(create_options_exit_exception(
                "The remote downloader can only be used in combination with gRPC caching",
                failure_details::remote_options::Code::DownloaderWithoutGrpcCache,
            ));
        }

        let temp_path_generator = Arc::new(get_temp_path_generator(
            &action_temps_directory,
            reporter.as_ref(),
        )?);
        self.temp_path_generator = Some(temp_path_generator.clone());

        let known_missing_cas_digests: Arc<Mutex<HashSet<Digest>>> =
            Arc::new(Mutex::new(HashSet::new()));
        let retry_scheduler: Arc<dyn RetryScheduler> = Arc::new(TokioRetryScheduler);
        let remote_options_arc_for = |o: &RemoteOptions| Arc::new(o.clone());

        // PORT: the remote output tree (see `get_remote_output_tree`) is created for a build
        // that enables a cache, unless the integrator provides its own injector.
        let mut hooks = hooks;
        let skyframe_delete_by_function_name = hooks.skyframe_delete_by_function_name.clone();
        let any_cache =
            enable_disk_cache || enable_http_cache || enable_grpc_cache || enable_remote_execution;
        if any_cache && hooks.remote_output_injector.is_none() {
            let tree = Arc::new(RemoteOutputTree::new());
            hooks.remote_output_injector = Some(tree.clone());
            self.remote_output_tree = Some(tree);
        }

        let provider_env = ProviderEnvironment {
            reporter: reporter.clone(),
            exec_root: exec_root.clone(),
            working_directory: working_directory.clone(),
            workspace_name: workspace_name.clone(),
            build_request_id: build_request_id.clone(),
            command_id: command_id.clone(),
            execution_options: execution_options.clone(),
            experimental_sibling_repository_layout,
            scrubber: scrubber.clone(),
            hooks,
        };

        if !enable_disk_cache
            && !enable_http_cache
            && !enable_grpc_cache
            && !enable_remote_execution
        {
            // Quit if no remote caching or execution was enabled.
            self.action_context_provider = Some(RemoteActionContextProvider::new(
                provider_env,
                remote_options_arc_for(&remote_options),
                /* combined_cache= */ None,
                /* remote_execution_cache= */ None,
                /* remote_executor= */ None,
                retry_scheduler,
                digest_util,
                /* log_dir= */ None,
                /* remote_output_checker= */ None,
                known_missing_cas_digests,
                temp_path_generator,
            ));
            return Ok(false);
        }

        if enable_http_cache && enable_remote_execution {
            return Err(create_options_exit_exception(
                "Cannot combine gRPC based remote execution with HTTP-based caching",
                failure_details::remote_options::Code::ExecutionWithInvalidCache,
            ));
        }

        let enable_scrubbing = scrubber.is_some();
        if enable_scrubbing && enable_remote_execution {
            reporter.handle(&Event::warn(
                "Cache key scrubbing is incompatible with remote execution. Actions that are scrubbed per the --experimental_remote_scrubbing_config configuration file will be executed locally instead.",
            ));
        }

        if digest_util.get_digest_function() == digest_function::Value::Unknown {
            return Err(AbruptExitException::new(FailureDetail {
                message: format!("Unsupported digest function: {hash_fn}"),
                category: Some(failure_detail::Category::Execution(
                    failure_details::Execution {
                        code: failure_details::execution::Code::ExecutionUnknown as i32,
                    },
                )),
            }));
        }

        let remote_output_checker =
            Arc::new(RemoteOutputChecker::new_with_last_remote_output_checker(
                &command_name,
                remote_options.remote_outputs_mode,
                RemoteOutputChecker::patterns_to_download(&remote_options),
                LAST_REMOTE_OUTPUT_CHECKER.lock().checker.clone(),
            ));
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:574 RemoteModule.beforeCommand
        if let Some(delete) = &skyframe_delete_by_function_name {
            remote_output_checker.maybe_invalidate_skyframe_values(&mut |predicate| {
                // The predicate lives only for this call: copy its verdict for the two
                // function names it can match (TARGET_COMPLETION, ASPECT_COMPLETION).
                let names: Vec<&'static str> = ["TARGET_COMPLETION", "ASPECT_COMPLETION"]
                    .into_iter()
                    .filter(|n| predicate(n))
                    .collect();
                delete(Arc::new(move |name: &str| names.contains(&name)));
            });
        }
        self.remote_output_checker = Some(remote_output_checker.clone());

        let invocation_id = command_id.clone();
        reporter.handle(&Event::info(format!("Invocation ID: {invocation_id}")));

        let log_dir = output_base.get_relative_str(&format!("{product_name}-remote-logs"));
        clean_and_create_remote_logs_dir(&log_dir)?;

        let command_line_path_factory = CommandLinePathFactory::create_for(
            file_system.clone(),
            workspace.clone(),
            install_base,
        );
        let credentials = match create_credentials(
            &CredentialHelperEnvironment {
                event_reporter: reporter.clone(),
                workspace_path: workspace,
                client_environment: client_env,
                helper_execution_timeout: auth_and_tls_options.credential_helper_timeout,
            },
            credential_module().get_credential_cache(),
            &command_line_path_factory,
            &file_system,
            &auth_and_tls_options,
            &remote_options,
        ) {
            Ok(c) => c,
            Err(e) => {
                return Err(handle_init_failure(
                    reporter.as_ref(),
                    &e,
                    failure_details::remote_execution::Code::CredentialsInitFailure,
                ));
            }
        };

        let circuit_breaker = circuit_breaker_factory::create_circuit_breaker(&remote_options);
        let retrier = Arc::new(RemoteRetrier::new(
            &remote_options,
            remote_retrier::EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
            retry_scheduler.clone(),
            circuit_breaker.clone(),
        ));

        let remote_grpc_service_config =
            match remote_grpc_service_config::create(&remote_options, &working_directory) {
                Ok(c) => c,
                Err(e) => {
                    return Err(create_options_exit_exception(
                        format!("Invalid --remote_grpc_service_config: {e}"),
                        failure_details::remote_options::Code::RemoteGrpcServiceConfigInvalid,
                    ));
                }
            };

        // Java: `new RemoteDownloadIdleTimeoutInterceptor(remoteGrpcDownloadIdleTimeout,
        // retryScheduler)` unless the timeout is zero.
        let download_idle_timeout =
            Some(remote_options.remote_grpc_download_idle_timeout).filter(|d| !d.is_zero());

        if !is_null_or_empty(&remote_options.remote_output_service) {
            blaze_util::unported!(
                "RemoteModule.setup (--experimental_remote_output_service: BazelOutputService)"
            );
        }

        // Verifying that the blobs referenced by a disk cache action result are present locally
        // turns most disk cache hits into misses when Build without the Bytes is enabled, as
        // outputs aren't downloaded and thus never added to the disk cache's CAS. With action
        // rewinding, a blob that is missing after all is cheap to recover from, so the check can
        // be skipped. This allows disk cache AC checks to be entirely local, reducing server
        // load and avoiding a network round trip.
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:705 RemoteModule.setup
        let check_disk_cache_action_result_integrity = !rewind_lost_inputs.unwrap_or(false);

        if (enable_http_cache || enable_disk_cache) && !enable_grpc_cache {
            // Java: com/google/devtools/build/lib/remote/RemoteModule.java:287 RemoteModule.initHttpAndDiskCache
            let combined_cache_client = match combined_cache_client_factory::create(
                &remote_options,
                Some(credentials.clone()),
                &auth_and_tls_options,
                Some(&working_directory),
                &digest_util,
                // new RemoteRetrier(remoteOptions, HTTP_RESULT_CLASSIFIER, retryScheduler,
                // circuitBreaker), with a circuit breaker of its own.
                Arc::new(RemoteRetrier::new(
                    &remote_options,
                    HTTP_RESULT_CLASSIFIER.clone(),
                    retry_scheduler.clone(),
                    circuit_breaker_factory::create_circuit_breaker(&remote_options),
                )),
                check_disk_cache_action_result_integrity,
            ) {
                Ok(c) => c,
                Err(e) => {
                    return Err(handle_init_failure(
                        reporter.as_ref(),
                        &e,
                        failure_details::remote_execution::Code::CacheInitFailure,
                    ));
                }
            };
            let combined_cache = Arc::new(CombinedCache::new(
                combined_cache_client.remote_cache_client().cloned(),
                combined_cache_client.disk_cache_client().cloned(),
                empty_to_null(&remote_options.remote_download_symlink_template),
                digest_util.clone(),
                remote_options.get_effective_chunking_function(),
            ));
            self.action_context_provider = Some(RemoteActionContextProvider::new(
                provider_env,
                remote_options_arc_for(&remote_options),
                Some(combined_cache),
                None,
                /* remote_executor= */ None,
                // Java passes `retryScheduler = null` here (it is only used for execution).
                retry_scheduler,
                digest_util,
                /* log_dir= */ None,
                Some(remote_output_checker),
                known_missing_cas_digests,
                temp_path_generator,
            ));
            return Ok(true);
        }

        let mut logging_interceptor: Option<Arc<dyn ClientInterceptor>> = None;
        if let Some(remote_grpc_log) = &remote_options.remote_grpc_log {
            let sink: Arc<dyn LogEntrySink> =
                match FileLogEntrySink::new(&working_directory.get_relative(remote_grpc_log.get()))
                {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        return Err(handle_init_failure(
                            reporter.as_ref(),
                            &e,
                            failure_details::remote_execution::Code::RpcLogFailure,
                        ));
                    }
                };
            self.rpc_log_file = Some(sink.clone());
            logging_interceptor = Some(Arc::new(LoggingInterceptor::new(sink, clock)));
        }

        let call_credentials_provider: Arc<dyn CallCredentialsProvider> =
            google_auth_utils::new_call_credentials_provider(Some(credentials));
        let call_credentials = call_credentials_provider.get_call_credentials();

        let rsc = Arc::new(RemoteServerCapabilities::new(
            &build_request_id,
            &invocation_id,
            Some(remote_options.remote_instance_name.as_str()),
            call_credentials,
            retrier.clone(),
        ));

        let remote_options_arc = remote_options_arc_for(&remote_options);
        let create = |headers_interceptor: Option<Arc<dyn ClientInterceptor>>,
                      target: &str,
                      requirement: ServerCapabilitiesRequirement|
         -> ReferenceCountedChannel {
            create_channel(
                &remote_options_arc,
                &auth_and_tls_options,
                headers_interceptor,
                logging_interceptor.clone(),
                download_idle_timeout,
                &remote_grpc_service_config,
                target,
                remote_options_arc.remote_proxy.as_deref(),
                reporter.clone(),
                rsc.clone(),
                digest_util.get_digest_function(),
                requirement,
            )
        };

        let mut exec_channel: Option<ReferenceCountedChannel> = None;
        let mut cache_channel: Option<ReferenceCountedChannel> = None;
        let remote_cache_target = remote_options_arc.remote_cache.clone().unwrap_or_default();

        // We only check required capabilities for a given endpoint.
        //
        // If --remote_executor and --remote_cache point to the same endpoint, we require that
        // endpoint has both execution and cache capabilities.
        //
        // If they point to different endpoints, we check the endpoint with execution or cache
        // capabilities respectively.
        if enable_remote_execution {
            let remote_executor_target = remote_options_arc
                .remote_executor
                .clone()
                .unwrap_or_default();
            let exec_headers: Arc<dyn ClientInterceptor> =
                Arc::new(headers::new_exec_headers_interceptor(&remote_options_arc));
            // Create a separate channel if --remote_executor and --remote_cache point to
            // different endpoints.
            if remote_cache_target == remote_executor_target {
                let channel = create(
                    Some(exec_headers),
                    &remote_executor_target,
                    ServerCapabilitiesRequirement::ExecutionAndCache,
                );
                cache_channel = Some(channel.retain());
                exec_channel = Some(channel);
            } else {
                exec_channel = Some(create(
                    Some(exec_headers),
                    &remote_executor_target,
                    ServerCapabilitiesRequirement::Execution,
                ));
            }
        }

        let cache_channel = match cache_channel {
            Some(c) => c,
            None => {
                let cache_headers: Arc<dyn ClientInterceptor> =
                    Arc::new(headers::new_cache_headers_interceptor(&remote_options_arc));
                create(
                    Some(cache_headers),
                    &remote_cache_target,
                    ServerCapabilitiesRequirement::Cache,
                )
            }
        };

        let remote_cache_client: Arc<dyn RemoteCacheClient> = Arc::new(GrpcCacheClient::new(
            cache_channel.retain(),
            call_credentials_provider.clone(),
            &remote_options_arc,
            retrier.clone(),
            digest_util.clone(),
        ));
        cache_channel.release();

        let mut disk_cache_client: Option<Arc<DiskCacheClient>> = None;
        if enable_disk_cache {
            match combined_cache_client_factory::create_disk_cache(
                &working_directory,
                &remote_options_arc,
                &digest_util,
                check_disk_cache_action_result_integrity,
            ) {
                Ok(c) => disk_cache_client = Some(Arc::new(c)),
                Err(e) => {
                    return Err(handle_init_failure(
                        reporter.as_ref(),
                        &e,
                        failure_details::remote_execution::Code::CacheInitFailure,
                    ));
                }
            }
        }

        if enable_remote_execution {
            let exec_channel = exec_channel.expect("created above");
            let exec_retrier = Arc::new(RemoteRetrier::new(
                &remote_options_arc,
                remote_retrier::GRPC_RESULT_CLASSIFIER.clone(),
                retry_scheduler.clone(),
                circuit_breaker,
            ));
            let remote_executor: Arc<dyn RemoteExecutionClient> =
                Arc::new(GrpcRemoteExecutor::new(
                    exec_channel.retain(),
                    call_credentials_provider,
                    exec_retrier,
                ));
            exec_channel.release();
            let remote_cache = Arc::new(RemoteExecutionCache::new(
                remote_cache_client,
                disk_cache_client,
                empty_to_null(&remote_options_arc.remote_download_symlink_template),
                digest_util.clone(),
                remote_options_arc.get_effective_chunking_function(),
            ));
            self.action_context_provider = Some(RemoteActionContextProvider::new(
                provider_env,
                remote_options_arc.clone(),
                None,
                Some(remote_cache),
                Some(remote_executor),
                retry_scheduler,
                digest_util,
                Some(log_dir),
                Some(remote_output_checker),
                known_missing_cas_digests,
                temp_path_generator,
            ));
        } else {
            let combined_cache = Arc::new(CombinedCache::new(
                Some(remote_cache_client),
                disk_cache_client,
                empty_to_null(&remote_options_arc.remote_download_symlink_template),
                digest_util.clone(),
                remote_options_arc.get_effective_chunking_function(),
            ));
            self.action_context_provider = Some(RemoteActionContextProvider::new(
                provider_env,
                remote_options_arc.clone(),
                Some(combined_cache),
                None,
                /* remote_executor= */ None,
                retry_scheduler,
                digest_util,
                /* log_dir= */ None,
                Some(remote_output_checker),
                known_missing_cas_digests,
                temp_path_generator,
            ));
        }

        // PORT: `createActionInputFetcher` is called by the integrator
        // (`RemoteModule::create_action_input_fetcher`); `initRepoHelpersAndOverlayFs` is not
        // ported.

        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:905 RemoteModule.setup
        let combined_cache = self
            .action_context_provider
            .as_ref()
            .and_then(|provider| provider.get_combined_cache().cloned())
            .expect("actionContextProvider.getCombinedCache()");
        check_state!(self.build_event_artifact_uploader_factory.is_none());
        self.build_event_artifact_uploader_factory =
            Some(Arc::new(ByteStreamBuildEventArtifactUploaderFactory::new(
                reporter.clone(),
                // Java: `verboseFailures = executionOptions.verboseFailures` (RemoteModule.java:446).
                execution_options.verbose_failures,
                combined_cache,
                remote_options_arc.remote_instance_name.clone(),
                remote_options_arc.remote_bytestream_uri_prefix.clone(),
                build_request_id.clone(),
                // invocationId
                command_id.clone(),
                remote_options_arc.remote_build_event_upload_mode,
            )));

        if enable_remote_downloader {
            blaze_util::unported!(
                "RemoteModule.setup (--experimental_remote_downloader: GrpcRemoteDownloader)"
            );
        }
        Ok(true)
    }

    /// `afterCommand()`: shuts the services down (waiting for background uploads), deletes the
    /// download temp directory and closes the gRPC log (`afterCommandTask`).
    ///
    /// PORT: Java runs `afterCommandTask` through the `BlockWaitingModule` once every module's
    /// `afterCommand` has run; the integrator calls this at that point. A crash (an `unported!`
    /// panic) skips it, as a Java crash skips `afterCommand` (`BlazeRuntime.cleanUpForCrash`).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1097 RemoteModule.afterCommand
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1139 RemoteModule.afterCommandTask
    pub fn after_command(&mut self) -> Result<(), AbruptExitException> {
        if let Some(provider) = self.action_context_provider.take() {
            provider.after_command();
        }

        // PORT: `remoteDownloader.close()`: the remote downloader is not ported (setup fails
        // loudly when it is requested).

        if let Some(temp_path_generator) = self.temp_path_generator.take() {
            let temp_dir = temp_path_generator.get_temp_dir();
            // Intentionally ignored.
            let _ = temp_dir.delete_tree();
        }

        let rpc_log_file = self.rpc_log_file.take();
        // lastRemoteOutputChecker = remoteOutputChecker
        // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1120 RemoteModule.afterCommand
        {
            let mut last = LAST_REMOTE_OUTPUT_CHECKER.lock();
            last.checker = self.remote_output_checker.clone();
            last.set_this_command = true;
        }
        self.remote_output_checker = None;
        self.action_input_fetcher = None;
        self.rewound_action_synchronizer = None;
        // buildEventArtifactUploaderFactoryDelegate.reset()
        self.build_event_artifact_uploader_factory = None;

        if let Some(rpc_log_file) = rpc_log_file
            && rpc_log_file.close().is_err()
        {
            return Err(create_exit_exception(
                "Partially wrote RPC log file",
                ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
                failure_details::remote_execution::Code::RpcLogFailure,
            ));
        }
        Ok(())
    }
}

/// `getTempPathGenerator(env)`: warns about and deletes the downloads a previous build left in
/// `<action temps>/remote`. `setup` runs it for every command that has `RemoteOptions`, also
/// when nothing remote is enabled (before it returns with the placeholder provider), so the
/// integration calls it for those commands too.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1215 RemoteModule.getTempPathGenerator
pub fn get_temp_path_generator(
    action_temps_directory: &Path,
    reporter: &dyn EventHandler,
) -> Result<TempPathGenerator, AbruptExitException> {
    let temp_dir = action_temps_directory.get_child("remote");
    if temp_dir.exists() {
        reporter.handle(&Event::warn(
            "Found stale downloads from previous build, deleting...",
        ));
        if let Err(e) = temp_dir.delete_tree() {
            return Err(create_exit_exception(
                format!("Failed to delete stale downloads: {e}"),
                ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
                failure_details::remote_execution::Code::DownloadedInputsDeletionFailure,
            ));
        }
    }
    Ok(TempPathGenerator::new(temp_dir))
}

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1083 RemoteModule.cleanAndCreateRemoteLogsDir
fn clean_and_create_remote_logs_dir(log_dir: &Path) -> Result<(), AbruptExitException> {
    let result = (|| -> io::Result<()> {
        // Clean out old logs files.
        if log_dir.exists() {
            log_dir.delete_tree()?;
        }
        log_dir.create_directory()?;
        Ok(())
    })();
    result.map_err(|_| {
        create_exit_exception(
            format!(
                "Could not create base directory for remote logs: {}",
                log_dir.get_path_string()
            ),
            ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
            failure_details::remote_execution::Code::LogDirCleanupFailure,
        )
    })
}

/// `createCredentials(...)` (`@VisibleForTesting`).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1448 RemoteModule.createCredentials
pub fn create_credentials(
    credential_helper_environment: &CredentialHelperEnvironment,
    credential_cache: &Arc<CredentialCache>,
    command_line_path_factory: &CommandLinePathFactory,
    file_system: &FileSystemRef,
    auth_and_tls_options: &AuthAndTLSOptions,
    remote_options: &RemoteOptions,
) -> io::Result<Arc<dyn crate::authandtls::Credentials>> {
    let credentials = google_auth_utils::new_credentials(
        credential_helper_environment,
        credential_cache,
        command_line_path_factory,
        file_system,
        auth_and_tls_options,
    )?;

    if let Some(remote_cache) = &remote_options.remote_cache
        && remote_cache.to_ascii_lowercase().starts_with("http://")
    {
        let uri = blaze_repo_fetch::uri::JavaUri::create(remote_cache)
            .map_err(|e| crate::common::errors::io_exception(e.message))?;
        let metadata = credentials
            .get_request_metadata(&uri)
            .map_err(|e| e.into_io_error())?;
        if !metadata.is_empty() {
            // TODO(yannic): Make this a error aborting the build.
            credential_helper_environment
                .event_reporter
                .handle(&Event::warn(format!(
                    "Credentials are transmitted in plaintext to {remote_cache}. Please consider using an HTTPS endpoint."
                )));
        }
    }

    Ok(credentials)
}

/// `createChannel(...)`: the channel with its interceptors (headers, then logging) and the
/// lazily fetched, verified server capabilities (`GoogleChannelConnectionFactory`).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:985 RemoteModule.createChannel
fn create_channel(
    remote_options: &Arc<RemoteOptions>,
    auth_and_tls_options: &AuthAndTLSOptions,
    headers_interceptor: Option<Arc<dyn ClientInterceptor>>,
    logging_interceptor: Option<Arc<dyn ClientInterceptor>>,
    download_idle_timeout: Option<std::time::Duration>,
    service_config: &serde_json::Map<String, serde_json::Value>,
    target: &str,
    proxy: Option<&str>,
    reporter: Arc<dyn ExtendedEventHandler>,
    remote_server_capabilities: Arc<RemoteServerCapabilities>,
    digest_function: digest_function::Value,
    requirement: ServerCapabilitiesRequirement,
) -> ReferenceCountedChannel {
    // `setup` turns `--experimental_remote_max_concurrency_per_connection` and
    // `--remote_max_connections` into `maxConcurrencyPerConnection` / `maxConnections` and
    // hands them to `createChannel`, whose `GoogleChannelConnectionFactory` and
    // `ReferenceCountedChannel` size a `DynamicConnectionPool` with them: only a gRPC channel
    // reads them (a `--disk_cache` alone never does). The port's channel multiplexes every call
    // on one HTTP/2 connection (`crate::grpc::channel`), so a value other than the default fails
    // loudly here (review 2026-11-19 R13).
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:623 RemoteModule.setup
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:70 ReferenceCountedChannel.<init>
    if remote_options.remote_max_concurrency_per_connection != 100
        || remote_options.remote_max_connections != 100
    {
        blaze_util::unported!(
            "DynamicConnectionPool",
            "--remote_max_connections / --remote_max_concurrency_per_connection other than 100 (one HTTP/2 connection per endpoint)"
        );
    }

    let mut interceptors: Vec<Arc<dyn ClientInterceptor>> = Vec::new();
    if let Some(headers_interceptor) = headers_interceptor {
        interceptors.push(headers_interceptor);
    }
    if let Some(logging_interceptor) = logging_interceptor {
        interceptors.push(logging_interceptor);
    }

    // `Single.fromCallable(() -> channelFactory.newChannel(..))`: the transport is built
    // lazily in Java, so a failure (an unreadable `--tls_certificate`, an invalid proxy) is the
    // failure of every call made on the channel, not of the command's setup.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:101 GoogleChannelConnectionFactory.create
    let managed = match google_auth_utils::new_channel(
        target,
        proxy,
        auth_and_tls_options,
        Some(interceptors),
        Some(service_config),
    ) {
        Ok(managed) => managed,
        Err(e) => return ReferenceCountedChannel::failed_to_create(target, e),
    };

    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:106 GoogleChannelConnectionFactory.getAndVerifyServerCapabilities
    let options = remote_options.clone();
    let fetcher: CapabilitiesFetcher = Arc::new(move |channel: ReferenceCountedChannel| {
        let remote_server_capabilities = remote_server_capabilities.clone();
        let options = options.clone();
        let reporter = reporter.clone();
        Box::pin(async move {
            let verified: io::Result<_> = async {
                let server_capabilities = remote_server_capabilities.get(&channel).await?;
                let result = check_client_server_compatibility(
                    &server_capabilities,
                    &options,
                    digest_function,
                    requirement,
                );
                for warning in result.get_warnings() {
                    reporter.handle(&Event::warn(warning.clone()));
                }
                let errors = result.get_errors();
                for error in errors.iter().take(errors.len().saturating_sub(1)) {
                    reporter.handle(&Event::error(error.clone()));
                }
                if let Some(last_error_message) = errors.last() {
                    return Err(io::Error::other(last_error_message.clone()));
                }
                Ok(server_capabilities)
            }
            .await;
            // Java: onFailure(error): serverCapabilities.setException(
            //     new RemoteExecutionCapabilitiesException(cause))
            verified.map_err(|error| {
                if is_interrupted(&error) {
                    error
                } else {
                    RemoteExecutionCapabilitiesException::new(capabilities_failure_cause(error))
                        .into()
                }
            })
        })
    });

    let channel = ReferenceCountedChannel::new(
        managed.channel,
        &managed.authority,
        managed.tls,
        managed.interceptors,
        managed.max_inbound_message_size,
        Some(fetcher),
    )
    .with_transport_lifecycle(managed.transport_lifecycle.clone());
    let channel = match &managed.service_config {
        Some(config) => match channel.with_service_config(config) {
            Ok(channel) => channel,
            // PORT: grpc-java rejects an invalid default service config when the channel is
            // built (`ManagedChannelImplBuilder.defaultServiceConfig`), inside `newChannel`.
            Err(e) => return ReferenceCountedChannel::failed_to_create(target, e),
        },
        None => channel,
    };
    // Every connection waits for the verified capabilities unless the requirement is NONE.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:110 GoogleChannelConnectionFactory.create
    let channel = if requirement != ServerCapabilitiesRequirement::None {
        channel.with_server_capabilities_gate()
    } else {
        channel
    };
    // The innermost interceptor in Java (added last); it only acts on ByteStream.Read.
    let channel = match download_idle_timeout {
        Some(timeout) => channel.with_download_idle_timeout(timeout),
        None => channel,
    };

    // Eagerly start creating the channel and verifying the capabilities in the background.
    // Don't issue GetCapabilities calls if the requirement is NONE because the endpoint, e.g.
    // Remote Asset API, might not implement the API. See #20342.
    if requirement != ServerCapabilitiesRequirement::None {
        let eager = channel.clone();
        crate::runtime::runtime().spawn(async move {
            let _ = eager.get_server_capabilities_async().await;
        });
    }
    channel
}

/// The cause `GoogleChannelConnectionFactory` gives the `RemoteExecutionCapabilitiesException`
/// of a failed capabilities fetch: `error.getCause()` when `error` is not an `IOException` (a
/// gRPC `StatusRuntimeException` from `GetCapabilities`) and its cause is one (the transport's
/// connection failure), else `error` itself.
///
/// PORT: the port carries a `StatusRuntimeException` as an `io::Error` whose message is
/// `new IOException(sre).getMessage()`; the cause keeps the status for the retriers
/// (`Status.fromThrowable`) and takes the message of the `StatusRuntimeException` itself
/// (`CODE: description`), which is what Java prints after "Failed to query remote execution
/// capabilities: ".
// Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:146 GoogleChannelConnectionFactory.getAndVerifyServerCapabilities
fn capabilities_failure_cause(error: io::Error) -> io::Error {
    let Some(sre) = errors::downcast_ref::<StatusRuntimeException>(&error) else {
        return error;
    };
    if let Some(io_exception) = crate::grpc::channel::transport_io_cause(&sre.status) {
        return io_exception;
    }
    let message = StatusRuntimeException::format_throwable_message(&sre.status);
    errors::io_exception_with_cause(message, error)
}

/// The values `RemoteActionContextProvider` reads from the `CommandEnvironment`.
struct ProviderEnvironment {
    reporter: Arc<dyn ExtendedEventHandler>,
    exec_root: Path,
    working_directory: Path,
    workspace_name: String,
    build_request_id: String,
    command_id: String,
    execution_options: Arc<ExecutionOptions>,
    experimental_sibling_repository_layout: bool,
    scrubber: Option<Arc<Scrubber>>,
    hooks: RemoteHooks,
}

/// `HTTP_RESULT_CLASSIFIER`: the retry classification of the HTTP remote cache.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:249 RemoteModule.HTTP_RESULT_CLASSIFIER
pub static HTTP_RESULT_CLASSIFIER: std::sync::LazyLock<Arc<dyn crate::retrier::ResultClassifier>> =
    std::sync::LazyLock::new(|| -> Arc<dyn crate::retrier::ResultClassifier> {
        Arc::new(http_result_classifier)
    });

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:249 RemoteModule.HTTP_RESULT_CLASSIFIER
fn http_result_classifier(e: &io::Error) -> crate::retrier::ResultClassifierResult {
    use crate::http::{ClosedChannelException, DownloadTimeoutException, HttpException};
    use crate::retrier::ResultClassifierResult;
    let mut retry = false;
    if crate::common::errors::downcast_ref::<ClosedChannelException>(e).is_some() {
        retry = true;
    } else if crate::common::errors::downcast_ref::<DownloadTimeoutException>(e).is_some() {
        retry = true;
    } else if let Some(http_exception) = crate::common::errors::downcast_ref::<HttpException>(e) {
        let status = http_exception.status;
        if status == 404 {
            return ResultClassifierResult::Success;
        }
        retry = matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
    } else {
        // `e instanceof IOException`: every error here is one.
        // PORT: the JDK's socket messages are `strerror` texts ("Connection reset by peer");
        // Rust appends " (os error N)", which these tests do not see.
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("connection reset") {
            retry = true;
        } else if msg.contains("operation timed out") {
            retry = true;
        }
    }
    // PORT: skipped, the workaround for netty#11815 (a `DecoderException` ending in
    // "functions:OPENSSL_internal:BAD_DECRYPT"): no Netty or BoringSSL here.
    if retry {
        ResultClassifierResult::TransientFailure
    } else {
        ResultClassifierResult::PermanentFailure
    }
}

/// Provides a remote execution context.
// Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:42 RemoteActionContextProvider
pub struct RemoteActionContextProvider {
    env: ProviderEnvironment,
    remote_options: Arc<RemoteOptions>,
    combined_cache: Option<Arc<CombinedCache>>,
    remote_execution_cache: Option<Arc<RemoteExecutionCache>>,
    remote_executor: Option<Arc<dyn RemoteExecutionClient>>,
    retry_scheduler: Arc<dyn RetryScheduler>,
    digest_util: DigestUtil,
    log_dir: Option<Path>,
    temp_path_generator: Arc<TempPathGenerator>,
    remote_execution_service: Mutex<Option<Arc<RemoteExecutionService>>>,
    remote_output_checker: Option<Arc<RemoteOutputChecker>>,
    known_missing_cas_digests: Arc<Mutex<HashSet<Digest>>>,
    /// `actionInputFetcher`, set by `RemoteModule.executorInit` (`setActionInputFetcher`).
    action_input_fetcher: Mutex<Option<Arc<RemoteActionInputFetcher>>>,
}

impl RemoteActionContextProvider {
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:58 RemoteActionContextProvider.<init>
    fn new(
        env: ProviderEnvironment,
        remote_options: Arc<RemoteOptions>,
        combined_cache: Option<Arc<CombinedCache>>,
        remote_execution_cache: Option<Arc<RemoteExecutionCache>>,
        remote_executor: Option<Arc<dyn RemoteExecutionClient>>,
        retry_scheduler: Arc<dyn RetryScheduler>,
        digest_util: DigestUtil,
        log_dir: Option<Path>,
        remote_output_checker: Option<Arc<RemoteOutputChecker>>,
        known_missing_cas_digests: Arc<Mutex<HashSet<Digest>>>,
        temp_path_generator: Arc<TempPathGenerator>,
    ) -> RemoteActionContextProvider {
        RemoteActionContextProvider {
            env,
            remote_options,
            combined_cache,
            remote_execution_cache,
            remote_executor,
            retry_scheduler,
            digest_util,
            log_dir,
            temp_path_generator,
            remote_execution_service: Mutex::new(None),
            action_input_fetcher: Mutex::new(None),
            remote_output_checker,
            known_missing_cas_digests,
        }
    }

    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:136 RemoteActionContextProvider.createRemotePathResolver
    fn create_remote_path_resolver(&self) -> Arc<dyn RemotePathResolver> {
        let exec_root = self.env.exec_root.clone();
        if self.env.experimental_sibling_repository_layout {
            Arc::new(SiblingRepositoryLayoutResolver::new(exec_root))
        } else {
            Arc::new(DefaultRemotePathResolver::new(exec_root))
        }
    }

    /// `getRemoteExecutionService()`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:158 RemoteActionContextProvider.getRemoteExecutionService
    pub fn get_remote_execution_service(&self) -> Arc<RemoteExecutionService> {
        let mut slot = self.remote_execution_service.lock();
        if let Some(service) = slot.as_ref() {
            return service.clone();
        }
        let capture_corrupted_outputs_dir = self
            .remote_options
            .remote_capture_corrupted_outputs
            .as_ref()
            .filter(|p| !p.get().is_empty())
            .map(|p| self.env.working_directory.get_relative(p.get()));
        let verbose_failures = self.env.execution_options.verbose_failures;
        let service = Arc::new(
            RemoteExecutionService::new(
                self.env.reporter.clone(),
                verbose_failures,
                self.env.exec_root.clone(),
                self.create_remote_path_resolver(),
                &self.env.build_request_id,
                &self.env.command_id,
                &self.env.workspace_name,
                self.digest_util.clone(),
                self.remote_options.clone(),
                self.env.execution_options.clone(),
                self.combined_cache.clone(),
                self.remote_execution_cache.clone(),
                self.remote_executor.clone(),
                self.temp_path_generator.clone(),
                capture_corrupted_outputs_dir,
                self.remote_output_checker.clone(),
                self.known_missing_cas_digests.clone(),
                self.env.scrubber.clone(),
            )
            .with_hooks(self.env.hooks.clone()),
        );
        // PORT: `env.getEventBus().register(remoteExecutionService)`: the integrator calls
        // `on_build_interrupted` / `on_build_complete` (see `RemoteExecutionService`).
        *slot = Some(service.clone());
        service
    }

    /// `registerRemoteSpawnStrategy(registryBuilder)`: the strategy to register as `"remote"`,
    /// together with its runner (for `set_remote_local_fallback_registry`).
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:201 RemoteActionContextProvider.registerRemoteSpawnStrategy
    pub fn get_remote_spawn_strategy(&self) -> (AbstractSpawnStrategy, Arc<RemoteSpawnRunner>) {
        let execution_options = self.env.execution_options.clone();
        let spawn_runner = Arc::new(RemoteSpawnRunner::new(
            self.remote_options.clone(),
            execution_options.verbose_failures,
            Some(self.env.reporter.clone()),
            self.retry_scheduler.clone(),
            // PORT: `logDir` is null unless remote execution is configured; the runner only
            // reads it when it executes remotely.
            self.log_dir
                .clone()
                .unwrap_or_else(|| self.env.exec_root.clone()),
            self.get_remote_execution_service(),
            self.digest_util.clone(),
        ));
        (
            new_remote_spawn_strategy(
                self.env.exec_root.clone(),
                spawn_runner.clone(),
                execution_options,
            ),
            spawn_runner,
        )
    }

    /// `registerSpawnCache(registryBuilder)`: the `SpawnCache` to register as `"remote-cache"`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:221 RemoteActionContextProvider.registerSpawnCache
    pub fn get_spawn_cache(&self) -> Arc<dyn SpawnCache> {
        Arc::new(RemoteSpawnCache::new(
            self.remote_options.clone(),
            self.env.execution_options.verbose_failures,
            self.get_remote_execution_service(),
            self.digest_util.clone(),
        ))
    }

    /// `getCombinedCache()`.
    pub fn get_combined_cache(&self) -> Option<&Arc<CombinedCache>> {
        match &self.remote_execution_cache {
            Some(c) => Some(c.combined_cache()),
            None => self.combined_cache.as_ref(),
        }
    }

    /// `(RemoteExecutionCache) getCombinedCache()`: the cache when remote execution is
    /// configured.
    pub fn get_remote_execution_cache(&self) -> Option<&Arc<RemoteExecutionCache>> {
        self.remote_execution_cache.as_ref()
    }

    /// `getRemoteExecutionClient()`.
    pub fn get_remote_execution_client(&self) -> Option<&Arc<dyn RemoteExecutionClient>> {
        self.remote_executor.as_ref()
    }

    /// `afterCommand()`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:243 RemoteActionContextProvider.afterCommand
    /// `setActionInputFetcher(actionInputFetcher)`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:148 RemoteActionContextProvider.setActionInputFetcher
    pub fn set_action_input_fetcher(&self, action_input_fetcher: Arc<RemoteActionInputFetcher>) {
        *self.action_input_fetcher.lock() = Some(action_input_fetcher);
    }

    pub fn after_command(&self) {
        // actionInputFetcher uses combinedCache to prefetch inputs, so it must be shut down first.
        let action_input_fetcher = self.action_input_fetcher.lock().take();
        if let Some(action_input_fetcher) = action_input_fetcher {
            (action_input_fetcher.as_ref()
                as &dyn crate::abstract_action_input_prefetcher::AbstractActionInputPrefetcher)
                .shutdown();
        }
        let service = self.remote_execution_service.lock().take();
        match service {
            Some(service) => service.shutdown(),
            None => {
                if let Some(combined_cache) = self.get_combined_cache() {
                    combined_cache.release();
                }
                if let Some(remote_executor) = &self.remote_executor {
                    remote_executor.close();
                }
            }
        }
    }
}
