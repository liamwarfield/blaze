//! Port of `AbstractBuildEventServiceTransportTest` with its gRPC subclass
//! `BuildEventServiceTransportGrpcTest` and the fake server `AbstractBuildEventRecorder` /
//! `BuildEventRecorderGrpc`.
//!
//! PORT: there is no in-process gRPC transport; the recorder listens on `127.0.0.1:0` and the
//! client talks to it over a plaintext channel. The `started` event of the Java tests is a
//! `BuildStartingEvent` over a mocked `BuildRequest`; a progress event stands in for it (the
//! tests compare the bytes the transport sends with the event's own serialization, whatever the
//! event).
// Java: com/google/devtools/build/lib/buildeventservice/AbstractBuildEventServiceTransportTest.java
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceTransportGrpcTest.java

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use std::collections::HashMap;

use blaze_bep::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, LocalFilesArtifactUploader, PathConverterFuture,
};
use blaze_bep::buildeventstream::build_completing_event::BuildCompletingEvent;
use blaze_bep::buildeventstream::build_event::BuildEvent;
use blaze_bep::buildeventstream::build_event::{LocalFile, LocalFileType};
use blaze_bep::buildeventstream::build_event_context::{
    ArtifactGroupNamer, BuildEventContext, FileUriPathConverter, PathConverter,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::buildeventstream::generic_build_event::ProgressEvent;
use blaze_bep::buildeventstream::generic_build_event::proto_chaining_with;
use blaze_bep::buildeventstream::transport::{BuildEventTransport, CloseOutcome};
use blaze_bep::runtime::counting_artifact_group_namer::CountingArtifactGroupNamer;
use blaze_bes::client::grpc_client::BuildEventServiceGrpcClient;
use blaze_bes::client::proto_util::{self, EventCase};
use blaze_bes::client::{CommandContext, Instant, Interrupted, InvocationStatus};
use blaze_bes::stubs::publish_build_event_server::{PublishBuildEvent, PublishBuildEventServer};
use blaze_bes::transport::{self, BuildEventServiceTransport};
use blaze_bes::uploader::{BesClock, Sleeper};
use blaze_events::{EventBus, Postable};
use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream as bes;
use blaze_protos::failure_details::build_progress::Code as BuildProgressCode;
use blaze_protos::google::devtools::build::v1::build_event::Event;
use blaze_protos::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest, StreamId,
};
use blaze_protos::google::protobuf::Empty;
use blaze_remote::grpc::channel::testing::channel_to;
use blaze_util::ExitCode;
use blaze_vfs::Path;
use indexmap::IndexSet;
use parking_lot::Mutex;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

const BUILD_REQUEST_ID: &str = "feedbeef-dead-4321-beef-deaddeaddead";
const BUILD_INVOCATION_ID: &str = "feedbeef-dead-4444-beef-deaddeaddead";

fn command_start_time() -> Instant {
    Instant::of_epoch_milli(500)
}

fn command_context() -> CommandContext {
    // BlazeVersionInfo, read by TracingMetadataUtils.buildMetadata.
    blaze_remote::util::tracing_metadata_utils::set_tool_version("");
    CommandContext::new(
        BUILD_REQUEST_ID.to_string(),
        BUILD_INVOCATION_ID.to_string(),
        1,
        ["foo=bar", "spam=eggs"]
            .iter()
            .map(|s| s.to_string())
            .collect::<IndexSet<_>>(),
        None,
        false,
    )
}

/// `ManualClock`, shared with the transport.
#[derive(Default)]
struct ManualClock(Mutex<i64>);

impl ManualClock {
    fn advance_millis(&self, millis: i64) {
        *self.0.lock() += millis;
    }
}

impl BesClock for ManualClock {
    fn now(&self) -> Instant {
        Instant::of_epoch_milli(*self.0.lock())
    }
}

/// "Reduce exponential backoff sleep times to speed up testing".
struct FastSleeper;

impl Sleeper for FastSleeper {
    fn sleep_millis(&self, millis: i64) -> Result<(), Interrupted> {
        std::thread::sleep(Duration::from_millis(
            (if millis > 10 { millis / 10 } else { millis }) as u64,
        ));
        Ok(())
    }
}

#[derive(Default)]
struct NullEventBus;

impl EventBus for NullEventBus {
    fn post(&self, _event: Arc<dyn Postable>) {}
}

type StreamPredicate = Box<dyn FnMut(&PublishBuildToolEventStreamRequest) -> bool + Send>;
type LifecyclePredicate = Box<dyn FnMut(&PublishLifecycleEventRequest) -> bool + Send>;

/// The recorder's state (`AbstractBuildEventRecorder` + `BuildEventRecorderGrpc`).
struct RecorderState {
    stream_event_predicate: StreamPredicate,
    stream_event_response_status: Option<Status>,
    lifecycle_event_predicate: LifecyclePredicate,
    lifecycle_event_response_status: Option<Status>,
    send_responses_on_request_predicate: StreamPredicate,
    response_buffer: Vec<PublishBuildToolEventStreamResponse>,
    lifecycle_events: Vec<(StreamId, PublishLifecycleEventRequest)>,
    stream_events: Vec<(StreamId, PublishBuildToolEventStreamRequest)>,
    successful_stream_events: Vec<(StreamId, PublishBuildToolEventStreamRequest)>,
    send_out_of_order_acknowledgments: bool,
    /// `publishBuildToolEventStreamAccepted`.
    publish_build_tool_event_stream_accepted: bool,
    /// `eventStreamError`: the status of a request stream that failed.
    event_stream_error: Option<Status>,
}

