//! The gRPC channel every remote call goes through.
//!
//! Java: `lib/remote/ReferenceCountedChannel.java` (a pooled Netty channel wrapped in
//! `ClientInterceptors.intercept(channel, interceptors)`), the per-call stub decoration every
//! client applies (`TracingMetadataUtils.attachMetadataInterceptor`, `NetworkTimeInterceptor`,
//! `withCallCredentials`, `withDeadlineAfter`), and the `io.grpc.ClientInterceptor` /
//! `ClientCall.Listener` contract the channel-level interceptors (`LoggingInterceptor`, the
//! header interceptors) are written against.
//!
//! PORT: tonic has no generic client interceptor that sees typed messages, so the three call
//! shapes the remote APIs use (`unary`, `server_streaming`, `client_streaming`) are methods of
//! [`ReferenceCountedChannel`], and each of them runs the same sequence Java's interceptor chain
//! produces: call-level headers first (request metadata, call credentials, deadline), then the
//! channel-level [`ClientInterceptor`]s in order, then the wire.
//!
//! PORT: Java pools up to `--remote_max_connections` Netty connections
//! (`DynamicConnectionPool`); here one HTTP/2 connection multiplexes all calls and reconnects
//! by itself.

use std::any::Any;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use blaze_protos::JavaMessage;
use futures::{Stream, StreamExt};
use http::uri::PathAndQuery;
use parking_lot::Mutex;
use tonic::client::Grpc;
use tonic::metadata::{
    AsciiMetadataKey, AsciiMetadataValue, BinaryMetadataKey, BinaryMetadataValue, MetadataMap,
};
use tonic::transport::Channel;
use tonic_prost::ProstCodec;

use crate::common::NetworkTime;
use crate::common::errors::status_to_io_error;
use crate::reapi::{RequestMetadata, ServerCapabilities};
use crate::util::tracing_metadata_utils::METADATA_KEY;

/// A gRPC method (`MethodDescriptor.getFullMethodName()` plus the path on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Method {
    /// `/package.Service/Method`.
    pub path: &'static str,
}

impl Method {
    /// A method by its wire path (`/package.Service/Method`).
    pub const fn new(path: &'static str) -> Method {
        Method { path }
    }

    /// `MethodDescriptor.getFullMethodName()`: `package.Service/Method`.
    pub fn full_method_name(&self) -> &'static str {
        &self.path[1..]
    }

    /// `MethodDescriptor.getServiceName()`: `package.Service`.
    pub fn service_name(&self) -> &'static str {
        let full = self.full_method_name();
        &full[..full.rfind('/').unwrap_or(full.len())]
    }
}

/// The methods of the Remote Execution API and ByteStream (`*Grpc.get*Method()`).
pub mod methods {
    use super::Method;

