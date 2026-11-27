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

/// A builder for [`BuildEventServiceTransport`]; its setters are the fields of the struct
/// literal that builds it.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:120 BuildEventServiceTransport.Builder
pub struct Builder {
    /// The setter `besClient`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:127 BuildEventServiceTransport.Builder.besClient
    pub bes_client: Arc<dyn BuildEventServiceClient>,
    /// The setter `localFileUploader`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:133 BuildEventServiceTransport.Builder.localFileUploader
    pub local_file_uploader: Arc<dyn BuildEventArtifactUploader>,
    /// The setter `besOptions`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:145 BuildEventServiceTransport.Builder.besOptions
    pub bes_options: BuildEventServiceOptions,
    /// The setter `bepOptions`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:139 BuildEventServiceTransport.Builder.bepOptions
    pub bep_options: Arc<BuildEventProtocolOptions>,
    /// The setter `clock`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:151 BuildEventServiceTransport.Builder.clock
    pub clock: Arc<dyn BesClock>,
    /// The setter `artifactGroupNamer`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:157 BuildEventServiceTransport.Builder.artifactGroupNamer
    pub artifact_group_namer: Arc<dyn ArtifactGroupNamer>,
    /// The setter `eventBus`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:163 BuildEventServiceTransport.Builder.eventBus
    pub event_bus: Arc<dyn EventBus>,
    /// The setter `sleeper`; `null` is a `JavaSleeper`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:169 BuildEventServiceTransport.Builder.sleeper
    pub sleeper: Option<Arc<dyn Sleeper>>,
    /// The setter `commandContext`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:176 BuildEventServiceTransport.Builder.commandContext
    pub command_context: CommandContext,
    /// The setter `commandStartTime`.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:182 BuildEventServiceTransport.Builder.commandStartTime
    pub command_start_time: Instant,
}

impl Builder {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransport.java:188 BuildEventServiceTransport.Builder.build
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
