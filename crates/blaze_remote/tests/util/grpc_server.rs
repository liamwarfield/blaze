//! In-process fake gRPC servers for the tests of the gRPC clients.
//!
//! Java: `InProcessServerBuilder.forName(name).fallbackHandlerRegistry(serviceRegistry)` with a
//! `MutableHandlerRegistry`, and the `*ImplBase` classes the tests subclass anonymously.
//!
//! PORT: there is no in-process transport; a fake server listens on `127.0.0.1:0` and the
//! clients talk to it over a plaintext channel ([`blaze_remote::grpc::channel::testing`]).
//! [`FakeServer`] binds its address when it is made, so a client can be created before the
//! services are registered (as the Java tests do), but the services of a tonic server are fixed
//! once it serves: register them, then call [`FakeServer::start`] before the first call.
//!
//! The `Fake*` services stand for Java's anonymous `*ImplBase` subclasses: every method is an
//! optional handler, and a method without a handler answers `UNIMPLEMENTED` as the generated
//! Java base class does. A handler that panics (a failed assertion) fails its call.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc::stubs::action_cache_server::{ActionCache, ActionCacheServer};
use blaze_remote::grpc::stubs::byte_stream_server::{ByteStream, ByteStreamServer};
use blaze_remote::grpc::stubs::capabilities_server::{Capabilities, CapabilitiesServer};
use blaze_remote::grpc::stubs::content_addressable_storage_server::{
    ContentAddressableStorage, ContentAddressableStorageServer,
};
use blaze_remote::grpc::stubs::execution_server::{Execution, ExecutionServer};
use blaze_remote::grpc::{ClientInterceptor, ReferenceCountedChannel};
use blaze_remote::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse,
};
use blaze_remote::reapi::{
    ActionResult, BatchReadBlobsRequest, BatchReadBlobsResponse, BatchUpdateBlobsRequest,
    BatchUpdateBlobsResponse, FindMissingBlobsRequest, FindMissingBlobsResponse,
    GetActionResultRequest, GetCapabilitiesRequest, GetTreeRequest, GetTreeResponse,
    ServerCapabilities, SpliceBlobRequest, SpliceBlobResponse, SplitBlobRequest,
    SplitBlobResponse, UpdateActionResultRequest,
};
use futures::Stream;
use futures::future::BoxFuture;
use tonic::service::{Routes, RoutesBuilder};
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status, Streaming};

/// Stops a fake server (`Server.shutdownNow()`); dropping it does the same.
pub struct ShutdownHandle {
    signal: Option<tokio::sync::oneshot::Sender<()>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl ShutdownHandle {
    /// `fakeServer.shutdownNow()`: stops accepting connections. Calls in flight end when their
    /// clients go away.
    pub fn shutdown_now(&mut self) {
        if let Some(signal) = self.signal.take() {
            let _ = signal.send(());
        }
        if let Some(server) = self.server.take() {
            server.abort();
        }
    }
}

impl Drop for ShutdownHandle {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

/// Serves `routes` on `listener` on the remote runtime.
fn serve(listener: std::net::TcpListener, routes: Routes) -> (SocketAddr, ShutdownHandle) {
    let addr = listener
        .local_addr()
        .expect("fake server: no local address");
    listener
        .set_nonblocking(true)
        .expect("fake server: cannot make the listener non-blocking");
    let (signal, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = blaze_remote::runtime::runtime().spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .expect("fake server: cannot register the listener");
        tonic::transport::Server::builder()
            .add_routes(routes)
            .serve_with_incoming_shutdown(TcpIncoming::from(listener), async move {
                let _ = stopped.await;
            })
            .await
            .expect("fake server failed");
    });
    (
        addr,
        ShutdownHandle {
            signal: Some(signal),
            server: Some(server),
        },
    )
}

/// Starts an in-process server for `routes` on `127.0.0.1:0` and returns its address and the
/// handle that stops it.
pub fn start_server(routes: Routes) -> (SocketAddr, ShutdownHandle) {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("fake server: cannot bind 127.0.0.1:0");
    serve(listener, routes)
}

/// A plaintext channel to the fake server at `addr` (Java: `InProcessChannelBuilder`).
pub fn channel_to(
    addr: SocketAddr,
    interceptors: Vec<Arc<dyn ClientInterceptor>>,
    capabilities_fetcher: Option<CapabilitiesFetcher>,
) -> ReferenceCountedChannel {
    blaze_remote::grpc::channel::testing::channel_to(addr, interceptors, capabilities_fetcher)
}

/// A fake server with a service registry (`fakeServer` + `serviceRegistry` of the Java tests).
pub struct FakeServer {
    addr: SocketAddr,
    listener: Option<std::net::TcpListener>,
    service_registry: Option<RoutesBuilder>,
    shutdown: Option<ShutdownHandle>,
}

impl FakeServer {
    /// Binds `127.0.0.1:0`. The server answers once [`FakeServer::start`] was called.
    pub fn new() -> FakeServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("fake server: cannot bind 127.0.0.1:0");
        let addr = listener
            .local_addr()
            .expect("fake server: no local address");
        FakeServer {
            addr,
            listener: Some(listener),
            service_registry: Some(Routes::builder()),
            shutdown: None,
        }
    }

