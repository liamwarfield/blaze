//! Wraps an external tool used to obtain credentials.

use std::io::{Read, Write};

use blaze_shell::{JavaFile, Subprocess, SubprocessBuilder};
use blaze_util::string_encoding::java_new_string_utf8;
use blaze_vfs::Path;

use super::get_credentials_response::ResponseParseError;
use super::{
    CredentialHelperEnvironment, GetCredentialsRequest, GetCredentialsResponse,
    credential_helper_exception,
};
use crate::errors::{IoException, IoExceptionKind};
use crate::uri::JavaUri;

/// Wraps an external tool used to obtain credentials.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:45 CredentialHelper
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialHelper {
    path: Path,
}

impl CredentialHelper {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:52 CredentialHelper.<init>
    pub fn new(path: Path) -> CredentialHelper {
        CredentialHelper { path }
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:57 CredentialHelper.getPath
    pub fn get_path(&self) -> &Path {
        &self.path
    }

    /// Fetches credentials for the specified URI by invoking the credential helper as subprocess
    /// according to the credential helper protocol
    /// (<https://github.com/bazelbuild/proposals/blob/main/designs/2022-06-07-bazel-credential-helpers.md>).
    ///
    /// Returns the response from the subprocess.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:71 CredentialHelper.getCredentials
    pub fn get_credentials(
        &self,
        environment: &CredentialHelperEnvironment,
        uri: &JavaUri,
    ) -> Result<GetCredentialsResponse, IoException> {
        // PORT: skipped, profiling (prof.profile(CREDENTIAL_HELPER, "calling credential helper")).
        let path = self.path.get_path_string();

        let process = match self.spawn_subprocess(environment, &["get"]) {
            Ok(process) => process,
            Err(e) => {
                return Err(credential_helper_exception(format!(
                    "Failed to get credentials for '{uri}' from helper '{path}': {e}"
                )));
            }
        };

        let stdout = process.get_input_stream();
        let stderr = process.get_error_stream();
        if let Some(mut stdin) = process.get_output_stream() {
            // This can fail if the helper prints a static set of credentials without reading
            // from stdin (e.g., with a simple shell script running `echo "{...}"`). This is fine
            // to ignore.
            let request = GetCredentialsRequest::new(uri.clone()).to_json();
            let _ = stdin
                .write_all(request.as_bytes())
                .and_then(|()| stdin.flush());
            // The writer is closed here (the end of Java's try-with-resources).
            drop(stdin);
        }

        if process.wait_for().is_err() {
            return Err(credential_helper_exception(format!(
                "Failed to get credentials for '{uri}' from helper '{path}': process was interrupted"
            )));
        }

        if process.timedout() {
            return Err(credential_helper_exception(format!(
                "Failed to get credentials for '{uri}' from helper '{path}': process timed out"
            )));
        }
        if process.exit_value() != 0 {
            return Err(credential_helper_exception(format!(
                "Failed to get credentials for '{}' from helper '{}': process exited with code {}. stderr: {}",
                uri,
                path,
                process.exit_value(),
                read_all_utf8(stderr)
            )));
        }

        match GetCredentialsResponse::parse(&read_all_utf8(stdout)) {
            Ok(Some(response)) => Ok(response),
            Ok(None) => Err(credential_helper_exception(format!(
                "Failed to get credentials for '{}' from helper '{}': process exited without output. stderr: {}",
                uri,
                path,
                read_all_utf8(stderr)
            ))),
            // Gson does not catch the IllegalArgumentException of `build()`: it leaves
            // getCredentials as it is.
            Err(ResponseParseError::IllegalArgument(message)) => Err(IoException::of(
                IoExceptionKind::IllegalArgumentException,
                message,
            )),
            Err(ResponseParseError::JsonSyntax(_)) => Err(credential_helper_exception(format!(
                "Failed to get credentials for '{}' from helper '{}': error parsing output. stderr: {}",
                uri,
                path,
                read_all_utf8(stderr)
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelper.java:160 CredentialHelper.spawnSubprocess
    fn spawn_subprocess(
        &self,
        environment: &CredentialHelperEnvironment,
        args: &[&str],
    ) -> std::io::Result<Box<dyn Subprocess>> {
        let mut argv = vec![self.path.get_path_string().to_string()];
        argv.extend(args.iter().map(|a| a.to_string()));
        SubprocessBuilder::new(&environment.client_environment)
            .set_argv(argv)
            .set_working_directory(
                environment
                    .workspace_path
                    .as_ref()
                    .map(|p| JavaFile::new(p.get_path_string())),
            )
            .set_env(Some(environment.client_environment.clone()))
            .set_timeout_millis(environment.helper_execution_timeout.as_millis() as i64)
            .start()
    }
}

/// `CharStreams.toString(new InputStreamReader(stream, UTF_8))`; an unreadable stream reads as
/// what was read before the error.
fn read_all_utf8(stream: Option<Box<dyn Read + Send>>) -> String {
    let mut bytes = Vec::new();
    if let Some(mut stream) = stream {
        // PORT: Java's CharStreams.toString throws on a read error (an IOException that
        // getCredentials propagates); a pipe from a finished process does not fail to read.
        let _ = stream.read_to_end(&mut bytes);
    }
    java_new_string_utf8(&bytes)
}
