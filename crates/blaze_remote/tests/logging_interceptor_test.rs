#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of
//! `src/test/java/com/google/devtools/build/lib/remote/logging/LoggingInterceptorTest.java`:
//! tests for `LoggingInterceptor`.
//!
//! PORT: Java runs the fake services on gRPC's in-process transport with a mutable service
//! registry; here every test starts a tonic server on `127.0.0.1:0` with the one service it
//! registers and calls it through a [`ReferenceCountedChannel`] that carries the interceptor.
//! The `@Mock AsynchronousMessageOutputStream<LogEntry> logStream` is an
//! [`InMemoryLogEntrySink`]: `verify(logStream).write(expectedEntry)` is "exactly this entry was
//! written".

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::task::Poll;

use async_trait::async_trait;
use blaze_events::ui::Clock;
use blaze_protos::JavaMessage;
use blaze_protos::google::protobuf::{Any, Timestamp};
use blaze_remote::grpc::channel::ResponseStream;
use blaze_remote::grpc::channel::testing::channel_to;
use blaze_remote::grpc::stubs::action_cache_server::{ActionCache, ActionCacheServer};
use blaze_remote::grpc::stubs::byte_stream_server::{ByteStream, ByteStreamServer};
use blaze_remote::grpc::stubs::capabilities_server::{Capabilities, CapabilitiesServer};
use blaze_remote::grpc::stubs::content_addressable_storage_server::{
    ContentAddressableStorage, ContentAddressableStorageServer,
};
use blaze_remote::grpc::stubs::execution_server::{Execution, ExecutionServer};
use blaze_remote::grpc::{
    CallOptions, ClientInterceptor, Method, ReferenceCountedChannel, methods,
};
use blaze_remote::logging::{
    InMemoryLogEntrySink, LoggingHandler, LoggingInterceptor, RawLoggingHandler, raw,
};
use blaze_remote::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse,
};
use blaze_remote::reapi::longrunning::{Operation, operation};
use blaze_remote::reapi::remote_logging::{
    ExecuteDetails, FindMissingBlobsDetails, GetActionResultDetails, GetCapabilitiesDetails,
    LogEntry, QueryWriteStatusDetails, ReadDetails, RpcCallDetails, UpdateActionResultDetails,
    WaitExecutionDetails, WriteDetails, rpc_call_details,
};
use blaze_remote::reapi::{
    ActionResult, BatchReadBlobsRequest, BatchReadBlobsResponse, BatchUpdateBlobsRequest,
    BatchUpdateBlobsResponse, ExecuteRequest, ExecutionCapabilities, FindMissingBlobsRequest,
    FindMissingBlobsResponse, GetActionResultRequest, GetCapabilitiesRequest, GetTreeRequest,
    GetTreeResponse, OutputFile, ServerCapabilities, SpliceBlobRequest, SpliceBlobResponse,
    SplitBlobRequest, SplitBlobResponse, UpdateActionResultRequest, WaitExecutionRequest,
};
use blaze_remote::runtime::{block_on, runtime};
use blaze_remote::util::digest_util::DigestUtil;
use futures::StreamExt;
use parking_lot::Mutex;
use tonic::{Request, Response, Status, Streaming};

// ---------------------------------------------------------------------------------------------
// Test doubles
// ---------------------------------------------------------------------------------------------

/// `com.google.devtools.build.lib.testutil.ManualClock`: a fake clock for testing.
#[derive(Default)]
struct ManualClock {
    current_time_millis: AtomicI64,
}

impl ManualClock {
    fn advance_millis(&self, time: i64) {
        self.current_time_millis.fetch_add(time, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        self.current_time_millis.load(Ordering::SeqCst) * 1_000_000
    }
}

/// What a `Mockito.mock(LoggingHandler.class)` was called with.
struct MockCalls<ReqT, RespT> {
    handle_req: Vec<ReqT>,
    handle_resp: Vec<RespT>,
    get_details: i32,
}

/// `Mockito.mock(LoggingHandler.class)` with `when(handler.getDetails()).thenReturn(details)`.
/// Clones are the same mock.
struct MockLoggingHandler<ReqT, RespT> {
    calls: Arc<Mutex<MockCalls<ReqT, RespT>>>,
    details: RpcCallDetails,
}

impl<ReqT, RespT> MockLoggingHandler<ReqT, RespT> {
    fn new(details: RpcCallDetails) -> Self {
        MockLoggingHandler {
            calls: Arc::new(Mutex::new(MockCalls {
                handle_req: Vec::new(),
                handle_resp: Vec::new(),
                get_details: 0,
            })),
            details,
        }
    }
}

impl<ReqT, RespT> Clone for MockLoggingHandler<ReqT, RespT> {
    fn clone(&self) -> Self {
        MockLoggingHandler {
            calls: self.calls.clone(),
            details: self.details.clone(),
        }
    }
}

impl<ReqT, RespT> LoggingHandler<ReqT, RespT> for MockLoggingHandler<ReqT, RespT>
where
    ReqT: Clone + Send,
    RespT: Clone + Send,
{
    fn handle_req(&mut self, message: &ReqT) {
        self.calls.lock().handle_req.push(message.clone());
    }

    fn handle_resp(&mut self, message: &RespT) {
        self.calls.lock().handle_resp.push(message.clone());
    }

    fn get_details(&mut self) -> RpcCallDetails {
        self.calls.lock().get_details += 1;
        self.details.clone()
    }
}

// ---------------------------------------------------------------------------------------------
// The fake services (Java: the `*ImplBase` classes, whose methods are unimplemented until a test
// overrides them)
// ---------------------------------------------------------------------------------------------

type Unary<ReqT, RespT> = Box<dyn Fn(ReqT) -> Result<RespT, Status> + Send + Sync>;
/// The `onNext` calls of a server-streaming method in order; an `Err` is `onError` and ends the
/// call, the end of the list is `onCompleted`.
type ServerStreaming<ReqT, RespT> = Box<dyn Fn(ReqT) -> Vec<Result<RespT, Status>> + Send + Sync>;
/// A client-streaming method: called with the requests once the client has completed its side
/// (the `onCompleted` of the request observer). `None` never answers.
type ClientStreaming<ReqT, RespT> =
    Box<dyn Fn(Vec<ReqT>) -> Option<Result<RespT, Status>> + Send + Sync>;
type ResponseIter<RespT> = futures::stream::Iter<std::vec::IntoIter<Result<RespT, Status>>>;

/// `io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall`.
fn unimplemented(full_method_name: &str) -> Status {
    Status::unimplemented(format!("Method {full_method_name} is unimplemented"))
}

struct ByteStreamImplBase {
    read: ServerStreaming<ReadRequest, ReadResponse>,
    write: ClientStreaming<WriteRequest, WriteResponse>,
    query_write_status: Unary<QueryWriteStatusRequest, QueryWriteStatusResponse>,
}

impl ByteStreamImplBase {
    fn new() -> ByteStreamImplBase {
        ByteStreamImplBase {
            read: Box::new(|_| vec![Err(unimplemented(methods::READ.full_method_name()))]),
            write: Box::new(|_| Some(Err(unimplemented(methods::WRITE.full_method_name())))),
            query_write_status: Box::new(|_| {
                Err(unimplemented(
                    methods::QUERY_WRITE_STATUS.full_method_name(),
                ))
            }),
        }
    }
}

#[async_trait]
impl ByteStream for ByteStreamImplBase {
    type ReadStream = ResponseIter<ReadResponse>;