#[derive(Clone)]
struct Recorder(Arc<Mutex<RecorderState>>);

impl Recorder {
    fn new() -> Recorder {
        Recorder(Arc::new(Mutex::new(RecorderState {
            stream_event_predicate: Box::new(|_| false),
            stream_event_response_status: None,
            lifecycle_event_predicate: Box::new(|_| false),
            lifecycle_event_response_status: None,
            send_responses_on_request_predicate: Box::new(|_| true),
            response_buffer: Vec::new(),
            lifecycle_events: Vec::new(),
            stream_events: Vec::new(),
            successful_stream_events: Vec::new(),
            send_out_of_order_acknowledgments: false,
            publish_build_tool_event_stream_accepted: false,
            event_stream_error: None,
        })))
    }

    fn get_lifecycle_events(&self, stream_id: &StreamId) -> Vec<PublishLifecycleEventRequest> {
        self.0
            .lock()
            .lifecycle_events
            .iter()
            .filter(|(id, _)| id == stream_id)
            .map(|(_, r)| r.clone())
            .collect()
    }

    fn get_stream_events(&self, stream_id: &StreamId) -> Vec<PublishBuildToolEventStreamRequest> {
        self.0
            .lock()
            .stream_events
            .iter()
            .filter(|(id, _)| id == stream_id)
            .map(|(_, r)| r.clone())
            .collect()
    }

    fn get_successful_stream_events(
        &self,
        stream_id: &StreamId,
    ) -> Vec<PublishBuildToolEventStreamRequest> {
        self.0
            .lock()
            .successful_stream_events
            .iter()
            .filter(|(id, _)| id == stream_id)
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// `setStreamEventPredicateAndResponseStatus`; a `null` status (`None`) records the event
    /// as successful without answering it.
    fn set_stream_event_predicate_and_response_status(
        &self,
        predicate: StreamPredicate,
        status: impl Into<Option<Status>>,
    ) {
        let mut state = self.0.lock();
        state.stream_event_predicate = predicate;
        state.stream_event_response_status = status.into();
    }

    fn set_send_responses_on_request_predicate(&self, predicate: StreamPredicate) {
        self.0.lock().send_responses_on_request_predicate = predicate;
    }

    fn publish_build_tool_event_stream_accepted(&self) -> bool {
        self.0.lock().publish_build_tool_event_stream_accepted
    }

    fn event_stream_error(&self) -> Option<Status> {
        self.0.lock().event_stream_error.clone()
    }

    fn set_lifecycle_event_predicate_and_response_status(
        &self,
        predicate: LifecyclePredicate,
        status: Status,
    ) {
        let mut state = self.0.lock();
        state.lifecycle_event_predicate = predicate;
        state.lifecycle_event_response_status = Some(status);
    }

    fn send_out_of_order_acknowledgments(&self) {
        self.0.lock().send_out_of_order_acknowledgments = true;
    }

    /// Starts the recorder on `127.0.0.1:0`.
    fn start(&self) -> std::net::SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("address");
        listener.set_nonblocking(true).expect("non-blocking");
        let service = self.clone();
        blaze_remote::runtime::runtime().spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
            tonic::transport::Server::builder()
                .serve_with_incoming(
                    RecordResets(PublishBuildEventServer::new(service)),
                    tonic::transport::server::TcpIncoming::from(listener),
                )
                .await
                .expect("fake server failed");
        });
        addr
    }
}

/// Whether the request body of a call ended with an error: the client reset the stream
/// (`RST_STREAM`). tonic's server `Streaming` reports a `CANCELLED` request stream as the end of
/// the requests (tonic `codec/decode.rs`), where grpc-java's server reports the cancel.
#[derive(Clone, Default)]
struct RequestReset(Arc<std::sync::atomic::AtomicBool>);

