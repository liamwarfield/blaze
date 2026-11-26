//! The Build Event Service part of `BuildEventServiceModule` and `BazelBuildEventServiceModule`:
//! the client (kept between commands while its configuration does not change), the keywords,
//! the results URL and the creation of the transport (`createBesTransport`).
//!
//! The file transports, the streamer and the waiting for the transports are the runtime's
//! (`blaze_runtime::commands::bep`, `blaze_bep::bep_module`).

use std::io;
use std::sync::Arc;

use blaze_bep::buildeventstream::artifact_uploader::BuildEventArtifactUploader;
use blaze_bep::buildeventstream::build_event_context::ArtifactGroupNamer;
use blaze_events::{Event, EventBus, EventHandler};
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_options::OptionsBase;
use blaze_protos::failure_details::build_progress::Code as BuildProgressCode;
use blaze_remote::authandtls::google_auth_utils;
use blaze_remote::grpc::channel::{
    AttachHeadersInterceptor, CallCredentials, ClientInterceptor, ReferenceCountedChannel,
};
use blaze_repo_fetch::authandtls::credentialhelper::command_line_path_factory::CommandLinePathFactory;
use blaze_repo_fetch::authandtls::credentialhelper::credential_cache::credential_module;
use blaze_repo_fetch::authandtls::credentialhelper::credential_helper_environment::CredentialHelperEnvironment;
use blaze_vfs::{FileSystemRef, Path};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use crate::client::grpc_client::BuildEventServiceGrpcClient;
use crate::client::{BuildEventServiceClient, CommandContext, Instant};
use crate::transport::{self, BuildEventServiceTransport};
use crate::uploader::BesClock;

/// TargetComplete BEP events scale with the value of `--runs_per_test`, thus setting a very
/// large value can result in BEP events that are too big for BES to handle.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:113 BuildEventServiceModule.RUNS_PER_TEST_LIMIT
pub const RUNS_PER_TEST_LIMIT: i32 = 100000;

/// The commands that stream build events.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:188 BazelBuildEventServiceModule.ALLOWED_COMMANDS
pub const ALLOWED_COMMANDS: &[&str] = &[
    "fetch",
    "build",
    "test",
    "run",
    "query",
    "aquery",
    "cquery",
    "coverage",
    "mobile-install",
];

/// The configuration the cached client was created with.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:50 BazelBuildEventServiceModule.BackendConfig
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub bes_backend: String,
    pub bes_proxy: Option<String>,
    pub bes_headers: Vec<(String, String)>,
    pub auth_and_tls_options: AuthAndTLSOptions,
}

impl BackendConfig {
    /// `ImmutableMap.builder().putAll(besHeaders).buildKeepingLast()`: each name once, at the
    /// position of its first occurrence, with its last value.
    // Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:62 BazelBuildEventServiceModule.BackendConfig.create
    pub fn create(
        bes_options: &BuildEventServiceOptions,
        auth_and_tls_options: &AuthAndTLSOptions,
    ) -> BackendConfig {
        let mut headers: IndexMap<String, String> = IndexMap::new();
        for (name, value) in &bes_options.bes_headers {
            headers.insert(name.clone(), value.clone());
        }
        BackendConfig {
            bes_backend: bes_options.bes_backend.clone(),
            bes_proxy: bes_options.bes_proxy.clone(),
            bes_headers: headers.into_iter().collect(),
            auth_and_tls_options: auth_and_tls_options.clone(),
        }
    }
}

impl PartialEq for BackendConfig {
    /// The record's `equals`: `AuthAndTLSOptions` compares as `OptionsBase.equals` does (every
    /// option value).
    fn eq(&self, other: &BackendConfig) -> bool {
        let a: &dyn OptionsBase = &self.auth_and_tls_options;
        self.bes_backend == other.bes_backend
            && self.bes_proxy == other.bes_proxy
            && self.bes_headers == other.bes_headers
            && a.options_equal(&other.auth_and_tls_options)
    }
}

