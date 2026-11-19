//! Port of `CredentialHelper`.

use std::io::{Read, Write};

use blaze_shell::{JavaFile, Subprocess, SubprocessBuilder};
use blaze_vfs::Path;

use super::credential_helper_environment::CredentialHelperEnvironment;
use super::credential_helper_exception::CredentialHelperException;
use super::get_credentials_request::GetCredentialsRequest;
use super::get_credentials_response::GetCredentialsResponse;
use super::unchecked_exception::UncheckedException;
use crate::errors::IoException;
use crate::uri::JavaUri;

/// Wraps an external tool used to obtain credentials.
///
/// Equality and hashing are by path, as in Java.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:42 CredentialHelper
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialHelper {
    path: Path,
}

/// `InputStreamReader(stream, UTF_8)` read to the end (`CharStreams.toString`): malformed
/// UTF-8 becomes U+FFFD, as Java's decoder replaces it.
fn read_to_string(stream: &mut Option<Box<dyn Read + Send>>) -> Result<String, IoException> {
    let mut bytes = Vec::new();
    if let Some(stream) = stream {
        stream.read_to_end(&mut bytes).map_err(IoException::from)?;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl CredentialHelper {
    /// Package-private in Java (`CredentialHelperProvider` and the tests create helpers).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:49 CredentialHelper.<init>
    pub fn new(path: Path) -> CredentialHelper {
        CredentialHelper { path }
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:53 CredentialHelper.getPath
    pub fn get_path(&self) -> &Path {
        &self.path
    }

    /// Fetches credentials for the specified URI by invoking the credential helper as subprocess
    /// according to the
    /// [credential helper protocol](https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md).
    ///
    /// * `environment`: the environment to run the subprocess in.
    /// * `uri`: the URI to fetch credentials for.
    ///
    /// Returns the response from the subprocess.
    ///
    /// PORT: the profiler task (`CREDENTIAL_HELPER`, "calling credential helper") is skipped.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:67 CredentialHelper.getCredentials
    pub fn get_credentials(
        &self,
        environment: &CredentialHelperEnvironment,
        uri: &JavaUri,
    ) -> Result<GetCredentialsResponse, IoException> {
        let path = self.path.get_path_string();
        let process = match self.spawn_subprocess(environment, &["get"]) {
            Ok(process) => process,
            Err(e) => {
                return Err(CredentialHelperException::new(format!(
                    "Failed to get credentials for '{}' from helper '{}': {}",
                    uri,
                    path,
                    IoException::from(e).get_message()
                ))
                .into());
            }
        };

        let mut stdout = process.get_input_stream();
        let mut stderr = process.get_error_stream();
        if let Some(mut stdin) = process.get_output_stream() {
            // This can fail if the helper prints a static set of credentials without reading
            // from stdin (e.g., with a simple shell script running `echo "{...}"`). This is fine
            // to ignore.
            let request = GetCredentialsRequest::new_builder()
                .set_uri(uri.clone())
                .build()
                .expect("uri is set");
            let _ = stdin
                .write_all(request.to_json().as_bytes())
                .and_then(|()| stdin.flush());
            // Closing stdin (end of the try-with-resources block).
            drop(stdin);
        }

        if process.wait_for().is_err() {
            return Err(CredentialHelperException::new(format!(
                "Failed to get credentials for '{}' from helper '{}': process was interrupted",
                uri, path
            ))
            .into());
        }

        if process.timedout() {
            return Err(CredentialHelperException::new(format!(
                "Failed to get credentials for '{}' from helper '{}': process timed out",
                uri, path
            ))
            .into());
        }
        if process.exit_value() != 0 {
            return Err(CredentialHelperException::new(format!(
                "Failed to get credentials for '{}' from helper '{}': process exited with code \
                 {}. stderr: {}",
                uri,
                path,
                process.exit_value(),
                read_to_string(&mut stderr)?
            ))
            .into());
        }

        // GSON.fromJson(stdout, GetCredentialsResponse.class). Gson turns an IOException while
        // reading into a JsonSyntaxException.
        let parsed = read_to_string(&mut stdout)
            .map_err(|e| e.get_message())
            .and_then(|json| GetCredentialsResponse::from_json(&json).map_err(|e| e.message));
        match parsed {
            Ok(Some(response)) => Ok(response),
            Ok(None) => Err(CredentialHelperException::new(format!(
                "Failed to get credentials for '{}' from helper '{}': process exited without \
                 output. stderr: {}",
                uri,
                path,
                read_to_string(&mut stderr)?
            ))
            .into()),
            Err(cause) => Err(CredentialHelperException::with_cause(
                format!(
                    "Failed to get credentials for '{}' from helper '{}': error parsing output. \
                     stderr: {}",
                    uri,
                    path,
                    read_to_string(&mut stderr)?
                ),
                cause,
            )
            .into()),
        }
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:155 CredentialHelper.spawnSubprocess
    fn spawn_subprocess(
        &self,
        environment: &CredentialHelperEnvironment,
        args: &[&str],
    ) -> std::io::Result<Box<dyn Subprocess>> {
        let mut argv = vec![self.path.get_path_string().to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        let mut builder = SubprocessBuilder::new(environment.client_environment());
        builder
            .set_argv(argv)
            .set_working_directory(
                environment
                    .workspace_path()
                    .map(|workspace| JavaFile::new(workspace.get_path_string())),
            )
            .set_env(Some(environment.client_environment().clone()))
            // Duration.toMillis()
            .set_timeout_millis(
                i64::try_from(environment.helper_execution_timeout().as_millis()).unwrap_or_else(
                    |_| UncheckedException::throw("java.lang.ArithmeticException", "long overflow"),
                ),
            );
        builder.start()
    }
}