impl RequestReset {
    fn is_reset(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The request body of a call, recording a reset in its [`RequestReset`].
struct ResetRecordingBody {
    inner: tonic::body::Body,
    reset: RequestReset,
}

impl http_body::Body for ResetRecordingBody {
    type Data = tonic::codegen::Bytes;
    type Error = Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let frame = Pin::new(&mut self.inner).poll_frame(cx);
        if let std::task::Poll::Ready(Some(Err(_))) = &frame {
            self.reset
                .0
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

/// The fake server's service with each request body wrapped in a [`ResetRecordingBody`], its
/// [`RequestReset`] in the request's extensions.
#[derive(Clone)]
struct RecordResets<S>(S);

impl<S> tonic::server::NamedService for RecordResets<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

impl<S> tonic::codegen::Service<tonic::codegen::http::Request<tonic::body::Body>>
    for RecordResets<S>
where
    S: tonic::codegen::Service<tonic::codegen::http::Request<tonic::body::Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, request: tonic::codegen::http::Request<tonic::body::Body>) -> Self::Future {
        let reset = RequestReset::default();
        let mut request = request.map(|inner| {
            tonic::body::Body::new(ResetRecordingBody {
                inner,
                reset: reset.clone(),
            })
        });
        request.extensions_mut().insert(reset);
        self.0.call(request)
    }
}

/// `statusFor(PublishLifecycleEventRequest)`.
fn lifecycle_status_for(request: &PublishLifecycleEventRequest) -> Status {
    let ordered = request.build_event.clone().unwrap_or_default();
    let case = ordered
        .event
        .as_ref()
        .map(EventCase::of)
        .unwrap_or(EventCase::EventNotSet);
    match case {
        EventCase::InvocationAttemptStarted | EventCase::BuildEnqueued
            if ordered.sequence_number == 1 =>
        {
            Status::ok("")
        }
        EventCase::InvocationAttemptFinished | EventCase::BuildFinished
            if ordered.sequence_number == 2 =>
        {
            Status::ok("")
        }
        _ => Status::unknown(""),
    }
}

/// `statusFor(PublishBuildToolEventStreamRequest)`: OK for the last event, else none.
fn stream_status_for(request: &PublishBuildToolEventStreamRequest) -> Option<Status> {
    let is_finished = matches!(
        request
            .ordered_build_event
            .as_ref()
            .and_then(|o| o.event.as_ref())
            .and_then(|e| e.event.as_ref()),
        Some(Event::ComponentStreamFinished(_))
    );
    is_finished.then(|| Status::ok(""))
}

fn response_for(
    request: &PublishBuildToolEventStreamRequest,
) -> PublishBuildToolEventStreamResponse {
    let ordered = request.ordered_build_event.clone().unwrap_or_default();
    PublishBuildToolEventStreamResponse {
        stream_id: ordered.stream_id,
        sequence_number: ordered.sequence_number,
    }
}

type ResponseStream =
    Pin<Box<dyn Stream<Item = Result<PublishBuildToolEventStreamResponse, Status>> + Send>>;

#[tonic::async_trait]
impl PublishBuildEvent for Recorder {
    async fn publish_lifecycle_event(
        &self,
        request: Request<PublishLifecycleEventRequest>,
    ) -> Result<Response<Empty>, Status> {
        let request = request.into_inner();
        let mut state = self.0.lock();
        let stream_id = request
            .build_event
            .as_ref()
            .and_then(|o| o.stream_id.clone())
            .unwrap_or_default();
        state.lifecycle_events.push((stream_id, request.clone()));
        // computeLifecycleResponse
        let status = if (state.lifecycle_event_predicate)(&request) {
            state.lifecycle_event_response_status.clone().unwrap()
        } else {
            lifecycle_status_for(&request)
        };
        if status.code() == tonic::Code::Ok {
            Ok(Response::new(Empty {}))
        } else {
            Err(status)
        }
    }

    type PublishBuildToolEventStreamStream = ResponseStream;

    async fn publish_build_tool_event_stream(
        &self,
        request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
    ) -> Result<Response<ResponseStream>, Status> {
        self.0.lock().publish_build_tool_event_stream_accepted = true;
        let reset = request
            .extensions()
            .get::<RequestReset>()
            .cloned()
            .expect("the server records resets");
        let mut requests = request.into_inner();
        let (sender, receiver) = tokio::sync::mpsc::channel(1024);
        let recorder = self.clone();
        tokio::spawn(async move {
            // onCompleted() {}: the client's half-close does not end the response stream; it
            // ends when a response status says so, or when the client cancels the call.
            loop {
                let request = match requests.message().await {
                    Ok(Some(request)) => request,
                    // A cancel before the half-close: grpc-java's server calls
                    // onError(Status.CANCELLED "client cancelled") (ServerCalls
                    // StreamingServerCallListener.onCancel); tonic reports it as the end of the
                    // requests, which RecordResets tells apart.
                    Ok(None) if reset.is_reset() => {
                        recorder.0.lock().event_stream_error =
                            Some(Status::cancelled("client cancelled"));
                        return;
                    }
                    Ok(None) => {
                        sender.closed().await;
                        return;
                    }
                    // onError(t): eventStreamError = Status.fromThrowable(t)
                    Err(status) => {
                        recorder.0.lock().event_stream_error = Some(status);
                        return;
                    }
                };
                let mut out: Vec<Result<PublishBuildToolEventStreamResponse, Status>> = Vec::new();
                let mut end = false;
                {
                    let mut state = recorder.0.lock();
                    let stream_id = request
                        .ordered_build_event
                        .as_ref()
                        .and_then(|o| o.stream_id.clone())
                        .unwrap_or_default();
                    state
                        .stream_events
                        .push((stream_id.clone(), request.clone()));
                    if state.send_out_of_order_acknowledgments {
                        let mut response = response_for(&request);
                        response.sequence_number += 1;
                        out.push(Ok(response));
                    } else {
                        // computeStreamResponse
                        let (status, responses) = if (state.stream_event_predicate)(&request) {
                            (state.stream_event_response_status.clone(), Vec::new())
                        } else if (state.send_responses_on_request_predicate)(&request) {
                            let mut responses = std::mem::take(&mut state.response_buffer);
                            responses.push(response_for(&request));
                            (stream_status_for(&request), responses)
                        } else {
                            state.response_buffer.push(response_for(&request));
                            (stream_status_for(&request), Vec::new())
                        };
                        match status {
                            Some(s) if s.code() != tonic::Code::Ok => {
                                out.push(Err(s));
                                end = true;
                            }
                            status => {
                                state
                                    .successful_stream_events
                                    .push((stream_id, request.clone()));
                                out.extend(responses.into_iter().map(Ok));
                                if status.is_some() {
                                    end = true;
                                }
                            }
                        }
                    }
                }
                for item in out {
                    if sender.send(item).await.is_err() {
                        return;
                    }
                }
                if end {
                    // stream.onCompleted() / onError(): the response stream ends.
                    return;
                }
            }
        });
        let stream: ResponseStream = Box::pin(ReceiverStream::new(receiver));
        Ok(Response::new(stream))
    }
}

/// What the tests share: the clock, the recorder and the events.
struct Fixture {
    clock: Arc<ManualClock>,
    server: Recorder,
    addr: std::net::SocketAddr,
    namer: Arc<dyn ArtifactGroupNamer>,
    started: Arc<dyn BuildEvent>,
    progress: Arc<dyn BuildEvent>,
    success: Arc<dyn BuildEvent>,
    failed: Arc<dyn BuildEvent>,
}

impl Fixture {
    fn new() -> Fixture {
        let server = Recorder::new();
        let addr = server.start();
        Fixture {
            clock: Arc::new(ManualClock::default()),
            server,
            addr,
            namer: Arc::new(CountingArtifactGroupNamer::new()),
            started: ProgressEvent::progress_update_empty(0),
            progress: ProgressEvent::progress_update_empty(1),
            success: Arc::new(BuildCompletingEvent::new(ExitCode::SUCCESS, 0, vec![])),
            failed: Arc::new(BuildCompletingEvent::new(
                ExitCode::BUILD_FAILURE,
                0,
                vec![],
            )),
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/AbstractBuildEventServiceTransportTest.java:938 AbstractBuildEventServiceTransportTest.newBuildEventServiceTransport
    fn new_transport(&self, publish_lifecycle_events: bool) -> BuildEventServiceTransport {
        self.new_transport_with(
            publish_lifecycle_events,
            Duration::ZERO,
            Arc::new(LocalFilesArtifactUploader),
        )
    }

    // Java: com/google/devtools/build/lib/buildeventservice/AbstractBuildEventServiceTransportTest.java:944 AbstractBuildEventServiceTransportTest.newBuildEventServiceTransport
    fn new_transport_with(
        &self,
        publish_lifecycle_events: bool,
        close_timeout: Duration,
        artifact_uploader: Arc<dyn BuildEventArtifactUploader>,
    ) -> BuildEventServiceTransport {
        let bes_options = BuildEventServiceOptions {
            bes_timeout: close_timeout,
            bes_lifecycle_events: publish_lifecycle_events,
            ..BuildEventServiceOptions::default()
        };
        transport::Builder {
            bes_client: Arc::new(BuildEventServiceGrpcClient::new(
                channel_to(self.addr, vec![], None),
                None,
            )),
            local_file_uploader: artifact_uploader,
            bes_options,
            bep_options: Arc::new(BuildEventProtocolOptions::default()),
            clock: self.clock.clone(),
            artifact_group_namer: self.namer.clone(),
            event_bus: Arc::new(NullEventBus),
            sleeper: Some(Arc::new(FastSleeper)),
            command_context: command_context(),
            command_start_time: command_start_time(),
        }
        .build()
    }

    /// `event.asStreamProto(buildEventContext).toByteString()`.
    fn bytes(&self, event: &Arc<dyn BuildEvent>) -> Vec<u8> {
        struct Ctx<'a> {
            converter: FileUriPathConverter,
            namer: &'a dyn ArtifactGroupNamer,
            options: BuildEventProtocolOptions,
        }
        impl BuildEventContext for Ctx<'_> {
            fn path_converter(&self) -> &dyn PathConverter {
                &self.converter
            }
            fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
                self.namer
            }
            fn get_options(&self) -> &BuildEventProtocolOptions {
                &self.options
            }
            fn get_file_mode_for_output_group(&self, _output_group: &str) -> OutputGroupFileMode {
                OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
            }
        }
        let ctx = Ctx {
            converter: FileUriPathConverter,
            namer: self.namer.as_ref(),
            options: BuildEventProtocolOptions::default(),
        };
        event
            .as_stream_proto(&ctx)
            .expect("not interrupted")
            .to_byte_array()
    }
}

fn stream_id(case: EventCase) -> StreamId {
    proto_util::stream_id(&command_context(), case)
}

/// `transport.close().get()` that must fail: the failure's code.
fn close_failure(transport: &BuildEventServiceTransport) -> &'static str {
    match transport.close().get_outcome(Some(Duration::from_secs(20))) {
        CloseOutcome::Done(Err(e)) => e.code,
        other => panic!("expected a failed close, got {other:?}"),
    }
}