    /// The address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn service_registry(&mut self) -> &mut RoutesBuilder {
        self.service_registry
            .as_mut()
            .expect("fake server: services must be added before start()")
    }

    /// `serviceRegistry.addService(new ByteStreamImplBase() {...})`.
    pub fn add_byte_stream<T: ByteStream>(&mut self, service: T) {
        self.service_registry().add_service(
            ByteStreamServer::new(service)
                .max_decoding_message_size(usize::MAX)
                .max_encoding_message_size(usize::MAX),
        );
    }

    /// `serviceRegistry.addService(new ContentAddressableStorageImplBase() {...})`.
    pub fn add_content_addressable_storage<T: ContentAddressableStorage>(&mut self, service: T) {
        self.service_registry().add_service(
            ContentAddressableStorageServer::new(service)
                .max_decoding_message_size(usize::MAX)
                .max_encoding_message_size(usize::MAX),
        );
    }

    /// `serviceRegistry.addService(new ActionCacheImplBase() {...})`.
    pub fn add_action_cache<T: ActionCache>(&mut self, service: T) {
        self.service_registry().add_service(
            ActionCacheServer::new(service)
                .max_decoding_message_size(usize::MAX)
                .max_encoding_message_size(usize::MAX),
        );
    }

    /// `serviceRegistry.addService(new CapabilitiesImplBase() {...})`.
    pub fn add_capabilities<T: Capabilities>(&mut self, service: T) {
        self.service_registry().add_service(
            CapabilitiesServer::new(service)
                .max_decoding_message_size(usize::MAX)
                .max_encoding_message_size(usize::MAX),
        );
    }

    /// `serviceRegistry.addService(new ExecutionImplBase() {...})`.
    pub fn add_execution<T: Execution>(&mut self, service: T) {
        self.service_registry().add_service(
            ExecutionServer::new(service)
                .max_decoding_message_size(usize::MAX)
                .max_encoding_message_size(usize::MAX),
        );
    }

    /// Starts serving the registered services. A service that is not registered answers
    /// `UNIMPLEMENTED`, as a `MutableHandlerRegistry` without it does.
    pub fn start(&mut self) {
        let listener = self
            .listener
            .take()
            .expect("fake server: start() called twice");
        let routes = self
            .service_registry
            .take()
            .expect("fake server: start() called twice")
            .routes();
        let (_, shutdown) = serve(listener, routes);
        self.shutdown = Some(shutdown);
    }

    /// A channel to this server with the given channel-level interceptors (in execution order)
    /// and capabilities.
    pub fn channel(
        &self,
        interceptors: Vec<Arc<dyn ClientInterceptor>>,
        capabilities_fetcher: Option<CapabilitiesFetcher>,
    ) -> ReferenceCountedChannel {
        channel_to(self.addr, interceptors, capabilities_fetcher)
    }

    /// `fakeServer.shutdownNow()`.
    pub fn shutdown_now(&mut self) {
        if let Some(mut shutdown) = self.shutdown.take() {
            shutdown.shutdown_now();
        }
        self.listener = None;
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}

/// `Status.UNIMPLEMENTED.withDescription("Method <name> is unimplemented")`, the answer of a
/// method the Java `*ImplBase` subclass does not override.
// Java: io/grpc/stub/ServerCalls.java ServerCalls.asyncUnimplementedUnaryCall
fn unimplemented(method: &str) -> Status {
    Status::unimplemented(format!("Method {method} is unimplemented"))
}

