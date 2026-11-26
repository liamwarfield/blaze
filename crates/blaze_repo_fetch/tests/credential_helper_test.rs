//! Port of `CredentialHelperTest`. The helper is Bazel's `test_credential_helper.py`
//! (`tests/data/credentialhelper/`), run through a `/bin/sh` wrapper with `python3`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperTest.java

use std::sync::Arc;
use std::time::Duration;

use blaze_events::StoredEventHandler;
use blaze_repo_fetch::authandtls::credentialhelper::{
    CredentialHelper, CredentialHelperEnvironment, GetCredentialsResponse,
    is_credential_helper_exception,
};
use blaze_repo_fetch::errors::IoException;
use blaze_repo_fetch::uri::JavaUri;
use blaze_vfs::unix_file_system::UnixFileSystem;
use blaze_vfs::{DigestHashFunction, Path};
use indexmap::IndexMap;

struct Fixture {
    _dir: tempfile::TempDir,
    /// `TEST_TMPDIR`: the workspace the helper runs in.
    workspace: std::path::PathBuf,
    helper: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/credentialhelper/test_credential_helper.py");
    let helper = dir.path().join("test_credential_helper");
    std::fs::write(
        &helper,
        format!("#!/bin/sh\nexec python3 '{}' \"$@\"\n", script.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fixture {
        _dir: dir,
        workspace,
        helper,
    }
}

fn get_credentials_from_helper_at(
    f: &Fixture,
    cred_helper_path: &str,
    uri: &str,
    env: &[(&str, &str)],
) -> Result<GetCredentialsResponse, IoException> {
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let credential_helper = CredentialHelper::new(Path::create_from_str(cred_helper_path, &fs));
    let mut client_env: IndexMap<String, String> = std::env::vars().collect();
    client_env.shift_remove("RUN_UNDER_RUNFILES");
    for (k, v) in env {
        client_env.insert(k.to_string(), v.to_string());
    }
    credential_helper.get_credentials(
        &CredentialHelperEnvironment {
            event_reporter: Arc::new(StoredEventHandler::new()),
            workspace_path: Some(Path::create_from_str(f.workspace.to_str().unwrap(), &fs)),
            client_environment: client_env,
            helper_execution_timeout: Duration::from_secs(5),
        },
        &JavaUri::create(uri).unwrap(),
    )
}

fn get_credentials_from_helper(
    f: &Fixture,
    uri: &str,
    env: &[(&str, &str)],
) -> Result<GetCredentialsResponse, IoException> {
    get_credentials_from_helper_at(f, f.helper.to_str().unwrap(), uri, env)
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn assert_credential_helper_exception(e: &IoException, parts: &[&str]) {
    assert!(is_credential_helper_exception(e), "{e:?}");
    for part in parts {
        assert!(
            e.get_message().contains(part),
            "{:?} in {}",
            part,
            e.get_message()
        );
    }
}

#[test]
fn known_uri_with_single_header() {
    let f = fixture();
    let response =
        get_credentials_from_helper(&f, "https://singleheader.example.com", &[]).unwrap();
    assert_eq!(response.headers().len(), 1);
    assert_eq!(response.headers()["header1"], strings(&["value1"]));
}

#[test]
fn known_uri_with_multiple_headers() {
    let f = fixture();
    let response =
        get_credentials_from_helper(&f, "https://multipleheaders.example.com", &[]).unwrap();
    assert_eq!(response.headers().len(), 3);
    assert_eq!(response.headers()["header1"], strings(&["value1"]));
    assert_eq!(
        response.headers()["header2"],
        strings(&["value1", "value2"])
    );
    assert_eq!(
        response.headers()["header3"],
        strings(&["value1", "value2", "value3"])
    );
}

#[test]
fn unknown_uri() {
    let f = fixture();
    let e = get_credentials_from_helper(&f, "https://unknown.example.com", &[]).unwrap_err();
    assert_credential_helper_exception(
        &e,
        &[
            "Failed to get credentials",
            "Unknown uri 'https://unknown.example.com'",
        ],
    );
}

#[test]
fn credential_helper_outputs_nothing() {
    let f = fixture();
    let e = get_credentials_from_helper(&f, "https://printnothing.example.com", &[]).unwrap_err();
    assert_credential_helper_exception(&e, &["Failed to get credentials", "exited without output"]);
}

#[test]
fn credential_helper_outputs_extra_fields() {
    let f = fixture();
    let response = get_credentials_from_helper(&f, "https://extrafields.example.com", &[]).unwrap();
    assert_eq!(response.headers().len(), 1);
    assert_eq!(response.headers()["header1"], strings(&["value1"]));
}

#[test]
fn helper_runs_in_workspace() {
    let f = fixture();
    let response = get_credentials_from_helper(&f, "https://cwd.example.com", &[]).unwrap();
    assert_eq!(
        response.headers()["cwd"][0],
        f.workspace.to_str().unwrap().to_string()
    );
}

#[test]
fn helper_get_environment() {
    let f = fixture();
    let response = get_credentials_from_helper(
        &f,
        "https://env.example.com",
        &[("FOO", "BAR!"), ("BAR", "123")],
    )
    .unwrap();
    assert_eq!(response.headers().len(), 2);
    assert_eq!(response.headers()["foo"], strings(&["BAR!"]));
    assert_eq!(response.headers()["bar"], strings(&["123"]));
}

#[test]
fn helper_timeout() {
    let f = fixture();
    let e = get_credentials_from_helper(&f, "https://timeout.example.com", &[]).unwrap_err();
    assert_credential_helper_exception(&e, &["Failed to get credentials", "process timed out"]);
}

#[test]
fn non_existent_helper() {
    let f = fixture();
    let e = get_credentials_from_helper_at(&f, "/no/such/file", "https://timeout.example.com", &[])
        .unwrap_err();
    assert_credential_helper_exception(&e, &["Failed to get credentials", "Cannot run program"]);
}

#[test]
fn huge_payload() {
    // Bazel reads the credential helper stdout/stderr from a pipe, and doesn't start reading
    // until the process terminates. Therefore, a response larger than the pipe buffer causes a
    // deadlock and timeout. This verifies that the pipe is sufficiently large.
    // See https://github.com/bazelbuild/bazel/issues/21287.
    let f = fixture();
    let response = get_credentials_from_helper(&f, "https://hugepayload.example.com", &[]).unwrap();
    assert_eq!(response.headers().len(), 1);
    assert_eq!(response.headers()["huge"], vec!["x".repeat(63 * 1024)]);
}