fn close_ok(transport: &BuildEventServiceTransport) {
    match transport.close().get_outcome(Some(Duration::from_secs(20))) {
        CloseOutcome::Done(Ok(())) => {}
        other => panic!("expected a successful close, got {other:?}"),
    }
}

/// `everyEventFailsOnce()`.
fn every_event_fails_once<T: JavaMessage + Send + 'static>() -> Box<dyn FnMut(&T) -> bool + Send> {
    let mut already_matched: HashSet<Vec<u8>> = HashSet::new();
    Box::new(move |o: &T| already_matched.insert(o.to_byte_array()))
}

fn test_publish_lifecycle_events(
    expected_status: InvocationStatus,
    last_event: fn(&Fixture) -> Arc<dyn BuildEvent>,
) {
    let f = Fixture::new();
    f.clock.advance_millis(750);
    let invocation_started_timestamp = f.clock.now();
    let transport = f.new_transport(true);
    f.clock.advance_millis(250);
    let timestamp = f.clock.now();
    let last_event = last_event(&f);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&last_event);
    close_ok(&transport);

    let ctx = command_context();
    // build lifecycle events
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::BuildEnqueued)),
        vec![
            proto_util::build_enqueued(&ctx, command_start_time()),
            proto_util::build_finished(&ctx, timestamp, expected_status),
        ]
    );
    // invocation lifecycle events
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::InvocationAttemptStarted)),
        vec![
            proto_util::invocation_started(&ctx, invocation_started_timestamp),
            proto_util::invocation_finished(&ctx, timestamp, expected_status),
        ]
    );
    // bazel stream events
    assert_eq!(
        f.server
            .get_stream_events(&stream_id(EventCase::BazelEvent)),
        vec![
            proto_util::bazel_event(&ctx, timestamp, 1, &f.bytes(&f.started)),
            proto_util::bazel_event(&ctx, timestamp, 2, &f.bytes(&f.progress)),
            proto_util::bazel_event(&ctx, timestamp, 3, &f.bytes(&last_event)),
            proto_util::stream_finished(&ctx, timestamp, 4),
        ]
    );
}

