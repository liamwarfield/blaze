#![cfg(feature = "wip")]
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

use blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions;
use blaze_remote::authandtls::Credentials;
use blaze_remote::authandtls::google_auth_utils::{
    is_tls_enabled, new_channel, new_credentials_from_netrc,
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

#[test]
#[ignore = "needs CommandLinePathFactory (blaze_runtime) and lib/authandtls/credentialhelper (CredentialHelperProvider.findCredentialHelper, CredentialHelper), which are not ported"]
fn test_credential_helper_provider() {
    // Every case of the Java test but the first resolves the path of a helper with
    // `CommandLinePathFactory.create` and looks it up with
    // `CredentialHelperProvider.findCredentialHelper`.
    panic!("not ported: GoogleAuthUtils.newCredentialHelperProvider with helpers");
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
