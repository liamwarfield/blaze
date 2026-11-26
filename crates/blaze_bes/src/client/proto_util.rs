//! Utility methods to create BES proto messages.

use blaze_protos::google::devtools::build::v1::build_event::build_component_stream_finished::FinishType;
use blaze_protos::google::devtools::build::v1::build_event::{
    BuildComponentStreamFinished, BuildEnqueued, BuildFinished, Event, InvocationAttemptFinished,
    InvocationAttemptStarted,
};
use blaze_protos::google::devtools::build::v1::build_status::Result as BuildResult;
use blaze_protos::google::devtools::build::v1::publish_lifecycle_event_request::ServiceLevel;
use blaze_protos::google::devtools::build::v1::stream_id::BuildComponent;
use blaze_protos::google::devtools::build::v1::{
    BuildEvent, BuildStatus, OrderedBuildEvent, PublishBuildToolEventStreamRequest,
    PublishLifecycleEventRequest, StreamId,
};
use blaze_protos::google::protobuf::{Any, Timestamp};

use super::{CommandContext, Instant, InvocationStatus, LifecycleEvent, StreamEvent};

/// `"type.googleapis.com/" + BuildEventStreamProtos.BuildEvent.getDescriptor().getFullName()`.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:46 BuildEventServiceProtoUtil.TYPE_URL
const TYPE_URL: &str = "type.googleapis.com/build_event_stream.BuildEvent";

/// `BuildEvent.getEventCase()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCase {
    InvocationAttemptStarted,
    InvocationAttemptFinished,
    BuildEnqueued,
    BuildFinished,
    ConsoleOutput,
    ComponentStreamFinished,
    BazelEvent,
    BuildExecutionEvent,
    SourceFetchEvent,
    EventNotSet,
}

impl EventCase {
    /// `getEventCase()` of a BES `BuildEvent`.
    pub fn of(event: &BuildEvent) -> EventCase {
        match &event.event {
            Some(Event::InvocationAttemptStarted(_)) => EventCase::InvocationAttemptStarted,
            Some(Event::InvocationAttemptFinished(_)) => EventCase::InvocationAttemptFinished,
            Some(Event::BuildEnqueued(_)) => EventCase::BuildEnqueued,
            Some(Event::BuildFinished(_)) => EventCase::BuildFinished,
            Some(Event::ConsoleOutput(_)) => EventCase::ConsoleOutput,
            Some(Event::ComponentStreamFinished(_)) => EventCase::ComponentStreamFinished,
            Some(Event::BazelEvent(_)) => EventCase::BazelEvent,
            Some(Event::BuildExecutionEvent(_)) => EventCase::BuildExecutionEvent,
            Some(Event::SourceFetchEvent(_)) => EventCase::SourceFetchEvent,
            None => EventCase::EventNotSet,
        }
    }

    /// `EventCase.name()`.
    fn name(self) -> &'static str {
        match self {
            EventCase::InvocationAttemptStarted => "INVOCATION_ATTEMPT_STARTED",
            EventCase::InvocationAttemptFinished => "INVOCATION_ATTEMPT_FINISHED",
            EventCase::BuildEnqueued => "BUILD_ENQUEUED",
            EventCase::BuildFinished => "BUILD_FINISHED",
            EventCase::ConsoleOutput => "CONSOLE_OUTPUT",
            EventCase::ComponentStreamFinished => "COMPONENT_STREAM_FINISHED",
            EventCase::BazelEvent => "BAZEL_EVENT",
            EventCase::BuildExecutionEvent => "BUILD_EXECUTION_EVENT",
            EventCase::SourceFetchEvent => "SOURCE_FETCH_EVENT",
            EventCase::EventNotSet => "EVENT_NOT_SET",
        }
    }
}