    async fn read(
        &self,
        request: Request<ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        Ok(Response::new(futures::stream::iter((self.read)(
            request.into_inner(),
        ))))
    }

    async fn write(
        &self,
        request: Request<Streaming<WriteRequest>>,
    ) -> Result<Response<WriteResponse>, Status> {
        let mut requests = request.into_inner();
        let mut write_requests = Vec::new();
        while let Some(write_request) = requests.message().await? {
            write_requests.push(write_request);
        }
        match (self.write)(write_requests) {
            Some(result) => result.map(Response::new),
            None => std::future::pending().await,
        }
    }

    async fn query_write_status(
        &self,
        request: Request<QueryWriteStatusRequest>,
    ) -> Result<Response<QueryWriteStatusResponse>, Status> {
        (self.query_write_status)(request.into_inner()).map(Response::new)
    }
}

struct ExecutionImplBase {
    execute: ServerStreaming<ExecuteRequest, Operation>,
    wait_execution: ServerStreaming<WaitExecutionRequest, Operation>,
}

impl ExecutionImplBase {
    fn new() -> ExecutionImplBase {
        ExecutionImplBase {
            execute: Box::new(|_| vec![Err(unimplemented(methods::EXECUTE.full_method_name()))]),
            wait_execution: Box::new(|_| {
                vec![Err(unimplemented(
                    methods::WAIT_EXECUTION.full_method_name(),
                ))]
            }),
        }
    }
}

#[async_trait]
impl Execution for ExecutionImplBase {
    type ExecuteStream = ResponseIter<Operation>;

    async fn execute(
        &self,
        request: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        Ok(Response::new(futures::stream::iter((self.execute)(
            request.into_inner(),
        ))))
    }

    type WaitExecutionStream = ResponseIter<Operation>;

    async fn wait_execution(
        &self,
        request: Request<WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        Ok(Response::new(futures::stream::iter((self.wait_execution)(
            request.into_inner(),
        ))))
    }
}

struct ActionCacheImplBase {
    get_action_result: Unary<GetActionResultRequest, ActionResult>,
    update_action_result: Unary<UpdateActionResultRequest, ActionResult>,
}

impl ActionCacheImplBase {
    fn new() -> ActionCacheImplBase {
        ActionCacheImplBase {
            get_action_result: Box::new(|_| {
                Err(unimplemented(methods::GET_ACTION_RESULT.full_method_name()))
            }),
            update_action_result: Box::new(|_| {
                Err(unimplemented(
                    methods::UPDATE_ACTION_RESULT.full_method_name(),
                ))
            }),
        }
    }
}

#[async_trait]
impl ActionCache for ActionCacheImplBase {
    async fn get_action_result(
        &self,
        request: Request<GetActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        (self.get_action_result)(request.into_inner()).map(Response::new)
    }

    async fn update_action_result(
        &self,
        request: Request<UpdateActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        (self.update_action_result)(request.into_inner()).map(Response::new)
    }
}

struct ContentAddressableStorageImplBase {
    find_missing_blobs: Unary<FindMissingBlobsRequest, FindMissingBlobsResponse>,
}

#[async_trait]
impl ContentAddressableStorage for ContentAddressableStorageImplBase {
    async fn find_missing_blobs(
        &self,
        request: Request<FindMissingBlobsRequest>,
    ) -> Result<Response<FindMissingBlobsResponse>, Status> {
        (self.find_missing_blobs)(request.into_inner()).map(Response::new)
    }

    async fn batch_update_blobs(
        &self,
        _request: Request<BatchUpdateBlobsRequest>,
    ) -> Result<Response<BatchUpdateBlobsResponse>, Status> {
        Err(unimplemented(
            methods::BATCH_UPDATE_BLOBS.full_method_name(),
        ))
    }

    async fn batch_read_blobs(
        &self,
        _request: Request<BatchReadBlobsRequest>,
    ) -> Result<Response<BatchReadBlobsResponse>, Status> {
        Err(unimplemented(methods::BATCH_READ_BLOBS.full_method_name()))
    }

