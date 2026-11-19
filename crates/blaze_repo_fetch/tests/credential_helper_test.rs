//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperTest.java`.
//!
//! The helper is Java's `test_credential_helper.py` (copied to
//! `tests/data/credentialhelper/`), started through a small `sh` wrapper because Java runs it as
//! a `py_binary`. `TEST_TMPDIR` (the workspace) is a temporary directory.

use std::sync::Arc;
use std::time::Duration;

use blaze_events::{EventHandler, NullEventHandler};
use blaze_repo_fetch::authandtls::credentialhelper::{
    CredentialHelper, CredentialHelperCredentials, CredentialHelperEnvironment,
    CredentialHelperProvider, CredentialModule, GetCredentialsResponse, catch_unchecked,
};
use blaze_repo_fetch::authandtls::google_auth_utils::new_credential_helper_provider;
use blaze_repo_fetch::authandtls::{Credentials, Headers, StaticCredentials};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::runtime::command_line_path_factory::CommandLinePathFactory;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use indexmap::IndexMap;

/// `new Reporter(new EventBus())`: the tests never look at its events.
struct Reporter;

impl EventHandler for Reporter {
    fn handle(&self, event: &blaze_events::Event) {
        NullEventHandler::INSTANCE.handle(event);
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    fs: FileSystemRef,
    /// `TEST_WORKSPACE_PATH`.
    workspace: String,
    /// The `test_credential_helper` binary.
    helper: String,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let script = std::fs::canonicalize(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/credentialhelper/test_credential_helper.py"
        ))
        .unwrap();
        let helper = root.join("test_credential_helper");
        std::fs::write(
            &helper,
            format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", script.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        Fixture {
            _dir: dir,
            fs: UnixFileSystem::create(DigestHashFunction::Sha256),
            workspace: workspace.to_str().unwrap().to_string(),
            helper: helper.to_str().unwrap().to_string(),
        }
    }

    fn environment(&self, env: &[(&str, &str)]) -> CredentialHelperEnvironment {
        let mut client_env: IndexMap<String, String> = std::env::vars().collect();
        // Don't cd to the Python credential helper's temporary directory on Windows, which would
        // throw off test assertions. This variable is set to "1" by the surrounding Java test.
        client_env.shift_remove("RUN_UNDER_RUNFILES");
        for (k, v) in env {
            client_env.insert(k.to_string(), v.to_string());
        }
        CredentialHelperEnvironment::new_builder()
            .set_event_reporter(Arc::new(Reporter))
            .set_workspace_path(Some(Path::create_from_str(&self.workspace, &self.fs)))
            .set_client_environment(client_env)
            .set_helper_execution_timeout(Duration::from_secs(5))
            .build()
    }

    fn get_credentials_from_helper_at(
        &self,
        cred_helper_path: &str,
        uri: &str,
        env: &[(&str, &str)],
    ) -> Result<GetCredentialsResponse, IoException> {
        let credential_helper =
            CredentialHelper::new(Path::create_from_str(cred_helper_path, &self.fs));
        credential_helper.get_credentials(&self.environment(env), &JavaUri::create(uri).unwrap())
    }

    fn get_credentials_from_helper_with_env(
        &self,
        uri: &str,
        env: &[(&str, &str)],
    ) -> Result<GetCredentialsResponse, IoException> {
        self.get_credentials_from_helper_at(&self.helper, uri, env)
    }

    fn get_credentials_from_helper(
        &self,
        uri: &str,
    ) -> Result<GetCredentialsResponse, IoException> {
        self.get_credentials_from_helper_with_env(uri, &[])
    }
}

fn headers(entries: &[(&str, &[&str])]) -> IndexMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn known_uri_with_single_header() {
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper("https://singleheader.example.com")
        .unwrap();
    assert_eq!(response.headers(), &headers(&[("header1", &["value1"])]));
}

#[test]
fn known_uri_with_multiple_headers() {
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper("https://multipleheaders.example.com")
        .unwrap();
    assert_eq!(
        response.headers(),
        &headers(&[
            ("header1", &["value1"]),
            ("header2", &["value1", "value2"]),
            ("header3", &["value1", "value2", "value3"]),
        ])
    );
}

