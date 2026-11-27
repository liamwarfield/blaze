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
    /// `FetchGrpc.getFetchBlobMethod()` (Remote Asset API).
    pub const FETCH_BLOB: Method = Method::new("/build.bazel.remote.asset.v1.Fetch/FetchBlob");
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
    /// `withInterceptors(...)` of the stub besides the request metadata (the remote
    /// downloader's `newDownloaderHeadersInterceptor`); they add their headers after the
    /// request metadata, before the channel's interceptors.
    pub stub_interceptors: Vec<Arc<dyn ClientInterceptor>>,
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

    /// `withInterceptors(interceptor)` of the stub.
    pub fn with_interceptor(mut self, interceptor: Arc<dyn ClientInterceptor>) -> CallOptions {
        self.stub_interceptors.push(interceptor);
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
    /// The lifecycle of the channel's connections, when the channel was built by
    /// `google_auth_utils::new_channel` (see [`ReferenceCountedChannel::with_transport_lifecycle`]).
    transport_lifecycle: Option<Arc<TransportLifecycle>>,
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
                transport_lifecycle: None,
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
                transport_lifecycle: None,
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

    /// The lifecycle of the transport's connections ([`TransportLifecycle`], recorded by the
    /// connector of `google_auth_utils::new_channel`), which gives a call that failed because
    /// its connection went away the connection's shutdown status. Must be called before the
    /// channel is shared.
    pub fn with_transport_lifecycle(
        mut self,
        lifecycle: Option<Arc<TransportLifecycle>>,
    ) -> ReferenceCountedChannel {
        match Arc::get_mut(&mut self.inner) {
            Some(inner) => inner.transport_lifecycle = lifecycle,
            None => panic!("IllegalStateException: the channel is already shared"),
        }
        self
    }

    fn lifecycle(&self) -> Option<&TransportLifecycle> {
        self.inner.transport_lifecycle.as_deref()
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
            // Java: `applier.fail(Status.UNAUTHENTICATED.withDescription("Failed computing
            // credential metadata").withCause(e))` (e is not a google-auth `Retryable`).
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
        for interceptor in options
            .stub_interceptors
            .iter()
            .chain(self.inner.interceptors.iter())
        {
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

    /// [`Self::request`] with the guard that cancels it: dropping the guard resets the HTTP/2
    /// stream (`RST_STREAM CANCEL`) if the request side has not ended, which grpc-java's
    /// `ClientCall.cancel` does for a call that is given up (a dropped future, an interrupt, an
    /// aborted stream). It matters once the response headers have arrived: hyper resets the
    /// stream itself when the call's future is dropped before them, but afterwards it keeps the
    /// request side open in a task of its own until the request stream ends, and then
    /// half-closes as if the call had finished normally (BES's `abortStream`). A request side that
    /// has ended (unary, server streaming) is reset by h2 when the response is dropped; the guard
    /// changes nothing there. Tested in `tests/grpc_call_cancellation_test.rs`.
    ///
    /// PORT: hyper polls the request body (and so sees the cancel) only when it wants the next
    /// chunk; while a chunk it already took waits for HTTP/2 flow-control window, a dropped call
    /// is not reset until the window opens or the connection closes (review 2026-11-19 round 8,
    /// G1). grpc-java resets at once. Only a server that grants no window for the stream shows
    /// the difference, bounded by the channel's life.
    fn cancellable_request<T>(
        message: T,
        headers: MetadataMap,
        deadline: Option<Duration>,
    ) -> (tonic::Request<T>, CancelOnDrop) {
        let mut request = Self::request(message, headers, deadline);
        let cancel = CancelOnDrop(Some(request.cancellation_handle()));
        (request, cancel)
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
        let (request, _cancel) = Self::cancellable_request(request, headers, deadline);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority, self.lifecycle()))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.unary(request, PathAndQuery::from_static(method.path), codec)
                .await
        };
        let result = with_deadline(deadline, call)
            .await
            .map_err(|status| transport_status(status, &self.inner.authority, self.lifecycle()));
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
        let (request, cancel) = Self::cancellable_request(request, headers, deadline);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority, self.lifecycle()))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.server_streaming(request, PathAndQuery::from_static(method.path), codec)
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
            .map_err(|status| transport_status(status, &self.inner.authority, self.lifecycle()))
        {
            Ok(response) => Ok(ResponseStream {
                authority: self.inner.authority.clone(),
                lifecycle: self.inner.transport_lifecycle.clone(),
                inner: response.into_inner(),
                listeners,
                deadline: deadline_at,
                idle_timeout,
                method,
                _cancel: cancel,
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
        // Dropping this call (an interrupt) or its end with requests left (the server answered
        // early) resets the stream: hyper would otherwise go on sending the rest.
        let (request, _cancel) = Self::cancellable_request(observed, headers, deadline);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority, self.lifecycle()))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.client_streaming(request, PathAndQuery::from_static(method.path), codec)
                .await
        };
        let result = with_deadline(deadline, call)
            .await
            .map_err(|status| transport_status(status, &self.inner.authority, self.lifecycle()));
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

    /// A bidirectional-streaming call (`ClientCalls.asyncBidiStreamingCall`): sends every item
    /// of `requests` (the call half-closes when the stream ends) and returns the responses.
    /// Dropping the returned stream (or this future) cancels the call: the HTTP/2 stream is
    /// reset (`RST_STREAM CANCEL`) even while `requests` is still open, so the server's call
    /// fails with `CANCELLED`, as after grpc-java's `ClientCall.cancel` / `onError`.
    ///
    /// PORT: added for the Build Event Service client
    /// (`PublishBuildEvent.PublishBuildToolEventStream`, blaze_bes). The channel-level listeners
    /// see the responses and the close, not the requests (no interceptor of that channel
    /// observes messages).
    pub async fn bidi_streaming<Req, Resp, S>(
        &self,
        method: Method,
        requests: S,
        options: CallOptions,
    ) -> Result<ResponseStream<Resp>, tonic::Status>
    where
        Req: prost::Message + Send + Sync + 'static,
        Resp: prost::Message + Default + Send + Sync + 'static,
        S: Stream<Item = Req> + Send + 'static,
    {
        let (headers, mut listeners) = self.start_call(&method, &options).await?;
        let deadline = self.deadline_for(&method, &options);
        let deadline_at = deadline.map(|d| tokio::time::Instant::now() + d);
        let (request, cancel) = Self::cancellable_request(requests, headers, deadline);
        let call = async {
            let mut grpc = self.grpc();
            grpc.ready()
                .await
                .map_err(|e| not_ready(e, &self.inner.authority, self.lifecycle()))?;
            let codec: ProstCodec<Req, Resp> = ProstCodec::default();
            grpc.streaming(request, PathAndQuery::from_static(method.path), codec)
                .await
        };
        match with_deadline_at(deadline_at, call)
            .await
            .map_err(|status| transport_status(status, &self.inner.authority, self.lifecycle()))
        {
            Ok(response) => Ok(ResponseStream {
                authority: self.inner.authority.clone(),
                lifecycle: self.inner.transport_lifecycle.clone(),
                inner: response.into_inner(),
                listeners,
                deadline: deadline_at,
                idle_timeout: None,
                method,
                _cancel: cancel,
            }),
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
fn not_ready(
    e: tonic::transport::Error,
    authority: &str,
    lifecycle: Option<&TransportLifecycle>,
) -> tonic::Status {
    let mut status = tonic::Status::unavailable(e.to_string());
    status.set_source(Arc::new(e));
    transport_status(status, authority, lifecycle)
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
                if is_tls_alert(&connector_error.to_string()) {
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
                if is_tls_alert(&h2_error.to_string()) {
                    return Some(TransportThrowable::Ssl);
                }
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
            if is_tls_alert(&io_error.to_string()) {
                return Some(TransportThrowable::Ssl);
            }
            return Some(TransportThrowable::Io(io::Error::new(
                io_error.kind(),
                io_error.to_string(),
            )));
        }
        source = e.source();
    }
    None
}

/// Whether an I/O error of the TLS session is a fatal alert the server sent (rustls' text
/// "received fatal alert: <description>"). grpc-netty's `Utils.statusFromThrowable` makes the
/// `DecoderException` caused by an `SSLException` that Netty's `SslHandler` raises for it
/// "UNAVAILABLE: ssl exception", whether it arrives during the handshake (the connector's error)
/// or after it, on the established connection (TLS 1.3: a server that requires a client
/// certificate sends `certificate_required` after the client's Finished, so the alert reaches
/// h2 or hyper as an I/O error; scripts/remote/tls-probe row mtls-noclient).
// Java: io/grpc/netty/Utils.java Utils.statusFromThrowable
fn is_tls_alert(text: &str) -> bool {
    text.starts_with("received fatal alert: ")
}

/// The first failure of a connection: what grpc-netty's `ClientTransportLifecycleManager`
/// keeps as the transport's shutdown status (`notifyShutdown`: the first status wins).
#[derive(Debug, Clone)]
enum ShutdownCause {
    /// An `IOException` of the connection (a read error, a TLS alert, or a write the peer
    /// refused that no read-side failure preceded).
    Io(Arc<io::Error>),
    /// Netty's `Http2Exception` for a peer whose first frame is not SETTINGS (a server that
    /// does not speak HTTP/2), with its message.
    Http2(String),
}

impl std::fmt::Display for ShutdownCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShutdownCause::Io(e) => std::fmt::Display::fmt(e, f),
            ShutdownCause::Http2(message) => f.write_str(message),
        }
    }
}