    type GetTreeStream = ResponseIter<GetTreeResponse>;

    async fn get_tree(
        &self,
        _request: Request<GetTreeRequest>,
    ) -> Result<Response<Self::GetTreeStream>, Status> {
        Err(unimplemented(methods::GET_TREE.full_method_name()))
    }

    async fn split_blob(
        &self,
        _request: Request<SplitBlobRequest>,
    ) -> Result<Response<SplitBlobResponse>, Status> {
        Err(unimplemented(methods::SPLIT_BLOB.full_method_name()))
    }

    async fn splice_blob(
        &self,
        _request: Request<SpliceBlobRequest>,
    ) -> Result<Response<SpliceBlobResponse>, Status> {
        Err(unimplemented(methods::SPLICE_BLOB.full_method_name()))
    }
}

struct CapabilitiesImplBase {
    get_capabilities: Unary<GetCapabilitiesRequest, ServerCapabilities>,
}

#[async_trait]
impl Capabilities for CapabilitiesImplBase {
    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<ServerCapabilities>, Status> {
        (self.get_capabilities)(request.into_inner()).map(Response::new)
    }
}

/// The fake server (`InProcessServerBuilder...build().start()`); dropping it shuts it down.
struct Server {
    addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

/// `serviceRegistry.addService(service)` on a server of its own.
fn start_server<S>(service: S) -> Server
where
    S: tonic::codegen::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<tonic::body::Body>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    let listener = block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).expect("bind");
    let addr = listener.local_addr().expect("local address");
    let (shutdown, signal) = tokio::sync::oneshot::channel::<()>();
    runtime().spawn(async move {
        let _ = tonic::transport::Server::builder()
            .serve_with_incoming_shutdown(
                service,
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async move {
                    let _ = signal.await;
                },
            )
            .await;
    });
    Server {
        addr,
        _shutdown: shutdown,
    }
}

// ---------------------------------------------------------------------------------------------
// The test class
// ---------------------------------------------------------------------------------------------

/// The fields of the Java test class, as `setUp()` leaves them.
struct LoggingInterceptorTest {
    interceptor: Arc<LoggingInterceptor>,
    clock: Arc<ManualClock>,

    log_stream: Arc<InMemoryLogEntrySink>,

    /// The fake servers of this test.
    servers: Mutex<Vec<Server>>,
}

impl LoggingInterceptorTest {
    /// `setUp()`.
    fn set_up() -> LoggingInterceptorTest {
        let clock = Arc::new(ManualClock::default());
        let log_stream = Arc::new(InMemoryLogEntrySink::new());
        let interceptor = Arc::new(LoggingInterceptor::new(log_stream.clone(), clock.clone()));
        LoggingInterceptorTest {
            interceptor,
            clock,
            log_stream,
            servers: Mutex::new(Vec::new()),
        }
    }

    // This returns a logging interceptor where all calls are handled by the given handler.
    fn get_interceptor_with_always_this_handler<ReqT, RespT>(
        &self,
        handler: &MockLoggingHandler<ReqT, RespT>,
        output_file: Arc<InMemoryLogEntrySink>,
    ) -> Arc<LoggingInterceptor>
    where
        ReqT: Clone + Send + 'static,
        RespT: Clone + Send + 'static,
    {
        let handler = handler.clone();
        Arc::new(LoggingInterceptor::with_select_handler(
            output_file,
            self.clock.clone(),
            Arc::new(
                move |_method: &Method| -> Option<Box<dyn RawLoggingHandler>> {
                    Some(raw(handler.clone()))
                },
            ),
        ))
    }

    /// `serviceRegistry.addService(service)` and
    /// `ClientInterceptors.intercept(InProcessChannelBuilder.forName(fakeServerName).directExecutor().build(), interceptor)`.
    fn channel<S>(
        &self,
        service: S,
        interceptor: Arc<LoggingInterceptor>,
    ) -> ReferenceCountedChannel
    where
        S: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        let server = start_server(service);
        let addr = server.addr;
        self.servers.lock().push(server);
        let interceptors: Vec<Arc<dyn ClientInterceptor>> = vec![interceptor];
        channel_to(addr, interceptors, None)
    }

    /// `serviceRegistry.addService(service)`, then `loggedChannel`.
    fn logged_channel<S>(&self, service: S) -> ReferenceCountedChannel
    where
        S: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = std::convert::Infallible,
            > + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        self.channel(service, self.interceptor.clone())
    }

    /// `verify(logStream).write(expectedEntry)`.
    fn verify_log_stream_write(&self, expected_entry: LogEntry) {
        assert_eq!(self.log_stream.entries(), vec![expected_entry]);
    }
}

/// Reads a server-streaming call to its end: the messages (`replies.next()`) and how the
/// iterator ended (`Err` is the `StatusRuntimeException` of `replies.hasNext()`).
async fn read_all<RespT: Send + Sync + 'static>(
    call: Result<ResponseStream<RespT>, Status>,
) -> (Vec<RespT>, Result<(), Status>) {
    let mut messages = Vec::new();
    let mut replies = match call {
        Ok(replies) => replies,
        Err(status) => return (messages, Err(status)),
    };
    loop {
        match replies.message().await {
            Ok(Some(message)) => messages.push(message),
            Ok(None) => return (messages, Ok(())),
            Err(status) => return (messages, Err(status)),
        }
    }
}

/// A request of a client-streaming call, or a clock step between two of them.
enum Step<ReqT> {
    /// `requester.onNext(request)`.
    OnNext(ReqT),
    /// `clock.advanceMillis(millis)`.
    AdvanceMillis(i64),
}