/// The extra headers of every call (`makeGrpcMetadata(config)`).
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:142 BazelBuildEventServiceModule.makeGrpcMetadata
pub fn make_grpc_metadata(config: &BackendConfig) -> Vec<(String, String)> {
    config.bes_headers.clone()
}

/// The interceptor adding the `--bes_header`s; `None` without headers.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:134 BazelBuildEventServiceModule.makeGrpcInterceptor
fn make_grpc_interceptor(config: &BackendConfig) -> io::Result<Option<Arc<dyn ClientInterceptor>>> {
    if config.bes_headers.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(AttachHeadersInterceptor::new(
        &make_grpc_metadata(config),
    )?)))
}

// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:96 BazelBuildEventServiceModule.getBesKeywords
pub fn get_bes_keywords(
    command_name: &str,
    bes_options: &BuildEventServiceOptions,
) -> IndexSet<String> {
    let mut builder = IndexSet::new();
    builder.insert("protocol_name=BEP".to_string());
    builder.insert(format!("command_name={command_name}"));
    for system_keyword in &bes_options.bes_system_keywords {
        builder.insert(system_keyword.clone());
    }
    for user_keyword in &bes_options.bes_keywords {
        builder.insert(format!("user_keyword={user_keyword}"));
    }
    builder
}

/// `getInvocationIdPrefix()`: `--bes_results_url`, with a trailing `/`; empty without one.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:204 BazelBuildEventServiceModule.getInvocationIdPrefix
pub fn get_invocation_id_prefix(bes_options: &BuildEventServiceOptions) -> String {
    let url = &bes_options.bes_results_url;
    if url.is_empty() {
        return String::new();
    }
    if url.ends_with('/') {
        url.clone()
    } else {
        format!("{url}/")
    }
}

/// `getBuildRequestIdPrefix()`: empty in Bazel.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:214 BazelBuildEventServiceModule.getBuildRequestIdPrefix
pub fn get_build_request_id_prefix() -> String {
    String::new()
}

/// `constructAndMaybeReportInvocationIdUrl()`: `Streaming build results to: <url><id>`, in
/// cyan when the UI uses color.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:701 BuildEventServiceModule.constructAndMaybeReportInvocationIdUrl
pub fn construct_and_maybe_report_invocation_id_url(
    reporter: &dyn EventHandler,
    bes_options: &BuildEventServiceOptions,
    invocation_id: &str,
    ui_uses_color: bool,
) {
    let prefix = get_invocation_id_prefix(bes_options);
    if prefix.is_empty() {
        return;
    }
    let mut msg = String::from("Streaming build results to: ");
    if ui_uses_color {
        // Color.CYAN.getEscapeSeq()
        msg.push_str("\u{1b}[36m");
    }
    msg.push_str(&prefix);
    msg.push_str(invocation_id);
    if ui_uses_color {
        // Color.DEFAULT.getEscapeSeq()
        msg.push_str("\u{1b}[0m");
    }
    reporter.handle(&Event::info(msg));
}

/// `constructAndMaybeReportBuildRequestIdUrl()`: nothing in Bazel (no prefix).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:718 BuildEventServiceModule.constructAndMaybeReportBuildRequestIdUrl
pub fn construct_and_maybe_report_build_request_id_url(
    reporter: &dyn EventHandler,
    build_request_id: &str,
) {
    let prefix = get_build_request_id_prefix();
    if !prefix.is_empty() {
        reporter.handle(&Event::info(format!(
            "See {prefix}{build_request_id} for more information about your request."
        )));
    }
}

/// The module's client and the configuration it was created with (`client`, `config`). They
/// live as long as the server, as the module does.
static CLIENT: Mutex<Option<(BackendConfig, Arc<BuildEventServiceGrpcClient>)>> = Mutex::new(None);

/// What `getBesClient` reads of the command environment (the credentials of the backend).
pub struct CredentialsEnvironment<'a> {
    /// `env.getReporter()`.
    pub reporter: Arc<dyn EventHandler>,
    /// `env.getWorkspace()`.
    pub workspace: Option<Path>,
    /// `env.getClientEnv()`.
    pub client_env: IndexMap<String, String>,
    /// `env.getCommandLinePathFactory()`.
    pub command_line_path_factory: &'a CommandLinePathFactory,
    /// `env.getRuntime().getFileSystem()`.
    pub file_system: &'a FileSystemRef,
}