#[test]
fn unknown_uri() {
    let f = Fixture::new();
    let e = f
        .get_credentials_from_helper("https://unknown.example.com")
        .unwrap_err();
    assert!(e.get_message().contains("Failed to get credentials"), "{e}");
    assert!(
        e.get_message()
            .contains("Unknown uri 'https://unknown.example.com'"),
        "{e}"
    );
}

#[test]
fn credential_helper_outputs_nothing() {
    let f = Fixture::new();
    let e = f
        .get_credentials_from_helper("https://printnothing.example.com")
        .unwrap_err();
    assert!(e.get_message().contains("Failed to get credentials"), "{e}");
    assert!(e.get_message().contains("exited without output"), "{e}");
}

#[test]
fn credential_helper_outputs_extra_fields() {
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper("https://extrafields.example.com")
        .unwrap();
    assert_eq!(response.headers(), &headers(&[("header1", &["value1"])]));
}

#[test]
fn helper_runs_in_workspace() {
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper("https://cwd.example.com")
        .unwrap();
    let headers = response.headers();
    assert_eq!(headers.get("cwd").unwrap()[0], f.workspace);
}

#[test]
fn helper_get_environment() {
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper_with_env(
            "https://env.example.com",
            &[("FOO", "BAR!"), ("BAR", "123")],
        )
        .unwrap();
    assert_eq!(
        response.headers(),
        &headers(&[("foo", &["BAR!"]), ("bar", &["123"])])
    );
}

#[test]
fn helper_timeout() {
    let f = Fixture::new();
    let e = f
        .get_credentials_from_helper("https://timeout.example.com")
        .unwrap_err();
    assert!(e.get_message().contains("Failed to get credentials"), "{e}");
    assert!(e.get_message().contains("process timed out"), "{e}");
}

#[test]
fn non_existent_helper() {
    let f = Fixture::new();
    let e = f
        .get_credentials_from_helper_at("/no/such/file", "https://timeout.example.com", &[])
        .unwrap_err();
    assert!(e.get_message().contains("Failed to get credentials"), "{e}");
    assert!(e.get_message().contains("Cannot run program"), "{e}");
}

#[test]
fn huge_payload() {
    // Bazel reads the credential helper stdout/stderr from a pipe, and doesn't start reading
    // until the process terminates. Therefore, a response larger than the pipe buffer causes
    // a deadlock and timeout. This verifies that the pipe is sufficiently large.
    // See https://github.com/bazelbuild/bazel/issues/21287.
    let f = Fixture::new();
    let response = f
        .get_credentials_from_helper("https://hugepayload.example.com")
        .unwrap();
    let huge = "x".repeat(63 * 1024);
    assert_eq!(response.headers(), &headers(&[("huge", &[huge.as_str()])]));
}

// The tests below have no Java counterpart: the exact error texts of `getCredentials`, and
// `CredentialHelperCredentials` (cache, fallback, refresh) over the test helper.

#[test]
fn error_messages_are_java_s() {
    let f = Fixture::new();
    let e = f
        .get_credentials_from_helper("https://unknown.example.com")
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "Failed to get credentials for 'https://unknown.example.com' from helper '{}': \
             process exited with code 1. stderr: Unknown uri 'https://unknown.example.com'\n",
            f.helper
        )
    );
    let e = f
        .get_credentials_from_helper("https://printnothing.example.com")
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "Failed to get credentials for 'https://printnothing.example.com' from helper '{}': \
             process exited without output. stderr: ",
            f.helper
        )
    );
    let e = f
        .get_credentials_from_helper("https://timeout.example.com")
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "Failed to get credentials for 'https://timeout.example.com' from helper '{}': \
             process timed out",
            f.helper
        )
    );
}

