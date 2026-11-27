// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under
//! a temporary directory, so Java's absolute paths (`/home/foo`) are prefixed with that
//! directory.
//!
//! The tests after the Java ones (`new_channel_*`, `is_tls_enabled_*`) have no Java counterpart:
//! Java has no unit test for `GoogleAuthUtils.newChannel`. They pin the decisions the port has
//! to take itself (which scheme encrypts, the texts of the TLS and proxy errors).

use std::sync::Arc;

use blaze_events::StoredEventHandler;
use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_optdefs::misc::auth_and_tls_options::CredentialHelperOptionConverter;
use blaze_remote::authandtls::Credentials;
use blaze_remote::authandtls::google_auth_utils::{
    http_cache_tls_config, is_tls_enabled, new_channel, new_credential_helper_provider,
    new_credentials_from_netrc,
};
use blaze_repo_fetch::authandtls::credentialhelper::{
    CommandLinePathFactory, CredentialHelperEnvironment, CredentialHelperProvider,
};
use blaze_repo_fetch::authandtls::{Headers, basic_http_authentication_encode};
use blaze_repo_fetch::fs_utils;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;

/// `new InMemoryFileSystem(DigestHashFunction.SHA256)` and the directory Java's `/` is.
struct Fixture {
    _dir: tempfile::TempDir,
    file_system: FileSystemRef,
    root: String,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        Fixture {
            _dir: dir,
            file_system: UnixFileSystem::create(DigestHashFunction::Sha256),
            root,
        }
    }

    /// `scratch.file(path, content)`.
    fn scratch_file(&self, path: &str, content: &str) {
        let file = Path::create_from_str(path, &self.file_system);
        file.get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        fs_utils::write_content(&file, content.as_bytes()).unwrap();
    }
}

