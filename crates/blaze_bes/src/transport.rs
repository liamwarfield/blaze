//! A [`BuildEventTransport`] that streams build events to the Build Event Service.

use std::sync::Arc;
use std::time::Duration;

use blaze_bep::buildeventstream::artifact_uploader::BuildEventArtifactUploader;
use blaze_bep::buildeventstream::build_event::BuildEvent;
use blaze_bep::buildeventstream::build_event_context::ArtifactGroupNamer;
use blaze_bep::buildeventstream::transport::{BuildEventTransport, CloseFuture};
use blaze_events::EventBus;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::{BesUploadMode, BuildEventServiceOptions};

use crate::client::{BuildEventServiceClient, CommandContext, Instant};
use crate::uploader::{self, BesClock, BuildEventServiceUploader, JavaSleeper, Sleeper};

/// A [`BuildEventTransport`] that streams build events to BuildEventService.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:37 BuildEventServiceTransport
pub struct BuildEventServiceTransport {
    bes_uploader: BuildEventServiceUploader,
    bes_timeout: Duration,
    bes_upload_mode: BesUploadMode,
}

impl BuildEventTransport for BuildEventServiceTransport {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:94 BuildEventServiceTransport.name
    fn name(&self) -> String {
        "Build Event Service".to_string()
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:109 BuildEventServiceTransport.sendBuildEvent
    fn send_build_event(&self, event: &Arc<dyn BuildEvent>) {
        self.bes_uploader.enqueue_event(event);
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:79 BuildEventServiceTransport.close
    fn close(&self) -> CloseFuture {
        self.bes_uploader.close()
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:84 BuildEventServiceTransport.getHalfCloseFuture
    fn get_half_close_future(&self) -> CloseFuture {
        self.bes_uploader.get_half_close_future()
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:114 BuildEventServiceTransport.getTimeout
    fn get_timeout(&self) -> Duration {
        self.bes_timeout
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:99 BuildEventServiceTransport.mayBeSlow
    fn may_be_slow(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:104 BuildEventServiceTransport.getBesUploadMode
    fn get_bes_upload_mode(&self) -> BesUploadMode {
        self.bes_upload_mode
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:89 BuildEventServiceTransport.getUploader
    fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>> {
        Some(self.bes_uploader.get_build_event_uploader())
    }
}

/// A builder for [`BuildEventServiceTransport`].
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:120 BuildEventServiceTransport.Builder
pub struct Builder {
    pub bes_client: Arc<dyn BuildEventServiceClient>,
    pub local_file_uploader: Arc<dyn BuildEventArtifactUploader>,
    pub bes_options: BuildEventServiceOptions,
    pub bep_options: Arc<BuildEventProtocolOptions>,
    pub clock: Arc<dyn BesClock>,
    pub artifact_group_namer: Arc<dyn ArtifactGroupNamer>,
    pub event_bus: Arc<dyn EventBus>,
    /// `null` is a `JavaSleeper`.
    pub sleeper: Option<Arc<dyn Sleeper>>,
    pub command_context: CommandContext,
    pub command_start_time: Instant,
}

impl Builder {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:186 BuildEventServiceTransport.Builder.build
    pub fn build(self) -> BuildEventServiceTransport {
        // (besOptions.besTimeout != null) ? besOptions.besTimeout : Duration.ZERO
        let bes_timeout = self.bes_options.bes_timeout;
        let bes_uploader = uploader::Builder {
            bes_client: self.bes_client,
            local_file_uploader: self.local_file_uploader,
            bep_options: self.bep_options,
            clock: self.clock,
            publish_lifecycle_events: self.bes_options.bes_lifecycle_events,
            sleeper: self.sleeper.unwrap_or_else(|| Arc::new(JavaSleeper)),
            artifact_group_namer: self.artifact_group_namer,
            event_bus: self.event_bus,
            command_context: self.command_context,
            command_start_time: self.command_start_time,
        }
        .build();
        BuildEventServiceTransport {
            bes_uploader,
            bes_timeout,
            bes_upload_mode: self.bes_options.bes_upload_mode,
        }
    }
}
