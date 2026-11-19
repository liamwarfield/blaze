// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under
//! a temporary directory, so Java's absolute paths (`/home/foo`) are prefixed with that
//! directory.
//!
//! The tests after the Java ones have no Java counterpart: Java has no unit test for
//! `GoogleAuthUtils.newChannel` (`new_channel_*`, `is_tls_enabled_*` pin the decisions the port
//! has to take itself: which scheme encrypts, the texts of the TLS and proxy errors), nor for
//! `newCredentials` (`new_credentials_*`).

use std::sync::Arc;

use std::time::Duration;

use blaze_events::{Event, EventHandler, NullEventHandler};
use blaze_optdefs::misc::auth_and_tls_options::{
    AuthAndTLSOptions, CredentialHelperOption, CredentialHelperOptionConverter,
};
use blaze_remote::authandtls::Credentials;
use blaze_remote::authandtls::google_auth_utils::{
    CredentialHelperEnvironment, is_tls_enabled, new_channel, new_credential_helper_provider,
    new_credentials, new_credentials_from_netrc,
};
use blaze_repo_fetch::authandtls::credentialhelper::{self, CredentialModule};
use blaze_repo_fetch::authandtls::{Headers, basic_http_authentication_encode};
use blaze_repo_fetch::fs_utils;
use blaze_repo_fetch::runtime::command_line_path_factory::CommandLinePathFactory;
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

/// `new Reporter(new EventBus())`: the test never looks at its events.
struct Reporter;

impl EventHandler for Reporter {
    fn handle(&self, event: &Event) {
        NullEventHandler::INSTANCE.handle(event);
    }
}

// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java:323 GoogleAuthUtilsTest.createExecutable
fn create_executable(path: &Path) -> Path {
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    // Nothing to do.
    drop(path.get_output_stream().unwrap());
    path.set_executable(true).unwrap();
    path.clone()
}

// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java:364 GoogleAuthUtilsTest.createCredentialHelperOption
fn create_credential_helper_option(input: &str) -> CredentialHelperOption {
    CredentialHelperOptionConverter
        .convert_option(input)
        .unwrap_or_else(|e| panic!("IllegalStateException: {e:?}"))
}