impl ShutdownCause {
    /// `Utils.statusFromThrowable(cause)`, with the rules of [`classify_transport_error`].
    // Java: io/grpc/netty/Utils.java:327 Utils.statusFromThrowable
    fn to_throwable(&self) -> TransportThrowable {
        match self {
            ShutdownCause::Io(e) if is_tls_alert(&e.to_string()) => TransportThrowable::Ssl,
            ShutdownCause::Io(e) => TransportThrowable::Io(io::Error::new(e.kind(), e.to_string())),
            ShutdownCause::Http2(_) => TransportThrowable::Http2,
        }
    }
}

/// The state of one connection (one `NettyClientHandler` with its lifecycle manager).
#[derive(Debug, Default)]
struct ConnectionLifecycle {
    /// `lifecycleManager.getShutdownStatus()`: the first failure.
    shutdown: Mutex<Option<ShutdownCause>>,
    /// The first write the peer refused (EPIPE, ECONNRESET); later writes are dropped.
    write_error: Mutex<Option<Arc<io::Error>>>,
    /// The first bytes received (up to 5), until the first frame is verified.
    first_bytes: Mutex<Vec<u8>>,
    first_frame_verified: AtomicBool,
}

impl ConnectionLifecycle {
    /// `lifecycleManager.notifyShutdown(status)`: keeps the first cause.
    // Java: io/grpc/netty/ClientTransportLifecycleManager.java:73 ClientTransportLifecycleManager.notifyShutdown
    fn notify_shutdown(&self, cause: ShutdownCause) {
        let mut shutdown = self.shutdown.lock();
        if shutdown.is_none() {
            *shutdown = Some(cause);
        }
    }

