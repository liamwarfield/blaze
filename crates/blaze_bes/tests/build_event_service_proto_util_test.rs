//! Port of `BuildEventServiceProtoUtilTest`.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceProtoUtilTest.java

use blaze_bes::client::proto_util;
use blaze_bes::client::{CommandContext, Instant, InvocationStatus};
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
use indexmap::IndexSet;

const BUILD_REQUEST_ID: &str = "feedbeef-dead-4321-beef-deaddeaddead";
const BUILD_INVOCATION_ID: &str = "feedbeef-dead-4444-beef-deaddeaddead";
const ATTEMPT_NUMBER: i32 = 1;
const PROJECT_ID: &str = "my_project";

fn keywords() -> IndexSet<String> {
    ["foo=bar", "spam=eggs"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn command_context(attempt_number: i32, check_preceding: bool) -> CommandContext {
    CommandContext::new(
        BUILD_REQUEST_ID.to_string(),
        BUILD_INVOCATION_ID.to_string(),
        attempt_number,
        keywords(),
        Some(PROJECT_ID.to_string()),
        check_preceding,
    )
}

/// `ManualClock`: starts at the epoch.
struct ManualClock(i64);

impl ManualClock {
    fn advance_millis(&mut self, millis: i64) {
        self.0 += millis;
    }
    fn now(&self) -> Instant {
        Instant::of_epoch_milli(self.0)
    }
}

fn to_proto_timestamp(instant: Instant) -> Timestamp {
    Timestamp {
        seconds: instant.epoch_second,
        nanos: instant.nano,
    }
}

fn kw() -> Vec<String> {
    keywords().into_iter().collect()
}

#[test]
fn test_build_enqueued() {
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let expected = clock.now();
    assert_eq!(
        proto_util::build_enqueued(&command_context(ATTEMPT_NUMBER, false), expected),
        PublishLifecycleEventRequest {
            service_level: ServiceLevel::Interactive as i32,
            project_id: PROJECT_ID.to_string(),
            notification_keywords: kw(),
            build_event: Some(OrderedBuildEvent {
                stream_id: Some(StreamId {
                    build_id: BUILD_REQUEST_ID.to_string(),
                    component: BuildComponent::Controller as i32,
                    ..Default::default()
                }),
                sequence_number: 1,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(expected)),
                    event: Some(Event::BuildEnqueued(BuildEnqueued::default())),
                }),
            }),
            ..Default::default()
        }
    );
}

#[test]
fn test_invocation_attempt_started() {
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let expected = clock.now();
    assert_eq!(
        proto_util::invocation_started(&command_context(ATTEMPT_NUMBER, false), expected),
        PublishLifecycleEventRequest {
            service_level: ServiceLevel::Interactive as i32,
            project_id: PROJECT_ID.to_string(),
            notification_keywords: kw(),
            build_event: Some(OrderedBuildEvent {
                stream_id: Some(StreamId {
                    build_id: BUILD_REQUEST_ID.to_string(),
                    invocation_id: BUILD_INVOCATION_ID.to_string(),
                    component: BuildComponent::Controller as i32,
                }),
                sequence_number: 1,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(expected)),
                    event: Some(Event::InvocationAttemptStarted(InvocationAttemptStarted {
                        attempt_number: 1,
                        ..Default::default()
                    })),
                }),
            }),
            ..Default::default()
        }
    );
}

#[test]
fn invocation_attempt_started_attempt_number() {
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let expected = clock.now();
    let request = proto_util::invocation_started(&command_context(2, false), expected);
    let Some(Event::InvocationAttemptStarted(started)) = request
        .build_event
        .as_ref()
        .and_then(|o| o.event.as_ref())
        .and_then(|e| e.event.as_ref())
    else {
        panic!("not an InvocationAttemptStarted: {request:?}");
    };
    assert_eq!(started.attempt_number, 2);
    assert_eq!(request.notification_keywords, kw());
    assert_eq!(request.project_id, PROJECT_ID);
}

#[test]
fn test_invocation_attempt_finished() {
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let expected = clock.now();
    assert_eq!(
        proto_util::invocation_finished(
            &command_context(ATTEMPT_NUMBER, false),
            expected,
            InvocationStatus::Succeeded
        ),
        PublishLifecycleEventRequest {
            service_level: ServiceLevel::Interactive as i32,
            project_id: PROJECT_ID.to_string(),
            build_event: Some(OrderedBuildEvent {
                stream_id: Some(StreamId {
                    build_id: BUILD_REQUEST_ID.to_string(),
                    invocation_id: BUILD_INVOCATION_ID.to_string(),
                    component: BuildComponent::Controller as i32,
                }),
                sequence_number: 2,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(expected)),
                    event: Some(Event::InvocationAttemptFinished(
                        InvocationAttemptFinished {
                            invocation_status: Some(BuildStatus {
                                result: BuildResult::CommandSucceeded as i32,
                                ..Default::default()
                            }),
                            ..Default::default()
                        }
                    )),
                }),
            }),
            ..Default::default()
        }
    );
}

