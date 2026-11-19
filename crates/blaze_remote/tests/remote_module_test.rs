// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `lib/remote/RemoteModuleTest.java`, plus the capability-gate cases of review
//! 2026-11-19 (R3, R12, R13, R14), marked as not in the Java test.
//!
//! PORT: the Java test gives `RemoteModule` an in-process channel factory
//! (`remoteModule.setChannelFactory(...)`) and a `CommandEnvironment` built on an in-memory file
//! system. Here [`RemoteModule::before_command`] gets a [`RemoteModuleEnvironment`] on a
//! temporary directory, and the fake servers listen on `127.0.0.1` (tests/util/grpc_server.rs),
//! addressed as `grpc://127.0.0.1:<port>` through the production channel factory.

mod util;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use blaze_events::{JavaClock, StoredEventHandler};
use blaze_exec::exec::spawn_strategy_registry::AbruptExitException;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::remote_options::{CircuitBreakerStrategy, RemoteOptions};
use blaze_options::Options;
use blaze_remote::api_version::ApiVersion;
use blaze_remote::authandtls::google_auth_utils::CredentialHelperEnvironment;
use blaze_remote::circuitbreaker::failure_circuit_breaker::FailureCircuitBreaker;
use blaze_remote::common::errors;
use blaze_remote::common::remote_execution_capabilities_exception::RemoteExecutionCapabilitiesException;
use blaze_remote::grpc_cache_client::GrpcCacheClient;
use blaze_remote::grpc_remote_executor::GrpcRemoteExecutor;
use blaze_remote::hooks::RemoteHooks;
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, CacheCapabilities, ExecutionCapabilities, ServerCapabilities,
    digest_function, symlink_absolute_path_strategy,
};
use blaze_remote::remote_module::{RemoteModule, RemoteModuleEnvironment, create_credentials};
use blaze_remote::retrier::{ALLOW_ALL_CALLS, CircuitBreaker};
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;
use util::grpc_server::{FakeCapabilities, FakeServer};