/// The handler of a unary method: gets the request with its headers, returns the response
/// (`onNext` + `onCompleted`) or the status of `onError`.
pub type UnaryHandler<Req, Resp> =
    Box<dyn Fn(Request<Req>) -> Result<Resp, Status> + Send + Sync + 'static>;

/// The responses of a server-streaming method: each `Ok` item is an `onNext`, the end of the
/// stream is `onCompleted`, an `Err` item is `onError` (and ends the stream).
pub type ResponseStream<Resp> = Pin<Box<dyn Stream<Item = Result<Resp, Status>> + Send + 'static>>;

/// `onNext(item)` for each `Ok` item in order, then `onCompleted()`; an `Err` item is
/// `onError(status)`.
pub fn response_stream<Resp: Send + 'static>(
    items: Vec<Result<Resp, Status>>,
) -> ResponseStream<Resp> {
    Box::pin(futures::stream::iter(items))
}

/// A stream that sends `items` and then stays open: the server never completes the call.
/// `cancelled` is set when the call goes away (Java:
/// `ServerCallStreamObserver.setOnCancelHandler`).
pub fn hanging_response_stream<Resp: Send + 'static>(
    items: Vec<Result<Resp, Status>>,
    cancelled: Arc<AtomicBool>,
) -> ResponseStream<Resp> {
    Box::pin(HangingStream {
        items: items.into(),
        cancelled,
    })
}

struct HangingStream<Resp> {
    items: std::collections::VecDeque<Result<Resp, Status>>,
    cancelled: Arc<AtomicBool>,
}

impl<Resp> Unpin for HangingStream<Resp> {}

impl<Resp> Stream for HangingStream<Resp> {
    type Item = Result<Resp, Status>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.items.pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            // Never woken: the call ends only when the client cancels it.
            None => Poll::Pending,
        }
    }
}

impl<Resp> Drop for HangingStream<Resp> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// `ReadResponse.newBuilder().setData(data).build()`.
pub fn read_response(data: impl Into<Vec<u8>>) -> ReadResponse {
    ReadResponse { data: data.into() }
}

/// The handler of `ByteStream.Read`.
pub type ReadHandler =
    Box<dyn Fn(Request<ReadRequest>) -> ResponseStream<ReadResponse> + Send + Sync + 'static>;

/// The handler of `ByteStream.Write`: gets the stream of requests (`StreamObserver<WriteRequest>`
/// as a pull stream), resolves to the response or to the status of `onError`.
pub type WriteHandler = Box<
    dyn Fn(Request<Streaming<WriteRequest>>) -> BoxFuture<'static, Result<WriteResponse, Status>>
        + Send
        + Sync
        + 'static,
>;

/// `new ByteStreamImplBase() {...}`.
#[derive(Default)]
pub struct FakeByteStream {
    pub read: Option<ReadHandler>,
    pub write: Option<WriteHandler>,
    pub query_write_status: Option<UnaryHandler<QueryWriteStatusRequest, QueryWriteStatusResponse>>,
}

#[tonic::async_trait]
impl ByteStream for FakeByteStream {
    type ReadStream = ResponseStream<ReadResponse>;

    async fn read(
        &self,
        request: Request<ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        match &self.read {
            Some(handler) => Ok(Response::new(handler(request))),
            None => Err(unimplemented("google.bytestream.ByteStream/Read")),
        }
    }

    async fn write(
        &self,
        request: Request<Streaming<WriteRequest>>,
    ) -> Result<Response<WriteResponse>, Status> {
        match &self.write {
            Some(handler) => handler(request).await.map(Response::new),
            None => Err(unimplemented("google.bytestream.ByteStream/Write")),
        }
    }

    async fn query_write_status(
        &self,
        request: Request<QueryWriteStatusRequest>,
    ) -> Result<Response<QueryWriteStatusResponse>, Status> {
        match &self.query_write_status {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(
                "google.bytestream.ByteStream/QueryWriteStatus",
            )),
        }
    }
}