#[test]
fn test_build_finished() {
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let expected = clock.now();
    assert_eq!(
        proto_util::build_finished(
            &command_context(ATTEMPT_NUMBER, false),
            expected,
            InvocationStatus::Succeeded
        ),
        PublishLifecycleEventRequest {
            service_level: ServiceLevel::Interactive as i32,
            project_id: PROJECT_ID.to_string(),
            notification_keywords: kw(),
            build_event: Some(OrderedBuildEvent {
                stream_id: Some(StreamId {
                    build_id: BUILD_REQUEST_ID.to_string(),
                    component: BuildComponent::Controller as i32,
                    ..Default::default()
                }),
                sequence_number: 2,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(expected)),
                    event: Some(Event::BuildFinished(BuildFinished {
                        status: Some(BuildStatus {
                            result: BuildResult::CommandSucceeded as i32,
                            ..Default::default()
                        }),
                        ..Default::default()
                    })),
                }),
            }),
            ..Default::default()
        }
    );
}

fn tool_stream_id() -> StreamId {
    StreamId {
        build_id: BUILD_REQUEST_ID.to_string(),
        invocation_id: BUILD_INVOCATION_ID.to_string(),
        component: BuildComponent::Tool as i32,
    }
}

fn bazel_event_payload(payload: &[u8]) -> Option<Event> {
    Some(Event::BazelEvent(Any {
        type_url: "type.googleapis.com/build_event_stream.BuildEvent".to_string(),
        value: payload.to_vec(),
    }))
}

#[test]
fn test_stream_events() {
    let context = command_context(ATTEMPT_NUMBER, false);
    let mut clock = ManualClock(0);
    clock.advance_millis(100);
    let first_event_timestamp = clock.now();
    // ByteString.fromHex("deadbeef")
    let payload = [0xde, 0xad, 0xbe, 0xef];
    assert_eq!(
        proto_util::bazel_event(&context, first_event_timestamp, 1, &payload),
        PublishBuildToolEventStreamRequest {
            notification_keywords: kw(),
            project_id: PROJECT_ID.to_string(),
            ordered_build_event: Some(OrderedBuildEvent {
                stream_id: Some(tool_stream_id()),
                sequence_number: 1,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(first_event_timestamp)),
                    event: bazel_event_payload(&payload),
                }),
            }),
            ..Default::default()
        }
    );

    clock.advance_millis(100);
    let second_event_timestamp = clock.now();
    assert_eq!(
        proto_util::bazel_event(&context, second_event_timestamp, 2, &payload),
        PublishBuildToolEventStreamRequest {
            project_id: PROJECT_ID.to_string(),
            ordered_build_event: Some(OrderedBuildEvent {
                stream_id: Some(tool_stream_id()),
                sequence_number: 2,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(second_event_timestamp)),
                    event: bazel_event_payload(&payload),
                }),
            }),
            ..Default::default()
        }
    );

    clock.advance_millis(100);
    let third_event_timestamp = clock.now();
    assert_eq!(
        proto_util::stream_finished(&context, third_event_timestamp, 3),
        PublishBuildToolEventStreamRequest {
            project_id: PROJECT_ID.to_string(),
            ordered_build_event: Some(OrderedBuildEvent {
                stream_id: Some(tool_stream_id()),
                sequence_number: 3,
                event: Some(BuildEvent {
                    event_time: Some(to_proto_timestamp(third_event_timestamp)),
                    event: Some(Event::ComponentStreamFinished(
                        BuildComponentStreamFinished {
                            r#type: FinishType::Finished as i32,
                        }
                    )),
                }),
            }),
            ..Default::default()
        }
    );
}

#[test]
fn test_stream_events_with_check_preceding_lifecycle_events_enabled() {
    let payload = [0xde, 0xad, 0xbe, 0xef];
    let context = command_context(ATTEMPT_NUMBER, true);
    assert!(
        proto_util::bazel_event(&context, Instant::of_epoch_milli(100), 1, &payload)
            .check_preceding_lifecycle_events_present
    );
    // check_preceding_lifecycle_events_present is always false for events with
    // sequence_number > 1.
    assert!(
        !proto_util::bazel_event(&context, Instant::of_epoch_milli(100), 2, &payload)
            .check_preceding_lifecycle_events_present
    );
    assert!(
        !proto_util::bazel_event(&context, Instant::of_epoch_milli(100), 3, &payload)
            .check_preceding_lifecycle_events_present
    );
}
