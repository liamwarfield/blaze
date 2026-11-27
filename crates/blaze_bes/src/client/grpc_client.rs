//! Implementation of [`BuildEventServiceClient`] that uploads data using gRPC.
//!
//! PORT: the calls go through `blaze_remote`'s [`ReferenceCountedChannel`] (tonic), which gives
//! a failed call the status grpc-java gives it (`UNAVAILABLE: io exception`, ...). Its prost
//! encoder writes the fields of a message in declaration order where Java writes them in
//! field-number order (`StreamId` declares `invocation_id = 6` before `component = 3`): the
//! messages are the same, their bytes may differ.

use std::sync::Arc;
use std::time::Duration;

use blaze_protos::google::devtools::build::v1::{
    PublishBuildToolEventStreamRequest, PublishBuildToolEventStreamResponse,
};
use blaze_protos::google::protobuf::Empty;
use blaze_remote::grpc::channel::{CallCredentials, CallOptions, Method, ReferenceCountedChannel};
use blaze_remote::runtime::{block_on_interruptibly, runtime};
use blaze_remote::util::tracing_metadata_utils::build_metadata_from_parts;
use blaze_shell::thread_interrupt;
use parking_lot::Mutex;
use tokio::sync::{Notify, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::{
    AckCallback, BuildEventServiceClient, CommandContext, Interrupted, LifecycleEvent,
    PublishError, Status, StatusFuture, StreamContext, StreamEvent, code_name, proto_util,
};

/// Max wait time for a single non-streaming RPC to finish.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:51 BuildEventServiceGrpcClient.RPC_TIMEOUT
const RPC_TIMEOUT: Duration = Duration::from_secs(15);

/// `PublishBuildEventGrpc.getPublishLifecycleEventMethod()`.
pub const PUBLISH_LIFECYCLE_EVENT: Method =
    Method::new("/google.devtools.build.v1.PublishBuildEvent/PublishLifecycleEvent");
/// `PublishBuildEventGrpc.getPublishBuildToolEventStreamMethod()`.
pub const PUBLISH_BUILD_TOOL_EVENT_STREAM: Method =
    Method::new("/google.devtools.build.v1.PublishBuildEvent/PublishBuildToolEventStream");

/// Implementation of [`BuildEventServiceClient`] that uploads data using gRPC.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:48 BuildEventServiceGrpcClient
pub struct BuildEventServiceGrpcClient {
    /// The channel; the `--bes_header` interceptor is one of its channel-level interceptors
    /// (Java attaches it to the stubs: `configureStub(stub, callCredentials, interceptor)`).
    channel: ReferenceCountedChannel,
    /// `stub.withCallCredentials(callCredentials)`.
    call_credentials: Option<Arc<dyn CallCredentials>>,
}

impl BuildEventServiceGrpcClient {
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:57 BuildEventServiceGrpcClient.<init>
    pub fn new(
        channel: ReferenceCountedChannel,
        call_credentials: Option<Arc<dyn CallCredentials>>,
    ) -> BuildEventServiceGrpcClient {
        BuildEventServiceGrpcClient {
            channel,
            call_credentials,
        }
    }

    /// The decoration of a call: `TracingMetadataUtils.attachMetadataInterceptor(
    /// buildMetadata(buildId, invocationId, actionId, null))` and the call credentials.
    fn call_options(&self, command_context: &CommandContext, action_id: &str) -> CallOptions {
        CallOptions::new()
            .with_request_metadata(build_metadata_from_parts(
                &command_context.build_id,
                &command_context.invocation_id,
                action_id,
                None,
                None,
                None,
            ))
            .with_call_credentials(self.call_credentials.clone())
    }
}

/// `Throwables.throwIfInstanceOf(Throwables.getRootCause(e), InterruptedException.class)`.
fn root_cause_is_interrupted(status: &Status) -> bool {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(status);
    let mut last = None;
    while let Some(e) = cause {
        last = Some(e);
        cause = e.source();
    }
    last.is_some_and(|e| e.is::<thread_interrupt::InterruptedException>())
}

impl BuildEventServiceClient for BuildEventServiceGrpcClient {
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:83 BuildEventServiceGrpcClient.publish
    fn publish(
        &self,
        command_context: &CommandContext,
        lifecycle_event: &LifecycleEvent,
    ) -> Result<(), PublishError> {
        let request = proto_util::publish_lifecycle_event_request(command_context, lifecycle_event);
        // throwIfInterrupted()
        if thread_interrupt::interrupted() {
            return Err(PublishError::Interrupted);
        }
        let options = self
            .call_options(command_context, "publish_lifecycle_event")
            .with_deadline_after(RPC_TIMEOUT);
        let call = self
            .channel
            .unary::<_, Empty>(PUBLISH_LIFECYCLE_EVENT, request, options);
        match block_on_interruptibly(call) {
            None => Err(PublishError::Interrupted),
            Some(Ok(_)) => Ok(()),
            Some(Err(status)) => {
                if root_cause_is_interrupted(&status) {
                    return Err(PublishError::Interrupted);
                }
                Err(PublishError::Status(status))
            }
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:178 BuildEventServiceGrpcClient.openStream
    fn open_stream(
        &self,
        command_context: &CommandContext,
        ack_callback: AckCallback,
    ) -> Result<Box<dyn StreamContext>, Interrupted> {
        Ok(Box::new(BesGrpcStreamContext::new(
            self.channel.clone(),
            self.call_options(command_context, "publish_build_tool_event_stream"),
            command_context.clone(),
            ack_callback,
        )))
    }

    /// `userReadableError(t)` for a `StatusException`: the code's name and the message of the
    /// root cause (the exception itself when it has no cause: `CODE: description`).
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:205 BuildEventServiceGrpcClient.userReadableError
    fn user_readable_error(&self, status: &Status) -> String {
        user_readable_error(status)
    }

    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:217 BuildEventServiceGrpcClient.shutdown
    fn shutdown(&self) {
        self.channel.release();
    }
}

/// `BuildEventServiceGrpcClient.userReadableError` of a `StatusException`.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:205 BuildEventServiceGrpcClient.userReadableError
pub fn user_readable_error(status: &Status) -> String {
    let mut message = code_name(status.code()).to_string();
    message.push_str(": ");
    let mut root: Option<&(dyn std::error::Error + 'static)> = None;
    let mut cause = std::error::Error::source(status);
    while let Some(e) = cause {
        root = Some(e);
        cause = e.source();
    }
    // A connection that could not be established: the transport's IOException is Netty's
    // `AnnotatedConnectException`, whose cause (the root) is the `ConnectException` it annotates:
    // its message is the annotated one without `": " + remoteAddress`.
    // Java: io/netty/channel/AbstractChannel.java AbstractChannel.AnnotatedConnectException.<init>
    if let Some(io_cause) = blaze_remote::grpc::channel::transport_io_cause(status)
        && let Some(java) = io_cause
            .get_ref()
            .and_then(|e| e.downcast_ref::<blaze_remote::common::errors::JavaException>())
        && java.class_name == "io.netty.channel.AbstractChannel$AnnotatedConnectException"
        && java.message.matches(": ").count() >= 2
        && let Some((annotated_cause, _remote_address)) = java.message.rsplit_once(": ")
    {
        message.push_str(annotated_cause);
        return message;
    }
    match root {
        Some(root) => message.push_str(&root.to_string()),
        // The StatusException is its own root cause: `Status.formatThrowableMessage(status)`.
        None => message.push_str(
            &blaze_remote::common::errors::StatusRuntimeException::format_throwable_message(status),
        ),
    }
    message
}

/// A bidirectional `PublishBuildToolEventStream` call.
///
/// PORT: grpc-java's async stub sends the requests from the calling thread and delivers the
/// responses to a `StreamObserver`; here a task on the remote runtime runs the call, the
/// requests go to it through an unbounded queue (grpc-java's `onNext` does not block either),
/// and the task calls the ack callback and completes the status future.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:110 BuildEventServiceGrpcClient.BESGrpcStreamContext
struct BesGrpcStreamContext {
    /// `stream` (its request side); `None` once half-closed.
    requests: Mutex<Option<mpsc::UnboundedSender<PublishBuildToolEventStreamRequest>>>,
    stream_status: StatusFuture,
    command_context: CommandContext,
    /// Cancels the call (`stream.onError(...)`).
    abort: Arc<Notify>,
}

impl BesGrpcStreamContext {
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:115 BESGrpcStreamContext.<init>
    fn new(
        channel: ReferenceCountedChannel,
        options: CallOptions,
        command_context: CommandContext,
        ack_callback: AckCallback,
    ) -> BesGrpcStreamContext {
        let stream_status = StatusFuture::create();
        let (sender, receiver) = mpsc::unbounded_channel();
        let abort = Arc::new(Notify::new());
        let status = stream_status.clone();
        let abort_wait = abort.clone();
        runtime().spawn(async move {
            let call = async {
                let mut responses = channel
                    .bidi_streaming::<_, PublishBuildToolEventStreamResponse, _>(
                        PUBLISH_BUILD_TOOL_EVENT_STREAM,
                        UnboundedReceiverStream::new(receiver),
                        options,
                    )
                    .await?;
                loop {
                    match responses.message().await? {
                        // onNext(response)
                        Some(response) => ack_callback(response.sequence_number),
                        // onCompleted()
                        None => return Ok::<(), Status>(()),
                    }
                }
            };
            tokio::select! {
                biased;
                // abortStream: the status is already set; dropping `call` cancels the RPC.
                () = abort_wait.notified() => {}
                result = call => {
                    status.set(match result {
                        Ok(()) => Status::ok(""),
                        // onError(t): Status.fromThrowable(t).
                        Err(status) => status,
                    });
                }
            }
        });
        BesGrpcStreamContext {
            requests: Mutex::new(Some(sender)),
            stream_status,
            command_context,
            abort,
        }
    }
}

impl StreamContext for BesGrpcStreamContext {
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:155 BESGrpcStreamContext.sendOverStream
    fn send_over_stream(&self, stream_event: StreamEvent) -> Result<(), Interrupted> {
        let request = proto_util::publish_build_tool_event_stream_request(
            &self.command_context,
            &stream_event,
        );
        // throwIfInterrupted()
        if thread_interrupt::interrupted() {
            return Err(Interrupted);
        }
        // stream.onNext(request): after the call ended the message is dropped; its failure is
        // reported by the status future.
        if let Some(requests) = &*self.requests.lock() {
            let _ = requests.send(request);
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:168 BESGrpcStreamContext.halfCloseStream
    fn half_close_stream(&self) {
        // stream.onCompleted(): the end of the request stream half-closes the call.
        self.requests.lock().take();
    }

    /// `stream.onError(status.asException())`: the call is cancelled and the response observer
    /// sees `CANCELLED` caused by `status`, which `onError` unwraps to `status`.
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:173 BESGrpcStreamContext.abortStream
    ///
    /// The call's task drops the call, and dropping the response stream of
    /// `ReferenceCountedChannel::bidi_streaming` (blaze_remote) resets the HTTP/2 stream
    /// (`RST_STREAM CANCEL`) although the request side is still open, so the server's call fails
    /// with `CANCELLED`, as after grpc-java's `onError` (`test_file_upload_fails`).
    fn abort_stream(&self, status: Status) {
        self.stream_status.set(status);
        self.abort.notify_one();
    }

    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceGrpcClient.java:178 BESGrpcStreamContext.getStatus
    fn get_status(&self) -> StatusFuture {
        self.stream_status.clone()
    }
}
