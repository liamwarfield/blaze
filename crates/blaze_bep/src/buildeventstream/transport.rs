//! Port of `BuildEventTransport`.

use std::sync::Arc;

use blaze_optdefs::misc::build_event_service_options::BesUploadMode;

use super::artifact_uploader::BuildEventArtifactUploader;
use super::build_event::BuildEvent;

/// The failure of closing a transport (`AbruptExitException` with a `BuildProgress` failure
/// detail in Java).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct TransportCloseError {
    pub message: String,
    /// `FailureDetails.BuildProgress.Code` name.
    pub code: &'static str,
}

/// Interface for transports of the build-event stream.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:31 BuildEventTransport
pub trait BuildEventTransport: Send + Sync {
    /// The name of this transport as can be displayed to a user.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:33 BuildEventTransport.name
    fn name(&self) -> String;

    /// Writes a build event to an endpoint. This method will always return quickly and will not
    /// wait for the write to complete.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:44 BuildEventTransport.sendBuildEvent
    fn send_build_event(&self, event: &Arc<dyn BuildEvent>);

    /// Initiates a close. Callers may listen to the returned future to be notified when the
    /// close is complete. PORT: blocking; returns when the close is complete.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:55 BuildEventTransport.close
    fn close(&self) -> Result<(), TransportCloseError>;

    /// Returns how long a caller should wait for the transport to finish uploading events and
    /// closing gracefully. Setting the timeout to `0` means no wait.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:78 BuildEventTransport.getTimeout
    fn get_timeout(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:86 BuildEventTransport.mayBeSlow
    fn may_be_slow(&self) -> bool;

    /// Which `BesUploadMode` the transport is using.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:89 BuildEventTransport.getBesUploadMode
    fn get_bes_upload_mode(&self) -> BesUploadMode;

    /// Returns the artifact uploader used by this transport, if any.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:92 BuildEventTransport.getUploader
    fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>>;
}