fn client_env(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java:345 GoogleAuthUtilsTest.assertRequestMetadata
fn assert_request_metadata(request_metadata: &Headers, username: &str, password: &str) {
    assert_eq!(
        request_metadata.keys().collect::<Vec<_>>(),
        vec!["Authorization"]
    );
    assert_eq!(
        request_metadata.values().next().unwrap(),
        &vec![basic_http_authentication_encode(username, password)]
    );
}

fn request_metadata(credentials: &Arc<dyn Credentials>, uri: &str) -> Headers {
    credentials
        .get_request_metadata(&JavaUri::create(uri).unwrap())
        .unwrap()
}

#[test]
fn test_netrc_empty_env_should_ignore() {
    let f = Fixture::new();
    let client_env = client_env(&[]);

    assert!(
        new_credentials_from_netrc(&client_env, &f.file_system)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_netrc_netrc_not_exist_should_ignore() {
    let f = Fixture::new();
    let home = format!("{}/home/foo", f.root);
    let client_env = client_env(&[("HOME", &home)]);

    assert!(
        new_credentials_from_netrc(&client_env, &f.file_system)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_netrc_netrc_exist_should_use() {
    let f = Fixture::new();
    let home = format!("{}/home/foo", f.root);
    let client_env = client_env(&[("HOME", &home)]);
    f.scratch_file(
        &format!("{home}/.netrc"),
        "machine foo.example.org login foouser password foopass",
    );

    let credentials = new_credentials_from_netrc(&client_env, &f.file_system).unwrap();

    assert!(credentials.is_some());
    assert_request_metadata(
        &request_metadata(&credentials.unwrap(), "https://foo.example.org"),
        "foouser",
        "foopass",
    );
}

#[test]
fn test_netrc_netrc_from_netrc_env_exist_should_use() {
    let f = Fixture::new();
    let home = format!("{}/home/foo", f.root);
    let netrc = format!("{}/.netrc", f.root);
    let client_env = client_env(&[("HOME", &home), ("NETRC", &netrc)]);
    f.scratch_file(
        &format!("{home}/.netrc"),
        "machine foo.example.org login foouser password foopass",
    );
    f.scratch_file(
        &netrc,
        "machine foo.example.org login baruser password barpass",
    );

    let credentials = new_credentials_from_netrc(&client_env, &f.file_system).unwrap();

    assert!(credentials.is_some());
    assert_request_metadata(
        &request_metadata(&credentials.unwrap(), "https://foo.example.org"),
        "baruser",
        "barpass",
    );
}

#[test]
fn test_netrc_netrc_from_netrc_env_not_exist_should_ignore() {
    let f = Fixture::new();
    let home = format!("{}/home/foo", f.root);
    let netrc = format!("{}/.netrc", f.root);
    let client_env = client_env(&[("HOME", &home), ("NETRC", &netrc)]);
    f.scratch_file(
        &format!("{home}/.netrc"),
        "machine foo.example.org login foouser password foopass",
    );

    assert!(
        new_credentials_from_netrc(&client_env, &f.file_system)
            .unwrap()
            .is_none()
    );
}

/// `createExecutable(path)`: an empty executable file.
fn create_executable(path: &Path) -> Path {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    fs_utils::write_content(path, b"").unwrap();
    path.set_executable(true).unwrap();
    path.clone()
}

/// `newCredentialHelperProvider(environment, factory, inputs)` of the Java test: the inputs go
/// through `CredentialHelperOptionConverter`.
fn new_credential_helper_provider_of(
    environment: &CredentialHelperEnvironment,
    factory: &CommandLinePathFactory,
    inputs: &[String],
) -> CredentialHelperProvider {
    let options: Vec<_> = inputs
        .iter()
        .map(|s| CredentialHelperOptionConverter.convert_option(s).unwrap())
        .collect();
    new_credential_helper_provider(environment, factory, &options).unwrap()
}

#[test]
fn test_credential_helper_provider() {
    let f = Fixture::new();
    let fs = &f.file_system;
    let at = |p: &str| Path::create_from_str(&format!("{}{}", f.root, p), fs);

    let workspace = at("/workspace");
    let path_value = at("/usr/local/bin");
    path_value.create_directory_and_parents().unwrap();

    let environment = CredentialHelperEnvironment {
        event_reporter: Arc::new(StoredEventHandler::new()),
        workspace_path: Some(workspace.clone()),
        client_environment: client_env(&[("PATH", path_value.get_path_string())]),
        helper_execution_timeout: std::time::Duration::ZERO,
    };
    let mut roots = IndexMap::new();
    roots.insert("workspace".to_string(), workspace.clone());
    let factory = CommandLinePathFactory::new(fs.clone(), roots);

    let unused_helper = create_executable(&at("/unused/helper"));

    let default_helper = create_executable(&at("/default/helper"));
    let example_com_helper = create_executable(&at("/example/com/helper"));
    let foo_example_com_helper = create_executable(&at("/foo/example/com/helper"));
    let example_com_wildcard_helper = create_executable(&at("/example/com/wildcard/helper"));

    let example_org_helper = create_executable(&workspace.get_relative_str("helpers/example-org"));

    let s = |p: &Path| p.get_path_string().to_string();
    let scoped = |scope: &str, p: &Path| format!("{scope}={}", p.get_path_string());
    let find = |provider: &CredentialHelperProvider, uri: &str| {
        provider
            .find_credential_helper(&JavaUri::create(uri).unwrap())
            .map(|h| h.get_path().clone())
    };

    // No helpers.
    let provider1 = new_credential_helper_provider_of(&environment, &factory, &[]);
    assert_eq!(find(&provider1, "https://example.com"), None);
    assert_eq!(find(&provider1, "https://foo.example.com"), None);

    // Default helper only.
    let provider2 =
        new_credential_helper_provider_of(&environment, &factory, &[s(&default_helper)]);
    assert_eq!(
        find(&provider2, "https://example.com"),
        Some(default_helper.clone())
    );
    assert_eq!(
        find(&provider2, "https://foo.example.com"),
        Some(default_helper.clone())
    );

    // Default and exact match.
    let provider3 = new_credential_helper_provider_of(
        &environment,
        &factory,
        &[
            s(&default_helper),
            scoped("example.com", &example_com_helper),
        ],
    );
    assert_eq!(
        find(&provider3, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&provider3, "https://foo.example.com"),
        Some(default_helper.clone())
    );

    // Exact match without default.
    let provider4 = new_credential_helper_provider_of(
        &environment,
        &factory,
        &[scoped("example.com", &example_com_helper)],
    );
    assert_eq!(
        find(&provider4, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(find(&provider4, "https://foo.example.com"), None);

    // Multiple scoped helpers with default.
    let provider5 = new_credential_helper_provider_of(
        &environment,
        &factory,
        &[
            s(&default_helper),
            scoped("example.com", &example_com_helper),
            scoped("*.foo.example.com", &foo_example_com_helper),
            scoped("*.example.com", &example_com_wildcard_helper),
            "example.org=%workspace%/helpers/example-org".to_string(),
        ],
    );
    assert_eq!(
        find(&provider5, "https://anotherdomain.com"),
        Some(default_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://foo.example.com"),
        Some(foo_example_com_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://abc.foo.example.com"),
        Some(foo_example_com_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://bar.example.com"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://abc.bar.example.com"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        find(&provider5, "https://example.org"),
        Some(example_org_helper.clone())
    );

    // Helpers override.
    let provider6 = new_credential_helper_provider_of(
        &environment,
        &factory,
        &[
            // <system .bazelrc>
            s(&unused_helper),
            // <user .bazelrc>
            s(&default_helper),
            scoped("example.com", &unused_helper),
            scoped("*.example.com", &unused_helper),
            scoped("example.org", &unused_helper),
            scoped("*.example.org", &example_org_helper),
            // <workspace .bazelrc>
            scoped("*.example.com", &example_com_wildcard_helper),
            scoped("example.org", &example_org_helper),
            scoped("*.foo.example.com", &unused_helper),
            // <command-line>
            scoped("example.com", &example_com_helper),
            scoped("*.foo.example.com", &foo_example_com_helper),
        ],
    );
    assert_eq!(
        find(&provider6, "https://anotherdomain.com"),
        Some(default_helper)
    );
    assert_eq!(
        find(&provider6, "https://example.com"),
        Some(example_com_helper)
    );
    assert_eq!(
        find(&provider6, "https://foo.example.com"),
        Some(foo_example_com_helper)
    );
    assert_eq!(
        find(&provider6, "https://bar.example.com"),
        Some(example_com_wildcard_helper)
    );
    assert_eq!(
        find(&provider6, "https://example.org"),
        Some(example_org_helper.clone())
    );
    assert_eq!(
        find(&provider6, "https://foo.example.org"),
        Some(example_org_helper)
    );
}

// The tests below have no Java counterpart (see the file comment).

#[test]
fn is_tls_enabled_follows_the_scheme_of_the_target() {
    // 'grpcs://' or empty prefix => TLS-enabled
    assert!(is_tls_enabled("grpcs://remote.example.org"));
    assert!(is_tls_enabled("remote.example.org:443"));
    assert!(is_tls_enabled("localhost:1234"));
    // Only the exact prefixes turn TLS off.
    assert!(!is_tls_enabled("grpc://remote.example.org"));
    assert!(!is_tls_enabled("unix:/tmp/socket"));
    assert!(is_tls_enabled("GRPC://remote.example.org"));
    assert!(is_tls_enabled("http://remote.example.org"));
}

#[test]
fn new_channel_plaintext_only_for_grpc_scheme() {
    let options = AuthAndTLSOptions::default();

    let channel = new_channel("grpc://localhost:1234", None, &options, None, None).unwrap();
    assert!(!channel.tls);
    assert_eq!(channel.authority, "localhost:1234");
    assert_eq!(channel.max_inbound_message_size, i32::MAX as usize);

    let channel = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap();
    assert!(channel.tls);
    assert_eq!(channel.authority, "localhost:1234");

    let channel = new_channel("localhost:1234", Some(""), &options, None, None).unwrap();
    assert!(channel.tls);
    assert_eq!(channel.authority, "localhost:1234");
}

#[test]
fn new_channel_tls_authority_override_only_on_tls_channels() {
    let mut options = AuthAndTLSOptions::default();
    options.tls_authority_override = Some("foo.test.google.fr".to_string());

    let channel = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap();
    assert!(channel.tls);
    assert_eq!(channel.authority, "foo.test.google.fr");

    // if (sslContext != null) { ... builder.overrideAuthority(...) }
    let channel = new_channel("grpc://localhost:1234", None, &options, None, None).unwrap();
    assert!(!channel.tls);
    assert_eq!(channel.authority, "localhost:1234");
}

#[test]
fn new_channel_unsupported_proxy() {
    let options = AuthAndTLSOptions::default();

    let e = new_channel(
        "grpc://localhost:1234",
        Some("http://proxy:8080"),
        &options,
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "Remote proxy unsupported: http://proxy:8080");
}

#[test]
fn new_channel_root_certificate_must_hold_certificates() {
    let f = Fixture::new();
    let missing = format!("{}/missing.pem", f.root);
    let mut options = AuthAndTLSOptions::default();
    options.tls_certificate = Some(missing.clone());

    let e = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "Failed to init TLS infrastructure using '{missing}' as root certificate: \
             File does not contain valid certificates: {missing}"
        )
    );

    let garbage = format!("{}/garbage.pem", f.root);
    f.scratch_file(&garbage, "not a certificate");
    options.tls_certificate = Some(garbage.clone());

    let e = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "Failed to init TLS infrastructure using '{garbage}' as root certificate: \
             File does not contain valid certificates: {garbage}"
        )
    );

    // The certificate options are not read for a plaintext channel.
    let channel = new_channel("grpc://localhost:1234", None, &options, None, None).unwrap();
    assert!(!channel.tls);
}

#[test]
fn new_channel_client_certificate_and_key_must_be_valid() {
    let f = Fixture::new();
    let cert = format!("{}/client.crt", f.root);
    let key = format!("{}/client.key", f.root);
    let mut options = AuthAndTLSOptions::default();
    options.tls_client_certificate = Some(cert.clone());
    options.tls_client_key = Some(key.clone());

    let e = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "Failed to init TLS infrastructure using '{cert}' as client certificate: \
             File does not contain valid certificates: {cert}"
        )
    );

    // A certificate section in base64 (its content is only parsed by the TLS library).
    f.scratch_file(
        &cert,
        "-----BEGIN CERTIFICATE-----\nMAA=\n-----END CERTIFICATE-----\n",
    );
    // A PKCS#1 key is not a PKCS#8 key.
    f.scratch_file(
        &key,
        "-----BEGIN RSA PRIVATE KEY-----\nMAA=\n-----END RSA PRIVATE KEY-----\n",
    );

    let e = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "Failed to init TLS infrastructure using '{cert}' as client certificate: \
             File does not contain valid private key: {key}"
        )
    );

    // Only one of the two options: no client authentication is configured, as in Java
    // (`clientCert != null && clientKey != null`).
    options.tls_client_key = None;
    let channel = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap();
    assert!(channel.tls);
}

/// Not in the Java test (review round 6, M1): Netty reads a client key with the RSA, DSA and EC
/// key factories only, so an Ed25519 PKCS#8 key is not a valid private key. The oracle
/// (`--tls_client_key` with an Ed25519 key): `WARNING: Remote Cache: Failed to init TLS
/// infrastructure using '<crt>' as client certificate: File does not contain valid private key:
/// <key>`, then the build goes on without the cache.
#[test]
fn new_channel_ed25519_client_key_is_refused() {
    let f = Fixture::new();
    let cert = format!("{}/client.crt", f.root);
    let key = format!("{}/client.key", f.root);
    f.scratch_file(
        &cert,
        "-----BEGIN CERTIFICATE-----\nMAA=\n-----END CERTIFICATE-----\n",
    );
    f.scratch_file(
        &key,
        "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIMJPRDT3tnYa/U3MS8THMSLSvpjAGNIhzdwflzNt42yI\n-----END PRIVATE KEY-----\n",
    );
    let mut options = AuthAndTLSOptions::default();
    options.tls_client_certificate = Some(cert.clone());
    options.tls_client_key = Some(key.clone());

    let e = new_channel("grpcs://localhost:1234", None, &options, None, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        format!(
            "Failed to init TLS infrastructure using '{cert}' as client certificate: \
             File does not contain valid private key: {key}"
        )
    );
}

/// Not in the Java test (lead, round 2; root ruling 2026-11-19: X.509 v1 certificates stay
/// refused). rustls refuses the X.509 v1 client certificate of Bazel's own TLS tests
/// (`src/test/testdata/test_tls_certificate`, copied to `tests/testdata/x509v1`) when the TLS
/// configuration is built; Java judges certificates only in the handshake. The channel is
/// created, and every connection fails with the rejection as a transport error (so the
/// capabilities check fails the build, as without a client certificate), not with "Failed to
/// init TLS infrastructure".
#[test]
fn new_channel_rejected_client_certificate_fails_connections_not_channel_creation() {
    let data = format!("{}/tests/testdata/x509v1", env!("CARGO_MANIFEST_DIR"));
    let options = AuthAndTLSOptions {
        tls_certificate: Some(format!("{data}/ca.crt")),
        tls_client_certificate: Some(format!("{data}/client.crt")),
        tls_client_key: Some(format!("{data}/client.pem")),
        ..AuthAndTLSOptions::default()
    };

    let managed = new_channel("grpcs://localhost:1", None, &options, None, None).unwrap();

    let channel = blaze_remote::grpc::ReferenceCountedChannel::new(
        managed.channel,
        &managed.authority,
        managed.tls,
        managed.interceptors,
        managed.max_inbound_message_size,
        None,
    );
    let status = blaze_remote::runtime::block_on(channel.unary::<
        blaze_remote::reapi::GetCapabilitiesRequest,
        blaze_remote::reapi::ServerCapabilities,
    >(
        blaze_remote::grpc::methods::GET_CAPABILITIES,
        blaze_remote::reapi::GetCapabilitiesRequest::default(),
        blaze_remote::grpc::CallOptions::new(),
    ))
    .unwrap_err();
    let cause = blaze_remote::grpc::channel::transport_io_cause(&status).unwrap();
    assert!(
        cause.to_string().starts_with("invalid peer certificate: "),
        "{cause}"
    );
}

/// Review 2026-11-19 round 8, S2: the declared stricter host-name check of an HTTPS cache
/// (`http_cache_tls_config`). Netty's client context of `HttpCacheClient.createSSLContext` does
/// not identify the endpoint, so Bazel accepts a certificate for another name; Blaze refuses it.
/// The certificate is the fixture `v3_issued_san` (subjectAltName `localhost`, issued by the
/// fixtures' `ca.pem`), presented with its key; `localhost` is accepted, `example.com` refused.
#[test]
fn http_cache_tls_config_checks_the_host_name_stricter_than_netty() {
    use tokio_rustls::rustls::pki_types::pem::PemObject;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/jdk_cert_fixtures");
    let handshake = |host: &str| -> Result<(), String> {
        let mut options = AuthAndTLSOptions::default();
        options.tls_certificate = Some(dir.join("ca.pem").to_str().unwrap().to_string());
        let client_config = http_cache_tls_config(host, &options).map_err(|e| e.to_string())?;
        let cert = CertificateDer::from(std::fs::read(dir.join("v3_issued_san.der")).unwrap());
        let key = PrivateKeyDer::from_pem_slice(&std::fs::read(dir.join("keys/ee.key")).unwrap())
            .unwrap();
        let server_config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap();
        let server_name = ServerName::try_from(host.to_string()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let (client_io, server_io) = tokio::io::duplex(1 << 16);
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client_config));
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server_config));
            let server = async move {
                let _ = acceptor.accept(server_io).await;
            };
            let (client, ()) = tokio::join!(connector.connect(server_name, client_io), server);
            client.map(|_| ()).map_err(|e| e.to_string())
        })
    };
    handshake("localhost").unwrap();
    let e = handshake("example.com").unwrap_err();
    assert!(
        e.contains("certificate not valid for name \"example.com\""),
        "{e}"
    );
}
