//! The real uploader's close and half-close futures under the services of
//! `BazelBuildEventServiceModuleTest` (review BES-5).
//!
//! `blaze_runtime`'s `bes_module_tests` drive the module's closing and waiting code with a fake
//! transport whose futures take the shapes the real `BuildEventServiceTransport` has under
//! Java's `DelayingPublishBuildEventService`. These tests run the real transport against a port
//! of that service and observe the shapes the fake assumes, one test per mode:
//!
//! | service | half-close future | close future |
//! | --- | --- | --- |
//! | no delay | done (ok) at once | done (ok) at once |
//! | `setDelayBeforeClosingStream(d)` | done (ok) at once | done (ok) after `d` |
//! | `setDelayBeforeHalfClosingStream(d)` | done (ok) at once | done (ok) after `d` |
//! | `setErrorMessage(m)` | done (ok) | done (failed) |
//!
//! The uploader completes the half-close future when it half-closes the stream after sending the
//! final event (Java `BuildEventServiceUploader.halfCloseEventUploadingStream`), whatever the
//! service does with its acknowledgements; so the half-close is never late.
//!
//! PORT: the service listens on `127.0.0.1:0` (no in-process gRPC transport), as in
//! `build_event_service_transport_test`.
// Java: com/google/devtools/build/lib/buildeventservice/DelayingPublishBuildEventService.java
use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use blaze_bep::buildeventstream::artifact_uploader::LocalFilesArtifactUploader;
use blaze_bep::buildeventstream::build_completing_event::BuildCompletingEvent;
use blaze_bep::buildeventstream::build_event::BuildEvent;
use blaze_bep::buildeventstream::generic_build_event::ProgressEvent;
use blaze_bep::buildeventstream::transport::{BuildEventTransport, CloseOutcome};
use blaze_bep::runtime::counting_artifact_group_namer::CountingArtifactGroupNamer;
use blaze_bes::client::grpc_client::BuildEventServiceGrpcClient;
use blaze_bes::client::{CommandContext, Instant, Interrupted};
use blaze_bes::stubs::publish_build_event_server::{PublishBuildEvent, PublishBuildEventServer};
use blaze_bes::transport::{self, BuildEventServiceTransport};
use blaze_bes::uploader::{BesClock, Sleeper};
use blaze_events::{EventBus, Postable};
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_optdefs::misc::build_event_service_options::BuildEventServiceOptions;
use blaze_protos::google::devtools::build::v1::build_event::Event;
use blaze_protos::google::devtools::build::v1::build_event::build_component_stream_finished::FinishType;
use blaze_protos::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
    PublishLifecycleEventRequest,
};
use blaze_protos::google::protobuf::Empty;
use blaze_remote::grpc::channel::testing::channel_to;
use blaze_util::ExitCode;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

/// The scaled delay of the module tests' slow services (Java: 10 s).
const DELAY: Duration = Duration::from_secs(2);

/// `DelayingPublishBuildEventService`'s settings.
#[derive(Clone, Default)]
struct DelayingService {
    delay_before_closing_stream: Duration,
    delay_before_half_closing_stream: Duration,
    error_message: Option<&'static str>,
}

type ResponseStream =
    Pin<Box<dyn Stream<Item = Result<PublishBuildToolEventStreamResponse, Status>> + Send>>;

fn ack(request: &PublishBuildToolEventStreamRequest) -> PublishBuildToolEventStreamResponse {
    let ordered = request.ordered_build_event.clone().unwrap_or_default();
    PublishBuildToolEventStreamResponse {
        stream_id: ordered.stream_id,
        sequence_number: ordered.sequence_number,
    }
}

/// Whether the request carries the final event (`ComponentStreamFinished` of a known type).
fn is_final(request: &PublishBuildToolEventStreamRequest) -> bool {
    match request
        .ordered_build_event
        .as_ref()
        .and_then(|o| o.event.as_ref())
        .and_then(|e| e.event.as_ref())
    {
        Some(Event::ComponentStreamFinished(finished)) => {
            finished.r#type != FinishType::Unspecified as i32
        }
        _ => false,
    }
}