    /// Netty's `PrefaceDecoder.verifyFirstFrameIsSettings` on the client: the first frame of
    /// the server must be a SETTINGS frame without the ACK flag; otherwise the connection fails
    /// with a PROTOCOL_ERROR `Http2Exception` (`onConnectionError`).
    // Java: io/netty/handler/codec/http2/Http2ConnectionHandler.java PrefaceDecoder.verifyFirstFrameIsSettings
    fn on_bytes(&self, bytes: &[u8]) {
        if bytes.is_empty() || self.first_frame_verified.load(Ordering::SeqCst) {
            return;
        }
        let mut first = self.first_bytes.lock();
        let take = (5 - first.len()).min(bytes.len());
        first.extend_from_slice(&bytes[..take]);
        if first.len() < 5 {
            return;
        }
        self.first_frame_verified.store(true, Ordering::SeqCst);
        // Http2FrameTypes.SETTINGS = 0x4, Http2Flags.ACK = 0x1.
        let (frame_type, flags) = (first[3], first[4]);
        if frame_type != 0x4 || (flags & 0x1) != 0 {
            let hex: String = first.iter().map(|b| format!("{b:02x}")).collect();
            self.notify_shutdown(ShutdownCause::Http2(format!(
                "First received frame was not SETTINGS. Hex dump for first 5 bytes: {hex}"
            )));
        }
    }

    /// A read error, or the end of the stream: an `IOException` of the connection
    /// (`exceptionCaught` -> `onConnectionError`). At the end of the stream the connection
    /// fails with the write the peer refused, if any (Netty notices a closed peer at a write
    /// as well); a clean end of the stream alone records nothing.
    fn on_read_end(&self, error: Option<&io::Error>) {
        match error {
            Some(e) => self.notify_shutdown(ShutdownCause::Io(Arc::new(io::Error::new(
                e.kind(),
                e.to_string(),
            )))),
            None => {
                let write_error = self.write_error.lock().clone();
                if let Some(e) = write_error {
                    self.notify_shutdown(ShutdownCause::Io(e));
                }
            }
        }
    }
}