/// Creates a `PublishLifecycleEventRequest` from a [`LifecycleEvent`].
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:50 BuildEventServiceProtoUtil.publishLifecycleEventRequest
pub fn publish_lifecycle_event_request(
    command_context: &CommandContext,
    lifecycle_event: &LifecycleEvent,
) -> PublishLifecycleEventRequest {
    match lifecycle_event {
        LifecycleEvent::BuildEnqueued { event_time } => {
            build_enqueued(command_context, *event_time)
        }
        LifecycleEvent::InvocationStarted { event_time } => {
            invocation_started(command_context, *event_time)
        }
        LifecycleEvent::InvocationFinished { event_time, status } => {
            invocation_finished(command_context, *event_time, *status)
        }
        LifecycleEvent::BuildFinished { event_time, status } => {
            build_finished(command_context, *event_time, *status)
        }
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:64 BuildEventServiceProtoUtil.buildEnqueued
pub fn build_enqueued(
    command_context: &CommandContext,
    instant: Instant,
) -> PublishLifecycleEventRequest {
    lifecycle_request(
        command_context,
        1,
        BuildEvent {
            event_time: Some(to_proto_timestamp(instant)),
            event: Some(Event::BuildEnqueued(BuildEnqueued::default())),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:75 BuildEventServiceProtoUtil.buildFinished
pub fn build_finished(
    command_context: &CommandContext,
    event_time: Instant,
    status: InvocationStatus,
) -> PublishLifecycleEventRequest {
    lifecycle_request(
        command_context,
        2,
        BuildEvent {
            event_time: Some(to_proto_timestamp(event_time)),
            event: Some(Event::BuildFinished(BuildFinished {
                status: Some(build_status(status)),
                ..Default::default()
            })),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:86 BuildEventServiceProtoUtil.invocationStarted
pub fn invocation_started(
    command_context: &CommandContext,
    instant: Instant,
) -> PublishLifecycleEventRequest {
    lifecycle_request(
        command_context,
        1,
        BuildEvent {
            event_time: Some(to_proto_timestamp(instant)),
            event: Some(Event::InvocationAttemptStarted(InvocationAttemptStarted {
                attempt_number: i64::from(command_context.attempt_number),
                ..Default::default()
            })),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:100 BuildEventServiceProtoUtil.invocationFinished
pub fn invocation_finished(
    command_context: &CommandContext,
    event_time: Instant,
    status: InvocationStatus,
) -> PublishLifecycleEventRequest {
    lifecycle_request(
        command_context,
        2,
        BuildEvent {
            event_time: Some(to_proto_timestamp(event_time)),
            event: Some(Event::InvocationAttemptFinished(
                InvocationAttemptFinished {
                    invocation_status: Some(build_status(status)),
                    ..Default::default()
                },
            )),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:114 BuildEventServiceProtoUtil.buildStatus
fn build_status(status: InvocationStatus) -> BuildStatus {
    let result = match status {
        InvocationStatus::Unknown => BuildResult::UnknownStatus,
        InvocationStatus::Succeeded => BuildResult::CommandSucceeded,
        InvocationStatus::Failed => BuildResult::CommandFailed,
    };
    BuildStatus {
        result: result as i32,
        ..Default::default()
    }
}

/// Creates a `PublishBuildToolEventStreamRequest` from a [`StreamEvent`].
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:123 BuildEventServiceProtoUtil.publishBuildToolEventStreamRequest
pub fn publish_build_tool_event_stream_request(
    command_context: &CommandContext,
    stream_event: &StreamEvent,
) -> PublishBuildToolEventStreamRequest {
    match stream_event {
        StreamEvent::BazelEvent {
            event_time,
            sequence_number,
            payload,
        } => bazel_event(command_context, *event_time, *sequence_number, payload),
        StreamEvent::StreamFinished {
            event_time,
            sequence_number,
        } => stream_finished(command_context, *event_time, *sequence_number),
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:133 BuildEventServiceProtoUtil.bazelEvent
pub fn bazel_event(
    command_context: &CommandContext,
    event_time: Instant,
    sequence_number: i64,
    payload: &[u8],
) -> PublishBuildToolEventStreamRequest {
    // Any.pack() would require us to parse the payload into a Message, which is wasteful.
    // Implement it manually instead.
    let packed = Any {
        type_url: TYPE_URL.to_string(),
        value: payload.to_vec(),
    };
    stream_request(
        command_context,
        sequence_number,
        to_proto_timestamp(event_time),
        BuildEvent {
            event_time: None,
            event: Some(Event::BazelEvent(packed)),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:145 BuildEventServiceProtoUtil.streamFinished
pub fn stream_finished(
    command_context: &CommandContext,
    event_time: Instant,
    sequence_number: i64,
) -> PublishBuildToolEventStreamRequest {
    stream_request(
        command_context,
        sequence_number,
        to_proto_timestamp(event_time),
        BuildEvent {
            event_time: None,
            event: Some(Event::ComponentStreamFinished(
                BuildComponentStreamFinished {
                    r#type: FinishType::Finished as i32,
                },
            )),
        },
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:157 BuildEventServiceProtoUtil.streamRequest
pub fn stream_request(
    command_context: &CommandContext,
    sequence_number: i64,
    timestamp: Timestamp,
    mut bes_event: BuildEvent,
) -> PublishBuildToolEventStreamRequest {
    let event_case = EventCase::of(&bes_event);
    bes_event.event_time = Some(timestamp);
    let mut builder = PublishBuildToolEventStreamRequest {
        ordered_build_event: Some(OrderedBuildEvent {
            sequence_number,
            event: Some(bes_event),
            stream_id: Some(stream_id(command_context, event_case)),
        }),
        ..Default::default()
    };
    if sequence_number == 1 {
        builder
            .notification_keywords
            .extend(command_context.keywords.iter().cloned());
        builder.check_preceding_lifecycle_events_present =
            command_context.check_preceding_lifecycle_events;
    }
    if let Some(project_id) = &command_context.project_id {
        builder.project_id = project_id.clone();
    }
    builder
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:181 BuildEventServiceProtoUtil.lifecycleRequest
pub fn lifecycle_request(
    command_context: &CommandContext,
    sequence_number: i32,
    lifecycle_event: BuildEvent,
) -> PublishLifecycleEventRequest {
    let event_case = EventCase::of(&lifecycle_event);
    let mut builder = PublishLifecycleEventRequest {
        service_level: ServiceLevel::Interactive as i32,
        build_event: Some(OrderedBuildEvent {
            sequence_number: i64::from(sequence_number),
            stream_id: Some(stream_id(command_context, event_case)),
            event: Some(lifecycle_event),
        }),
        ..Default::default()
    };
    if let Some(project_id) = &command_context.project_id {
        builder.project_id = project_id.clone();
    }
    match event_case {
        EventCase::BuildEnqueued
        | EventCase::InvocationAttemptStarted
        | EventCase::BuildFinished => {
            builder
                .notification_keywords
                .extend(command_context.keywords.iter().cloned());
        }
        _ => {}
    }
    builder
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:203 BuildEventServiceProtoUtil.streamId
pub fn stream_id(command_context: &CommandContext, event_case: EventCase) -> StreamId {
    let mut stream_id = StreamId {
        build_id: command_context.build_id.clone(),
        ..Default::default()
    };
    match event_case {
        EventCase::BuildEnqueued | EventCase::BuildFinished => {
            stream_id.component = BuildComponent::Controller as i32;
        }
        EventCase::InvocationAttemptStarted | EventCase::InvocationAttemptFinished => {
            stream_id.invocation_id = command_context.invocation_id.clone();
            stream_id.component = BuildComponent::Controller as i32;
        }
        EventCase::BazelEvent | EventCase::ComponentStreamFinished => {
            stream_id.invocation_id = command_context.invocation_id.clone();
            stream_id.component = BuildComponent::Tool as i32;
        }
        _ => panic!(
            "IllegalArgumentException: Illegal EventCase {}",
            event_case.name()
        ),
    }
    stream_id
}

// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceProtoUtil.java:220 BuildEventServiceProtoUtil.toProtoTimestamp
pub fn to_proto_timestamp(instant: Instant) -> Timestamp {
    Timestamp {
        seconds: instant.epoch_second,
        nanos: instant.nano,
    }
}