fn cache_only_caps() -> ServerCapabilities {
    ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn exec_and_cache_caps() -> ServerCapabilities {
    ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            digest_function: digest_function::Value::Sha256 as i32,
            ..Default::default()
        }),
        cache_capabilities: Some(CacheCapabilities {
            digest_functions: vec![digest_function::Value::Sha256 as i32],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn exec_only_caps() -> ServerCapabilities {
    ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            digest_function: digest_function::Value::Sha256 as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `CapabilitiesImpl`: answers `caps` and counts the requests.
struct CapabilitiesImpl {
    request_count: Arc<AtomicUsize>,
}

impl CapabilitiesImpl {
    fn get_request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }
}

/// `createFakeServer(serverName, new CapabilitiesImpl(caps))`, started.
fn create_fake_server(caps: ServerCapabilities) -> (FakeServer, CapabilitiesImpl) {
    let request_count = Arc::new(AtomicUsize::new(0));
    let counter = request_count.clone();
    let mut server = FakeServer::new();
    server.service_registry.add_capabilities(FakeCapabilities {
        get_capabilities: Some(Box::new(move |_request| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(caps.clone())
        })),
    });
    server.start();
    (server, CapabilitiesImpl { request_count })
}

fn target(server: &FakeServer) -> String {
    format!("grpc://{}", server.addr())
}

/// `parseRemoteOptions(args...)`.
fn parse_remote_options(args: &[&str]) -> RemoteOptions {
    Options::<RemoteOptions>::parse(args)
        .unwrap()
        .into_options()
}

/// The `CommandEnvironment` of a test (`createTestCommandEnvironment`): directories in a
/// temporary directory, default options but `RemoteOptions`.
struct TestEnv {
    _dir: tempfile::TempDir,
    file_system: FileSystemRef,
    root: Path,
    reporter: Arc<StoredEventHandler>,
}

impl TestEnv {
    fn new() -> TestEnv {
        let dir = tempfile::tempdir().unwrap();
        let file_system = UnixFileSystem::create(DigestHashFunction::Sha256);
        let root = Path::create_from_str(
            std::fs::canonicalize(dir.path()).unwrap().to_str().unwrap(),
            &file_system,
        );
        TestEnv {
            _dir: dir,
            file_system,
            root,
            reporter: Arc::new(StoredEventHandler::new()),
        }
    }

    fn workspace(&self) -> Path {
        self.root.get_relative_str("workspace")
    }

    fn environment(&self, remote_options: RemoteOptions) -> RemoteModuleEnvironment {
        let output_base = self.root.get_relative_str("output");
        let exec_root = output_base.get_relative_str("execroot/_main");
        exec_root.create_directory_and_parents().unwrap();
        self.workspace().create_directory_and_parents().unwrap();
        RemoteModuleEnvironment {
            command_name: "build".to_string(),
            remote_options,
            auth_and_tls_options: Options::<AuthAndTLSOptions>::get_defaults(),
            execution_options: Arc::new(Options::<ExecutionOptions>::get_defaults()),
            // The test command's options do not include BuildRequestOptions.
            rewind_lost_inputs: None,
            experimental_sibling_repository_layout: false,
            reporter: self.reporter.clone(),
            file_system: self.file_system.clone(),
            output_base: output_base.clone(),
            exec_root: exec_root.clone(),
            working_directory: self.workspace(),
            workspace: Some(self.workspace()),
            workspace_name: "_main".to_string(),
            action_temps_directory: output_base.get_relative_str("action_temps"),
            build_request_id: "build-request-id".to_string(),
            command_id: "command-id".to_string(),
            product_name: "bazel".to_string(),
            tool_version: "9.3.2".to_string(),
            client_env: IndexMap::new(),
            clock: Arc::new(JavaClock),
            scrubber: None,
            hooks: RemoteHooks::new(),
        }
    }
}

/// `beforeCommand()`: `remoteModule.beforeCommand(env); env.throwPendingException()`.
fn before_command(
    env: &TestEnv,
    remote_options: RemoteOptions,
) -> Result<RemoteModule, AbruptExitException> {
    RemoteModule::before_command(env.environment(remote_options))
}

/// `assertCircuitBreakerInstance()`.
fn assert_circuit_breaker_instance(remote_module: &RemoteModule, remote_options: &RemoteOptions) {
    let action_context_provider = remote_module
        .get_action_context_provider()
        .expect("actionContextProvider");

    let circuit_breaker: Arc<dyn CircuitBreaker> = if let Some(combined_cache) =
        action_context_provider.get_combined_cache()
    {
        combined_cache
            .remote_cache_client()
            .and_then(|c| c.as_any())
            .and_then(|c| c.downcast_ref::<GrpcCacheClient>())
            .expect("(GrpcCacheClient) combinedCache.remoteCacheClient")
            .get_retrier()
            .get_circuit_breaker()
            .clone()
    } else if let Some(remote_executor) = action_context_provider.get_remote_execution_client() {
        remote_executor
            .as_any()
            .and_then(|c| c.downcast_ref::<GrpcRemoteExecutor>())
            .expect("(GrpcRemoteExecutor) remoteExecutionClient")
            .get_retrier()
            .get_circuit_breaker()
            .clone()
    } else {
        // no remote cache or execution configured, circuitBreaker is null
        return;
    };

    if remote_options.circuit_breaker_strategy == Some(CircuitBreakerStrategy::FAILURE) {
        assert!(
            circuit_breaker
                .as_any()
                .is_some_and(|c| c.is::<FailureCircuitBreaker>())
        );
    }
    if remote_options.circuit_breaker_strategy.is_none() {
        assert!(Arc::ptr_eq(&circuit_breaker, &ALLOW_ALL_CALLS));
    }
}

fn remote_cache_capabilities(remote_module: &RemoteModule) -> CacheCapabilities {
    remote_module
        .get_action_context_provider()
        .unwrap()
        .get_combined_cache()
        .unwrap()
        .get_remote_cache_capabilities()
        .unwrap()
}

#[test]
#[ignore = "needs a channel factory seam (RemoteModule.setChannelFactory): the service config handed to ChannelFactory.newChannel is not observable"]
fn remote_grpc_service_config_passes_remote_timeout_config_to_channel_factory() {}

#[test]
#[ignore = "needs a channel factory seam (RemoteModule.setChannelFactory): the service config handed to ChannelFactory.newChannel is not observable"]
fn remote_grpc_service_config_passes_user_supplied_json_file_to_channel_factory() {}

#[test]
#[ignore = "needs the remote downloader (--experimental_remote_downloader fails loudly: GrpcRemoteDownloader is not ported)"]
fn test_verify_capabilities_none() {}

#[test]
fn test_verify_capabilities_execution_and_cache_for_single_endpoint() {
    let env = TestEnv::new();
    let (mut execution_server, execution_server_capabilities_impl) =
        create_fake_server(exec_and_cache_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_executor = Some(target(&execution_server));

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_cache_capabilities(&remote_module),
        exec_and_cache_caps().cache_capabilities.unwrap()
    );
    assert_eq!(
        remote_module
            .get_action_context_provider()
            .unwrap()
            .get_remote_execution_client()
            .unwrap()
            .get_server_capabilities()
            .unwrap(),
        exec_and_cache_caps()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(execution_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    execution_server.shutdown_now();
}

#[test]
fn test_verify_capabilities_cache_only_endpoint() {
    let env = TestEnv::new();
    let (mut cache_server, cache_server_capabilities_impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_cache_capabilities(&remote_module),
        cache_only_caps().cache_capabilities.unwrap()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(cache_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    cache_server.shutdown_now();
}

#[test]
fn test_verify_capabilities_execution_and_cache_for_different_endpoints() {
    let env = TestEnv::new();
    let (mut execution_server, execution_server_capabilities_impl) =
        create_fake_server(exec_and_cache_caps());
    let (mut cache_server, cache_server_capabilities_impl) =
        create_fake_server(exec_and_cache_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_executor = Some(target(&execution_server));
    remote_options.remote_cache = Some(target(&cache_server));

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_cache_capabilities(&remote_module),
        exec_and_cache_caps().cache_capabilities.unwrap()
    );
    assert_eq!(
        remote_module
            .get_action_context_provider()
            .unwrap()
            .get_remote_execution_client()
            .unwrap()
            .get_server_capabilities()
            .unwrap(),
        exec_and_cache_caps()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(execution_server_capabilities_impl.get_request_count(), 1);
    assert_eq!(cache_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    execution_server.shutdown_now();
    cache_server.shutdown_now();
}

#[test]
fn test_verify_capabilities_execution_only_and_cache_only_endpoints() {
    let env = TestEnv::new();
    let (mut execution_server, execution_server_capabilities_impl) =
        create_fake_server(exec_only_caps());
    let (mut cache_server, cache_server_capabilities_impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_executor = Some(target(&execution_server));
    remote_options.remote_cache = Some(target(&cache_server));

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_cache_capabilities(&remote_module),
        cache_only_caps().cache_capabilities.unwrap()
    );
    assert_eq!(
        remote_module
            .get_action_context_provider()
            .unwrap()
            .get_remote_execution_client()
            .unwrap()
            .get_server_capabilities()
            .unwrap(),
        exec_only_caps()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(execution_server_capabilities_impl.get_request_count(), 1);
    assert_eq!(cache_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    execution_server.shutdown_now();
    cache_server.shutdown_now();
}

#[test]
fn test_netrc_netrc_without_remote_cache() {
    let env = TestEnv::new();
    let netrc = env.root.get_relative_str(".netrc");
    std::fs::write(
        netrc.get_path_string(),
        "machine foo.example.org login baruser password barpass",
    )
    .unwrap();
    let auth_and_tls_options = Options::<AuthAndTLSOptions>::get_defaults();
    let remote_options = Options::<RemoteOptions>::get_defaults();
    let reporter = StoredEventHandler::new();
    let mut client_env = IndexMap::new();
    client_env.insert("NETRC".to_string(), netrc.get_path_string().to_string());

    let credentials = create_credentials(
        &CredentialHelperEnvironment {
            event_reporter: &reporter,
            workspace_path: Some(env.workspace()),
            client_environment: client_env,
            helper_execution_timeout: Duration::ZERO,
        },
        &env.file_system,
        &auth_and_tls_options,
        &remote_options,
    )
    .unwrap();

    assert!(
        !credentials
            .get_request_metadata(&JavaUri::create("https://foo.example.org").unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        credentials
            .get_request_metadata(&JavaUri::create("https://bar.example.org").unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_cache_capabilities_propagated_to_remote_cache() {
    let env = TestEnv::new();
    let (mut cache_server, _cache_server_capabilities_impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));

    let remote_module = before_command(&env, remote_options).unwrap();

    assert!(!blaze_shell::thread_interrupt::interrupted());
    let action_context_provider = remote_module.get_action_context_provider();
    assert!(action_context_provider.is_some());
    assert!(
        action_context_provider
            .unwrap()
            .get_combined_cache()
            .is_some()
    );
    assert_eq!(
        remote_cache_capabilities(&remote_module),
        cache_only_caps().cache_capabilities.unwrap()
    );
    cache_server.shutdown_now();
}

#[test]
fn test_cache_capabilities_propagated_to_remote_execution_cache() {
    let env = TestEnv::new();
    let (mut execution_server, _execution_server_capabilities_impl) =
        create_fake_server(exec_and_cache_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_executor = Some(target(&execution_server));

    let remote_module = before_command(&env, remote_options).unwrap();

    assert!(!blaze_shell::thread_interrupt::interrupted());
    let action_context_provider = remote_module.get_action_context_provider();
    assert!(action_context_provider.is_some());
    assert!(
        action_context_provider
            .unwrap()
            .get_combined_cache()
            .is_some()
    );
    assert_eq!(
        remote_cache_capabilities(&remote_module),
        exec_and_cache_caps().cache_capabilities.unwrap()
    );
    execution_server.shutdown_now();
}

#[test]
fn test_verify_capabilities_execution_and_cache_for_single_endpoint_with_circuit_breaker() {
    let env = TestEnv::new();
    let (mut execution_server, execution_server_capabilities_impl) =
        create_fake_server(exec_and_cache_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_executor = Some(target(&execution_server));
    remote_options.circuit_breaker_strategy = Some(CircuitBreakerStrategy::FAILURE);

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_module
            .get_action_context_provider()
            .unwrap()
            .get_remote_execution_client()
            .unwrap()
            .get_server_capabilities()
            .unwrap(),
        exec_and_cache_caps()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(execution_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    execution_server.shutdown_now();
}

#[test]
fn test_verify_capabilities_cache_only_endpoint_with_circuit_breaker() {
    let env = TestEnv::new();
    let (mut cache_server, cache_server_capabilities_impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));
    remote_options.circuit_breaker_strategy = Some(CircuitBreakerStrategy::FAILURE);

    let remote_module = before_command(&env, remote_options.clone()).unwrap();

    assert_eq!(
        remote_cache_capabilities(&remote_module),
        cache_only_caps().cache_capabilities.unwrap()
    );
    assert!(!blaze_shell::thread_interrupt::interrupted());
    assert_eq!(cache_server_capabilities_impl.get_request_count(), 1);
    assert_circuit_breaker_instance(&remote_module, &remote_options);
    cache_server.shutdown_now();
}

#[test]
fn bazel_output_service_no_remote_cache_exit() {
    let env = TestEnv::new();
    let mut output_service_server = FakeServer::new();
    output_service_server.start();
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_output_service = Some(target(&output_service_server));

    let exception = before_command(&env, remote_options)
        .err()
        .expect("AbruptExitException");

    assert!(
        exception
            .get_message()
            .contains("--experimental_remote_output_service")
    );
    output_service_server.shutdown_now();
}

#[test]
fn disk_cache_garbage_collection_idle_task_disabled() {
    let env = TestEnv::new();
    let disk_cache_dir = env.root.get_relative_str("disk_cache");
    let remote_options = parse_remote_options(&[&format!(
        "--disk_cache={}",
        disk_cache_dir.get_path_string()
    )]);

    // `assertThat(env.getIdleTasks()).isEmpty()`: no garbage collection is requested, so setup
    // succeeds (Blaze has no idle tasks; a requested collection fails loudly, see below).
    assert!(before_command(&env, remote_options).is_ok());
}

#[test]
#[ignore = "server idle tasks are not ported (Blaze has no server): a disk cache garbage collection policy fails loudly at setup (disk_cache_gc_policy_fails_loudly)"]
fn disk_cache_garbage_collection_idle_task_enabled() {}

#[test]
#[ignore = "needs RepositoryRemoteHelpersFactory (the remote repository contents cache is not ported)"]
fn repository_remote_helpers_factory_initialized_for_disk_cache_only_path() {}

#[test]
#[ignore = "needs the remote downloader (--experimental_remote_downloader fails loudly: GrpcRemoteDownloader is not ported)"]
fn remote_downloader_separate_endpoint_channels_are_closed_after_command() {}

#[test]
#[ignore = "needs the remote downloader (--experimental_remote_downloader fails loudly: GrpcRemoteDownloader is not ported)"]
fn remote_downloader_shared_endpoint_channel_is_closed_once_cache_is_released() {}

#[test]
#[ignore = "no executor service: the remote calls run on the shared runtime (crate::runtime), which --jobs does not size"]
fn executor_service_without_build_request_options_preserves_pool_size() {}

#[test]
fn compute_action_execution_salt_remote_endpoint_or_instance_changes_does_not_change() {
    let base_options = parse_remote_options(&[
        "--remote_cache=grpc://cache1",
        "--remote_executor=grpc://exec1",
        "--remote_instance_name=instance1",
    ]);
    let base_salt = RemoteModule::compute_action_execution_salt(Some(&base_options));
    assert_ne!(RemoteModule::compute_action_execution_salt(None), base_salt);
    assert_ne!(
        RemoteModule::compute_action_execution_salt(Some(&parse_remote_options(&[]))),
        base_salt
    );
    assert_ne!(
        RemoteModule::compute_action_execution_salt(Some(&parse_remote_options(&[
            "--remote_cache=grpc://cache1",
            "--remote_default_exec_properties=OSFamily=linux",
        ]))),
        base_salt
    );

    for args in [
        &[
            "--remote_cache=grpc://cache1",
            "--remote_executor=grpc://exec1",
            "--remote_instance_name=instance2",
        ][..],
        &[
            "--remote_cache=grpc://cache2",
            "--remote_executor=grpc://exec1",
            "--remote_instance_name=instance1",
        ][..],
        &[
            "--remote_cache=grpc://cache1",
            "--remote_executor=grpc://exec2",
            "--remote_instance_name=instance1",
        ][..],
        &[
            "--remote_cache=grpc://cache1",
            "--remote_executor=grpc://exec1",
            "--remote_instance_name=instance1",
            "--experimental_remote_downloader=grpc://downloader1",
        ][..],
        &[
            "--remote_cache=grpc://cache1",
            "--remote_executor=grpc://exec1",
            "--remote_instance_name=instance1",
            "--remote_bytestream_uri_prefix=host/instance2",
        ][..],
        &[
            "--remote_cache=grpc://cache1",
            "--remote_executor=grpc://exec1",
            "--remote_instance_name=instance1",
            "--remote_proxy=unix:/socket2",
        ][..],
    ] {
        assert_eq!(
            RemoteModule::compute_action_execution_salt(Some(&parse_remote_options(args))),
            base_salt,
            "{args:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// PORT: not in RemoteModuleTest.java at the pin. Review 2026-11-19: every call of a channel
// whose capabilities requirement is not NONE waits for the verified capabilities and fails with
// their error (GoogleChannelConnectionFactory.java:101-121); the expected texts are the oracle's
// (scripts/remote/caps-matrix, docs/reviews/2026-11-19-remote-response.md).
// ---------------------------------------------------------------------------------------------

/// The capability error of the first remote call after setup, through the combined cache.
fn first_cache_call_error(remote_module: &RemoteModule) -> io::Error {
    remote_module
        .get_action_context_provider()
        .unwrap()
        .get_combined_cache()
        .unwrap()
        .get_remote_cache_capabilities()
        .unwrap_err()
}

#[test]
fn verify_capabilities_missing_compressor_fails_every_call() {
    let env = TestEnv::new();
    let (mut cache_server, cache_server_capabilities_impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));
    remote_options.cache_compression = true;

    let remote_module = before_command(&env, remote_options).unwrap();

    let e = first_cache_call_error(&remote_module);
    let capabilities_exception = errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&e)
        .expect("RemoteExecutionCapabilitiesException");
    assert_eq!(
        capabilities_exception.get_cause().to_string(),
        "--remote_cache_compression requested but remote does not support compression"
    );
    // The check runs once per channel; its failure is sticky.
    let again = first_cache_call_error(&remote_module);
    assert!(errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&again).is_some());
    assert_eq!(cache_server_capabilities_impl.get_request_count(), 1);
    cache_server.shutdown_now();
}

#[test]
fn verify_capabilities_unimplemented_capabilities_fail_with_status_message() {
    let env = TestEnv::new();
    // A server without the Capabilities service answers UNIMPLEMENTED.
    let mut cache_server = FakeServer::new();
    cache_server.start();
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));

    let remote_module = before_command(&env, remote_options).unwrap();

    let e = first_cache_call_error(&remote_module);
    let capabilities_exception = errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&e)
        .expect("RemoteExecutionCapabilitiesException");
    // The cause is the StatusRuntimeException itself, whose message is `CODE: description`.
    assert!(
        capabilities_exception
            .get_cause()
            .to_string()
            .starts_with("UNIMPLEMENTED"),
        "{}",
        capabilities_exception.get_cause()
    );
    assert_eq!(
        errors::status_from_throwable(&e).map(|s| s.code()),
        Some(tonic::Code::Unimplemented)
    );
    cache_server.shutdown_now();
}

#[test]
fn invalid_tls_certificate_fails_calls_not_setup() {
    let env = TestEnv::new();
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some("grpcs://127.0.0.1:1".to_string());
    let mut environment = env.environment(remote_options);
    environment.auth_and_tls_options.tls_certificate = Some("/nonexistent/ca.pem".to_string());

    // Java builds the channel lazily (GoogleChannelConnectionFactory.create): setup succeeds.
    let remote_module = RemoteModule::before_command(environment).unwrap();

    let e = first_cache_call_error(&remote_module);
    assert!(errors::downcast_ref::<RemoteExecutionCapabilitiesException>(&e).is_none());
    assert_eq!(
        e.to_string(),
        "Failed to init TLS infrastructure using '/nonexistent/ca.pem' as root certificate: File does not contain valid certificates: /nonexistent/ca.pem"
    );
}

#[test]
#[should_panic(expected = "not ported yet: DiskCacheGarbageCollectorIdleTask.create")]
fn disk_cache_gc_policy_fails_loudly() {
    let env = TestEnv::new();
    let disk_cache_dir = env.root.get_relative_str("disk_cache");
    let mut remote_options = parse_remote_options(&[
        &format!("--disk_cache={}", disk_cache_dir.get_path_string()),
        "--experimental_disk_cache_gc_max_size=1234567890",
    ]);
    remote_options.disk_cache_gc_idle_delay = Duration::from_secs(120);
    let _ = before_command(&env, remote_options);
}

#[test]
#[should_panic(expected = "not ported yet: DynamicConnectionPool")]
fn remote_max_connections_fails_loudly() {
    let env = TestEnv::new();
    let (_cache_server, _impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&_cache_server));
    remote_options.remote_max_connections = 5;
    let _ = before_command(&env, remote_options);
}

/// Review 2026-11-19 R12: with a gRPC cache, `setup` installs the ByteStream uploader factory
/// (`RemoteModule.java:905`) with `--remote_build_event_upload`; the delegate creates the
/// ByteStream uploader (slow: `mayBeSlow() == true`) once.
#[test]
fn remote_build_event_upload_all_installs_byte_stream_uploader() {
    let env = TestEnv::new();
    let (cache_server, _impl) = create_fake_server(cache_only_caps());
    let remote_options = parse_remote_options(&[
        &format!("--remote_cache={}", target(&cache_server)),
        "--remote_build_event_upload=all",
        "--remote_bytestream_uri_prefix=host/instance2",
    ]);
    let remote_module = before_command(&env, remote_options).unwrap();
    assert!(
        remote_module
            .get_build_event_artifact_uploader_factory()
            .is_some()
    );
}

#[test]
fn build_event_artifact_uploader_with_grpc_cache_is_byte_stream() {
    let env = TestEnv::new();
    let (cache_server, _impl) = create_fake_server(cache_only_caps());
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_cache = Some(target(&cache_server));
    let remote_module = before_command(&env, remote_options).unwrap();
    let uploader = remote_module.create_build_event_artifact_uploader();
    assert!(uploader.may_be_slow());
    let factory = remote_module
        .get_build_event_artifact_uploader_factory()
        .unwrap();
    assert!(factory.get().is_some());
    uploader.release();
}

#[test]
fn build_event_artifact_uploader_without_grpc_cache_is_local() {
    let env = TestEnv::new();
    let disk_cache_dir = env.root.get_relative_str("disk_cache");
    let remote_options = parse_remote_options(&[&format!(
        "--disk_cache={}",
        disk_cache_dir.get_path_string()
    )]);
    let remote_module = before_command(&env, remote_options).unwrap();
    assert!(
        remote_module
            .get_build_event_artifact_uploader_factory()
            .is_none()
    );
    assert!(
        !remote_module
            .create_build_event_artifact_uploader()
            .may_be_slow()
    );
}

#[test]
#[should_panic(expected = "not ported yet: RemoteLeaseExtension")]
fn remote_cache_lease_extension_fails_loudly() {
    let env = TestEnv::new();
    let disk_cache_dir = env.root.get_relative_str("disk_cache");
    let remote_options = parse_remote_options(&[
        &format!("--disk_cache={}", disk_cache_dir.get_path_string()),
        "--experimental_remote_cache_lease_extension",
    ]);
    let _ = before_command(&env, remote_options);
}

/// Review 2026-11-19 R15: `getTempPathGenerator`'s failure carries Java's explicit
/// `ExitCode.LOCAL_ENVIRONMENTAL_ERROR` (36), not the metadata of
/// `DOWNLOADED_INPUTS_DELETION_FAILURE` (34).
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1215 RemoteModule.getTempPathGenerator
#[test]
fn stale_downloads_deletion_failure_exits_with_local_environmental_error() {
    use std::os::unix::fs::PermissionsExt;
    let env = TestEnv::new();
    let action_temps = env.root.get_relative_str("output/action_temps");
    let stale = action_temps.get_relative_str("remote/stale");
    stale.create_directory_and_parents().unwrap();
    // The stale directory cannot be removed from its parent.
    let action_temps_os = std::path::PathBuf::from(action_temps.get_path_string());
    std::fs::set_permissions(&action_temps_os, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.disk_cache = Some(blaze_options::Foreign(blaze_vfs::PathFragment::create(
        env.root.get_relative_str("disk").get_path_string(),
    )));

    let result = before_command(&env, remote_options);
    std::fs::set_permissions(&action_temps_os, std::fs::Permissions::from_mode(0o755)).unwrap();

    let exception = result.err().expect("AbruptExitException");
    assert!(
        exception
            .get_message()
            .starts_with("Failed to delete stale downloads: "),
        "{}",
        exception.get_message()
    );
    assert_eq!(exception.exit_code, Some(36));
    assert!(
        env.reporter
            .get_events()
            .iter()
            .any(|e| e.get_message() == "Found stale downloads from previous build, deleting...")
    );
}

/// Review 2026-11-19 R15: an options failure (`createOptionsExitException`) has no explicit exit
/// code; its exit code is the failure detail's metadata.
// Java: com/google/devtools/build/lib/remote/RemoteModule.java:1370 RemoteModule.createOptionsExitException
#[test]
fn options_exit_exception_has_no_explicit_exit_code() {
    let env = TestEnv::new();
    let mut output_service_server = FakeServer::new();
    output_service_server.start();
    let mut remote_options = Options::<RemoteOptions>::get_defaults();
    remote_options.remote_output_service = Some(target(&output_service_server));

    let exception = before_command(&env, remote_options)
        .err()
        .expect("AbruptExitException");

    assert_eq!(exception.exit_code, None);
    output_service_server.shutdown_now();
}