/// The lifecycle of the connections of one channel: the shutdown status of its latest
/// connection, for the calls that fail because their connection went away.
///
/// PORT: grpc-netty fails every stream of a connection, and every stream started on it after
/// its failure, with the connection's shutdown status: the status of its *first* failure
/// (`NettyClientHandler.onConnectionError` -> `ClientTransportLifecycleManager.notifyShutdown`,
/// then `channelInactive` / `createStream` report `getShutdownStatus()`). hyper and h2 give the
/// calls of a failed connection whatever they saw last: the I/O error of the connection, or
/// `Canceled("connection closed")` for a call that was still waiting for it, and the
/// connection's own error is lost. The channel's connector therefore records the first failure
/// of each connection here, on the connection's byte stream (above TLS): a read error (a TLS
/// alert included), a first frame that is not SETTINGS, or a write the peer refused. A write
/// the peer refused (EPIPE, ECONNRESET) does not end the connection at once: the write is
/// dropped and the failure is the read side's, if the peer sent anything before it went away.
/// Netty writes the connection preface, the SETTINGS and the first request in one flush right
/// after the TLS handshake (`WriteBufferingAndExceptionHandler`), before the server can have
/// answered, so its first failure is what it then reads (the alert, the HTTP/1 reply); h2
/// writes the request later, after the server may have closed, and would otherwise fail with
/// EPIPE first. The connection whose failure applies is the channel's latest one (one
/// connection per channel: `crate::grpc::channel`, no pool).
// Java: io/grpc/netty/ClientTransportLifecycleManager.java:26 ClientTransportLifecycleManager
#[derive(Debug, Default)]
pub struct TransportLifecycle {
    latest: Mutex<Option<Arc<ConnectionLifecycle>>>,
}

impl TransportLifecycle {
    pub fn new() -> Arc<TransportLifecycle> {
        Arc::new(TransportLifecycle::default())
    }

    fn adopt(&self, connection: Arc<ConnectionLifecycle>) {
        *self.latest.lock() = Some(connection);
    }

    /// The shutdown status of the latest connection, if it failed.
    fn shutdown_cause(&self) -> Option<ShutdownCause> {
        let latest = self.latest.lock().clone()?;
        latest.shutdown.lock().clone()
    }
}

/// Below TLS: a write the peer refused (EPIPE, ECONNRESET) is recorded and dropped, and so is
/// every later write, so that the connection ends with what its read side sees (see
/// [`TransportLifecycle`]).
struct DroppingRefusedWrites<I> {
    inner: I,
    connection: Arc<ConnectionLifecycle>,
    refused: bool,
}

impl<I> DroppingRefusedWrites<I> {
    fn refused(&mut self, e: &io::Error) -> bool {
        if matches!(
            e.kind(),
            io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
        ) {
            let mut write_error = self.connection.write_error.lock();
            if write_error.is_none() {
                *write_error = Some(Arc::new(io::Error::new(e.kind(), e.to_string())));
            }
            self.refused = true;
            true
        } else {
            false
        }
    }
}

impl<I: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for DroppingRefusedWrites<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<I: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for DroppingRefusedWrites<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        use std::task::Poll;
        if self.refused {
            return Poll::Ready(Ok(buf.len()));
        }
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Err(e)) if self.refused(&e) => Poll::Ready(Ok(buf.len())),
            other => other,
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> std::task::Poll<io::Result<usize>> {
        use std::task::Poll;
        let len = bufs.iter().map(|b| b.len()).sum();
        if self.refused {
            return Poll::Ready(Ok(len));
        }
        match Pin::new(&mut self.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(Err(e)) if self.refused(&e) => Poll::Ready(Ok(len)),
            other => other,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if self.refused {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        if self.refused {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Above TLS: records the read side of the connection (its first bytes, a read error, the end
/// of the stream) in its [`ConnectionLifecycle`] (see [`TransportLifecycle`]).
pub struct RecordingReads<I> {
    inner: I,
    connection: Arc<ConnectionLifecycle>,
}

impl<I: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for RecordingReads<I> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        use std::task::Poll;
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        match &result {
            Poll::Ready(Ok(())) => {
                let read = &buf.filled()[before..];
                if read.is_empty() && buf.remaining() > 0 {
                    self.connection.on_read_end(None);
                } else {
                    self.connection.on_bytes(read);
                }
            }
            Poll::Ready(Err(e)) => self.connection.on_read_end(Some(e)),
            Poll::Pending => {}
        }
        result
    }
}

impl<I: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for RecordingReads<I> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// The byte stream of a connection: plain TCP or TLS over TCP, below the recording layer.
pub trait ConnectionIo: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}

impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> ConnectionIo for T {}

/// The connection of a channel built by `google_auth_utils::new_channel`.
pub type LifecycleIo = hyper_util::rt::TokioIo<RecordingReads<Box<dyn ConnectionIo>>>;

/// TLS as tonic's `TlsConnector` does it (rustls with ALPN `h2`, which the server must
/// select); the configuration is built by `google_auth_utils::new_channel`.
#[derive(Clone)]
pub struct ConnectorTls {
    pub config: Arc<tokio_rustls::rustls::ClientConfig>,
    pub server_name: tokio_rustls::rustls::pki_types::ServerName<'static>,
}

/// tonic's `TlsError::H2NotNegotiated`, with its text.
#[derive(Debug)]
struct H2NotNegotiated;

impl std::fmt::Display for H2NotNegotiated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HTTP/2 was not negotiated.")
    }
}

