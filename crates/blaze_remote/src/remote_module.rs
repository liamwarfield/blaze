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
//! helpers, the disk cache garbage collection idle task, action rewinding, and the HTTP cache.

use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use blaze_events::ui::Clock;
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_exec::exec::abstract_spawn_strategy::AbstractSpawnStrategy;
use blaze_exec::exec::action_execution_context::SpawnCache;
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_vfs::{FileSystemRef, Path};
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::authandtls::call_credentials_provider::CallCredentialsProvider;
use crate::authandtls::google_auth_utils::{self, CredentialHelperEnvironment};
use crate::authandtls::headers;
use crate::circuitbreaker::circuit_breaker_factory;
use crate::combined_cache::CombinedCache;
use crate::combined_cache_client_factory;
use crate::common::errors::is_interrupted;
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

/// `RemoteModule`, as far as one command goes.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:139 RemoteModule
pub struct RemoteModule {
    action_context_provider: Option<RemoteActionContextProvider>,
    remote_output_checker: Option<Arc<RemoteOutputChecker>>,
    temp_path_generator: Option<Arc<TempPathGenerator>>,
    rpc_log_file: Option<Arc<dyn LogEntrySink>>,
    cache_available: bool,
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

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1381 RemoteModule.createExitException
fn create_exit_exception(
    message: impl Into<String>,
    remote_execution_code: failure_details::remote_execution::Code,
) -> AbruptExitException {
    AbruptExitException::new(FailureDetail {
        message: message.into(),
        category: Some(failure_detail::Category::RemoteExecution(
            failure_details::RemoteExecution {
                code: remote_execution_code as i32,
            },
        )),
    })
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
    create_exit_exception("Error initializing RemoteModule", remote_execution_code)
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
        };
        // PORT: `knownMissingCasDigests` survives across commands in the Bazel server (cleared
        // by `clean`); one command per process here.
        module.cache_available = module.setup(env)?;
        Ok(module)
    }

    /// Whether any cache (disk or remote) is enabled (the result of `setup`).
    pub fn cache_available(&self) -> bool {
        self.cache_available
    }

    /// `actionContextProvider`.
    pub fn get_action_context_provider(&self) -> Option<&RemoteActionContextProvider> {
        self.action_context_provider.as_ref()
    }

    /// `remoteOutputChecker`.
    pub fn get_remote_output_checker(&self) -> Option<&Arc<RemoteOutputChecker>> {
        self.remote_output_checker.as_ref()
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
        let hash_fn = file_system.get_digest_function();
        let digest_util = DigestUtil::new(hash_fn);

        // If --remote_cache is empty but --remote_executor is not, reuse the latter for the
        // former.
        if !is_null_or_empty(&remote_options.remote_executor)
            && is_null_or_empty(&remote_options.remote_cache)
        {
            remote_options.remote_cache = remote_options.remote_executor.clone();
        }

        if !is_null_or_empty(&remote_options.remote_output_service) {
            blaze_util::unported!(
                "RemoteModule.setup (--experimental_remote_output_service: BazelOutputService)"
            );
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
            // PORT: `DiskCacheGarbageCollectorIdleTask.create(...)` is registered as a server
            // idle task in Java; Blaze has no idle server, so no garbage collection runs
            // (`crate::disk::DiskCacheGarbageCollector` is the collector itself).
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

        let remote_output_checker = Arc::new(RemoteOutputChecker::new(
            &command_name,
            remote_options.remote_outputs_mode,
            RemoteOutputChecker::patterns_to_download(&remote_options),
        ));
        // PORT: `remoteOutputChecker.maybeInvalidateSkyframeValues(evaluator)` compares with the
        // checker of the previous command of the server; one command per process here.
        self.remote_output_checker = Some(remote_output_checker.clone());

        let invocation_id = command_id.clone();
        reporter.handle(&Event::info(format!("Invocation ID: {invocation_id}")));

        let log_dir = output_base.get_relative_str(&format!("{product_name}-remote-logs"));
        clean_and_create_remote_logs_dir(&log_dir)?;

        let credentials = match create_credentials(
            &CredentialHelperEnvironment {
                event_reporter: reporter.as_ref(),
                workspace_path: workspace,
                client_environment: client_env,
                helper_execution_timeout: auth_and_tls_options.credential_helper_timeout,
            },
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

        // PORT: `--remote_max_connections` / `--experimental_remote_max_concurrency_per_connection`
        // size Java's connection pool; the channel here multiplexes one connection
        // (`crate::grpc::channel`).

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

        // Verifying that the blobs referenced by a disk cache action result are present locally
        // turns most disk cache hits into misses when Build without the Bytes is enabled, as
        // outputs aren't downloaded and thus never added to the disk cache's CAS. With action
        // rewinding, a blob that is missing after all is cheap to recover from, so the check can
        // be skipped. This allows disk cache AC checks to be entirely local, reducing server
        // load and avoiding a network round trip.
        let check_disk_cache_action_result_integrity = !rewind_lost_inputs.unwrap_or(false);

        if (enable_http_cache || enable_disk_cache) && !enable_grpc_cache {
            // Java: com/google/devtools/build/lib/remote/RemoteModule.java:287 RemoteModule.initHttpAndDiskCache
            let combined_cache_client = match combined_cache_client_factory::create(
                &remote_options,
                Some(credentials.clone()),
                &auth_and_tls_options,
                Some(&working_directory),
                &digest_util,
                // PORT: Java passes a retrier with `HTTP_RESULT_CLASSIFIER`; only the HTTP
                // cache client (not ported) uses it.
                retrier.clone(),
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
         -> Result<ReferenceCountedChannel, AbruptExitException> {
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
            .map_err(|e| {
                handle_init_failure(
                    reporter.as_ref(),
                    &e,
                    failure_details::remote_execution::Code::RemoteExecutionUnknown,
                )
            })
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
                )?;
                cache_channel = Some(channel.retain());
                exec_channel = Some(channel);
            } else {
                exec_channel = Some(create(
                    Some(exec_headers),
                    &remote_executor_target,
                    ServerCapabilitiesRequirement::Execution,
                )?);
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
                )?
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

        // PORT: not ported: `createActionInputFetcher`, `initRepoHelpersAndOverlayFs`, the
        // ByteStream build event artifact uploader.
        if enable_remote_downloader {
            blaze_util::unported!(
                "RemoteModule.setup (--experimental_remote_downloader: GrpcRemoteDownloader)"
            );
        }
        Ok(true)
    }

    /// `afterCommand()`: shuts the services down (waiting for background uploads), closes the
    /// gRPC log and deletes the download temp directory.
    // Java: com/google/devtools/build/lib/remote/RemoteModule.java:1097 RemoteModule.afterCommand
    pub fn after_command(&mut self) -> Result<(), AbruptExitException> {
        let mut failure: Option<AbruptExitException> = None;
        if let Some(provider) = self.action_context_provider.take() {
            provider.after_command();
        }
        // Java: RemoteModule.closeRpcLogFile
        if let Some(rpc_log_file) = self.rpc_log_file.take()
            && let Err(e) = rpc_log_file.close()
        {
            failure = Some(create_exit_exception(
                format!("Failed to close the remote gRPC log: {e}"),
                failure_details::remote_execution::Code::RpcLogFailure,
            ));
        }
        // Java: RemoteModule.deleteDownloadedInputs
        if let Some(temp_path_generator) = self.temp_path_generator.take() {
            let temp_dir = temp_path_generator.get_temp_dir();
            if let Err(e) = temp_dir.delete_tree()
                && e.kind() != io::ErrorKind::NotFound
            {
                failure = Some(create_exit_exception(
                    format!(
                        "Failed to delete downloaded inputs in '{}': {}",
                        temp_dir.get_path_string(),
                        e
                    ),
                    failure_details::remote_execution::Code::DownloadedInputsDeletionFailure,
                ));
            }
        }
        self.remote_output_checker = None;
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1215 RemoteModule.getTempPathGenerator
fn get_temp_path_generator(
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
            failure_details::remote_execution::Code::LogDirCleanupFailure,
        )
    })
}

// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1448 RemoteModule.createCredentials
fn create_credentials(
    credential_helper_environment: &CredentialHelperEnvironment<'_>,
    file_system: &FileSystemRef,
    auth_and_tls_options: &AuthAndTLSOptions,
    remote_options: &RemoteOptions,
) -> io::Result<Arc<dyn crate::authandtls::Credentials>> {
    let credentials = google_auth_utils::new_credentials(
        credential_helper_environment,
        file_system,
        auth_and_tls_options,
    )?;

    if let Some(remote_cache) = &remote_options.remote_cache
        && remote_cache.to_ascii_lowercase().starts_with("http://")
    {
        // Java warns "Credentials are transmitted in plaintext to ..." when the credentials
        // have request metadata for the HTTP cache URI; the HTTP cache is not ported.
        blaze_util::unported!("RemoteModule.createCredentials (HTTP remote cache)");
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
) -> io::Result<ReferenceCountedChannel> {
    let mut interceptors: Vec<Arc<dyn ClientInterceptor>> = Vec::new();
    if let Some(headers_interceptor) = headers_interceptor {
        interceptors.push(headers_interceptor);
    }
    if let Some(logging_interceptor) = logging_interceptor {
        interceptors.push(logging_interceptor);
    }

    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:86 GoogleChannelConnectionFactory.create
    let managed = google_auth_utils::new_channel(
        target,
        proxy,
        auth_and_tls_options,
        Some(interceptors),
        Some(service_config),
    )?;

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
            verified.map_err(|e| {
                if is_interrupted(&e) {
                    e
                } else {
                    RemoteExecutionCapabilitiesException::new(e).into()
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
    );
    let channel = match &managed.service_config {
        Some(config) => channel.with_service_config(config)?,
        None => channel,
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
    Ok(channel)
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

    /// `getRemoteExecutionClient()`.
    pub fn get_remote_execution_client(&self) -> Option<&Arc<dyn RemoteExecutionClient>> {
        self.remote_executor.as_ref()
    }

    /// `afterCommand()`.
    // Java: com/google/devtools/build/lib/remote/RemoteActionContextProvider.java:243 RemoteActionContextProvider.afterCommand
    pub fn after_command(&self) {
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