// Java: com/google/devtools/build/lib/authandtls/GoogleAuthUtilsTest.java:349 GoogleAuthUtilsTest.newCredentialHelperProvider
fn new_credential_helper_provider_of(
    credential_helper_environment: &credentialhelper::CredentialHelperEnvironment,
    command_line_path_factory: &CommandLinePathFactory,
    inputs: &[String],
) -> credentialhelper::CredentialHelperProvider {
    new_credential_helper_provider(
        credential_helper_environment,
        command_line_path_factory,
        &inputs
            .iter()
            .map(|s| create_credential_helper_option(s))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn test_credential_helper_provider() {
    let f = Fixture::new();
    let file_system = &f.file_system;
    let at = |p: &str| Path::create_from_str(&format!("{}{p}", f.root), file_system);

    let workspace = at("/workspace");
    let path_value = at("/usr/local/bin");
    path_value.create_directory_and_parents().unwrap();

    let credential_helper_environment =
        credentialhelper::CredentialHelperEnvironment::new_builder()
            .set_event_reporter(Arc::new(Reporter))
            .set_workspace_path(Some(workspace.clone()))
            .set_client_environment(client_env(&[("PATH", path_value.get_path_string())]))
            .set_helper_execution_timeout(Duration::ZERO)
            .build();
    let command_line_path_factory = CommandLinePathFactory::new(
        file_system.clone(),
        [("workspace".to_string(), workspace.clone())]
            .into_iter()
            .collect(),
    );

    let unused_helper = create_executable(&at("/unused/helper"));

    let default_helper = create_executable(&at("/default/helper"));
    let example_com_helper = create_executable(&at("/example/com/helper"));
    let foo_example_com_helper = create_executable(&at("/foo/example/com/helper"));
    let example_com_wildcard_helper = create_executable(&at("/example/com/wildcard/helper"));

    let example_org_helper = create_executable(
        &workspace.get_relative(&blaze_vfs::PathFragment::create("helpers/example-org")),
    );

    let find = |provider: &credentialhelper::CredentialHelperProvider, uri: &str| {
        provider
            .find_credential_helper(&JavaUri::create(uri).unwrap())
            .map(|helper| helper.get_path().clone())
    };
    let new_provider = |inputs: &[String]| {
        new_credential_helper_provider_of(
            &credential_helper_environment,
            &command_line_path_factory,
            inputs,
        )
    };
    let p = |path: &Path| path.get_path_string().to_string();
    let scoped = |scope: &str, path: &Path| format!("{scope}={}", path.get_path_string());

    // No helpers.
    let credential_helper_provider1 = new_provider(&[]);
    assert_eq!(
        find(&credential_helper_provider1, "https://example.com"),
        None
    );
    assert_eq!(
        find(&credential_helper_provider1, "https://foo.example.com"),
        None
    );

    // Default helper only.
    let credential_helper_provider2 = new_provider(&[p(&default_helper)]);
    assert_eq!(
        find(&credential_helper_provider2, "https://example.com"),
        Some(default_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider2, "https://foo.example.com"),
        Some(default_helper.clone())
    );

    // Default and exact match.
    let credential_helper_provider3 = new_provider(&[
        p(&default_helper),
        scoped("example.com", &example_com_helper),
    ]);
    assert_eq!(
        find(&credential_helper_provider3, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider3, "https://foo.example.com"),
        Some(default_helper.clone())
    );

    // Exact match without default.
    let credential_helper_provider4 = new_provider(&[scoped("example.com", &example_com_helper)]);
    assert_eq!(
        find(&credential_helper_provider4, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider4, "https://foo.example.com"),
        None
    );

    // Multiple scoped helpers with default.
    let credential_helper_provider5 = new_provider(&[
        p(&default_helper),
        scoped("example.com", &example_com_helper),
        scoped("*.foo.example.com", &foo_example_com_helper),
        scoped("*.example.com", &example_com_wildcard_helper),
        "example.org=%workspace%/helpers/example-org".to_string(),
    ]);
    assert_eq!(
        find(&credential_helper_provider5, "https://anotherdomain.com"),
        Some(default_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://foo.example.com"),
        Some(foo_example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://abc.foo.example.com"),
        Some(foo_example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://bar.example.com"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://abc.bar.example.com"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider5, "https://example.org"),
        Some(example_org_helper.clone())
    );

    // Helpers override.
    let credential_helper_provider6 = new_provider(&[
        // <system .bazelrc>
        p(&unused_helper),
        // <user .bazelrc>
        p(&default_helper),
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
    ]);
    assert_eq!(
        find(&credential_helper_provider6, "https://anotherdomain.com"),
        Some(default_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider6, "https://example.com"),
        Some(example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider6, "https://foo.example.com"),
        Some(foo_example_com_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider6, "https://bar.example.com"),
        Some(example_com_wildcard_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider6, "https://example.org"),
        Some(example_org_helper.clone())
    );
    assert_eq!(
        find(&credential_helper_provider6, "https://foo.example.org"),
        Some(example_org_helper.clone())
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

/// `RemoteModule`'s call: a helper scoped to the host wins over `.netrc`, which stays the
/// fallback for other hosts; `refresh()` drops the cached helper response.
#[test]
fn new_credentials_wrap_netrc_in_the_credential_helper() {
    let f = Fixture::new();
    let home = format!("{}/home/foo", f.root);
    f.scratch_file(
        &format!("{home}/.netrc"),
        "machine foo.example.org login foouser password foopass",
    );
    let counter = format!("{}/count", f.root);
    let helper = format!("{}/bin/helper", f.root);
    f.scratch_file(
        &helper,
        &format!(
            "#!/bin/sh\necho x >> '{counter}'\ncat > /dev/null\n\
             echo '{{\"headers\":{{\"Authorization\":[\"Bearer t\"]}}}}'\n"
        ),
    );
    Path::create_from_str(&helper, &f.file_system)
        .set_executable(true)
        .unwrap();

    // The runtime's CredentialModule.beforeCommand (default --credential_helper_cache_duration).
    let credential_module = CredentialModule::new();
    credential_module.before_command("build", Duration::from_secs(30 * 60));
    let mut options = AuthAndTLSOptions::default();
    options.credential_helpers = vec![create_credential_helper_option(&format!(
        "*.example.com={helper}"
    ))];
    let workspace = Path::create_from_str(&f.root, &f.file_system);
    let credentials = new_credentials(
        &CredentialHelperEnvironment::new_builder()
            .set_event_reporter(Arc::new(Reporter))
            .set_workspace_path(Some(workspace.clone()))
            .set_client_environment(client_env(&[("HOME", &home)]))
            .set_helper_execution_timeout(Duration::from_secs(10))
            .build(),
        credential_module.get_credential_cache(),
        &CommandLinePathFactory::create_for_directories(
            f.file_system.clone(),
            Some(workspace),
            None,
        ),
        &f.file_system,
        &options,
    )
    .unwrap();

    let bearer: Headers = [("Authorization".to_string(), vec!["Bearer t".to_string()])]
        .into_iter()
        .collect();
    assert_eq!(
        request_metadata(
            &credentials,
            "https://remote.example.com/build.bazel.remote.execution.v2.ContentAddressableStorage"
        ),
        bearer
    );
    assert_eq!(
        request_metadata(
            &credentials,
            "https://remote.example.com/build.bazel.remote.execution.v2.ContentAddressableStorage"
        ),
        bearer
    );
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "x\n");
    assert_request_metadata(
        &request_metadata(&credentials, "https://foo.example.org"),
        "foouser",
        "foopass",
    );

    credentials.refresh().unwrap();
    assert_eq!(
        request_metadata(
            &credentials,
            "https://remote.example.com/build.bazel.remote.execution.v2.ContentAddressableStorage"
        ),
        bearer
    );
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "x\nx\n");
}