#[test]
fn test_publish_lifecyle_events_command_succeeded() {
    test_publish_lifecycle_events(InvocationStatus::Succeeded, |f| f.success.clone());
}

#[test]
fn test_publish_lifecycle_events_command_failed() {
    test_publish_lifecycle_events(InvocationStatus::Failed, |f| f.failed.clone());
}

#[test]
fn test_publish_lifecycle_events_status_unknown() {
    test_publish_lifecycle_events(InvocationStatus::Unknown, |f| f.progress.clone());
}

#[test]
fn disabling_lifecycle_events_works() {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&f.success);
    close_ok(&transport);
    let ctx = command_context();
    assert_eq!(
        f.server
            .get_stream_events(&stream_id(EventCase::BazelEvent)),
        vec![
            proto_util::bazel_event(&ctx, timestamp, 1, &f.bytes(&f.started)),
            proto_util::bazel_event(&ctx, timestamp, 2, &f.bytes(&f.progress)),
            proto_util::bazel_event(&ctx, timestamp, 3, &f.bytes(&f.success)),
            proto_util::stream_finished(&ctx, timestamp, 4),
        ]
    );
    assert!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::BuildEnqueued))
            .is_empty()
    );
}

#[test]
fn send_events_in_lock_step() {
    // A test that only sends the next build event after the previous build event has been ACKed
    // by the server.
    let f = Fixture::new();
    let transport = f.new_transport(false);
    let to_send = [f.started.clone(), f.progress.clone(), f.success.clone()];
    for (i, event) in to_send.iter().enumerate() {
        transport.send_build_event(event);
        while f
            .server
            .get_successful_stream_events(&stream_id(EventCase::BazelEvent))
            .len()
            != i + 1
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    close_ok(&transport);
}

#[test]
fn retries_for_last_event_should_work() {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    // Send UNAVAILABLE on streamFinished event
    let expected = proto_util::stream_finished(&command_context(), timestamp, 4);
    f.server.set_stream_event_predicate_and_response_status(
        Box::new(move |req| *req == expected),
        Status::unavailable(""),
    );
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&f.success);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesUploadRetryLimitExceededFailure.as_str_name()
    );
}

fn test_permanent_errors_cause_blaze_exit(status: Status, build_progress_code: BuildProgressCode) {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    f.server
        .set_stream_event_predicate_and_response_status(Box::new(|_| true), status);
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    assert_eq!(close_failure(&transport), build_progress_code.as_str_name());
    assert!(
        f.server
            .get_stream_events(&stream_id(EventCase::BazelEvent))
            .contains(&proto_util::bazel_event(
                &command_context(),
                timestamp,
                1,
                &f.bytes(&f.started)
            ))
    );
}

#[test]
fn persistent_errors_should_not_be_retried_event_stream_invalid_argument() {
    test_permanent_errors_cause_blaze_exit(
        Status::invalid_argument(""),
        BuildProgressCode::BesStreamNotRetryingFailure,
    );
}

#[test]
fn persistent_errors_should_not_be_retried_event_stream_failed_precondition() {
    test_permanent_errors_cause_blaze_exit(
        Status::failed_precondition(""),
        BuildProgressCode::BesUploadTimeoutError,
    );
}

#[test]
fn persistent_errors_should_not_be_retried_lifecycle_events() {
    let f = Fixture::new();
    f.server.set_lifecycle_event_predicate_and_response_status(
        Box::new(|_| true),
        Status::failed_precondition(""),
    );
    let transport = f.new_transport(true);
    transport.send_build_event(&f.started);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesStreamNotRetryingFailure.as_str_name()
    );
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::BuildEnqueued)),
        vec![proto_util::build_enqueued(
            &command_context(),
            command_start_time()
        )]
    );
}

