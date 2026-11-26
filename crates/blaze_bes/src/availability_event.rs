//! `BuildEventServiceAvailabilityEvent` and `LargeBuildEventSerializedEvent`, posted by the
//! uploader.

use blaze_events::Postable;
use blaze_protos::failure_details::FailureDetail;
use blaze_util::ExitCode;

/// Event fired from the `BuildEventServiceUploader`.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceAvailabilityEvent.java:22 BuildEventServiceAvailabilityEvent
#[derive(Debug, Clone)]
pub struct BuildEventServiceAvailabilityEvent {
    exit_code: ExitCode,
    failure_detail: Option<FailureDetail>,
}

impl BuildEventServiceAvailabilityEvent {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceAvailabilityEvent.java:26 BuildEventServiceAvailabilityEvent.<init>
    pub fn new(
        exit_code: ExitCode,
        failure_detail: Option<FailureDetail>,
    ) -> BuildEventServiceAvailabilityEvent {
        BuildEventServiceAvailabilityEvent {
            exit_code,
            failure_detail,
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceAvailabilityEvent.java:32 BuildEventServiceAvailabilityEvent.ofSuccess
    pub fn of_success() -> BuildEventServiceAvailabilityEvent {
        BuildEventServiceAvailabilityEvent::new(ExitCode::SUCCESS, None)
    }

    /// `SUCCESS` if the build event upload was a success, otherwise an exit code that
    /// corresponds to the error that occurred during the build event upload.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceAvailabilityEvent.java:40 BuildEventServiceAvailabilityEvent.getExitCode
    pub fn get_exit_code(&self) -> &ExitCode {
        &self.exit_code
    }

    /// The failure detail of a failed upload; `None` if the upload completed successfully.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceAvailabilityEvent.java:50 BuildEventServiceAvailabilityEvent.getFailureDetail
    pub fn get_failure_detail(&self) -> Option<&FailureDetail> {
        self.failure_detail.as_ref()
    }
}

impl Postable for BuildEventServiceAvailabilityEvent {}

/// An event signalling that a large build event was serialized.
// Java: com/google/devtools/build/lib/buildeventstream/LargeBuildEventSerializedEvent.java:24 LargeBuildEventSerializedEvent
#[derive(Debug, Clone)]
pub struct LargeBuildEventSerializedEvent {
    build_event_id: String,
    size: i32,
}

impl LargeBuildEventSerializedEvent {
    // Java: com/google/devtools/build/lib/buildeventstream/LargeBuildEventSerializedEvent.java:30 LargeBuildEventSerializedEvent.SIZE_OF_LARGE_BUILD_EVENTS_IN_BYTES
    pub const SIZE_OF_LARGE_BUILD_EVENTS_IN_BYTES: usize = 10_000_000;

    // Java: com/google/devtools/build/lib/buildeventstream/LargeBuildEventSerializedEvent.java:35 LargeBuildEventSerializedEvent.<init>
    pub fn new(build_event_id: String, size: i32) -> LargeBuildEventSerializedEvent {
        LargeBuildEventSerializedEvent {
            build_event_id,
            size,
        }
    }

    pub fn get_build_event_id(&self) -> &str {
        &self.build_event_id
    }

    pub fn get_size(&self) -> i32 {
        self.size
    }
}

impl Postable for LargeBuildEventSerializedEvent {}