impl std::error::Error for H2NotNegotiated {}

/// The connector of a channel built by `google_auth_utils::new_channel`: tonic's own
/// (`Endpoint::http_connector` and, for TLS, `TlsConnector::connect`), with the connection's
/// byte stream recorded in the channel's [`TransportLifecycle`].
///
/// PORT: tonic applies its TLS to a connector's stream itself, out of reach of a recording
/// layer above it; this connector therefore does the TLS handshake (the endpoint's URI is
/// `http://`, its origin `https://`).
#[derive(Clone)]
pub struct LifecycleConnector {
    http: hyper_util::client::legacy::connect::HttpConnector,
    tls: Option<ConnectorTls>,
    lifecycle: Arc<TransportLifecycle>,
}

impl LifecycleConnector {
    /// `http` is configured as tonic's `Endpoint::http_connector` configures it.
    pub fn new(
        http: hyper_util::client::legacy::connect::HttpConnector,
        tls: Option<ConnectorTls>,
        lifecycle: Arc<TransportLifecycle>,
    ) -> LifecycleConnector {
        LifecycleConnector {
            http,
            tls,
            lifecycle,
        }
    }
}

impl tower::Service<http::Uri> for LifecycleConnector {
    type Response = LifecycleIo;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<LifecycleIo, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        tower::Service::<http::Uri>::poll_ready(&mut self.http, cx).map_err(Into::into)
    }

    fn call(&mut self, uri: http::Uri) -> Self::Future {
        let connect = tower::Service::call(&mut self.http, uri);
        let tls = self.tls.clone();
        let lifecycle = self.lifecycle.clone();
        Box::pin(async move {
            let tcp = connect.await?.into_inner();
            let connection = Arc::new(ConnectionLifecycle::default());
            let raw = DroppingRefusedWrites {
                inner: tcp,
                connection: connection.clone(),
                refused: false,
            };
            let io: Box<dyn ConnectionIo> = match tls {
                None => Box::new(raw),
                Some(tls) => {
                    let stream = tokio_rustls::TlsConnector::from(tls.config)
                        .connect(tls.server_name, raw)
                        .await?;
                    let (_, session) = stream.get_ref();
                    if session.alpn_protocol() != Some(b"h2") {
                        return Err(H2NotNegotiated.into());
                    }
                    Box::new(stream)
                }
            };
            // The connection exists once it is established (`NettyClientHandler` is created
            // for a connected channel).
            lifecycle.adopt(connection.clone());
            Ok(hyper_util::rt::TokioIo::new(RecordingReads {
                inner: io,
                connection,
            }))
        })
    }
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
///
/// A call that failed because its connection went away (an error of the established
/// connection, or hyper's "connection closed" for a call that was waiting for it) gets the
/// connection's shutdown status instead, when `lifecycle` recorded one: the status of the
/// connection's first failure ([`TransportLifecycle`]), as grpc-netty fails every stream of a
/// connection with `lifecycleManager.getShutdownStatus()`.
// Java: io/grpc/netty/NettyClientHandler.java:478 NettyClientHandler.channelInactive
// Java: io/grpc/netty/NettyClientHandler.java:581 NettyClientHandler.createStream
fn transport_status(
    status: tonic::Status,
    authority: &str,
    lifecycle: Option<&TransportLifecycle>,
) -> tonic::Status {
    let host = authority_host(authority);
    let throwable = classify_transport_error(&status, host);
    let connection_level = match &throwable {
        Some(TransportThrowable::Io(_) | TransportThrowable::Ssl | TransportThrowable::Http2) => {
            !from_connector(&status)
        }
        None => is_connection_closed_cancel(&status),
        _ => false,
    };
    let mut cause_text = status.to_string();
    let throwable = match lifecycle.and_then(|l| l.shutdown_cause()) {
        Some(cause) if connection_level => {
            cause_text = cause.to_string();
            cause.to_throwable()
        }
        _ => match throwable {
            Some(throwable) => throwable,
            None => return status,
        },
    };
    match throwable {
        TransportThrowable::UnresolvedHost => {
            let mut mapped = tonic::Status::unavailable(format!("Unable to resolve host {host}"));
            mapped.set_source(Arc::new(TransportException(cause_text)));
            mapped
        }
        TransportThrowable::Connect(cause) | TransportThrowable::Io(cause) => {
            let mut mapped = tonic::Status::unavailable("io exception");
            mapped.set_source(Arc::new(TransportIoException(Arc::new(cause))));
            mapped
        }
        TransportThrowable::Ssl => {
            let mut mapped = tonic::Status::unavailable("ssl exception");
            mapped.set_source(Arc::new(TransportException(cause_text)));
            mapped
        }
        TransportThrowable::Http2 => {
            let mut mapped = tonic::Status::internal("http2 exception");
            mapped.set_source(Arc::new(TransportException(cause_text)));
            mapped
        }
    }
}