#[test]
fn error_parsing_output() {
    let f = Fixture::new();
    let helper = format!("{}/bad_helper", f.workspace);
    std::fs::write(&helper, "#!/bin/sh\necho 'not json'\necho oops >&2\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let e = f
        .get_credentials_from_helper_at(&helper, "https://example.com", &[])
        .unwrap_err();
    assert_eq!(
        e.get_message(),
        format!(
            "Failed to get credentials for 'https://example.com' from helper '{helper}': \
             error parsing output. stderr: oops\n"
        )
    );
}

fn credentials(
    f: &Fixture,
    module: &CredentialModule,
    provider: CredentialHelperProvider,
    fallback: Option<Arc<dyn Credentials>>,
) -> CredentialHelperCredentials {
    CredentialHelperCredentials::new(
        Arc::new(provider),
        f.environment(&[]),
        module.get_credential_cache().clone(),
        fallback,
    )
}

#[test]
fn credentials_use_the_helper_then_the_cache() {
    let f = Fixture::new();
    let module = CredentialModule::new();
    module.before_command("build", Duration::from_secs(1800));
    let counter = format!("{}/count", f.workspace);
    let helper = format!("{}/counting_helper", f.workspace);
    std::fs::write(
        &helper,
        format!(
            "#!/bin/sh\necho x >> '{counter}'\ncat > /dev/null\n\
             echo '{{\"headers\":{{\"Authorization\":[\"Bearer t\"]}}}}'\n"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let provider = CredentialHelperProvider::builder()
        .add_for_pattern("*.example.com", Path::create_from_str(&helper, &f.fs))
        .build();
    let creds = credentials(&f, &module, provider, None);
    let uri = JavaUri::create("https://foo.example.com/a").unwrap();

    let expected = headers(&[("Authorization", &["Bearer t"])]);
    assert_eq!(creds.get_request_metadata(&uri).unwrap(), expected);
    assert_eq!(creds.get_request_metadata(&uri).unwrap(), expected);
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "x\n");

    // refresh() drops the cache: the helper runs again.
    creds.refresh(|_| Ok::<(), ()>(())).unwrap();
    assert_eq!(creds.get_request_metadata(&uri).unwrap(), expected);
    assert_eq!(std::fs::read_to_string(&counter).unwrap(), "x\nx\n");
}

#[test]
fn credentials_fall_back_without_a_helper() {
    let f = Fixture::new();
    let module = CredentialModule::new();
    module.before_command("build", Duration::from_secs(1800));
    let uri = JavaUri::create("https://other.org/a").unwrap();
    let provider = CredentialHelperProvider::builder()
        .add_for_pattern("example.com", Path::create_from_str(&f.helper, &f.fs))
        .build();

    let none = credentials(&f, &module, provider.clone(), None);
    assert_eq!(none.get_request_metadata(&uri).unwrap(), Headers::new());

    let mut static_headers = std::collections::HashMap::new();
    static_headers.insert(uri.clone(), headers(&[("X-Static", &["s"])]));
    let with_fallback = credentials(
        &f,
        &module,
        provider,
        Some(Arc::new(StaticCredentials::new(static_headers))),
    );
    assert_eq!(
        with_fallback.get_request_metadata(&uri).unwrap(),
        headers(&[("X-Static", &["s"])])
    );
}

#[test]
fn credentials_report_helper_errors() {
    let f = Fixture::new();
    let module = CredentialModule::new();
    module.before_command("build", Duration::from_secs(1800));
    let provider = CredentialHelperProvider::builder()
        .add(Path::create_from_str(&f.helper, &f.fs))
        .build();
    let creds = credentials(&f, &module, provider, None);
    let uri = JavaUri::create("https://unknown.example.com").unwrap();

    let e = creds.get_request_metadata(&uri).unwrap_err();
    assert!(
        e.get_message()
            .contains("Unknown uri 'https://unknown.example.com'"),
        "{e}"
    );
    assert!(
        module
            .get_credential_cache()
            .get_entry_if_present_quietly(&uri)
            .is_none()
    );
}

/// An unchecked exception raised by the helper's answer (Java `ArithmeticException` from
/// `Duration.toNanos()` for an `expires` centuries ahead) leaves the credentials as an
/// `UncheckedException`, which the gRPC path catches.
#[test]
fn far_future_expires_is_an_arithmetic_exception() {
    let f = Fixture::new();
    let module = CredentialModule::new();
    module.before_command("build", Duration::from_secs(1800));
    let helper = format!("{}/far_future_helper", f.workspace);
    std::fs::write(
        &helper,
        "#!/bin/sh\ncat > /dev/null\n\
         echo '{\"headers\":{\"a\":[\"b\"]},\"expires\":\"9999-12-31T23:59:59Z\"}'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let provider = CredentialHelperProvider::builder()
        .add(Path::create_from_str(&helper, &f.fs))
        .build();
    let creds = credentials(&f, &module, provider, None);
    let uri = JavaUri::create("https://example.com").unwrap();

    let e = catch_unchecked(|| creds.get_request_metadata(&uri)).unwrap_err();
    assert_eq!(e.get_class_name(), "java.lang.ArithmeticException");
    assert_eq!(e.get_message(), "long overflow");
}

/// Run with `--nocapture`: inside `catch_unchecked` the panic hook prints nothing, so the
/// header values of an `IllegalArgumentException` never reach stderr on the gRPC path.
#[test]
fn caught_unchecked_exception_prints_nothing() {
    let e = catch_unchecked(|| {
        GetCredentialsResponse::from_json(
            "{\"headers\": {\"a\": [\"PLACEHOLDER-VALUE-1\"], \"a\": [\"PLACEHOLDER-VALUE-2\"]}}",
        )
    })
    .unwrap_err();
    assert_eq!(e.get_class_name(), "java.lang.IllegalArgumentException");
    // Debug output names the class only.
    assert!(!format!("{e:?}").contains("PLACEHOLDER-VALUE"));
}

/// Outside `catch_unchecked` (the downloader path, where Bazel crashes) the exception is a panic,
/// and the hook prints it as an uncaught Java exception (`toString()`), message included.
#[test]
#[should_panic]
fn uncaught_unchecked_exception_is_a_panic() {
    let _ = GetCredentialsResponse::from_json(
        "{\"headers\": {\"a\": [\"PLACEHOLDER-UNCAUGHT-1\"], \"a\": [\"PLACEHOLDER-UNCAUGHT-2\"]}}",
    );
}

/// The real target: `--credential_helper=aw-remote-ext.buildremote.stairwell.io=aspect`, with
/// `aspect` found on `PATH`. Asserts only that an `Authorization` header with a non-empty value
/// comes back; never prints it.
#[test]
#[ignore = "needs the aspect CLI and a login"]
fn real_aspect_helper_returns_authorization() {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let client_env: IndexMap<String, String> = std::env::vars().collect();
    let options = [
        blaze_optdefs::misc::auth_and_tls_options::CredentialHelperOption {
            scope: Some("aw-remote-ext.buildremote.stairwell.io".to_string()),
            path: "aspect".to_string(),
        },
    ];
    let environment = CredentialHelperEnvironment::new_builder()
        .set_event_reporter(Arc::new(Reporter))
        .set_workspace_path(None)
        .set_client_environment(client_env)
        .set_helper_execution_timeout(Duration::from_secs(10))
        .build();
    let path_factory = CommandLinePathFactory::create_for_directories(fs.clone(), None, None);
    let provider = new_credential_helper_provider(&environment, &path_factory, &options).unwrap();
    let module = CredentialModule::new();
    module.before_command("build", Duration::from_secs(1800));
    let creds: CredentialHelperCredentials = CredentialHelperCredentials::new(
        Arc::new(provider),
        environment,
        module.get_credential_cache().clone(),
        None,
    );

    let uri = JavaUri::create("https://aw-remote-ext.buildremote.stairwell.io").unwrap();
    let metadata = match creds.get_request_metadata(&uri) {
        Ok(m) => m,
        Err(e) => panic!("credential helper failed: {}", e.get_message()),
    };
    let ok = metadata
        .get("Authorization")
        .is_some_and(|values| !values.is_empty() && values.iter().all(|v| !v.is_empty()));
    assert!(
        ok,
        "no non-empty Authorization header; header names: {:?}",
        metadata.keys().collect::<Vec<_>>()
    );
}