/// `getBesClient(env, besOptions, authAndTLSOptions)`: the cached client, or a new one when the
/// configuration changed.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:111 BazelBuildEventServiceModule.getBesClient
pub fn get_bes_client(
    env: &CredentialsEnvironment<'_>,
    bes_options: &BuildEventServiceOptions,
    auth_and_tls_options: &AuthAndTLSOptions,
) -> io::Result<Arc<BuildEventServiceGrpcClient>> {
    let new_config = BackendConfig::create(bes_options, auth_and_tls_options);
    let mut client = CLIENT.lock();
    if let Some((config, client)) = &*client
        && *config == new_config
    {
        return Ok(client.clone());
    }
    clear_bes_client_locked(&mut client);
    let credentials = google_auth_utils::new_credentials(
        &CredentialHelperEnvironment {
            event_reporter: env.reporter.clone(),
            workspace_path: env.workspace.clone(),
            client_environment: env.client_env.clone(),
            helper_execution_timeout: auth_and_tls_options.credential_helper_timeout,
        },
        credential_module().get_credential_cache(),
        env.command_line_path_factory,
        env.file_system,
        &new_config.auth_and_tls_options,
    )?;
    // credentials != null ? MoreCallCredentials.from(credentials) : null
    let call_credentials: Option<Arc<dyn CallCredentials>> = Some(Arc::new(
        google_auth_utils::GoogleAuthLibraryCallCredentials::new(credentials),
    ));
    let channel = new_grpc_channel(&new_config)?;
    let new_client = Arc::new(BuildEventServiceGrpcClient::new(channel, call_credentials));
    *client = Some((new_config, new_client.clone()));
    Ok(new_client)
}

/// `newGrpcChannel(config)`: a channel to `--bes_backend` (through `--bes_proxy`), with the
/// `--bes_header` interceptor.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:154 BazelBuildEventServiceModule.newGrpcChannel
fn new_grpc_channel(config: &BackendConfig) -> io::Result<ReferenceCountedChannel> {
    let managed = google_auth_utils::new_channel(
        &config.bes_backend,
        config.bes_proxy.as_deref(),
        &config.auth_and_tls_options,
        /* interceptors= */ None,
        /* serviceConfig= */ None,
    )?;
    let mut interceptors = managed.interceptors;
    if let Some(interceptor) = make_grpc_interceptor(config)? {
        interceptors.push(interceptor);
    }
    Ok(ReferenceCountedChannel::new(
        managed.channel,
        &managed.authority,
        managed.tls,
        interceptors,
        managed.max_inbound_message_size,
        None,
    )
    .with_transport_lifecycle(managed.transport_lifecycle))
}

/// `clearBesClient()`: shuts the cached client down.
// Java: com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java:166 BazelBuildEventServiceModule.clearBesClient
pub fn clear_bes_client() {
    clear_bes_client_locked(&mut CLIENT.lock());
}

fn clear_bes_client_locked(client: &mut Option<(BackendConfig, Arc<BuildEventServiceGrpcClient>)>) {
    if let Some((_, client)) = client.take() {
        client.shutdown();
    }
}

/// An error `createBesTransport` reports (`reportError`): `Event.error(exception message)`, then
/// `moduleEnvironment.exit(createAbruptExitException(exception, msg, code))`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BesReportedError {
    /// Whether the error is reported as an error event (`reportError`) before the exit; the
    /// failure of the uploader supplier only exits (`createBepTransports`' caller).
    pub report_event: bool,
    /// `exception.getMessage()`: the reported error event.
    pub exception_message: String,
    /// `msg`.
    pub message: String,
    pub code: BuildProgressCode,
}

impl BesReportedError {
    /// The message of the exit's failure detail (`createAbruptExitException`):
    /// `msg + " " + e.getMessage()`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:934 BuildEventServiceModule.createAbruptExitException
    pub fn failure_message(&self) -> String {
        format!("{} {}", self.message, self.exception_message)
    }
}