/// Whether the error of a status comes from the connector (the connection was never
/// established: TCP connect or TLS handshake), whose failure belongs to no connection.
fn from_connector(status: &tonic::Status) -> bool {
    let mut source = std::error::Error::source(status);
    while let Some(e) = source {
        if e.downcast_ref::<tonic::ConnectError>().is_some() {
            return true;
        }
        source = e.source();
    }
    false
}

/// Whether a status is hyper's cancellation of a call because its connection ended before the
/// call was handed to it ("operation was canceled" caused by "connection closed" or
/// "connection was not ready"). A call cancelled by Blaze (a deadline, an interrupt, a dropped
/// future) has no such source.
///
/// PORT: hyper is not a dependency of this crate; its error is recognised by its `Display`
/// texts, fixed in the pinned hyper 1.x (`client/dispatch.rs` `Envelope::drop`,
/// `client/conn/http2.rs` `SendRequest::send_request`).
fn is_connection_closed_cancel(status: &tonic::Status) -> bool {
    let mut source = std::error::Error::source(status);
    while let Some(e) = source {
        if e.downcast_ref::<TransportIoException>().is_some()
            || e.downcast_ref::<TransportException>().is_some()
        {
            return false;
        }
        if e.to_string() == "operation was canceled"
            && let Some(cause) = e.source()
        {
            let cause = cause.to_string();
            return cause == "connection closed" || cause == "connection was not ready";
        }
        source = e.source();
    }
    false
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
    /// The channel's transport lifecycle (see [`transport_status`]).
    lifecycle: Option<Arc<TransportLifecycle>>,
    inner: tonic::Streaming<Resp>,
    listeners: Listeners,
    deadline: Option<tokio::time::Instant>,
    idle_timeout: Option<Duration>,
    method: Method,
    /// Cancels the call when the stream is dropped (see
    /// [`ReferenceCountedChannel::cancellable_request`]).
    _cancel: CancelOnDrop,
}