#[tonic::async_trait]
impl PublishBuildEvent for DelayingService {
    async fn publish_lifecycle_event(
        &self,
        _request: Request<PublishLifecycleEventRequest>,
    ) -> Result<Response<Empty>, Status> {
        Ok(Response::new(Empty {}))
    }

    type PublishBuildToolEventStreamStream = ResponseStream;

    async fn publish_build_tool_event_stream(
        &self,
        request: Request<Streaming<PublishBuildToolEventStreamRequest>>,
    ) -> Result<Response<ResponseStream>, Status> {
        let mut requests = request.into_inner();
        let (sender, receiver) = tokio::sync::mpsc::channel(1024);
        let service = self.clone();
        tokio::spawn(async move {
            if let Some(message) = service.error_message {
                // ErroringPublishBuildStreamObserver: acks every event; the client's half-close
                // (onCompleted) gets the error.
                loop {
                    match requests.message().await {
                        Ok(Some(request)) => {
                            let _ = sender.send(Ok(ack(&request))).await;
                        }
                        Ok(None) => {
                            let _ = sender.send(Err(Status::data_loss(message))).await;
                            return;
                        }
                        Err(_) => return,
                    }
                }
            }
            // DelayingPublishBuildStreamObserver: acks the previous event of each non-final
            // event; on the final event sleeps delayBeforeHalfClosingStream and acks all; after
            // the client's half-close sleeps delayBeforeClosingStream and completes.
            let mut unacked: BTreeSet<i64> = BTreeSet::new();
            let mut last: Option<PublishBuildToolEventStreamRequest> = None;
            loop {
                match requests.message().await {
                    Ok(Some(request)) => {
                        let seq = request
                            .ordered_build_event
                            .as_ref()
                            .map(|o| o.sequence_number)
                            .unwrap_or(0);
                        if !unacked.insert(seq) {
                            continue; // dupe, ignore
                        }
                        let mut to_ack = Vec::new();
                        if !is_final(&request) {
                            if unacked.len() > 1 {
                                to_ack.push(unacked.pop_first().unwrap());
                            }
                        } else {
                            tokio::time::sleep(service.delay_before_half_closing_stream).await;
                            while let Some(s) = unacked.pop_first() {
                                to_ack.push(s);
                            }
                        }
                        for s in to_ack {
                            let mut response = ack(&request);
                            response.sequence_number = s;
                            let _ = sender.send(Ok(response)).await;
                        }
                        last = Some(request);
                    }
                    Ok(None) => {
                        tokio::time::sleep(service.delay_before_closing_stream).await;
                        drop(last);
                        // Dropping the sender ends the responses: onCompleted.
                        return;
                    }
                    Err(_) => return,
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }
}

fn start(service: DelayingService) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("address");
    listener.set_nonblocking(true).expect("non-blocking");
    blaze_remote::runtime::runtime().spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
        tonic::transport::Server::builder()
            .serve_with_incoming(
                PublishBuildEventServer::new(service),
                tonic::transport::server::TcpIncoming::from(listener),
            )
            .await
            .expect("fake server failed");
    });
    addr
}

struct WallClock;

impl BesClock for WallClock {
    fn now(&self) -> Instant {
        Instant::of_epoch_milli(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        )
    }
}

struct FastSleeper;

impl Sleeper for FastSleeper {
    fn sleep_millis(&self, _millis: i64) -> Result<(), Interrupted> {
        Ok(())
    }
}

struct NullEventBus;

impl EventBus for NullEventBus {
    fn post(&self, _event: Arc<dyn Postable>) {}
}

fn transport_to(addr: std::net::SocketAddr) -> BuildEventServiceTransport {
    blaze_remote::util::tracing_metadata_utils::set_tool_version("");
    transport::Builder {
        bes_client: Arc::new(BuildEventServiceGrpcClient::new(
            channel_to(addr, vec![], None),
            None,
        )),
        local_file_uploader: Arc::new(LocalFilesArtifactUploader),
        bes_options: BuildEventServiceOptions {
            bes_lifecycle_events: false,
            ..BuildEventServiceOptions::default()
        },
        bep_options: Arc::new(BuildEventProtocolOptions::default()),
        clock: Arc::new(WallClock),
        artifact_group_namer: Arc::new(CountingArtifactGroupNamer::new()),
        event_bus: Arc::new(NullEventBus),
        sleeper: Some(Arc::new(FastSleeper)),
        command_context: CommandContext::new(
            "feedbeef-dead-4321-beef-deaddeaddead".to_owned(),
            "feedbeef-dead-4444-beef-deaddeaddead".to_owned(),
            1,
            Default::default(),
            None,
            false,
        ),
        command_start_time: Instant::of_epoch_milli(500),
    }
    .build()
}

/// Sends a build's events and closes; returns the half-close and close futures.
fn run_build(
    service: DelayingService,
) -> (
    blaze_bep::buildeventstream::transport::CloseFuture,
    blaze_bep::buildeventstream::transport::CloseFuture,
) {
    let transport = transport_to(start(service));
    let started: Arc<dyn BuildEvent> = ProgressEvent::progress_update_empty(0);
    let finished: Arc<dyn BuildEvent> =
        Arc::new(BuildCompletingEvent::new(ExitCode::SUCCESS, 0, vec![]));
    transport.send_build_event(&started);
    transport.send_build_event(&finished);
    let close = transport.close();
    let half_close = transport.get_half_close_future();
    (half_close, close)
}

/// The future is still pending after `wait`.
fn pending(future: &blaze_bep::buildeventstream::transport::CloseFuture, wait: Duration) {
    match future.get_outcome(Some(wait)) {
        CloseOutcome::TimedOut => {}
        other => panic!("expected a pending future, got {other:?}"),
    }
}

/// The future completes successfully within `wait`.
fn done_ok(future: &blaze_bep::buildeventstream::transport::CloseFuture, wait: Duration) {
    match future.get_outcome(Some(wait)) {
        CloseOutcome::Done(Ok(())) => {}
        other => panic!("expected a successful completion, got {other:?}"),
    }
}

const SOON: Duration = Duration::from_millis(1500);
const LATE: Duration = Duration::from_secs(20);

#[test]
fn no_delay_both_futures_done_at_once() {
    let (half_close, close) = run_build(DelayingService::default());
    done_ok(&half_close, SOON);
    done_ok(&close, SOON);
}

#[test]
fn delay_before_closing_stream_half_close_done_close_late() {
    let (half_close, close) = run_build(DelayingService {
        delay_before_closing_stream: DELAY,
        ..DelayingService::default()
    });
    done_ok(&half_close, SOON);
    pending(&close, Duration::from_millis(500));
    done_ok(&close, LATE);
}

#[test]
fn delay_before_half_closing_stream_half_close_done_close_late() {
    let (half_close, close) = run_build(DelayingService {
        delay_before_half_closing_stream: DELAY,
        ..DelayingService::default()
    });
    done_ok(&half_close, SOON);
    pending(&close, Duration::from_millis(500));
    done_ok(&close, LATE);
}

#[test]
fn error_message_half_close_done_close_fails() {
    let (half_close, close) = run_build(DelayingService {
        error_message: Some("Some error"),
        ..DelayingService::default()
    });
    match close.get_outcome(Some(LATE)) {
        CloseOutcome::Done(Err(_)) => {}
        other => panic!("expected a failed close, got {other:?}"),
    }
    done_ok(&half_close, SOON);
}