#[test]
fn retries_for_invocation_started_event_should_work() {
    let f = Fixture::new();
    f.clock.advance_millis(750);
    let invocation_started_timestamp = f.clock.now();
    // Respond with UNAVAILABLE to invocation started lifecycle event
    let expected = proto_util::invocation_started(&command_context(), invocation_started_timestamp);
    f.server.set_lifecycle_event_predicate_and_response_status(
        Box::new(move |req| *req == expected),
        Status::unavailable(""),
    );
    let transport = f.new_transport(true);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesUploadRetryLimitExceededFailure.as_str_name()
    );
    // should not proceed as lifecycle event failed
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::BuildEnqueued)),
        vec![proto_util::build_enqueued(
            &command_context(),
            command_start_time()
        )]
    );
    // should retry only the rpc that failed: 1 + --bes_upload_max_retries attempts.
    let max_retries = BuildEventProtocolOptions::default().bes_upload_max_retries;
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::InvocationAttemptStarted))
            .len(),
        (max_retries + 1) as usize
    );
}

#[test]
fn lifecycle_events_are_retried() {
    let f = Fixture::new();
    f.clock.advance_millis(750);
    let invocation_started_timestamp = f.clock.now();
    f.server.set_lifecycle_event_predicate_and_response_status(
        every_event_fails_once(),
        Status::unavailable(""),
    );
    let transport = f.new_transport(true);
    f.clock.advance_millis(250);
    let timestamp = f.clock.now();
    close_ok(&transport);
    let ctx = command_context();
    // all build lifecycle events
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::BuildEnqueued)),
        vec![
            proto_util::build_enqueued(&ctx, command_start_time()),
            proto_util::build_enqueued(&ctx, command_start_time()),
            proto_util::build_finished(&ctx, timestamp, InvocationStatus::Unknown),
            proto_util::build_finished(&ctx, timestamp, InvocationStatus::Unknown),
        ]
    );
    // all invocation lifecycle events
    assert_eq!(
        f.server
            .get_lifecycle_events(&stream_id(EventCase::InvocationAttemptStarted)),
        vec![
            proto_util::invocation_started(&ctx, invocation_started_timestamp),
            proto_util::invocation_started(&ctx, invocation_started_timestamp),
            proto_util::invocation_finished(&ctx, timestamp, InvocationStatus::Unknown),
            proto_util::invocation_finished(&ctx, timestamp, InvocationStatus::Unknown),
        ]
    );
}

#[test]
fn response_status_ok_with_acks_missing() {
    let f = Fixture::new();
    f.server
        .set_stream_event_predicate_and_response_status(every_event_fails_once(), Status::ok(""));
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&f.success);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesStreamCompletedWithUnackEventsError.as_str_name()
    );
}

#[test]
fn test_wrong_ack_should_fail_the_upload() {
    let f = Fixture::new();
    f.server.send_out_of_order_acknowledgments();
    let transport = f.new_transport(true);
    transport.send_build_event(&f.started);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesUploadTimeoutError.as_str_name()
    );
}

#[test]
fn test_acks_in_batch_mode() {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    // Send the first ACK only after the last event has been received.
    let last = proto_util::stream_finished(&command_context(), timestamp, 4);
    f.server
        .set_send_responses_on_request_predicate(Box::new(move |req| *req == last));
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&f.success);
    close_ok(&transport);
}

#[test]
fn test_retries_for_build_events_one_event_fails_always() {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    let ctx = command_context();
    let expected_payload = f.bytes(&f.progress);
    let failing = proto_util::bazel_event(&ctx, timestamp, 2, &expected_payload);
    let predicate_failing = failing.clone();
    f.server.set_stream_event_predicate_and_response_status(
        Box::new(move |req| *req == predicate_failing),
        Status::cancelled(""),
    );
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&f.success);
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesUploadRetryLimitExceededFailure.as_str_name()
    );
    assert!(
        f.server
            .get_successful_stream_events(&stream_id(EventCase::BazelEvent))
            .contains(&proto_util::bazel_event(
                &ctx,
                timestamp,
                1,
                &f.bytes(&f.started)
            ))
    );
    // containsAtLeast: the failing event at least five times (1 + --bes_upload_max_retries).
    let sent = f
        .server
        .get_stream_events(&stream_id(EventCase::BazelEvent));
    assert!(sent.iter().filter(|r| **r == failing).count() >= 5);
}

#[test]
fn test_retries_for_build_events_every_event_fails_once() {
    let f = Fixture::new();
    f.clock.advance_millis(1000);
    let timestamp = f.clock.now();
    f.server.set_stream_event_predicate_and_response_status(
        every_event_fails_once(),
        Status::unavailable(""),
    );
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.success);
    close_ok(&transport);
    let ctx = command_context();
    let started = proto_util::bazel_event(&ctx, timestamp, 1, &f.bytes(&f.started));
    let success = proto_util::bazel_event(&ctx, timestamp, 2, &f.bytes(&f.success));
    let finished = proto_util::stream_finished(&ctx, timestamp, 3);
    let successful = f
        .server
        .get_successful_stream_events(&stream_id(EventCase::BazelEvent));
    for expected in [&started, &success, &finished] {
        assert!(successful.contains(expected), "missing {expected:?}");
    }
    // containsAtLeast: each event sent at least twice (failed once, then sent again).
    let sent = f
        .server
        .get_stream_events(&stream_id(EventCase::BazelEvent));
    for expected in [&started, &success, &finished] {
        assert!(sent.iter().filter(|r| *r == expected).count() >= 2);
    }
}

