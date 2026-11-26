//! Environment for running credential helpers in.

use std::sync::Arc;
use std::time::Duration;

use blaze_events::EventHandler;
use blaze_vfs::Path;
use indexmap::IndexMap;

/// Environment for running [`super::CredentialHelper`]s in.
///
/// * `event_reporter`: the reporter for reporting events related to credential helpers.
/// * `workspace_path`: the absolute path to the workspace, or `None` if Bazel was invoked outside
///   a workspace. If available, it is the working directory of the helper subprocess; otherwise
///   the working directory is inherited from the Bazel server process.
/// * `client_environment`: the environment from the Bazel client, passed as environment variables
///   to the subprocess.
/// * `helper_execution_timeout`: the execution timeout for the helper subprocess.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperEnvironment.java:39 CredentialHelperEnvironment
#[derive(Clone)]
pub struct CredentialHelperEnvironment {
    pub event_reporter: Arc<dyn EventHandler>,
    pub workspace_path: Option<Path>,
    pub client_environment: IndexMap<String, String>,
    pub helper_execution_timeout: Duration,
}

impl std::fmt::Debug for CredentialHelperEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialHelperEnvironment")
            .field("workspace_path", &self.workspace_path)
            .field("client_environment", &self.client_environment)
            .field("helper_execution_timeout", &self.helper_execution_timeout)
            .finish_non_exhaustive()
    }
}