/// What `createBesTransport` reads of the command and the module.
pub struct BesTransportEnvironment<'a> {
    pub credentials: CredentialsEnvironment<'a>,
    pub bes_options: &'a BuildEventServiceOptions,
    pub auth_and_tls_options: &'a AuthAndTLSOptions,
    pub bep_options: Arc<BuildEventProtocolOptions>,
    /// `isRunsPerTestOverTheLimit`.
    pub is_runs_per_test_over_the_limit: bool,
    /// `cmdEnv.getBuildRequestId()`.
    pub build_request_id: String,
    /// `cmdEnv.getCommandId().toString()`.
    pub invocation_id: String,
    /// `cmdEnv.getAttemptNumber()`.
    pub attempt_number: i32,
    /// `cmdEnv.getCommandName()`.
    pub command_name: String,
    /// `cmdEnv.getCommandStartTime()` (milliseconds since the epoch).
    pub command_start_time_millis: i64,
    pub clock: Arc<dyn BesClock>,
    pub event_bus: Arc<dyn EventBus>,
    pub artifact_group_namer: Arc<dyn ArtifactGroupNamer>,
}

/// `createBesTransport(cmdEnv, uploaderSupplier, artifactGroupNamer)`: `Ok(None)` without
/// `--bes_backend`; `Err` with what `reportError` reports when the transport cannot be created.
/// `uploader` is called for the local file uploader only when the transport is created
/// (`uploaderSupplier.get()`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceModule.java:737 BuildEventServiceModule.createBesTransport
pub fn create_bes_transport(
    env: BesTransportEnvironment<'_>,
    uploader: impl FnOnce() -> io::Result<Arc<dyn BuildEventArtifactUploader>>,
) -> Result<Option<BuildEventServiceTransport>, BesReportedError> {
    if env.bes_options.bes_backend.is_empty() {
        clear_bes_client();
        return Ok(None);
    }

    if env.is_runs_per_test_over_the_limit {
        let msg = format!(
            "The value of --runs_per_test is bigger than {RUNS_PER_TEST_LIMIT} and it will \
             produce build events that are too big for the Build Event Service to handle."
        );
        return Err(BesReportedError {
            report_event: true,
            exception_message: msg.clone(),
            message: msg,
            code: BuildProgressCode::BesRunsPerTestLimitUnsupported,
        });
    }

    // logIds(): logging is not ported.
    // The connectivity status (ConnectivityStatusProvider) is always OK in Bazel.

    let bes_client: Arc<dyn BuildEventServiceClient> =
        match get_bes_client(&env.credentials, env.bes_options, env.auth_and_tls_options) {
            Ok(client) => client,
            Err(e) => {
                let message = e.to_string();
                return Err(BesReportedError {
                    report_event: true,
                    exception_message: message.clone(),
                    message,
                    code: BuildProgressCode::BesInitializationError,
                });
            }
        };

    let command_context = CommandContext::new(
        env.build_request_id,
        env.invocation_id,
        env.attempt_number,
        get_bes_keywords(&env.command_name, env.bes_options),
        env.bes_options.instance_name.clone(),
        env.bes_options.bes_check_preceding_lifecycle_events,
    );

    // uploaderSupplier.get(): the supplier's IOException leaves createBepTransports.
    let local_file_uploader = uploader().map_err(|e| BesReportedError {
        report_event: false,
        exception_message: e.to_string(),
        message: "Could not create BEP transports.".to_string(),
        code: BuildProgressCode::BesInitializationError,
    })?;

    Ok(Some(
        transport::Builder {
            bes_client,
            local_file_uploader,
            bes_options: env.bes_options.clone(),
            bep_options: env.bep_options,
            clock: env.clock,
            artifact_group_namer: env.artifact_group_namer,
            event_bus: env.event_bus,
            sleeper: None,
            command_context,
            command_start_time: Instant::of_epoch_milli(env.command_start_time_millis),
        }
        .build(),
    ))
}