/// The request stream of a client-streaming call: the steps in order, then
/// `requester.onCompleted()`.
fn requester<ReqT: Send + 'static>(
    clock: &Arc<ManualClock>,
    steps: Vec<Step<ReqT>>,
) -> impl futures::Stream<Item = ReqT> + Send + 'static + use<ReqT> {
    let clock = clock.clone();
    let mut steps = steps.into_iter();
    futures::stream::iter(std::iter::from_fn(move || {
        loop {
            match steps.next()? {
                Step::OnNext(request) => return Some(request),
                Step::AdvanceMillis(millis) => clock.advance_millis(millis),
            }
        }
    }))
}

/// `Any.pack(message)`.
fn any_pack<M: JavaMessage>(message: &M) -> Any {
    Any {
        type_url: format!("type.googleapis.com/{}", M::FULL_NAME),
        value: message.to_byte_array(),
    }
}

/// `com.google.rpc.Status.getDefaultInstance()`.
fn status_default_instance() -> blaze_remote::reapi::rpc::Status {
    blaze_remote::reapi::rpc::Status::default()
}

/// `com.google.rpc.Status.newBuilder().setCode(error.getCode().value()).setMessage(error.getDescription())`.
fn status_proto(error: &Status) -> blaze_remote::reapi::rpc::Status {
    blaze_remote::reapi::rpc::Status {
        code: error.code() as i32,
        message: error.message().to_string(),
        ..Default::default()
    }
}

fn timestamp(seconds: i64, nanos: i32) -> Timestamp {
    Timestamp { seconds, nanos }
}

