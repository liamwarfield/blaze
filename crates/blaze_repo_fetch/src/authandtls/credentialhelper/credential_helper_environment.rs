//! Port of `CredentialHelperEnvironment`.

use std::sync::Arc;
use std::time::Duration;

use blaze_events::EventHandler;
use blaze_vfs::Path;
use indexmap::IndexMap;

/// Environment for running [`super::CredentialHelper`]s in.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperEnvironment.java:39 CredentialHelperEnvironment
#[derive(Clone)]
pub struct CredentialHelperEnvironment {
    /// The reporter for reporting events related to credential helpers.
    pub event_reporter: Arc<dyn EventHandler>,
    /// The absolute path to the workspace, or `None` if Bazel was invoked outside a workspace.
    ///
    /// If available, it will be used as the working directory when invoking the helper
    /// subprocess. Otherwise, the working directory is inherited from the Bazel server process.
    pub workspace_path: Option<Path>,
    /// The environment from the Bazel client. Passed as environment variables to the subprocess.
    pub client_environment: IndexMap<String, String>,
    /// The execution timeout for the helper subprocess.
    pub helper_execution_timeout: Duration,
}

impl CredentialHelperEnvironment {
    /// Returns a new builder for [`CredentialHelperEnvironment`].
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperEnvironment.java:51 CredentialHelperEnvironment.newBuilder
    pub fn new_builder() -> CredentialHelperEnvironmentBuilder {
        CredentialHelperEnvironmentBuilder::default()
    }

    pub fn event_reporter(&self) -> &Arc<dyn EventHandler> {
        &self.event_reporter
    }

    pub fn workspace_path(&self) -> Option<&Path> {
        self.workspace_path.as_ref()
    }

    pub fn client_environment(&self) -> &IndexMap<String, String> {
        &self.client_environment
    }

    pub fn helper_execution_timeout(&self) -> Duration {
        self.helper_execution_timeout
    }
}

/// Builder for [`CredentialHelperEnvironment`].
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialHelperEnvironment.java:56 CredentialHelperEnvironment.Builder
#[derive(Default)]
pub struct CredentialHelperEnvironmentBuilder {
    event_reporter: Option<Arc<dyn EventHandler>>,
    workspace_path: Option<Path>,
    client_environment: Option<IndexMap<String, String>>,
    helper_execution_timeout: Option<Duration>,
}

impl CredentialHelperEnvironmentBuilder {
    /// Sets the reporter for reporting events related to credential helpers.
    pub fn set_event_reporter(mut self, reporter: Arc<dyn EventHandler>) -> Self {
        self.event_reporter = Some(reporter);
        self
    }

    /// Sets the absolute path to the workspace, or `None` if Bazel was invoked outside a
    /// workspace.
    pub fn set_workspace_path(mut self, path: Option<Path>) -> Self {
        self.workspace_path = path;
        self
    }

    /// Sets the environment from the Bazel client to pass as environment variables to the
    /// subprocess.
    pub fn set_client_environment(mut self, environment: IndexMap<String, String>) -> Self {
        self.client_environment = Some(environment);
        self
    }

    /// Sets the execution timeout for the helper subprocess.
    pub fn set_helper_execution_timeout(mut self, timeout: Duration) -> Self {
        self.helper_execution_timeout = Some(timeout);
        self
    }

    /// Returns the newly constructed [`CredentialHelperEnvironment`]. Panics (AutoBuilder's
    /// `IllegalStateException`) when a required property is missing.
    pub fn build(self) -> CredentialHelperEnvironment {
        let mut missing = Vec::new();
        if self.event_reporter.is_none() {
            missing.push(" eventReporter");
        }
        if self.client_environment.is_none() {
            missing.push(" clientEnvironment");
        }
        if self.helper_execution_timeout.is_none() {
            missing.push(" helperExecutionTimeout");
        }
        if !missing.is_empty() {
            panic!(
                "IllegalStateException: Missing required properties:{}",
                missing.concat()
            );
        }
        CredentialHelperEnvironment {
            event_reporter: self.event_reporter.unwrap(),
            workspace_path: self.workspace_path,
            client_environment: self.client_environment.unwrap(),
            helper_execution_timeout: self.helper_execution_timeout.unwrap(),
        }
    }
}