    /// `ExecutionGrpc.getExecuteMethod()`.
    pub const EXECUTE: Method = Method::new("/build.bazel.remote.execution.v2.Execution/Execute");
    /// `ExecutionGrpc.getWaitExecutionMethod()`.
    pub const WAIT_EXECUTION: Method =
        Method::new("/build.bazel.remote.execution.v2.Execution/WaitExecution");
    /// `ActionCacheGrpc.getGetActionResultMethod()`.
    pub const GET_ACTION_RESULT: Method =
        Method::new("/build.bazel.remote.execution.v2.ActionCache/GetActionResult");
    /// `ActionCacheGrpc.getUpdateActionResultMethod()`.
    pub const UPDATE_ACTION_RESULT: Method =
        Method::new("/build.bazel.remote.execution.v2.ActionCache/UpdateActionResult");
    /// `ContentAddressableStorageGrpc.getFindMissingBlobsMethod()`.
    pub const FIND_MISSING_BLOBS: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/FindMissingBlobs");
    /// `ContentAddressableStorageGrpc.getBatchUpdateBlobsMethod()`.
    pub const BATCH_UPDATE_BLOBS: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/BatchUpdateBlobs");
    /// `ContentAddressableStorageGrpc.getBatchReadBlobsMethod()`.
    pub const BATCH_READ_BLOBS: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/BatchReadBlobs");
    /// `ContentAddressableStorageGrpc.getGetTreeMethod()`.
    pub const GET_TREE: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/GetTree");
    /// `ContentAddressableStorageGrpc.getSplitBlobMethod()`.
    pub const SPLIT_BLOB: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/SplitBlob");
    /// `ContentAddressableStorageGrpc.getSpliceBlobMethod()`.
    pub const SPLICE_BLOB: Method =
        Method::new("/build.bazel.remote.execution.v2.ContentAddressableStorage/SpliceBlob");
    /// `CapabilitiesGrpc.getGetCapabilitiesMethod()`.
    pub const GET_CAPABILITIES: Method =
        Method::new("/build.bazel.remote.execution.v2.Capabilities/GetCapabilities");
    /// `ByteStreamGrpc.getReadMethod()`.
    pub const READ: Method = Method::new("/google.bytestream.ByteStream/Read");
    /// `ByteStreamGrpc.getWriteMethod()`.
    pub const WRITE: Method = Method::new("/google.bytestream.ByteStream/Write");
    /// `ByteStreamGrpc.getQueryWriteStatusMethod()`.
    pub const QUERY_WRITE_STATUS: Method =
        Method::new("/google.bytestream.ByteStream/QueryWriteStatus");
}

/// What a channel-level interceptor observes of one call (`ClientCall` + `ClientCall.Listener`
/// as seen by a `ForwardingClientCall`). Messages are passed as `&dyn Any`: downcast to the
/// request/response type of the method.
pub trait CallListener: Send {
    /// `ClientCall.sendMessage(message)`.
    fn on_request(&mut self, _message: &(dyn Any + Send)) {}
    /// `Listener.onMessage(message)`.
    fn on_response(&mut self, _message: &(dyn Any + Send)) {}
    /// `Listener.onClose(status, trailers)`; `status` is OK for a successful call.
    fn on_close(&mut self, _status: &tonic::Status) {}
}

/// A channel-level client interceptor (`io.grpc.ClientInterceptor.interceptCall`).
pub trait ClientInterceptor: Send + Sync {
    /// Called when a call starts (`ClientCall.start(listener, headers)`), after the call-level
    /// headers were added. May add headers; may return a listener for the rest of the call.
    fn intercept_call(
        &self,
        method: &Method,
        headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>>;
}

/// `MetadataUtils.newAttachHeadersInterceptor(metadata)` for ASCII headers.
// Java: com/google/devtools/build/lib/remote/util/TracingMetadataUtils.java:131 TracingMetadataUtils.newMetadataForHeaders
pub struct AttachHeadersInterceptor {
    headers: Vec<(AsciiMetadataKey, AsciiMetadataValue)>,
}

impl AttachHeadersInterceptor {
    /// Builds the interceptor from `name=value` pairs in order (`Metadata.put` appends, so a
    /// repeated name gives several values).
    ///
    /// Fails like `Metadata.Key.of(name, ASCII_STRING_MARSHALLER)` on an invalid name.
    pub fn new(headers: &[(String, String)]) -> io::Result<AttachHeadersInterceptor> {
        let mut out = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            let key = AsciiMetadataKey::from_bytes(name.to_ascii_lowercase().as_bytes()).map_err(
                |_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Invalid character in header name: {name}"),
                    )
                },
            )?;
            let value = AsciiMetadataValue::try_from(value.as_str()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Invalid character in the value of header {name}"),
                )
            })?;
            out.push((key, value));
        }
        Ok(AttachHeadersInterceptor { headers: out })
    }
}