fn details(details: rpc_call_details::Details) -> RpcCallDetails {
    RpcCallDetails {
        details: Some(details),
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = ReadRequest {
        resource_name: "test".to_string(),
        ..Default::default()
    };
    let response = ReadResponse {
        data: b"abc".to_vec(),
    };

    let clock = t.clock.clone();
    let response_for_service = response.clone();
    let service = ByteStreamServer::new(ByteStreamImplBase {
        read: Box::new(move |_request| {
            clock.advance_millis(1234);
            vec![Ok(response_for_service.clone())]
        }),
        ..ByteStreamImplBase::new()
    });

    let details = RpcCallDetails::default();
    let handler: MockLoggingHandler<ReadRequest, ReadResponse> =
        MockLoggingHandler::new(details.clone());

    let interceptor = t.get_interceptor_with_always_this_handler(&handler, t.log_stream.clone());
    let channel = t.channel(service, interceptor);

    let expected_entry = LogEntry {
        method_name: methods::READ.full_method_name().to_string(),
        details: Some(details),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(12, 300000000)),
        end_time: Some(timestamp(13, 534000000)),
        ..Default::default()
    };

    t.clock.advance_millis(12300);
    // stub.read(request).next(), which also asks for what follows the first response.
    let (replies, end) = block_on(async {
        read_all(
            channel
                .server_streaming::<ReadRequest, ReadResponse>(
                    methods::READ,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    assert_eq!(replies, vec![response.clone()]);
    end.unwrap();
    let calls = handler.calls.lock();
    assert_eq!(calls.handle_req, vec![request]);
    assert_eq!(calls.handle_resp, vec![response]);
    assert_eq!(calls.get_details, 1);
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_call_ok_multiple_responses() {
    let t = LoggingInterceptorTest::set_up();
    let request = ReadRequest {
        resource_name: "test".to_string(),
        ..Default::default()
    };
    let response1 = ReadResponse {
        data: b"abc".to_vec(),
    };
    let response2 = ReadResponse {
        data: b"def".to_vec(),
    };
    let clock = t.clock.clone();
    let (response1_for_service, response2_for_service) = (response1.clone(), response2.clone());
    let service = ByteStreamServer::new(ByteStreamImplBase {
        read: Box::new(move |_request| {
            clock.advance_millis(50);
            let response1 = Ok(response1_for_service.clone());
            clock.advance_millis(1500);
            let response2 = Ok(response2_for_service.clone());
            vec![response1, response2]
        }),
        ..ByteStreamImplBase::new()
    });

    let details = RpcCallDetails::default();
    let handler: MockLoggingHandler<ReadRequest, ReadResponse> =
        MockLoggingHandler::new(details.clone());

    let interceptor = t.get_interceptor_with_always_this_handler(&handler, t.log_stream.clone());
    let channel = t.channel(service, interceptor);

    // Read both responses.
    let (_replies, end) = block_on(async {
        read_all(
            channel
                .server_streaming::<ReadRequest, ReadResponse>(
                    methods::READ,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    end.unwrap();

    let expected_entry = LogEntry {
        method_name: methods::READ.full_method_name().to_string(),
        details: Some(details),
        status: Some(status_default_instance()),
        start_time: Some(Timestamp::default()),
        end_time: Some(timestamp(1, 550000000)),
        ..Default::default()
    };

    let calls = handler.calls.lock();
    assert_eq!(calls.handle_req, vec![request]);
    assert_eq!(calls.handle_resp.len(), 2);
    assert_eq!(calls.handle_resp[0], response1);
    assert_eq!(calls.handle_resp[1], response2);
    assert_eq!(calls.get_details, 1);
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_call_ok_multiple_requests() {
    let t = LoggingInterceptorTest::set_up();
    let request1 = WriteRequest {
        resource_name: "test".to_string(),
        data: b"abc".to_vec(),
        ..Default::default()
    };
    let request2 = WriteRequest {
        resource_name: "test".to_string(),
        data: b"def".to_vec(),
        ..Default::default()
    };
    let response = WriteResponse { committed_size: 6 };
    let service = ByteStreamServer::new(ByteStreamImplBase {
        write: Box::new(move |_write_requests| Some(Ok(response))),
        ..ByteStreamImplBase::new()
    });

    let details = RpcCallDetails::default();
    let handler: MockLoggingHandler<WriteRequest, WriteResponse> =
        MockLoggingHandler::new(details.clone());

    let interceptor = t.get_interceptor_with_always_this_handler(&handler, t.log_stream.clone());
    let channel = t.channel(service, interceptor);

    t.clock.advance_millis(1000);
    // Write both responses.
    let requester = requester(
        &t.clock,
        vec![
            Step::OnNext(request1.clone()),
            Step::OnNext(request2.clone()),
            Step::AdvanceMillis(1000),
        ],
    );
    block_on(channel.client_streaming::<WriteRequest, WriteResponse, _>(
        methods::WRITE,
        requester,
        CallOptions::new(),
    ))
    .unwrap();

    let expected_entry = LogEntry {
        method_name: methods::WRITE.full_method_name().to_string(),
        details: Some(details),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(1, 0)),
        end_time: Some(timestamp(2, 0)),
        ..Default::default()
    };

    let calls = handler.calls.lock();
    assert_eq!(calls.handle_req.len(), 2);
    assert_eq!(calls.handle_req[0], request1);
    assert_eq!(calls.handle_req[1], request2);
    assert_eq!(calls.handle_resp, vec![response]);
    assert_eq!(calls.get_details, 1);
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_call_with_error() {
    let t = LoggingInterceptorTest::set_up();
    let request = ReadRequest {
        resource_name: "test".to_string(),
        ..Default::default()
    };
    let error = Status::not_found("not found");

    let clock = t.clock.clone();
    let error_for_service = error.clone();
    let service = ByteStreamServer::new(ByteStreamImplBase {
        read: Box::new(move |_request| {
            clock.advance_millis(100);
            vec![Err(error_for_service.clone())]
        }),
        ..ByteStreamImplBase::new()
    });

    let details = RpcCallDetails::default();
    let handler: MockLoggingHandler<ReadRequest, ReadResponse> =
        MockLoggingHandler::new(details.clone());

    let interceptor = t.get_interceptor_with_always_this_handler(&handler, t.log_stream.clone());
    let channel = t.channel(service, interceptor);

    t.clock.advance_millis(1500);
    // assertThrows(StatusRuntimeException.class, () -> stub.read(request).next())
    let (replies, end) = block_on(async {
        read_all(
            channel
                .server_streaming::<ReadRequest, ReadResponse>(
                    methods::READ,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    assert!(replies.is_empty());
    end.unwrap_err();

    let expected_entry = LogEntry {
        method_name: methods::READ.full_method_name().to_string(),
        details: Some(details),
        status: Some(status_proto(&error)),
        start_time: Some(timestamp(1, 500000000)),
        end_time: Some(timestamp(1, 600000000)),
        ..Default::default()
    };

    let calls = handler.calls.lock();
    assert_eq!(calls.handle_req, vec![request]);
    assert!(calls.handle_resp.is_empty());
    assert_eq!(calls.get_details, 1);
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_execute_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = ExecuteRequest {
        instance_name: "test-instance".to_string(),
        action_digest: Some(DigestUtil::build_digest("test", 8)),
        ..Default::default()
    };
    let response1 = Operation {
        name: "test-name".to_string(),
        ..Default::default()
    };
    let response2 = Operation {
        name: "test-name".to_string(),
        done: true,
        result: Some(operation::Result::Response(any_pack(&request))),
        ..Default::default()
    };

    let clock = t.clock.clone();
    let (response1_for_service, response2_for_service) = (response1.clone(), response2.clone());
    let logged_channel = t.logged_channel(ExecutionServer::new(ExecutionImplBase {
        execute: Box::new(move |_request| {
            let response1 = Ok(response1_for_service.clone());
            clock.advance_millis(2200);
            let response2 = Ok(response2_for_service.clone());
            clock.advance_millis(1100);
            vec![response1, response2]
        }),
        ..ExecutionImplBase::new()
    }));

    t.clock.advance_millis(50000);
    // Read both responses.
    let (_replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<ExecuteRequest, Operation>(
                    methods::EXECUTE,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    end.unwrap();

    let expected_entry = LogEntry {
        method_name: methods::EXECUTE.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Execute(
            ExecuteDetails {
                request: Some(request),
                responses: vec![response1, response2],
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(50, 0)),
        end_time: Some(timestamp(53, 300000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_execute_call_fail() {
    let t = LoggingInterceptorTest::set_up();
    let request = ExecuteRequest {
        instance_name: "test-instance".to_string(),
        action_digest: Some(DigestUtil::build_digest("test", 8)),
        ..Default::default()
    };
    let error = Status::not_found("not found");
    let clock = t.clock.clone();
    let error_for_service = error.clone();
    let logged_channel = t.logged_channel(ExecutionServer::new(ExecutionImplBase {
        execute: Box::new(move |_request| {
            clock.advance_millis(1100);
            vec![Err(error_for_service.clone())]
        }),
        ..ExecutionImplBase::new()
    }));
    t.clock.advance_millis(20000000000001);
    let (_replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<ExecuteRequest, Operation>(
                    methods::EXECUTE,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    // assertThrows(StatusRuntimeException.class, () -> replies.hasNext())
    end.unwrap_err();
    let expected_entry = LogEntry {
        method_name: methods::EXECUTE.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Execute(
            ExecuteDetails {
                request: Some(request),
                ..Default::default()
            },
        ))),
        status: Some(status_proto(&error)),
        start_time: Some(timestamp(20000000000, 1000000)),
        end_time: Some(timestamp(20000000001, 101000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_find_missing_blobs_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let test_digest = DigestUtil::build_digest("test", 8);
    let request = FindMissingBlobsRequest {
        blob_digests: vec![test_digest.clone()],
        instance_name: "test-instance".to_string(),
        ..Default::default()
    };
    let response = FindMissingBlobsResponse {
        missing_blob_digests: vec![test_digest],
    };
    let clock = t.clock.clone();
    let response_for_service = response.clone();
    let logged_channel = t.logged_channel(ContentAddressableStorageServer::new(
        ContentAddressableStorageImplBase {
            find_missing_blobs: Box::new(move |_request| {
                clock.advance_millis(200);
                Ok(response_for_service.clone())
            }),
        },
    ));

    t.clock.advance_millis(14900);
    block_on(
        logged_channel.unary::<FindMissingBlobsRequest, FindMissingBlobsResponse>(
            methods::FIND_MISSING_BLOBS,
            request.clone(),
            CallOptions::new(),
        ),
    )
    .unwrap();
    let expected_entry = LogEntry {
        method_name: methods::FIND_MISSING_BLOBS.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::FindMissingBlobs(
            FindMissingBlobsDetails {
                request: Some(request),
                response: Some(response),
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(14, 900000000)),
        end_time: Some(timestamp(15, 100000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_get_action_result_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let test_digest = DigestUtil::build_digest("test", 8);
    let request = GetActionResultRequest {
        action_digest: Some(test_digest.clone()),
        instance_name: "test-instance".to_string(),
        ..Default::default()
    };
    let response = ActionResult {
        output_files: vec![OutputFile {
            digest: Some(test_digest),
            path: "root/test".to_string(),
            ..Default::default()
        }],
        exit_code: 1,
        ..Default::default()
    };

    let clock = t.clock.clone();
    let response_for_service = response.clone();
    let logged_channel = t.logged_channel(ActionCacheServer::new(ActionCacheImplBase {
        get_action_result: Box::new(move |_request| {
            clock.advance_millis(22222);
            Ok(response_for_service.clone())
        }),
        ..ActionCacheImplBase::new()
    }));

    t.clock.advance_millis(11111);
    block_on(
        logged_channel.unary::<GetActionResultRequest, ActionResult>(
            methods::GET_ACTION_RESULT,
            request.clone(),
            CallOptions::new(),
        ),
    )
    .unwrap();
    let expected_entry = LogEntry {
        method_name: methods::GET_ACTION_RESULT.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::GetActionResult(
            GetActionResultDetails {
                request: Some(request),
                response: Some(response),
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(11, 111000000)),
        end_time: Some(timestamp(33, 333000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_update_action_result_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let test_digest = DigestUtil::build_digest("test", 8);
    let action_result = ActionResult {
        output_files: vec![OutputFile {
            digest: Some(test_digest.clone()),
            path: "root/test".to_string(),
            ..Default::default()
        }],
        exit_code: 1,
        ..Default::default()
    };

    let request = UpdateActionResultRequest {
        action_digest: Some(test_digest),
        instance_name: "test-instance".to_string(),
        action_result: Some(action_result.clone()),
        ..Default::default()
    };

    let clock = t.clock.clone();
    let action_result_for_service = action_result.clone();
    let logged_channel = t.logged_channel(ActionCacheServer::new(ActionCacheImplBase {
        update_action_result: Box::new(move |_request| {
            clock.advance_millis(22222);
            Ok(action_result_for_service.clone())
        }),
        ..ActionCacheImplBase::new()
    }));

    t.clock.advance_millis(11111);
    block_on(
        logged_channel.unary::<UpdateActionResultRequest, ActionResult>(
            methods::UPDATE_ACTION_RESULT,
            request.clone(),
            CallOptions::new(),
        ),
    )
    .unwrap();
    let expected_entry = LogEntry {
        method_name: methods::UPDATE_ACTION_RESULT.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::UpdateActionResult(
            UpdateActionResultDetails {
                request: Some(request),
                response: Some(action_result),
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(11, 111000000)),
        end_time: Some(timestamp(33, 333000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_get_capabilities_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = GetCapabilitiesRequest {
        instance_name: "test-instance".to_string(),
    };
    let response = ServerCapabilities {
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let clock = t.clock.clone();
    let response_for_service = response.clone();
    let logged_channel = t.logged_channel(CapabilitiesServer::new(CapabilitiesImplBase {
        get_capabilities: Box::new(move |_request| {
            clock.advance_millis(22222);
            Ok(response_for_service.clone())
        }),
    }));

    t.clock.advance_millis(11111);
    block_on(
        logged_channel.unary::<GetCapabilitiesRequest, ServerCapabilities>(
            methods::GET_CAPABILITIES,
            request.clone(),
            CallOptions::new(),
        ),
    )
    .unwrap();
    let expected_entry = LogEntry {
        method_name: methods::GET_CAPABILITIES.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::GetCapabilities(
            GetCapabilitiesDetails {
                request: Some(request),
                response: Some(response),
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(11, 111000000)),
        end_time: Some(timestamp(33, 333000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_wait_execution_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = WaitExecutionRequest {
        name: "test-name".to_string(),
    };
    let response1 = Operation {
        name: "test-name".to_string(),
        ..Default::default()
    };
    let response2 = Operation {
        name: "test-name".to_string(),
        done: true,
        result: Some(operation::Result::Response(any_pack(&request))),
        ..Default::default()
    };

    let clock = t.clock.clone();
    let (response1_for_service, response2_for_service) = (response1.clone(), response2.clone());
    let logged_channel = t.logged_channel(ExecutionServer::new(ExecutionImplBase {
        wait_execution: Box::new(move |_request| {
            let response1 = Ok(response1_for_service.clone());
            clock.advance_millis(2200);
            let response2 = Ok(response2_for_service.clone());
            clock.advance_millis(1100);
            vec![response1, response2]
        }),
        ..ExecutionImplBase::new()
    }));

    t.clock.advance_millis(50000);
    // Read both responses.
    let (_replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<WaitExecutionRequest, Operation>(
                    methods::WAIT_EXECUTION,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    end.unwrap();

    let expected_entry = LogEntry {
        method_name: methods::WAIT_EXECUTION.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::WaitExecution(
            WaitExecutionDetails {
                request: Some(request),
                responses: vec![response1, response2],
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(50, 0)),
        end_time: Some(timestamp(53, 300000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_wait_execution_call_fail() {
    let t = LoggingInterceptorTest::set_up();
    let request = WaitExecutionRequest {
        name: "test-name".to_string(),
    };
    let response = Operation {
        name: "test-name".to_string(),
        ..Default::default()
    };
    let error = Status::deadline_exceeded("timed out");

    let clock = t.clock.clone();
    let (response_for_service, error_for_service) = (response.clone(), error.clone());
    let logged_channel = t.logged_channel(ExecutionServer::new(ExecutionImplBase {
        wait_execution: Box::new(move |_request| {
            clock.advance_millis(100);
            let response = Ok(response_for_service.clone());
            clock.advance_millis(100);
            vec![response, Err(error_for_service.clone())]
        }),
        ..ExecutionImplBase::new()
    }));

    t.clock.advance_millis(2000);
    let (replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<WaitExecutionRequest, Operation>(
                    methods::WAIT_EXECUTION,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    // assertThat(replies.hasNext()).isTrue(); assertThat(replies.next()).isEqualTo(response);
    assert_eq!(replies, vec![response.clone()]);
    // assertThrows(StatusRuntimeException.class, () -> replies.hasNext())
    end.unwrap_err();

    let expected_entry = LogEntry {
        method_name: methods::WAIT_EXECUTION.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::WaitExecution(
            WaitExecutionDetails {
                request: Some(request),
                responses: vec![response],
            },
        ))),
        status: Some(status_proto(&error)),
        start_time: Some(timestamp(2, 0)),
        end_time: Some(timestamp(2, 200000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_read_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = ReadRequest {
        resource_name: "test-resource".to_string(),
        ..Default::default()
    };
    let response1 = ReadResponse {
        data: b"abc".to_vec(),
    };
    let response2 = ReadResponse {
        data: b"def".to_vec(),
    };

    let clock = t.clock.clone();
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        read: Box::new(move |_request| {
            let responses = vec![Ok(response1.clone()), Ok(response2.clone())];
            clock.advance_millis(2000);
            responses
        }),
        ..ByteStreamImplBase::new()
    }));

    t.clock.advance_millis(500000);
    // Read both responses.
    let (_replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<ReadRequest, ReadResponse>(
                    methods::READ,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    end.unwrap();

    let expected_entry = LogEntry {
        method_name: methods::READ.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Read(ReadDetails {
            request: Some(request),
            num_reads: 2,
            bytes_read: 6,
        }))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(500, 0)),
        end_time: Some(timestamp(502, 0)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_read_call_fail() {
    let t = LoggingInterceptorTest::set_up();
    let request = ReadRequest {
        resource_name: "test-resource".to_string(),
        ..Default::default()
    };
    let response1 = ReadResponse {
        data: b"abc".to_vec(),
    };
    let error = Status::deadline_exceeded("timeout");

    let clock = t.clock.clone();
    let (response1_for_service, error_for_service) = (response1.clone(), error.clone());
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        read: Box::new(move |_request| {
            let response1 = Ok(response1_for_service.clone());
            clock.advance_millis(100);
            vec![response1, Err(error_for_service.clone())]
        }),
        ..ByteStreamImplBase::new()
    }));
    let (replies, end) = block_on(async {
        read_all(
            logged_channel
                .server_streaming::<ReadRequest, ReadResponse>(
                    methods::READ,
                    request.clone(),
                    CallOptions::new(),
                )
                .await,
        )
        .await
    });
    // assertThat(replies.hasNext()).isTrue(); assertThat(replies.next()).isEqualTo(response1);
    assert_eq!(replies, vec![response1]);
    // assertThrows(StatusRuntimeException.class, () -> replies.hasNext())
    end.unwrap_err();

    let expected_entry = LogEntry {
        method_name: methods::READ.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Read(ReadDetails {
            request: Some(request),
            num_reads: 1,
            bytes_read: 3,
        }))),
        status: Some(status_proto(&error)),
        start_time: Some(Timestamp::default()),
        end_time: Some(timestamp(0, 100000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_write_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request1 = WriteRequest {
        resource_name: "test1".to_string(),
        data: b"abc".to_vec(),
        ..Default::default()
    };
    let request2 = WriteRequest {
        resource_name: "test2".to_string(),
        data: b"def".to_vec(),
        ..Default::default()
    };
    let response = WriteResponse { committed_size: 6 };
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        write: Box::new(move |_write_requests| Some(Ok(response))),
        ..ByteStreamImplBase::new()
    }));

    t.clock.advance_millis(10000);
    // Request three writes, the first identical with the third.
    let requester = requester(
        &t.clock,
        vec![
            Step::OnNext(request1.clone()),
            Step::AdvanceMillis(100),
            Step::OnNext(request2.clone()),
            Step::AdvanceMillis(200),
            Step::OnNext(request1.clone()),
            Step::AdvanceMillis(100),
        ],
    );
    block_on(
        logged_channel.client_streaming::<WriteRequest, WriteResponse, _>(
            methods::WRITE,
            requester,
            CallOptions::new(),
        ),
    )
    .unwrap();

    let expected_entry = LogEntry {
        method_name: methods::WRITE.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Write(WriteDetails {
            resource_names: vec!["test1".to_string(), "test2".to_string()],
            offsets: vec![0, 0, 0],
            // finish write is empty
            finish_writes: vec![],
            response: Some(response),
            bytes_sent: 9,
            num_writes: 3,
        }))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(10, 0)),
        end_time: Some(timestamp(10, 400000000)),
        ..Default::default()
    };

    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_write_call_offset_and_finish_write_compounding() {
    let t = LoggingInterceptorTest::set_up();
    let request1 = WriteRequest {
        resource_name: "test1".to_string(),
        data: b"abc".to_vec(),
        write_offset: 10,
        ..Default::default()
    };
    let request2 = WriteRequest {
        data: b"def".to_vec(),
        write_offset: request1.write_offset + request1.data.len() as i64,
        ..Default::default()
    };
    let response = WriteResponse { committed_size: 6 };
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        write: Box::new(move |_write_requests| Some(Ok(response))),
        ..ByteStreamImplBase::new()
    }));

    t.clock.advance_millis(10000);
    // Request three writes, the first identical with the third, but offset correctly and
    // finish_writing
    let requester = requester(
        &t.clock,
        vec![
            Step::OnNext(request1.clone()),
            Step::AdvanceMillis(100),
            Step::OnNext(request2.clone()),
            Step::AdvanceMillis(200),
            Step::OnNext(WriteRequest {
                write_offset: request2.write_offset + request2.data.len() as i64,
                finish_write: true,
                ..request1.clone()
            }),
            Step::AdvanceMillis(100),
        ],
    );
    block_on(
        logged_channel.client_streaming::<WriteRequest, WriteResponse, _>(
            methods::WRITE,
            requester,
            CallOptions::new(),
        ),
    )
    .unwrap();

    let expected_entry = LogEntry {
        method_name: methods::WRITE.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::Write(WriteDetails {
            resource_names: vec!["test1".to_string(), "".to_string()],
            offsets: vec![request1.write_offset],
            finish_writes: vec![10 + request1.data.len() as i64 * 2 + request2.data.len() as i64],
            response: Some(response),
            bytes_sent: 9,
            num_writes: 3,
        }))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(10, 0)),
        end_time: Some(timestamp(10, 400000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
#[ignore = "needs a channel API to cancel a client-streaming call with a cause (Java: StreamObserver.onError on the request stream closes the call with CANCELLED and that cause); a call dropped by the client closes with the message \"Cancelled by client\""]
fn test_write_call_fail() {
    let t = LoggingInterceptorTest::set_up();
    let request = WriteRequest {
        resource_name: "test".to_string(),
        data: b"abc".to_vec(),
        ..Default::default()
    };
    let error = Status::deadline_exceeded("timeout");
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        // return Mockito.mock(StreamObserver.class): the service never answers.
        write: Box::new(|_write_requests| None),
        ..ByteStreamImplBase::new()
    }));
    t.clock.advance_millis(10000000000);

    // Write both responses.
    let (written, cancel) = tokio::sync::oneshot::channel::<()>();
    let mut written = Some(written);
    let clock = t.clock.clone();
    let requester =
        futures::stream::iter(vec![request.clone()]).chain(futures::stream::poll_fn(move |_cx| {
            if let Some(written) = written.take() {
                clock.advance_millis(10000000000);
                let _ = written.send(());
            }
            Poll::<Option<WriteRequest>>::Pending
        }));
    block_on(async {
        let call = logged_channel.client_streaming::<WriteRequest, WriteResponse, _>(
            methods::WRITE,
            requester,
            CallOptions::new(),
        );
        tokio::pin!(call);
        tokio::select! {
            _ = &mut call => panic!("the call must not end by itself"),
            _ = cancel => {}
        }
        // requester.onError(error.asRuntimeException()): the call is dropped here.
    });

    // Status expectedCancel = Status.CANCELLED.withCause(error.asRuntimeException());
    // expectedCancel.getCause().toString()
    let expected_cancel_cause = format!(
        "io.grpc.StatusRuntimeException: DEADLINE_EXCEEDED: {}",
        error.message()
    );
    let expected_entry = LogEntry {
        method_name: methods::WRITE.full_method_name().to_string(),
        status: Some(blaze_remote::reapi::rpc::Status {
            code: tonic::Code::Cancelled as i32,
            message: expected_cancel_cause,
            ..Default::default()
        }),
        details: Some(details(rpc_call_details::Details::Write(WriteDetails {
            resource_names: vec!["test".to_string()],
            offsets: vec![0],
            num_writes: 1,
            bytes_sent: 3,
            ..Default::default()
        }))),
        start_time: Some(timestamp(10000000, 0)),
        end_time: Some(timestamp(20000000, 0)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}

#[test]
fn test_query_write_status_call_ok() {
    let t = LoggingInterceptorTest::set_up();
    let request = QueryWriteStatusRequest {
        resource_name: "test".to_string(),
    };
    let response = QueryWriteStatusResponse {
        committed_size: 10,
        ..Default::default()
    };
    let clock = t.clock.clone();
    let logged_channel = t.logged_channel(ByteStreamServer::new(ByteStreamImplBase {
        query_write_status: Box::new(move |_request| {
            clock.advance_millis(22222);
            Ok(response)
        }),
        ..ByteStreamImplBase::new()
    }));

    t.clock.advance_millis(11111);
    block_on(
        logged_channel.unary::<QueryWriteStatusRequest, QueryWriteStatusResponse>(
            methods::QUERY_WRITE_STATUS,
            request.clone(),
            CallOptions::new(),
        ),
    )
    .unwrap();

    let expected_entry = LogEntry {
        method_name: methods::QUERY_WRITE_STATUS.full_method_name().to_string(),
        details: Some(details(rpc_call_details::Details::QueryWriteStatus(
            QueryWriteStatusDetails {
                request: Some(request),
                response: Some(response),
            },
        ))),
        status: Some(status_default_instance()),
        start_time: Some(timestamp(11, 111000000)),
        end_time: Some(timestamp(33, 333000000)),
        ..Default::default()
    };
    t.verify_log_stream_write(expected_entry);
}