/// Cancels a call's request side when dropped (`ClientCall.cancel`).
struct CancelOnDrop(Option<tonic::CancellationHandle>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            // A no-op for a request side that has already ended.
            handle.cancel();
        }
    }
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
        .map_err(|status| transport_status(status, &self.authority, self.lifecycle.as_deref()));
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

    /// A channel made by `google_auth_utils::new_channel` (with the transport lifecycle) for
    /// `target`, and the status of a `GetCapabilities` call on it.
    fn new_channel_get_capabilities_status(
        target: &str,
        options: &blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions,
    ) -> tonic::Status {
        let managed =
            crate::authandtls::google_auth_utils::new_channel(target, None, options, None, None)
                .expect("new_channel");
        assert!(managed.transport_lifecycle.is_some());
        let channel = ReferenceCountedChannel::new(
            managed.channel,
            &managed.authority,
            managed.tls,
            managed.interceptors,
            managed.max_inbound_message_size,
            None,
        )
        .with_transport_lifecycle(managed.transport_lifecycle);
        crate::runtime::block_on(channel.unary::<GetCapabilitiesRequest, ServerCapabilities>(
            methods::GET_CAPABILITIES,
            GetCapabilitiesRequest::default(),
            CallOptions::new(),
        ))
        .unwrap_err()
    }

    /// Oracle: "Failed to query remote execution capabilities: INTERNAL: http2 exception" for a
    /// gRPC call to an HTTP/1 server (row http-cache); the cause is not an `IOException`.
    /// Netty's first failure is the first frame, which is not SETTINGS
    /// (`PrefaceDecoder.verifyFirstFrameIsSettings`), whether the request's write or the read of
    /// the reply comes first here (review 2026-11-19 round 3, P5).
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
        let status = new_channel_get_capabilities_status(
            &format!("grpc://127.0.0.1:{port}"),
            &Default::default(),
        );
        assert_eq!(status.code(), tonic::Code::Internal, "{status:?}");
        assert_eq!(status.message(), "http2 exception");
        assert!(transport_io_cause(&status).is_none());
        server.join().unwrap();
    }

    /// A peer that resets the connection after reading the client's preface, without sending
    /// anything: the connection's first failure is an `IOException` (the read's
    /// ECONNRESET, or the refused write when the reset is noticed there first), so the call is
    /// UNAVAILABLE "io exception" with that cause.
    // Java: io/grpc/netty/Utils.java:354 Utils.statusFromThrowable
    #[test]
    fn plain_peer_reset_is_unavailable_io_exception() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::Read;
            use std::os::fd::AsRawFd;
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 24];
            let _ = socket.read_exact(&mut buf);
            let linger = libc::linger {
                l_onoff: 1,
                l_linger: 0,
            };
            // SAFETY: setsockopt on a valid socket with a valid `linger`.
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_LINGER,
                    (&raw const linger).cast(),
                    std::mem::size_of::<libc::linger>() as libc::socklen_t,
                );
            }
        });
        let status = new_channel_get_capabilities_status(
            &format!("grpc://127.0.0.1:{port}"),
            &Default::default(),
        );
        server.join().unwrap();
        assert_eq!(status.code(), tonic::Code::Unavailable, "{status:?}");
        assert_eq!(status.message(), "io exception", "{status:?}");
        assert!(transport_io_cause(&status).is_some(), "{status:?}");
    }

    /// An error with a fixed text and an optional source (hyper's and tonic's errors, which are
    /// recognised by their texts).
    #[derive(Debug)]
    struct TextError(&'static str, Option<Box<TextError>>);

    impl std::fmt::Display for TextError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for TextError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|e| e as _)
        }
    }

    /// A call that hyper cancelled because its connection closed gets the connection's
    /// shutdown status; a call cancelled by Blaze (or by anything else than its connection)
    /// keeps its CANCELLED status, even when the connection failed.
    #[test]
    fn only_a_call_cancelled_by_its_connection_gets_the_shutdown_status() {
        let lifecycle = TransportLifecycle::new();
        let connection = Arc::new(ConnectionLifecycle::default());
        lifecycle.adopt(connection.clone());
        connection.notify_shutdown(ShutdownCause::Io(Arc::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "received fatal alert: CertificateRequired",
        ))));
        // The first failure wins.
        connection.notify_shutdown(ShutdownCause::Io(Arc::new(io::Error::from(
            io::ErrorKind::BrokenPipe,
        ))));

        let canceled = |cause: &'static str| {
            let mut status = tonic::Status::cancelled("operation was canceled");
            status.set_source(Arc::new(TextError(
                "transport error",
                Some(Box::new(TextError(
                    "operation was canceled",
                    Some(Box::new(TextError(cause, None))),
                ))),
            )));
            status
        };
        let mapped = transport_status(
            canceled("connection closed"),
            "localhost:1",
            Some(&lifecycle),
        );
        assert_eq!(mapped.code(), tonic::Code::Unavailable);
        assert_eq!(mapped.message(), "ssl exception");
        let mapped = transport_status(
            canceled("connection was not ready"),
            "localhost:1",
            Some(&lifecycle),
        );
        assert_eq!(mapped.message(), "ssl exception");

        // Cancelled for another reason, or by the caller (no transport source at all).
        let other = transport_status(canceled("user dropped"), "localhost:1", Some(&lifecycle));
        assert_eq!(other.code(), tonic::Code::Cancelled);
        assert_eq!(other.message(), "operation was canceled");
        let by_caller = transport_status(
            tonic::Status::cancelled("Timeout expired"),
            "localhost:1",
            Some(&lifecycle),
        );
        assert_eq!(by_caller.code(), tonic::Code::Cancelled);
        assert_eq!(by_caller.message(), "Timeout expired");
        // Without a recorded failure the cancellation is kept.
        let no_failure = TransportLifecycle::new();
        let kept = transport_status(
            canceled("connection closed"),
            "localhost:1",
            Some(&no_failure),
        );
        assert_eq!(kept.code(), tonic::Code::Cancelled);
    }

    /// A call cancelled by Blaze itself (its deadline) while the server does not answer keeps
    /// its own status (CANCELLED from tonic's timeout, or DEADLINE_EXCEEDED).
    #[test]
    fn call_cancelled_by_its_deadline_keeps_its_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            drop(socket);
        });
        let managed = crate::authandtls::google_auth_utils::new_channel(
            &format!("grpc://127.0.0.1:{port}"),
            None,
            &Default::default(),
            None,
            None,
        )
        .unwrap();
        let channel = ReferenceCountedChannel::new(
            managed.channel,
            &managed.authority,
            managed.tls,
            managed.interceptors,
            managed.max_inbound_message_size,
            None,
        )
        .with_transport_lifecycle(managed.transport_lifecycle);
        let status =
            crate::runtime::block_on(channel.unary::<GetCapabilitiesRequest, ServerCapabilities>(
                methods::GET_CAPABILITIES,
                GetCapabilitiesRequest::default(),
                CallOptions::new().with_deadline_after(Duration::from_millis(300)),
            ))
            .unwrap_err();
        // The deadline fires either in tonic (its `grpc-timeout` layer: CANCELLED "Timeout
        // expired") or in `with_deadline` (DEADLINE_EXCEEDED); neither is the connection's.
        assert!(
            matches!(
                (status.code(), status.message()),
                (tonic::Code::Cancelled, "Timeout expired") | (tonic::Code::DeadlineExceeded, _)
            ),
            "{status:?}"
        );
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

    /// Oracle: "Failed to query remote execution capabilities: UNAVAILABLE: ssl exception" for a
    /// TLS server that requires a client certificate, the client having none
    /// (scripts/remote/tls-probe row mtls-noclient). In TLS 1.3 the server judges the client's
    /// certificate after the client's Finished: the client's handshake has succeeded and the
    /// server's `certificate_required` alert arrives on the established connection, as an I/O
    /// error of the HTTP/2 connection (review 2026-11-19 round 3, P5). The certificates are
    /// tests/testdata/mtls (a CA and a server certificate for localhost and 127.0.0.1 signed by
    /// it). The target is 127.0.0.1: `localhost` would try ::1 first, where another process may
    /// listen on the same port number.
    #[test]
    fn tls_alert_after_the_handshake_is_unavailable_ssl_exception() {
        use crate::grpc::stubs::capabilities_server::{Capabilities, CapabilitiesServer};
        use tonic::transport::{Certificate, Identity, ServerTlsConfig};

        struct NoCapabilities;
        #[tonic::async_trait]
        impl Capabilities for NoCapabilities {
            async fn get_capabilities(
                &self,
                _request: tonic::Request<GetCapabilitiesRequest>,
            ) -> Result<tonic::Response<ServerCapabilities>, tonic::Status> {
                Ok(tonic::Response::new(ServerCapabilities::default()))
            }
        }

        let data = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/mtls");
        let read = |name: &str| std::fs::read(format!("{data}/{name}")).unwrap();
        let server_tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(read("server.crt"), read("server.key")))
            .client_ca_root(Certificate::from_pem(read("ca.crt")));
        let listener =
            crate::runtime::block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = crate::runtime::runtime().spawn(async move {
            tonic::transport::Server::builder()
                .tls_config(server_tls)
                .expect("server TLS")
                .serve_with_incoming(
                    CapabilitiesServer::new(NoCapabilities),
                    tonic::transport::server::TcpIncoming::from(listener),
                )
                .await
        });

        let options = blaze_optdefs::misc::auth_and_tls_options::AuthAndTLSOptions {
            tls_certificate: Some(format!("{data}/ca.crt")),
            ..Default::default()
        };
        let status =
            new_channel_get_capabilities_status(&format!("grpcs://127.0.0.1:{port}"), &options);
        server.abort();
        assert_eq!(status.code(), tonic::Code::Unavailable, "{status:?}");
        assert_eq!(status.message(), "ssl exception", "{status:?}");
        assert!(transport_io_cause(&status).is_none());
    }

    #[test]
    fn is_tls_alert_matches_rustls_alert_text_only() {
        assert!(is_tls_alert("received fatal alert: CertificateRequired"));
        assert!(is_tls_alert("received fatal alert: HandshakeFailure"));
        assert!(!is_tls_alert("invalid peer certificate: UnknownIssuer"));
        assert!(!is_tls_alert("Connection reset by peer (os error 104)"));
    }
}
