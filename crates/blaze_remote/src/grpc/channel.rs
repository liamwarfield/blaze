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
    /// The `methodConfig[].timeout` entries of the channel's default service config.
    method_timeouts: Vec<MethodTimeout>,
    /// `RemoteDownloadIdleTimeoutInterceptor` (`--experimental_remote_grpc_download_idle_timeout`).
    download_idle_timeout: Option<Duration>,
    capabilities_fetcher: Option<CapabilitiesFetcher>,
    server_capabilities: tokio::sync::OnceCell<Result<ServerCapabilities, Arc<io::Error>>>,
    /// Whether every call waits for the verified server capabilities (see
    /// [`ReferenceCountedChannel::with_capabilities_gate`]).
    capabilities_gate: bool,
    /// The `IOException` of `channelFactory.newChannel` (see
    /// [`ReferenceCountedChannel::failed`]): every call fails with it.
    creation_error: Option<Arc<io::Error>>,
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
                method_timeouts: Vec::new(),
                download_idle_timeout: None,
                capabilities_fetcher,
                server_capabilities: tokio::sync::OnceCell::new(),
                capabilities_gate: false,
                creation_error: None,
                ref_cnt: AtomicI32::new(1),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// A channel whose transport could not be created: every call (and the server capabilities)
    /// fails with `error`.
    ///
    /// PORT: Java creates the transport inside the connection factory
    /// (`Single.fromCallable(() -> channelFactory.newChannel(...))`), so an `IOException` of
    /// `newChannel` (a bad `--tls_certificate`, a malformed target) fails each call's future,
    /// not the command. The transport given to tonic is a lazy placeholder that is never used.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:103 GoogleChannelConnectionFactory.create
    pub fn failed(
        authority: &str,
        error: io::Error,
        capabilities_fetcher: Option<CapabilitiesFetcher>,
    ) -> ReferenceCountedChannel {
        let placeholder = {
            let _guard = crate::runtime::runtime().enter();
            tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy()
        };
        let mut channel = ReferenceCountedChannel::new(
            placeholder,
            authority,
            false,
            Vec::new(),
            usize::MAX,
            capabilities_fetcher,
        );
        match Arc::get_mut(&mut channel.inner) {
            Some(inner) => inner.creation_error = Some(Arc::new(error)),
            None => unreachable!("a new channel is not shared"),
        }
        channel
    }

    /// `builder.defaultServiceConfig(serviceConfig)`: applies the `methodConfig[].timeout`
    /// entries of a gRPC service config (`RemoteGrpcServiceConfig`) as the deadline of every
    /// call to a matching method. Must be called before the channel is shared.
    ///
    /// PORT: grpc-java applies the timeout of the selected method config as the call's deadline
    /// (`ManagedChannelImpl.ConfigSelectingClientCall`); the other service config features are
    /// rejected by `RemoteGrpcServiceConfig` before they get here.
    pub fn with_service_config(
        mut self,
        service_config: &serde_json::Map<String, serde_json::Value>,
    ) -> io::Result<ReferenceCountedChannel> {
        let timeouts = parse_method_timeouts(service_config)?;
        match Arc::get_mut(&mut self.inner) {
            Some(inner) => inner.method_timeouts = timeouts,
            None => panic!("IllegalStateException: the channel is already shared"),
        }
        Ok(self)
    }

    /// Cancels remote downloads (`ByteStream.Read`) after `timeout` without response data,
    /// with DEADLINE_EXCEEDED. Must be called before the channel is shared.
    ///
    /// PORT: Java's `RemoteDownloadIdleTimeoutInterceptor` is a channel interceptor that wraps
    /// the call and its listener; here the response stream of a Read call applies the timeout
    /// between messages (and before the first one).
    // Java: com/google/devtools/build/lib/remote/RemoteDownloadIdleTimeoutInterceptor.java:38 RemoteDownloadIdleTimeoutInterceptor
    pub fn with_download_idle_timeout(mut self, timeout: Duration) -> ReferenceCountedChannel {
        match Arc::get_mut(&mut self.inner) {
            Some(inner) => inner.download_idle_timeout = Some(timeout),
            None => panic!("IllegalStateException: the channel is already shared"),
        }
        self
    }

    /// Makes every call (except `GetCapabilities` itself) wait for the fetched and verified
    /// server capabilities, and fail with the verification's exception (a
    /// `RemoteExecutionCapabilitiesException`) when they are incompatible. Must be called before
    /// the channel is shared.
    ///
    /// PORT: Java creates the connection of every call through
    /// `ChannelConnectionWithServerCapabilitiesFactory.create`, which for a requirement other
    /// than `NONE` maps the verified capabilities into the connection; the capabilities are
    /// fetched once (`getAndVerifyServerCapabilitiesStarted`) over the raw channel.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:103 GoogleChannelConnectionFactory.create
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:156 ReferenceCountedChannel.withChannelConnection
    pub fn with_capabilities_gate(mut self) -> ReferenceCountedChannel {
        match Arc::get_mut(&mut self.inner) {
            Some(inner) => inner.capabilities_gate = true,
            None => panic!("IllegalStateException: the channel is already shared"),
        }
        self
    }

    /// The deadline of a call: the earlier of the explicit one and the service config's.
    fn deadline_for(&self, method: &Method, options: &CallOptions) -> Option<Duration> {
        let configured = lookup_method_timeout(&self.inner.method_timeouts, method);
        match (effective_deadline(options), configured) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
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
        if let Some(e) = &self.inner.creation_error {
            return Err(crate::common::errors::shared(e));
        }
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
        if let Some(e) = &self.inner.creation_error {
            return Err(crate::common::errors::io_error_status(
                crate::common::errors::shared(e),
            ));
        }
        // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:156 ReferenceCountedChannel.withChannelConnection
        if self.inner.capabilities_gate
            && *method != methods::GET_CAPABILITIES
            && let Err(e) = self.get_server_capabilities_async().await
        {
            return Err(crate::common::errors::io_error_status(e));
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
            // GoogleAuthLibraryCallCredentials' callback (onFailure, for an error that is not
            // `Retryable`): `Status.UNAUTHENTICATED.withDescription("Failed computing credential
            // metadata").withCause(e)`. `Utils.grpcAwareErrorMessage` shows the cause's message
            // only for a `CredentialHelperException`.
            let metadata = credentials.get_request_metadata(&uri).await.map_err(|e| {
                let mut status =
                    tonic::Status::unauthenticated("Failed computing credential metadata");
                status.set_source(Arc::new(e));
                status
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

    fn request<T>(
        message: T,
        headers: MetadataMap,
        deadline: Option<Duration>,
    ) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        *request.metadata_mut() = headers;
        if let Some(deadline) = deadline {
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
        let deadline = self.deadline_for(&method, &options);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.unary(
                Self::request(request, headers, deadline),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(deadline, call).await;
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
        let deadline = self.deadline_for(&method, &options);
        let deadline_at = deadline.map(|d| tokio::time::Instant::now() + d);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.server_streaming(
                Self::request(request, headers, deadline),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let idle_timeout = if method == methods::READ {
            self.inner.download_idle_timeout
        } else {
            None
        };
        let started = with_idle_timeout(idle_timeout, &method, with_deadline_at(deadline_at, call));
        match started.await {
            Ok(response) => Ok(ResponseStream {
                inner: response.into_inner(),
                listeners,
                deadline: deadline_at,
                idle_timeout,
                method,
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
        let deadline = self.deadline_for(&method, &options);
        let observed = {
            let listeners = listeners.clone();
            requests.inspect(move |message| listeners.lock().on_request(message))
        };
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready().await.map_err(not_ready)?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.client_streaming(
                Self::request(observed, headers, deadline),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(deadline, call).await;
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

/// One `methodConfig[]` name with its timeout: `service` empty is the default config, `method`
/// empty covers the whole service.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MethodTimeout {
    service: String,
    method: String,
    timeout: Duration,
}

/// Reads the `methodConfig[].timeout` entries of a service config (grpc-java
/// `ManagedChannelServiceConfig.fromServiceConfig` + `ServiceConfigUtil.getTimeoutFromMethodConfig`).
fn parse_method_timeouts(
    service_config: &serde_json::Map<String, serde_json::Value>,
) -> io::Result<Vec<MethodTimeout>> {
    let invalid = |m: String| io::Error::new(io::ErrorKind::InvalidInput, m);
    let mut out = Vec::new();
    let Some(method_configs) = service_config.get("methodConfig") else {
        return Ok(out);
    };
    let method_configs = method_configs
        .as_array()
        .ok_or_else(|| invalid("methodConfig must be a list".to_string()))?;
    for method_config in method_configs {
        let method_config = method_config
            .as_object()
            .ok_or_else(|| invalid("methodConfig entry must be an object".to_string()))?;
        let Some(timeout) = method_config.get("timeout") else {
            continue;
        };
        let timeout = timeout
            .as_str()
            .ok_or_else(|| invalid("timeout must be a string".to_string()))?;
        let timeout = parse_proto_duration(timeout)
            .ok_or_else(|| invalid(format!("Invalid duration string: {timeout}")))?;
        let names = match method_config.get("name") {
            Some(serde_json::Value::Array(names)) => names.as_slice(),
            Some(_) => return Err(invalid("name must be a list".to_string())),
            None => &[],
        };
        for name in names {
            let name = name
                .as_object()
                .ok_or_else(|| invalid("name entry must be an object".to_string()))?;
            let field = |key: &str| -> String {
                name.get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            out.push(MethodTimeout {
                service: field("service"),
                method: field("method"),
                timeout,
            });
        }
    }
    Ok(out)
}

/// A JSON `google.protobuf.Duration` (`"30s"`, `"1.5s"`), as `JsonUtil.parseDuration` reads it.
fn parse_proto_duration(value: &str) -> Option<Duration> {
    let seconds = value.strip_suffix('s')?;
    let (whole, fraction) = match seconds.split_once('.') {
        Some((w, f)) => (w, f),
        None => (seconds, ""),
    };
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 9
    {
        return None;
    }
    let secs: u64 = whole.parse().ok()?;
    let nanos: u32 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<9}").parse().ok()?
    };
    Some(Duration::new(secs, nanos))
}

/// The timeout of the method config gRPC selects for `method`: the exact method, else its
/// service, else the default config.
fn lookup_method_timeout(timeouts: &[MethodTimeout], method: &Method) -> Option<Duration> {
    let service = method.service_name();
    let full = method.full_method_name();
    let name = &full[service.len() + 1..];
    let find = |s: &str, m: &str| {
        timeouts
            .iter()
            .find(|t| t.service == s && t.method == m)
            .map(|t| t.timeout)
    };
    find(service, name)
        .or_else(|| find(service, ""))
        .or_else(|| find("", ""))
}

/// An explicit zero deadline and an absent one both mean "no deadline".
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

/// `RemoteDownloadIdleTimeoutInterceptor.IdleTimeoutCall`: fails with DEADLINE_EXCEEDED when
/// `step` (waiting for the headers or for the next message) takes longer than the idle timeout.
// Java: com/google/devtools/build/lib/remote/RemoteDownloadIdleTimeoutInterceptor.java:176 IdleTimeoutCall.onTimeout
async fn with_idle_timeout<T>(
    idle_timeout: Option<Duration>,
    method: &Method,
    step: impl Future<Output = Result<T, tonic::Status>>,
) -> Result<T, tonic::Status> {
    match idle_timeout {
        None => step.await,
        Some(timeout) => match tokio::time::timeout(timeout, step).await {
            Ok(result) => result,
            // Java: IdleTimeoutCall.timeoutMessage
            Err(_) => Err(tonic::Status::deadline_exceeded(format!(
                "Remote download idle timeout exceeded after {} for {}",
                java_duration_to_string(timeout),
                method.full_method_name()
            ))),
        },
    }
}

/// `java.time.Duration.toString()` (`PT1M`, `PT1H2M3.5S`, `PT0S`) for a non-negative duration.
pub fn java_duration_to_string(d: Duration) -> String {
    if d.is_zero() {
        return "PT0S".to_string();
    }
    let total = d.as_secs();
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    let nanos = d.subsec_nanos();
    let mut out = String::from("PT");
    if hours != 0 {
        out.push_str(&format!("{hours}H"));
    }
    if minutes != 0 {
        out.push_str(&format!("{minutes}M"));
    }
    if secs == 0 && nanos == 0 {
        return out;
    }
    out.push_str(&secs.to_string());
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        out.push('.');
        out.push_str(fraction.trim_end_matches('0'));
    }
    out.push('S');
    out
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
    idle_timeout: Option<Duration>,
    method: Method,
}

impl<Resp: Send + Sync + 'static> ResponseStream<Resp> {
    /// The next message: `Ok(Some(m))` is `onNext(m)`, `Ok(None)` is `onCompleted()`, `Err(s)`
    /// is `onError(s.asRuntimeException())`.
    pub async fn message(&mut self) -> Result<Option<Resp>, tonic::Status> {
        if self.listeners.closed {
            return Ok(None);
        }
        let start = tokio::time::Instant::now();
        let deadline = self.deadline;
        let inner = &mut self.inner;
        let next = with_idle_timeout(self.idle_timeout, &self.method, async {
            match deadline {
                None => inner.message().await,
                Some(at) => match tokio::time::timeout_at(at, inner.message()).await {
                    Ok(result) => result,
                    Err(_) => Err(deadline_exceeded(start.elapsed())),
                },
            }
        })
        .await;
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