impl ClientInterceptor for AttachHeadersInterceptor {
    fn intercept_call(
        &self,
        _method: &Method,
        headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>> {
        for (key, value) in &self.headers {
            headers.append(key.clone(), value.clone());
        }
        None
    }
}

/// `io.grpc.CallCredentials`: per-call request metadata (authorization headers).
#[async_trait]
pub trait CallCredentials: Send + Sync {
    /// `Credentials.getRequestMetadata(uri)`: the headers for a call to the service at `uri`
    /// (`https://<authority>/<package.Service>`, as `GoogleAuthLibraryCallCredentials.serviceUri`
    /// builds it).
    async fn get_request_metadata(&self, uri: &str) -> io::Result<Vec<(String, Vec<String>)>>;
}

/// The per-call decoration Java applies to a stub.
#[derive(Clone, Default)]
pub struct CallOptions {
    /// `TracingMetadataUtils.attachMetadataInterceptor(requestMetadata)`.
    pub request_metadata: Option<RequestMetadata>,
    /// `new NetworkTimeInterceptor(context::getNetworkTime)`.
    pub network_time: Option<Arc<NetworkTime>>,
    /// `withCallCredentials(callCredentialsProvider.getCallCredentials())`.
    pub call_credentials: Option<Arc<dyn CallCredentials>>,
    /// `withDeadlineAfter(timeout)`.
    pub deadline: Option<Duration>,
}

impl CallOptions {
    /// No decoration.
    pub fn new() -> CallOptions {
        CallOptions::default()
    }

    /// Adds the request metadata header.
    pub fn with_request_metadata(mut self, request_metadata: RequestMetadata) -> CallOptions {
        self.request_metadata = Some(request_metadata);
        self
    }

    /// Measures the call in `network_time`.
    pub fn with_network_time(mut self, network_time: Arc<NetworkTime>) -> CallOptions {
        self.network_time = Some(network_time);
        self
    }

    /// Adds call credentials (`None` is Java's `withCallCredentials(null)`).
    pub fn with_call_credentials(
        mut self,
        call_credentials: Option<Arc<dyn CallCredentials>>,
    ) -> CallOptions {
        self.call_credentials = call_credentials;
        self
    }

    /// `withDeadlineAfter(timeout)`; a zero timeout means no deadline, as the callers treat
    /// `--remote_timeout=0`.
    pub fn with_deadline_after(mut self, timeout: Duration) -> CallOptions {
        self.deadline = Some(timeout);
        self
    }
}

type CapabilitiesFuture = Pin<Box<dyn Future<Output = io::Result<ServerCapabilities>> + Send>>;
/// Fetches the server capabilities over a channel
/// (`ChannelConnectionWithServerCapabilitiesFactory`).
pub type CapabilitiesFetcher =
    Arc<dyn Fn(ReferenceCountedChannel) -> CapabilitiesFuture + Send + Sync>;

struct Inner {
    channel: Channel,
    authority: String,
    /// Whether the transport is TLS (`SecurityLevel.PRIVACY_AND_INTEGRITY`).
    tls: bool,
    interceptors: Vec<Arc<dyn ClientInterceptor>>,
    max_inbound_message_size: usize,
    capabilities_fetcher: Option<CapabilitiesFetcher>,
    server_capabilities: tokio::sync::OnceCell<Result<ServerCapabilities, Arc<io::Error>>>,
    ref_cnt: AtomicI32,
    shutdown: AtomicBool,
}

/// A wrapper around a gRPC channel adding reference counting, the channel-level interceptors
/// and the server capabilities of the connection.
// Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:40 ReferenceCountedChannel
#[derive(Clone)]
pub struct ReferenceCountedChannel {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ReferenceCountedChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReferenceCountedChannel")
            .field("authority", &self.inner.authority)
            .field("tls", &self.inner.tls)
            .finish_non_exhaustive()
    }
}

