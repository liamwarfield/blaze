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
            // `AsciiKey.toBytes`: `value.getBytes(US_ASCII)`, which replaces every character
            // outside ASCII with '?'.
            // Java: io/grpc/Metadata.java:981 AsciiKey.toBytes
            let ascii: String = value
                .chars()
                .map(|c| if c.is_ascii() { c } else { '?' })
                .collect();
            // PORT: a control character other than a tab is sent as is by Java; the HTTP/2
            // library of the port refuses it, which fails here.
            let value = AsciiMetadataValue::try_from(ascii.as_str()).map_err(|_| {
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
    /// The transport; `None` when it could not be built (see `connection_failure`).
    channel: Option<Channel>,
    /// The `IOException` of `ChannelFactory.newChannel` when the transport could not be built
    /// (see [`ReferenceCountedChannel::failed_to_create`]).
    connection_failure: Option<Arc<io::Error>>,
    /// Whether every connection waits for (and fails with) the verified server capabilities
    /// (`ServerCapabilitiesRequirement` other than `NONE`; see
    /// [`ReferenceCountedChannel::with_server_capabilities_gate`]).
    capabilities_gate: bool,
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
                channel: Some(channel),
                connection_failure: None,
                capabilities_gate: false,
                authority: authority.to_string(),
                tls,
                interceptors,
                max_inbound_message_size,
                method_timeouts: Vec::new(),
                download_idle_timeout: None,
                capabilities_fetcher,
                server_capabilities: tokio::sync::OnceCell::new(),
                ref_cnt: AtomicI32::new(1),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// A channel whose connections cannot be created: `ChannelFactory.newChannel` throws
    /// `error` (an invalid `--tls_certificate`, proxy or target). Java builds connections
    /// lazily (`GoogleChannelConnectionFactory.create` runs `newChannel` inside
    /// `Single.fromCallable`), so the failure is not reported when the channel is created but by
    /// every call, which fails with `error` before it starts.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:101 GoogleChannelConnectionFactory.create
    pub fn failed_to_create(authority: &str, error: io::Error) -> ReferenceCountedChannel {
        ReferenceCountedChannel {
            inner: Arc::new(Inner {
                channel: None,
                connection_failure: Some(Arc::new(error)),
                capabilities_gate: false,
                authority: authority.to_string(),
                tls: false,
                interceptors: Vec::new(),
                max_inbound_message_size: usize::MAX,
                method_timeouts: Vec::new(),
                download_idle_timeout: None,
                capabilities_fetcher: None,
                server_capabilities: tokio::sync::OnceCell::new(),
                ref_cnt: AtomicI32::new(1),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// Makes every connection of the channel wait for the server capabilities, so that every
    /// call except `GetCapabilities` itself fails with the error of the capabilities fetch (a
    /// `RemoteExecutionCapabilitiesException`) before it starts. Must be called before the
    /// channel is shared.
    ///
    /// PORT: Java creates each pooled connection through
    /// `serverCapabilitiesSingle.map(sc -> new ChannelConnectionWithServerCapabilities(..))` when
    /// the requirement is not `NONE`; the port has one connection, so the wait is at the start of
    /// each call. `GetCapabilities` is exempt because Java sends it on the raw `ManagedChannel`
    /// (`RemoteServerCapabilities.get(channel)`), not through the pool.
    // Java: com/google/devtools/build/lib/remote/GoogleChannelConnectionFactory.java:101 GoogleChannelConnectionFactory.create
    pub fn with_server_capabilities_gate(mut self) -> ReferenceCountedChannel {
        match Arc::get_mut(&mut self.inner) {
            Some(inner) => inner.capabilities_gate = true,
            None => panic!("IllegalStateException: the channel is already shared"),
        }
        self
    }

    /// Obtains a connection for a call (`DynamicConnectionPool.create()` through
    /// `withChannel`): fails with the `IOException` of a transport that cannot be built, or,
    /// behind a capabilities gate, with the error of the capabilities fetch.
    // Java: com/google/devtools/build/lib/remote/ReferenceCountedChannel.java:131 ReferenceCountedChannel.withChannel
    pub async fn connection(&self) -> io::Result<()> {
        if let Some(failure) = &self.inner.connection_failure {
            return Err(crate::common::errors::shared(failure));
        }
        if self.inner.capabilities_gate {
            self.get_server_capabilities_async().await?;
        }
        Ok(())
    }

    /// `withChannelFuture(channel -> immediateFuture(channel.authority()))`.
    pub async fn get_authority_async(&self) -> io::Result<String> {
        self.connection().await?;
        Ok(self.inner.authority.clone())
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
        if let Some(failure) = &self.inner.connection_failure {
            return Err(crate::common::errors::shared(failure));
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
        // The connection the call runs on (`withChannelFuture` / `withChannelBlocking`); the
        // call itself, and so every interceptor, never starts when it cannot be obtained.
        let connection = if *method == methods::GET_CAPABILITIES {
            match &self.inner.connection_failure {
                Some(failure) => Err(crate::common::errors::shared(failure)),
                None => Ok(()),
            }
        } else {
            self.connection().await
        };
        if let Err(e) = connection {
            return Err(crate::common::errors::call_not_started(e));
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
                blocking_stub: method.path == "/build.bazel.remote.execution.v2.Execution/Execute"
                    || method.path == "/build.bazel.remote.execution.v2.Execution/WaitExecution",
            },
        ))
    }

    fn grpc(&self) -> Grpc<Channel> {
        Grpc::new(
            self.inner
                .channel
                .clone()
                .expect("start_call fails a call on a channel that could not be created"),
        )
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
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.unary(
                Self::request(request, headers, deadline),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(deadline, call)
            .await
            .map_err(|status| transport_status(status, &self.inner.authority));
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
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority))?;
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
        match started
            .await
            .map_err(|status| transport_status(status, &self.inner.authority))
        {
            Ok(response) => Ok(ResponseStream {
                authority: self.inner.authority.clone(),
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
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.client_streaming(
                Self::request(observed, headers, deadline),
                PathAndQuery::from_static(method.path),
                codec,
            )
            .await
        };
        let result = with_deadline(deadline, call)
            .await
            .map_err(|status| transport_status(status, &self.inner.authority));
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

/// The status of a call whose connection could not be made ready (`grpc.ready()` failed).
fn not_ready(e: tonic::transport::Error, authority: &str) -> tonic::Status {
    let mut status = tonic::Status::unavailable(e.to_string());
    status.set_source(Arc::new(e));
    transport_status(status, authority)
}

/// What a transport error of the port stands for among the exceptions grpc-java's transport
/// turns into a status.
enum TransportThrowable {
    /// `DnsNameResolver`: `UnknownHostException` while resolving the target's host.
    UnresolvedHost,
    /// A connection that could not be established: Netty's `AnnotatedConnectException` (an
    /// `IOException`), with its message.
    Connect(io::Error),
    /// Any other `IOException` of the transport (a socket error on an established connection).
    Io(io::Error),
    /// A protocol violation this side detected (Netty's `Http2Exception`).
    Http2,
    /// A `DecoderException` caused by an `SSLException` (a TLS alert from the server).
    Ssl,
}

/// Classifies the error that made tonic fail a call (the source chain of a `tonic::Status`
/// that tonic built from a transport error), or `None` when it is not a transport error the
/// port maps (a status the server sent has no source; an HTTP/2 reset or GOAWAY the server sent,
/// a TLS handshake failure and Blaze's own statuses keep their text).
///
/// PORT: the transport crates (hyper, h2) are not dependencies of this crate, so their error
/// types are recognised by their `Display` texts, which are fixed in the pinned versions:
/// hyper-util's `ConnectError` displays its kind ("dns error", "tcp connect error") and carries
/// the address in its `Debug` text, hyper's HTTP/2 error displays "http2 error", h2 displays a
/// protocol error this side detected as "connection error detected: ..." / "stream error
/// detected: ..." / "protocol error: ...", and an I/O error inside h2 as the `io::Error` itself
/// (`Debug`: `Error { kind: Io(..) }`).
fn classify_transport_error(status: &tonic::Status, host: &str) -> Option<TransportThrowable> {
    let mut source = std::error::Error::source(status);
    while let Some(e) = source {
        if e.downcast_ref::<crate::common::errors::CallNotStarted>()
            .is_some()
            || e.downcast_ref::<TransportIoException>().is_some()
            || e.downcast_ref::<TransportException>().is_some()
        {
            return None;
        }
        if e.downcast_ref::<tonic::ConnectError>().is_some() {
            // The connector's error: hyper-util's `ConnectError` for the TCP connection (with
            // the OS error as its source), or the `io::Error` of the TLS handshake.
            let connector_error = e.source()?;
            if connector_error.downcast_ref::<io::Error>().is_some() {
                // A TLS handshake failure. A fatal alert from the server reaches Netty's
                // `SslHandler` as a `DecoderException` caused by an `SSLException` (oracle:
                // "UNAVAILABLE: ssl exception" for a server that requires a client certificate,
                // scripts/remote/tls-probe row mtls-noclient). A failure this side detects (the
                // server's certificate) is an `SSLHandshakeException` (an `IOException`) whose
                // JDK/OpenSSL text is not reproduced: rustls' text is kept.
                if connector_error
                    .to_string()
                    .starts_with("received fatal alert: ")
                {
                    return Some(TransportThrowable::Ssl);
                }
                return None;
            }
            if connector_error.to_string() == "dns error" {
                return Some(TransportThrowable::UnresolvedHost);
            }
            if connector_error.to_string() != "tcp connect error" {
                return None;
            }
            let os_error = connector_error
                .source()
                .and_then(|c| c.downcast_ref::<io::Error>())?;
            let address = connect_error_address(&format!("{connector_error:?}"));
            return Some(TransportThrowable::Connect(annotated_connect_exception(
                os_error, host, address,
            )));
        }
        if e.to_string() == "http2 error"
            && let Some(h2_error) = e.source()
        {
            let debug = format!("{h2_error:?}");
            if debug.starts_with("Error { kind: Io(") {
                return Some(TransportThrowable::Io(io::Error::other(
                    h2_error.to_string(),
                )));
            }
            let text = h2_error.to_string();
            if text.starts_with("connection error detected: ")
                || text.starts_with("stream error detected: ")
                || text.starts_with("protocol error: ")
            {
                return Some(TransportThrowable::Http2);
            }
            return None;
        }
        if let Some(io_error) = e.downcast_ref::<io::Error>() {
            return Some(TransportThrowable::Io(io::Error::new(
                io_error.kind(),
                io_error.to_string(),
            )));
        }
        source = e.source();
    }
    None
}

/// The address field of hyper-util's `ConnectError` `Debug` text
/// (`ConnectError("tcp connect error", 127.0.0.1:9499, Os { .. })`).
fn connect_error_address(debug: &str) -> Option<std::net::SocketAddr> {
    let rest = debug.strip_prefix("ConnectError(\"tcp connect error\", ")?;
    rest.split(", ").next()?.parse().ok()
}

/// The `TransportThrowable::Connect` exception: Netty's `AnnotatedConnectException`, whose
/// message is `finishConnect(..) failed: <strerror>: <remote address>`, the remote address being
/// `InetSocketAddress.toString()`, `<host name>/<literal>:<port>` (the host name is empty for a
/// target given as an IP literal; an IPv6 literal is written in brackets, in Java's
/// uncompressed form).
///
/// PORT: Netty names the address it tried last, hyper-util the first one that failed; with more
/// than one address (`localhost`) the two may differ, as Bazel's own choice varies between runs.
/// Netty's `connect(..)` (a connect that fails at once) and connect timeouts are not told apart.
// Java: io/netty/channel/unix/Errors.java Errors.newConnectException0
// Java: io/netty/channel/epoll/AbstractEpollChannel.java AbstractEpollUnsafe.finishConnect
fn annotated_connect_exception(
    os_error: &io::Error,
    host: &str,
    address: Option<std::net::SocketAddr>,
) -> io::Error {
    let strerror = match os_error.raw_os_error() {
        Some(code) => {
            let text = io::Error::from_raw_os_error(code).to_string();
            match text.rfind(" (os error ") {
                Some(i) => text[..i].to_string(),
                None => text,
            }
        }
        None => os_error.to_string(),
    };
    let message = match address {
        Some(address) => format!(
            "finishConnect(..) failed: {strerror}: {}",
            java_inet_socket_address(host, &address)
        ),
        None => format!("finishConnect(..) failed: {strerror}"),
    };
    io::Error::new(
        os_error.kind(),
        crate::common::errors::JavaException {
            class_name: "io.netty.channel.AbstractChannel$AnnotatedConnectException",
            message,
        },
    )
}

/// `InetSocketAddress.toString()` of `address` resolved from `host`.
fn java_inet_socket_address(host: &str, address: &std::net::SocketAddr) -> String {
    // A target given as an IP literal has no host name.
    let unbracketed = host.trim_start_matches('[').trim_end_matches(']');
    let host_name = if unbracketed.parse::<std::net::IpAddr>().is_ok() {
        ""
    } else {
        host
    };
    let literal = match address.ip() {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => {
            // Inet6Address.getHostAddress: eight hexadecimal groups, no compression.
            let groups: Vec<String> = v6.segments().iter().map(|g| format!("{g:x}")).collect();
            format!("[{}]", groups.join(":"))
        }
    };
    format!("{host_name}/{literal}:{}", address.port())
}

/// The host of an authority (`host:port`, `[v6]:port` or `host`).
fn authority_host(authority: &str) -> &str {
    if authority.starts_with('[') {
        return match authority.find(']') {
            Some(i) => &authority[..=i],
            None => authority,
        };
    }
    match authority.rfind(':') {
        Some(i) => &authority[..i],
        None => authority,
    }
}

/// The status grpc-java's transport reports for the transport error that made tonic fail a call
/// (`status`), with the cause `Status.getCause()` would return; any other status is returned
/// unchanged.
///
/// * a connection that cannot be established, or an I/O error: UNAVAILABLE "io exception"
///   with the `IOException` as its cause ([`transport_io_cause`]);
/// * a protocol error (Netty's `Http2Exception`): INTERNAL "http2 exception";
/// * a TLS alert from the server during the handshake: UNAVAILABLE "ssl exception";
/// * a host that does not resolve: grpc's `DnsNameResolver` fails the call with UNAVAILABLE
///   "Unable to resolve host <host>" (the oracle shows no `IOException` cause for it).
///
/// `authority` is the channel's authority; its host stands for the target's host (PORT: with
/// `--tls_authority_override` Java prints the target's host instead).
// Java: io/grpc/netty/Utils.java:327 Utils.statusFromThrowable
// Java: io/grpc/internal/DnsNameResolver.java:285 DnsNameResolver.doResolve
fn transport_status(status: tonic::Status, authority: &str) -> tonic::Status {
    let host = authority_host(authority);
    let Some(throwable) = classify_transport_error(&status, host) else {
        return status;
    };
    match throwable {
        TransportThrowable::UnresolvedHost => {
            let mut mapped = tonic::Status::unavailable(format!("Unable to resolve host {host}"));
            mapped.set_source(Arc::new(TransportException(status.to_string())));
            mapped
        }
        TransportThrowable::Connect(cause) | TransportThrowable::Io(cause) => {
            let mut mapped = tonic::Status::unavailable("io exception");
            mapped.set_source(Arc::new(TransportIoException(Arc::new(cause))));
            mapped
        }
        TransportThrowable::Ssl => {
            let mut mapped = tonic::Status::unavailable("ssl exception");
            mapped.set_source(Arc::new(TransportException(status.to_string())));
            mapped
        }
        TransportThrowable::Http2 => {
            let mut mapped = tonic::Status::internal("http2 exception");
            mapped.set_source(Arc::new(TransportException(status.to_string())));
            mapped
        }
    }
}

/// The cause of a mapped transport status that is not an `IOException` (an `Http2Exception`,
/// the resolver's failure): keeps the original text for debugging, invisible to
/// [`transport_io_cause`].
#[derive(Debug, Clone)]
struct TransportException(String);

impl std::fmt::Display for TransportException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TransportException {}

/// The `IOException` of the transport that caused a status (`status.getCause()` when it is an
/// `IOException`, as Netty's connection failures are).
#[derive(Debug, Clone)]
struct TransportIoException(Arc<io::Error>);

impl std::fmt::Display for TransportIoException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for TransportIoException {}

/// `status.getCause()` when it is the transport's `IOException` (a connection that could not be
/// established: the first `io::Error` in the source chain of a status tonic or
/// [`ReferenceCountedChannel`] made from a transport error); `None` otherwise.
pub fn transport_io_cause(status: &tonic::Status) -> Option<io::Error> {
    let mut source = std::error::Error::source(status);
    while let Some(s) = source {
        if let Some(t) = s.downcast_ref::<TransportIoException>() {
            // Keep the Java class of the exception (`errors::java_to_string`).
            if let Some(java) =
                t.0.get_ref()
                    .and_then(|r| r.downcast_ref::<crate::common::errors::JavaException>())
            {
                return Some(io::Error::new(t.0.kind(), java.clone()));
            }
            return Some(io::Error::new(t.0.kind(), t.0.to_string()));
        }
        if let Some(io) = s.downcast_ref::<io::Error>() {
            return Some(io::Error::new(io.kind(), io.to_string()));
        }
        source = s.source();
    }
    None
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
    /// The call is one Java makes with a blocking stub (`Execution/Execute`,
    /// `Execution/WaitExecution`: `GrpcRemoteExecutor` iterates a blocking server stream).
    blocking_stub: bool,
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
            if self.blocking_stub && blaze_shell::thread_interrupt::is_current_interrupted() {
                // The waiting thread was interrupted (`crate::runtime::get_from_future` drops the
                // call on that thread): gRPC's blocking iterator cancels the call with
                // `call.cancel("Thread interrupted", e)`.
                // Java: io/grpc/stub/ClientCalls.java BlockingResponseStream.waitForNext
                let mut status = tonic::Status::cancelled("Thread interrupted");
                status.set_source(Arc::new(
                    blaze_shell::thread_interrupt::InterruptedException,
                ));
                self.on_close(&status);
                return;
            }
            self.on_close(&tonic::Status::cancelled("Cancelled by client"));
        }
    }
}

/// The responses of a server-streaming call (`StreamObserver.onNext/onError/onCompleted` as a
/// pull stream).
pub struct ResponseStream<Resp> {
    /// The channel's authority (see [`transport_status`]).
    authority: String,
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
        .await
        .map_err(|status| transport_status(status, &self.authority));
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

/// Channels for the in-process fake servers of the tests (Java: `InProcessChannelBuilder`);
/// only with the `testing` feature, which the crate's own tests enable.
#[cfg(any(test, feature = "testing"))]
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

/// Not from a Java test: the transport texts of review 2026-11-19 R3 (round 2), expected values
/// from the oracle (`scripts/remote/caps-matrix`, runs `ry-caps-tco-*`) and from grpc-netty's
/// `Utils.statusFromThrowable`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::reapi::{GetCapabilitiesRequest, ServerCapabilities};

    fn get_capabilities_status(target_authority: &str, endpoint: &str) -> tonic::Status {
        let endpoint =
            tonic::transport::Endpoint::from_shared(endpoint.to_string()).expect("endpoint");
        let lazy = {
            let _guard = crate::runtime::runtime().enter();
            endpoint.connect_lazy()
        };
        let channel = ReferenceCountedChannel::new(
            lazy,
            target_authority,
            false,
            Vec::new(),
            usize::MAX,
            None,
        );
        crate::runtime::block_on(channel.unary::<GetCapabilitiesRequest, ServerCapabilities>(
            methods::GET_CAPABILITIES,
            GetCapabilitiesRequest::default(),
            CallOptions::new(),
        ))
        .unwrap_err()
    }

    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn inet_socket_address_to_string() {
        let v4: std::net::SocketAddr = "127.0.0.1:9499".parse().unwrap();
        let v6: std::net::SocketAddr = "[::1]:9499".parse().unwrap();
        assert_eq!(
            java_inet_socket_address("localhost", &v4),
            "localhost/127.0.0.1:9499"
        );
        assert_eq!(
            java_inet_socket_address("localhost", &v6),
            "localhost/[0:0:0:0:0:0:0:1]:9499"
        );
        assert_eq!(
            java_inet_socket_address("127.0.0.1", &v4),
            "/127.0.0.1:9499"
        );
        assert_eq!(
            java_inet_socket_address("[::1]", &v6),
            "/[0:0:0:0:0:0:0:1]:9499"
        );
    }

    /// `--remote_header=x=aé`: the option value is an internal string (`a`, U+00C3, U+00A9) and
    /// `getBytes(US_ASCII)` sends one '?' per char: the oracle sends `a??`
    /// (`scripts/remote/nonascii-probe oracle tg-o1`, step hdr: the server receives the header
    /// `x` as the bytes `a??`). `--remote_header='bad name=v'` (oracle `tc-hdr-o` run2: Bazel
    /// crashes, exit 37) is refused.
    #[test]
    fn header_values_are_sent_as_us_ascii() {
        let interceptor =
            AttachHeadersInterceptor::new(&[("x".to_string(), "a\u{c3}\u{a9}".to_string())])
                .unwrap();
        let mut headers = MetadataMap::new();
        let _ = interceptor.intercept_call(&methods::GET_CAPABILITIES, &mut headers);
        assert_eq!(headers.get("x").unwrap().to_str().unwrap(), "a??");
        assert!(
            AttachHeadersInterceptor::new(&[("bad name".to_string(), "v".to_string())]).is_err()
        );
    }

    #[test]
    fn authority_host_strips_the_port() {
        assert_eq!(authority_host("localhost:9499"), "localhost");
        assert_eq!(authority_host("[::1]:9499"), "[::1]");
        assert_eq!(authority_host("example.com"), "example.com");
    }

    #[test]
    fn connect_error_address_from_debug_text() {
        assert_eq!(
            connect_error_address(
                "ConnectError(\"tcp connect error\", [::1]:9499, Os { code: 111, kind: ConnectionRefused, message: \"Connection refused\" })"
            ),
            Some("[::1]:9499".parse().unwrap())
        );
        assert_eq!(
            connect_error_address("ConnectError(\"dns error\", Custom { .. })"),
            None
        );
    }

    /// Oracle: "Failed to query remote execution capabilities: finishConnect(..) failed:
    /// Connection refused: /127.0.0.1:9499" (row unreach-cache-ip); the call's status is
    /// UNAVAILABLE "io exception" (the later "Remote Cache: UNAVAILABLE: io exception" warning,
    /// row closed-cache).
    #[test]
    fn connection_refused_is_unavailable_io_exception() {
        let port = closed_port();
        let status = get_capabilities_status(
            &format!("127.0.0.1:{port}"),
            &format!("http://127.0.0.1:{port}"),
        );
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(status.message(), "io exception");
        assert_eq!(
            transport_io_cause(&status).unwrap().to_string(),
            format!("finishConnect(..) failed: Connection refused: /127.0.0.1:{port}")
        );
    }

    /// Oracle: "Failed to query remote execution capabilities: INTERNAL: http2 exception" for a
    /// gRPC call to an HTTP/1 server (row http-cache); the cause is not an `IOException`.
    #[test]
    fn http1_server_is_internal_http2_exception() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf);
            let _ = socket.write_all(
                b"HTTP/1.0 400 Bad request version\r\nContent-Type: text/html\r\n\r\n<html></html>\n",
            );
        });
        let status = get_capabilities_status(
            &format!("localhost:{port}"),
            &format!("http://127.0.0.1:{port}"),
        );
        assert_eq!(status.code(), tonic::Code::Internal);
        assert_eq!(status.message(), "http2 exception");
        assert!(transport_io_cause(&status).is_none());
        server.join().unwrap();
    }

    /// Oracle: "Failed to query remote execution capabilities: UNAVAILABLE: Unable to resolve
    /// host blaze-unresolvable.invalid" (rows unresolved-cache, unresolved-exec).
    #[test]
    fn unresolvable_host_is_unavailable_unable_to_resolve_host() {
        let status = get_capabilities_status(
            "blaze-unresolvable.invalid:9621",
            "http://blaze-unresolvable.invalid:9621",
        );
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(
            status.message(),
            "Unable to resolve host blaze-unresolvable.invalid"
        );
        assert!(transport_io_cause(&status).is_none());
    }
}