#[test]
fn test_retries_for_build_events_acks_resets_attempts() {
    let f = Fixture::new();
    let failed_seq_numbers = Arc::new(Mutex::new(IndexSet::<i64>::new()));
    // Fail once with UNAVAILABLE (transient error) for every build event.
    let recorded = failed_seq_numbers.clone();
    f.server.set_stream_event_predicate_and_response_status(
        Box::new(move |req| {
            let seq_number = req.ordered_build_event.as_ref().unwrap().sequence_number;
            recorded.lock().insert(seq_number)
        }),
        Status::unavailable(""),
    );
    let transport = f.new_transport(false);
    transport.send_build_event(&f.started);
    for _ in 0..10 {
        transport.send_build_event(&f.progress);
    }
    transport.send_build_event(&f.success);
    close_ok(&transport);
    let successful_sequence_numbers: HashSet<i64> = f
        .server
        .get_successful_stream_events(&stream_id(EventCase::BazelEvent))
        .iter()
        .map(|e| e.ordered_build_event.as_ref().unwrap().sequence_number)
        .collect();
    let failed: HashSet<i64> = failed_seq_numbers.lock().iter().copied().collect();
    assert_eq!(successful_sequence_numbers, failed);
    assert_eq!(successful_sequence_numbers.len(), 13);
}

/// `BuildEventWithFiles`: a named set of the files, with the converted URIs.
#[derive(Debug)]
struct BuildEventWithFiles {
    files: Vec<LocalFile>,
}

impl BuildEvent for BuildEventWithFiles {
    fn as_stream_proto(
        &self,
        converters: &dyn BuildEventContext,
    ) -> Result<bes::BuildEvent, blaze_bep::buildeventstream::build_event::Interrupted> {
        let mut builder = bes::NamedSetOfFiles::default();
        for file in &self.files {
            if let Some(uri) = converters.path_converter().apply(&file.path) {
                builder.files.push(bes::File {
                    name: file.path.get_base_name().to_string(),
                    file: Some(bes::file::File::Uri(uri)),
                    ..Default::default()
                });
            }
        }
        Ok(proto_chaining_with(
            self,
            bes::build_event::Payload::NamedSetOfFiles(builder),
        ))
    }

    fn get_event_id(&self) -> bes::BuildEventId {
        build_event_id_util::from_artifact_group_name("list-of-files")
    }

    fn referenced_local_files(&self) -> Vec<LocalFile> {
        self.files.clone()
    }

    fn get_children_events(&self) -> Vec<bes::BuildEventId> {
        Vec::new()
    }
}

/// `BuildEventArtifactUploaderWithRefCounting` with `upload` given by the test (`Mockito.spy`
/// records the calls).
struct RecordingUploader {
    upload: Box<dyn Fn(&HashMap<Path, LocalFile>, usize) -> PathConverterFuture + Send + Sync>,
    calls: Mutex<Vec<HashMap<Path, LocalFile>>>,
}

impl BuildEventArtifactUploader for RecordingUploader {
    fn upload(&self, files: HashMap<Path, LocalFile>) -> PathConverterFuture {
        let call_count = {
            let mut calls = self.calls.lock();
            calls.push(files.clone());
            calls.len()
        };
        (self.upload)(&files, call_count)
    }

    fn may_be_slow(&self) -> bool {
        false
    }
}

/// A path converter over a map (`conversion::get`).
struct MapPathConverter(HashMap<Path, String>);

impl PathConverter for MapPathConverter {
    fn apply(&self, path: &Path) -> Option<String> {
        self.0.get(path).cloned()
    }
}

fn file_system() -> blaze_vfs::FileSystemRef {
    Arc::new(blaze_vfs::UnixFileSystem::new(
        blaze_vfs::DigestHashFunction::Sha256,
    ))
}

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> Path {
    let os_path = dir.join(name);
    std::fs::write(&os_path, content.as_bytes()).unwrap();
    Path::create(
        blaze_vfs::PathFragment::create(os_path.to_str().unwrap()),
        &file_system(),
    )
}

fn output_file(path: &Path) -> LocalFile {
    LocalFile::new(path.clone(), LocalFileType::OutputFile, None)
}