impl ReferenceCountedChannel {
    /// Wraps a transport channel.
    ///
    /// * `authority`: `Channel.authority()` (`host:port`, or the `--tls_authority_override`).
    /// * `tls`: whether the transport is encrypted.
    /// * `interceptors`: the channel-level interceptors, outermost first (Java:
    ///   `ClientInterceptors.intercept(channel, interceptors)` runs the *last* one first; pass
    ///   them here in execution order).
    /// * `max_inbound_message_size`: `--experimental_remote_... maxInboundMessageSize` (Netty's
    ///   default is 4 MiB); `capabilities_fetcher` is queried once, lazily.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:59 ReferenceCountedChannel.<init>
    pub fn new(
        channel: Channel,
        authority: &str,
        tls: bool,
        interceptors: Vec<Arc<dyn ClientInterceptor>>,
        max_inbound_message_size: usize,
        capabilities_fetcher: Option<CapabilitiesFetcher>,
    ) -> ReferenceCountedChannel {
        ReferenceCountedChannel {
            inner: Arc::new(Inner {
                channel,
                authority: authority.to_string(),
                tls,
                interceptors,
                max_inbound_message_size,
                capabilities_fetcher,
                server_capabilities: tokio::sync::OnceCell::new(),
                ref_cnt: AtomicI32::new(1),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// `Channel.authority()`.
    pub fn authority(&self) -> &str {
        &self.inner.authority
    }

    /// Whether the transport is TLS.
    pub fn is_tls(&self) -> bool {
        self.inner.tls
    }

    /// Returns the server capabilities of the connection, fetching them on first use.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:70 ReferenceCountedChannel.getServerCapabilities
    pub async fn get_server_capabilities_async(&self) -> io::Result<ServerCapabilities> {
        let Some(fetcher) = self.inner.capabilities_fetcher.clone() else {
            return Err(io::Error::other(
                "IllegalStateException: channel was created without a capabilities factory",
            ));
        };
        let this = self.clone();
        let result = self
            .inner
            .server_capabilities
            .get_or_init(|| async move { fetcher(this).await.map_err(Arc::new) })
            .await;
        match result {
            Ok(c) => Ok(c.clone()),
            Err(e) => Err(crate::common::errors::shared(e)),
        }
    }

    /// Blocking form of [`ReferenceCountedChannel::get_server_capabilities_async`].
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:70 ReferenceCountedChannel.getServerCapabilities
    pub fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        if let Some(result) = self.inner.server_capabilities.get() {
            return match result {
                Ok(c) => Ok(c.clone()),
                Err(e) => Err(crate::common::errors::shared(e)),
            };
        }
        crate::runtime::get_from_future(self.get_server_capabilities_async())
    }

    /// `isShutdown()`.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:80 ReferenceCountedChannel.isShutdown
    pub fn is_shutdown(&self) -> bool {
        self.inner.shutdown.load(Ordering::SeqCst)
    }

    /// `refCnt()`.
    pub fn ref_cnt(&self) -> i32 {
        self.inner.ref_cnt.load(Ordering::SeqCst)
    }

    /// `retain()`.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:174 ReferenceCountedChannel.retain
    pub fn retain(&self) -> ReferenceCountedChannel {
        self.inner.ref_cnt.fetch_add(1, Ordering::SeqCst);
        self.clone()
    }

    /// `release()`: closes the channel when the count reaches zero. Returns whether it did.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:201 ReferenceCountedChannel.release
    pub fn release(&self) -> bool {
        let remaining = self.inner.ref_cnt.fetch_sub(1, Ordering::SeqCst) - 1;
        blaze_util::check_state!(remaining >= 0, "IllegalReferenceCountException");
        if remaining == 0 {
            // PORT: the tonic channel closes its connection when the last clone is dropped.
            self.inner.shutdown.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Builds the headers of a call and runs the channel-level interceptors.
    async fn start_call(
        &self,
        method: &Method,
        options: &CallOptions,
    ) -> Result<(MetadataMap, Listeners), tonic::Status> {
        if self.is_shutdown() {
            return Err(tonic::Status::unavailable("Channel shutdown invoked"));
        }
        let mut headers = MetadataMap::new();
        // Java: TracingMetadataUtils.attachMetadataInterceptor
        if let Some(request_metadata) = &options.request_metadata {
            headers.insert_bin(
                BinaryMetadataKey::from_static(METADATA_KEY),
                BinaryMetadataValue::from_bytes(&request_metadata.to_byte_array()),
            );
        }
        // Java: io.grpc.auth.GoogleAuthLibraryCallCredentials.applyRequestMetadata
        if let Some(credentials) = &options.call_credentials {
            let uri = service_uri(&self.inner.authority, method);
            let metadata = credentials.get_request_metadata(&uri).await.map_err(|e| {
                tonic::Status::unauthenticated(format!("Failed computing credential metadata: {e}"))
            })?;
            for (name, values) in metadata {
                let lower = name.to_ascii_lowercase();
                for value in values {
                    if lower.ends_with("-bin") {
                        let key =
                            BinaryMetadataKey::from_bytes(lower.as_bytes()).map_err(|_| {
                                tonic::Status::unauthenticated(format!(
                                    "Invalid credential metadata name: {name}"
                                ))
                            })?;
                        use base64::Engine as _;
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(value.as_bytes())
                            .map_err(|_| {
                                tonic::Status::unauthenticated(format!(
                                    "Invalid base64 in credential metadata {name}"
                                ))
                            })?;
                        headers.append_bin(key, BinaryMetadataValue::from_bytes(&bytes));
                    } else {
                        let key = AsciiMetadataKey::from_bytes(lower.as_bytes()).map_err(|_| {
                            tonic::Status::unauthenticated(format!(
                                "Invalid credential metadata name: {name}"
                            ))
                        })?;
                        let value = AsciiMetadataValue::try_from(value.as_str()).map_err(|_| {
                            tonic::Status::unauthenticated(format!(
                                "Invalid credential metadata value for {name}"
                            ))
                        })?;
                        headers.append(key, value);
                    }
                }
            }
        }
        let mut listeners: Vec<Box<dyn CallListener>> = Vec::new();
        for interceptor in &self.inner.interceptors {
            if let Some(listener) = interceptor.intercept_call(method, &mut headers) {
                listeners.push(listener);
            }
        }
        // Java: NetworkTimeInterceptor starts the stopwatch when the call starts.
        if let Some(network_time) = &options.network_time {
            network_time.start();
        }
        Ok((
            headers,
            Listeners {
                listeners,
                network_time: options.network_time.clone(),
                closed: false,
            },
        ))
    }

    fn grpc(&self) -> Grpc<Channel> {
        Grpc::new(self.inner.channel.clone())
            .max_decoding_message_size(self.inner.max_inbound_message_size)
            .max_encoding_message_size(usize::MAX)
    }

    fn request<T>(message: T, headers: MetadataMap, options: &CallOptions) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        *request.metadata_mut() = headers;
        if let Some(deadline) = effective_deadline(options) {
            // The `grpc-timeout` header, as Java sends it for a call with a deadline.
            request.set_timeout(deadline);
        }
        request
    }

    /// A unary call (`ClientCalls.futureUnaryCall`).
    pub async fn unary<Req, Resp>(
        &self,
        method: Method,
        request: Req,
        options: CallOptions,
    ) -> Result<Resp, tonic::Status>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        let (headers, mut listeners) = self.start_call(&method, &options).await?;
        listeners.on_request(&request);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.unary(
                Self::request(request, headers, &options),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(effective_deadline(&options), call).await;
        match result {
            Ok(response) => {
                let message = response.into_inner();
                listeners.on_response(&message);
                listeners.on_close(&tonic::Status::ok(""));
                Ok(message)
            }
            Err(status) => {
                listeners.on_close(&status);
                Err(status)
            }
        }
    }

    /// A server-streaming call (`ClientCalls.asyncServerStreamingCall`).
    ///
    /// The deadline covers the whole stream, as a gRPC deadline does.
    pub async fn server_streaming<Req, Resp>(
        &self,
        method: Method,
        request: Req,
        options: CallOptions,
    ) -> Result<ResponseStream<Resp>, tonic::Status>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
    {
        let (headers, mut listeners) = self.start_call(&method, &options).await?;
        listeners.on_request(&request);
        let deadline = effective_deadline(&options).map(|d| tokio::time::Instant::now() + d);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.server_streaming(
                Self::request(request, headers, &options),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        match with_deadline_at(deadline, call).await {
            Ok(response) => Ok(ResponseStream {
                inner: response.into_inner(),
                listeners,
                deadline,
            }),
            Err(status) => {
                listeners.on_close(&status);
                Err(status)
            }
        }
    }

    /// A client-streaming call (`ClientCalls.asyncClientStreamingCall`): sends every item of
    /// `requests`, half-closes, and returns the single response. The server may answer before
    /// the stream is exhausted (ByteStream.Write does for an existing blob).
    pub async fn client_streaming<Req, Resp, S>(
        &self,
        method: Method,
        requests: S,
        options: CallOptions,
    ) -> Result<Resp, tonic::Status>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
        S: Stream<Item = Req> + Send + 'static,
    {
        let (headers, listeners) = self.start_call(&method, &options).await?;
        let listeners = Arc::new(Mutex::new(listeners));
        let observed = {
            let listeners = listeners.clone();
            requests.inspect(move |message| listeners.lock().on_request(message))
        };
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.client_streaming(
                Self::request(observed, headers, &options),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(effective_deadline(&options), call).await;
        let mut listeners = listeners.lock();
        match result {
            Ok(response) => {
                let message = response.into_inner();
                listeners.on_response(&message);
                listeners.on_close(&tonic::Status::ok(""));
                Ok(message)
            }
            Err(status) => {
                listeners.on_close(&status);
                Err(status)
            }
        }
    }
}

/// `--remote_timeout=0` and an absent deadline both mean "no deadline".
fn effective_deadline(options: &CallOptions) -> Option<Duration> {
    options.deadline.filter(|d| !d.is_zero())
}

fn not_ready(e: tonic::transport::Error) -> tonic::Status {
    // Java reports a connection that cannot be established as UNAVAILABLE.
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        message.push_str(": ");
        message.push_str(&s.to_string());
        source = s.source();
    }
    tonic::Status::unavailable(message)
}

fn deadline_exceeded(after: Duration) -> tonic::Status {
    // Java: "deadline exceeded after 59.999999s. [...]" (io.grpc.internal.ClientCallImpl); the
    // bracketed details are transport internals and are not reproduced.
    tonic::Status::deadline_exceeded(format!(
        "deadline exceeded after {:.9}s",
        after.as_secs_f64()
    ))
}

async fn with_deadline<T>(
    deadline: Option<Duration>,
    call: impl Future<Output = Result<T, tonic::Status>>,
) -> Result<T, tonic::Status> {
    match deadline {
        None => call.await,
        Some(d) => match tokio::time::timeout(d, call).await {
            Ok(result) => result,
            Err(_) => Err(deadline_exceeded(d)),
        },
    }
}

async fn with_deadline_at<T>(
    deadline: Option<tokio::time::Instant>,
    call: impl Future<Output = Result<T, tonic::Status>>,
) -> Result<T, tonic::Status> {
    match deadline {
        None => call.await,
        Some(at) => {
            let start = tokio::time::Instant::now();
            match tokio::time::timeout_at(at, call).await {
                Ok(result) => result,
                Err(_) => Err(deadline_exceeded(start.elapsed())),
            }
        }
    }
}

/// `GoogleAuthLibraryCallCredentials.serviceUri(authority, method)`:
/// `https://<authority without :443>/<package.Service>`.
// Java: io/grpc/auth/GoogleAuthLibraryCallCredentials.java GoogleAuthLibraryCallCredentials.serviceUri
pub fn service_uri(authority: &str, method: &Method) -> String {
    let authority = authority.strip_suffix(":443").unwrap_or(authority);
    format!("https://{}/{}", authority, method.service_name())
}

/// The listeners of one call plus its network-time bracket.
struct Listeners {
    listeners: Vec<Box<dyn CallListener>>,
    network_time: Option<Arc<NetworkTime>>,
    closed: bool,
}

impl Listeners {
    fn on_request(&mut self, message: &(dyn Any + Send)) {
        for l in &mut self.listeners {
            l.on_request(message);
        }
    }

    fn on_response(&mut self, message: &(dyn Any + Send)) {
        for l in &mut self.listeners {
            l.on_response(message);
        }
    }

    fn on_close(&mut self, status: &tonic::Status) {
        if self.closed {
            return;
        }
        self.closed = true;
        // Java: NetworkTimeInterceptor stops the stopwatch in onClose.
        if let Some(network_time) = &self.network_time {
            network_time.stop();
        }
        for l in &mut self.listeners {
            l.on_close(status);
        }
    }
}

impl Drop for Listeners {
    fn drop(&mut self) {
        // A call dropped before it finished was cancelled by the client
        // (`ClientCall.cancel`), which Java's listeners see as CANCELLED.
        if !self.closed {
            self.on_close(&tonic::Status::cancelled("Cancelled by client"));
        }
    }
}

/// The responses of a server-streaming call (`StreamObserver.onNext/onError/onCompleted` as a
/// pull stream).
pub struct ResponseStream<Resp> {
    inner: tonic::Streaming<Resp>,
    listeners: Listeners,
    deadline: Option<tokio::time::Instant>,
}

impl<Resp: Send + Sync + 'static> ResponseStream<Resp> {
    /// The next message: `Ok(Some(m))` is `onNext(m)`, `Ok(None)` is `onCompleted()`, `Err(s)`
    /// is `onError(s.asRuntimeException())`.
    pub async fn message(&mut self) -> Result<Option<Resp>, tonic::Status> {
        if self.listeners.closed {
            return Ok(None);
        }
        let start = tokio::time::Instant::now();
        let next = match self.deadline {
            None => self.inner.message().await,
            Some(at) => match tokio::time::timeout_at(at, self.inner.message()).await {
                Ok(result) => result,
                Err(_) => Err(deadline_exceeded(start.elapsed())),
            },
        };
        match next {
            Ok(Some(message)) => {
                self.listeners.on_response(&message);
                Ok(Some(message))
            }
            Ok(None) => {
                self.listeners.on_close(&tonic::Status::ok(""));
                Ok(None)
            }
            Err(status) => {
                self.listeners.on_close(&status);
                Err(status)
            }
        }
    }
}

/// `new IOException(new StatusRuntimeException(status))` for a `Result` of a call.
pub fn io_result<T>(result: Result<T, tonic::Status>) -> io::Result<T> {
    result.map_err(status_to_io_error)
}

/// Channels for the in-process fake servers of the tests (Java: `InProcessChannelBuilder`).
pub mod testing {
    use super::*;

    /// A plaintext channel to `http://<addr>` (a fake server listening on localhost) with the
    /// given channel-level interceptors.
    pub fn channel_to(
        addr: std::net::SocketAddr,
        interceptors: Vec<Arc<dyn ClientInterceptor>>,
        capabilities_fetcher: Option<CapabilitiesFetcher>,
    ) -> ReferenceCountedChannel {
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .expect("valid test endpoint");
        // `connect_lazy` needs a runtime context for its background task.
        let _guard = crate::runtime::runtime().enter();
        let channel = endpoint.connect_lazy();
        ReferenceCountedChannel::new(
            channel,
            &addr.to_string(),
            false,
            interceptors,
            usize::MAX,
            capabilities_fetcher,
        )
    }
}