/// `new ContentAddressableStorageImplBase() {...}`.
#[derive(Default)]
pub struct FakeContentAddressableStorage {
    pub find_missing_blobs: Option<UnaryHandler<FindMissingBlobsRequest, FindMissingBlobsResponse>>,
    pub batch_update_blobs: Option<UnaryHandler<BatchUpdateBlobsRequest, BatchUpdateBlobsResponse>>,
    pub batch_read_blobs: Option<UnaryHandler<BatchReadBlobsRequest, BatchReadBlobsResponse>>,
    pub get_tree: Option<
        Box<dyn Fn(Request<GetTreeRequest>) -> ResponseStream<GetTreeResponse> + Send + Sync>,
    >,
    pub split_blob: Option<UnaryHandler<SplitBlobRequest, SplitBlobResponse>>,
    pub splice_blob: Option<UnaryHandler<SpliceBlobRequest, SpliceBlobResponse>>,
}

const CAS: &str = "build.bazel.remote.execution.v2.ContentAddressableStorage";

#[tonic::async_trait]
impl ContentAddressableStorage for FakeContentAddressableStorage {
    async fn find_missing_blobs(
        &self,
        request: Request<FindMissingBlobsRequest>,
    ) -> Result<Response<FindMissingBlobsResponse>, Status> {
        match &self.find_missing_blobs {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{CAS}/FindMissingBlobs"))),
        }
    }

    async fn batch_update_blobs(
        &self,
        request: Request<BatchUpdateBlobsRequest>,
    ) -> Result<Response<BatchUpdateBlobsResponse>, Status> {
        match &self.batch_update_blobs {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{CAS}/BatchUpdateBlobs"))),
        }
    }

    async fn batch_read_blobs(
        &self,
        request: Request<BatchReadBlobsRequest>,
    ) -> Result<Response<BatchReadBlobsResponse>, Status> {
        match &self.batch_read_blobs {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{CAS}/BatchReadBlobs"))),
        }
    }

    type GetTreeStream = ResponseStream<GetTreeResponse>;

    async fn get_tree(
        &self,
        request: Request<GetTreeRequest>,
    ) -> Result<Response<Self::GetTreeStream>, Status> {
        match &self.get_tree {
            Some(handler) => Ok(Response::new(handler(request))),
            None => Err(unimplemented(&format!("{CAS}/GetTree"))),
        }
    }

    async fn split_blob(
        &self,
        request: Request<SplitBlobRequest>,
    ) -> Result<Response<SplitBlobResponse>, Status> {
        match &self.split_blob {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{CAS}/SplitBlob"))),
        }
    }

    async fn splice_blob(
        &self,
        request: Request<SpliceBlobRequest>,
    ) -> Result<Response<SpliceBlobResponse>, Status> {
        match &self.splice_blob {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{CAS}/SpliceBlob"))),
        }
    }
}

/// `new ActionCacheImplBase() {...}`.
#[derive(Default)]
pub struct FakeActionCache {
    pub get_action_result: Option<UnaryHandler<GetActionResultRequest, ActionResult>>,
    pub update_action_result: Option<UnaryHandler<UpdateActionResultRequest, ActionResult>>,
}

const ACTION_CACHE: &str = "build.bazel.remote.execution.v2.ActionCache";

#[tonic::async_trait]
impl ActionCache for FakeActionCache {
    async fn get_action_result(
        &self,
        request: Request<GetActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        match &self.get_action_result {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!("{ACTION_CACHE}/GetActionResult"))),
        }
    }

    async fn update_action_result(
        &self,
        request: Request<UpdateActionResultRequest>,
    ) -> Result<Response<ActionResult>, Status> {
        match &self.update_action_result {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(&format!(
                "{ACTION_CACHE}/UpdateActionResult"
            ))),
        }
    }
}

/// `new CapabilitiesImplBase() {...}`.
#[derive(Default)]
pub struct FakeCapabilities {
    pub get_capabilities: Option<UnaryHandler<GetCapabilitiesRequest, ServerCapabilities>>,
}

#[tonic::async_trait]
impl Capabilities for FakeCapabilities {
    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<ServerCapabilities>, Status> {
        match &self.get_capabilities {
            Some(handler) => handler(request).map(Response::new),
            None => Err(unimplemented(
                "build.bazel.remote.execution.v2.Capabilities/GetCapabilities",
            )),
        }
    }
}