/// PORT: an in-memory file system stands in Java; the files live in a temporary directory of
/// the local file system with SHA-256 digests (`makeVfsHashFunction()` of the gRPC test).
#[test]
fn test_file_upload() {
    let f = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    let file1 = write_file(tmp.path(), "file1", "file1");
    let file2 = write_file(tmp.path(), "file2", "file2");
    let with_files: Arc<dyn BuildEvent> = Arc::new(BuildEventWithFiles {
        files: vec![output_file(&file1), output_file(&file2)],
    });
    let uploader = Arc::new(RecordingUploader {
        upload: Box::new(|files, _| {
            let mut conversion = HashMap::new();
            for file in files.keys() {
                match file.get_digest() {
                    Ok(digest) => {
                        conversion.insert(file.clone(), format!("cas://{}", hex(&digest)));
                    }
                    Err(e) => return PathConverterFuture::immediate_failed(e),
                }
            }
            PathConverterFuture::immediate(Box::new(MapPathConverter(conversion)))
        }),
        calls: Mutex::new(Vec::new()),
    });
    let transport = f.new_transport_with(true, Duration::ZERO, uploader.clone());
    transport.send_build_event(&f.started);
    transport.send_build_event(&f.progress);
    transport.send_build_event(&with_files);
    transport.send_build_event(&f.success);
    close_ok(&transport);

    let expected: HashMap<Path, LocalFile> = [
        (file1.clone(), output_file(&file1)),
        (file2.clone(), output_file(&file2)),
    ]
    .into_iter()
    .collect();
    assert!(uploader.calls.lock().contains(&expected));

    let events = f
        .server
        .get_stream_events(&stream_id(EventCase::BazelEvent));
    let Some(Event::BazelEvent(any_event)) = events[2]
        .ordered_build_event
        .as_ref()
        .and_then(|o| o.event.as_ref())
        .and_then(|e| e.event.clone())
    else {
        panic!("not a bazel event: {:?}", events[2]);
    };
    let build_event = <bes::BuildEvent as prost::Message>::decode(any_event.value.as_slice())
        .expect("a build event");
    let Some(bes::build_event::Payload::NamedSetOfFiles(set)) = build_event.payload else {
        panic!("not a named set of files: {build_event:?}");
    };
    assert_eq!(set.files.len(), 2);
    let referenced_files: HashSet<String> = set
        .files
        .iter()
        .map(|f| match &f.file {
            Some(bes::file::File::Uri(uri)) => uri.clone(),
            other => panic!("not a uri: {other:?}"),
        })
        .collect();
    let file1_hash = hex(&sha256(b"file1"));
    let file2_hash = hex(&sha256(b"file2"));
    assert_eq!(
        referenced_files,
        [format!("cas://{file1_hash}"), format!("cas://{file2_hash}")]
            .into_iter()
            .collect()
    );
}

#[test]
fn test_file_upload_with_duplicate_paths() {
    let f = Fixture::new();
    let tmp = tempfile::tempdir().unwrap();
    let file1 = write_file(tmp.path(), "file1", "file1");
    let with_files: Arc<dyn BuildEvent> = Arc::new(BuildEventWithFiles {
        files: vec![output_file(&file1), output_file(&file1)],
    });
    let uploader = Arc::new(RecordingUploader {
        upload: Box::new(|_, _| PathConverterFuture::immediate(Box::new(FileUriPathConverter))),
        calls: Mutex::new(Vec::new()),
    });
    let transport = f.new_transport_with(true, Duration::ZERO, uploader.clone());
    transport.send_build_event(&with_files);
    close_ok(&transport);
    // Check to make sure the code path was exercised
    let expected: HashMap<Path, LocalFile> =
        [(file1.clone(), output_file(&file1))].into_iter().collect();
    assert!(uploader.calls.lock().contains(&expected));
}

#[test]
fn test_file_upload_fails() {
    // Test that a failed file upload is not retried and fails the whole upload.
    let f = Fixture::new();
    let uploader = Arc::new(RecordingUploader {
        upload: Box::new(|_, call_count| {
            // When we create a connection but immediately abort it, it seems like the server is
            // not notified at all, so we need to post at least one event before we abort.
            match call_count {
                1 => PathConverterFuture::immediate(Box::new(NoConversion)),
                2 => PathConverterFuture::immediate_failed(std::io::Error::other(
                    "File upload failed.",
                )),
                _ => panic!("Expected exactly two calls to upload."),
            }
        }),
        calls: Mutex::new(Vec::new()),
    });
    let transport = f.new_transport_with(true, Duration::ZERO, uploader.clone());
    transport.send_build_event(&f.started);
    // Wait for lifecycle events to be sent.
    while !f.server.publish_build_tool_event_stream_accepted() {
        std::thread::sleep(Duration::from_millis(10));
    }
    // This event will trigger a upload that fails.
    transport.send_build_event(&f.success);
    // Wait until the server error is found _before_ we shut down the transport. Otherwise the
    // close might race with the error.
    while f.server.event_stream_error().is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        close_failure(&transport),
        BuildProgressCode::BesUploadLocalFileError.as_str_name()
    );
    let code = f.server.event_stream_error().unwrap().code();
    assert!(
        code == tonic::Code::Cancelled || code == tonic::Code::Internal,
        "{code:?}"
    );
    assert_eq!(uploader.calls.lock().len(), 2);
}

/// `PathConverter.NO_CONVERSION`.
struct NoConversion;

impl PathConverter for NoConversion {
    fn apply(&self, _path: &Path) -> Option<String> {
        None
    }
}

#[test]
fn test_close_timeout() {
    let f = Fixture::new();
    f.server
        .set_stream_event_predicate_and_response_status(Box::new(|_| true), None);
    // Timeout 1 second after calling close()
    let transport = f.new_transport_with(
        true,
        Duration::from_secs(1),
        Arc::new(LocalFilesArtifactUploader),
    );
    transport.send_build_event(&f.started);
    assert!(matches!(
        transport.close().get_outcome(Some(transport.get_timeout())),
        CloseOutcome::TimedOut
    ));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).to_vec()
}
