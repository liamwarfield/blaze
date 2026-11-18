#![cfg(feature = "wip")]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/ByteStreamUploaderTest.java`:
//! tests for `ByteStreamUploader`.
//!
//! PORT: Java runs the fake `ByteStream` service on gRPC's in-process transport; here it is a
//! tonic server on `127.0.0.1:0` and the uploader talks to it over HTTP/2. The Java service
//! classes (`ByteStreamImplBase` with its `StreamObserver`s) are kept as the small observer API
//! at the top of this file, so that the test bodies read like the Java ones.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read};
use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use blaze_remote::authandtls::call_credentials_provider::{
    CallCredentialsProvider, NO_CREDENTIALS,
};
use blaze_remote::byte_stream_uploader::ByteStreamUploader;
use blaze_remote::chunker::Chunker;
use blaze_remote::common::errors::{self, StatusRuntimeException};
use blaze_remote::common::{Blob, RemoteActionExecutionContext};
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc::channel::testing::channel_to;
use blaze_remote::grpc::stubs::byte_stream_server::{ByteStream, ByteStreamServer};
use blaze_remote::grpc::{
    AttachHeadersInterceptor, CallCredentials, CallListener, ClientInterceptor, Method,
    ReferenceCountedChannel, methods,
};
use blaze_remote::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse,
};
use blaze_remote::reapi::{Digest, RequestMetadata, ServerCapabilities, digest_function};
use blaze_remote::remote_retrier::{EXPERIMENTAL_GRPC_RESULT_CLASSIFIER, RemoteRetrier};
use blaze_remote::remote_retrier_utils;
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifier,
    ResultClassifierResult, RetryScheduler, TokioRetryScheduler,
};
use blaze_remote::runtime::{block_on, get_from_future, runtime};
use blaze_remote::util::digest_util::DigestUtil;
use blaze_remote::util::tracing_metadata_utils::{self, METADATA_KEY};
use blaze_remote::util::utils::wait_for_bulk_transfer;
use blaze_vfs::DigestHashFunction;
use parking_lot::{Condvar, Mutex};
use prost::Message as _;
use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status, Streaming};

static DIGEST_UTIL: LazyLock<DigestUtil> =
    LazyLock::new(|| DigestUtil::new(DigestHashFunction::Sha256));

const CHUNK_SIZE: i32 = 10;
const INSTANCE_NAME: &str = "foo";

/// `BlazeVersionInfo.instance().getVersion()` of a test run (no build label).
const TOOL_VERSION: &str = "";

// ---------------------------------------------------------------------------------------------
// java.util.Random
// ---------------------------------------------------------------------------------------------

/// `java.util.Random` (the tests draw their blobs from `new Random()`).
struct Random {
    seed: u64,
}

impl Random {
    const MULTIPLIER: u64 = 0x5DEECE66D;
    const MASK: u64 = (1 << 48) - 1;

    /// `new Random()`: a seed very likely to be distinct from any other invocation.
    fn new() -> Random {
        let seed = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        Random {
            seed: (seed ^ Random::MULTIPLIER) & Random::MASK,
        }
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(Random::MULTIPLIER).wrapping_add(0xB)) & Random::MASK;
        (self.seed >> (48 - bits)) as u32 as i32
    }

    /// `nextInt(bound)`.
    fn next_int(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            // i.e., bound is a power of 2
            r = ((bound as i64 * r as i64) >> 31) as i32;
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    break;
                }
                u = self.next(31);
            }
        }
        r
    }

    /// `nextBytes(bytes)`.
    fn next_bytes(&mut self, bytes: &mut [u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let mut rnd = self.next(32);
            let mut n = std::cmp::min(len - i, 4);
            while n > 0 {
                bytes[i] = rnd as u8;
                i += 1;
                rnd >>= 8;
                n -= 1;
            }
        }
    }
}

/// `byte[] blob = new byte[size]; new Random().nextBytes(blob);`
fn random_blob(size: usize) -> Vec<u8> {
    let mut blob = vec![0u8; size];
    Random::new().next_bytes(&mut blob);
    blob
}

// ---------------------------------------------------------------------------------------------
// The fake ByteStream service (Java: ByteStreamImplBase, StreamObserver, MutableHandlerRegistry)
// ---------------------------------------------------------------------------------------------

/// The server's `StreamObserver<WriteResponse>`.
#[derive(Default)]
struct WriteResponseObserver {
    response: Option<WriteResponse>,
    closed: Option<Result<(), Status>>,
}

impl WriteResponseObserver {
    fn on_next(&mut self, response: WriteResponse) {
        self.response = Some(response);
    }

    fn on_completed(&mut self) {
        if self.closed.is_none() {
            self.closed = Some(Ok(()));
        }
    }

    fn on_error(&mut self, status: Status) {
        if self.closed.is_none() {
            self.closed = Some(Err(status));
        }
    }

    /// What the call ends with, once the observer was completed or failed.
    fn result(&self) -> Option<Result<Response<WriteResponse>, Status>> {
        match &self.closed {
            None => None,
            Some(Err(status)) => Some(Err(status.clone())),
            Some(Ok(())) => Some(match self.response {
                Some(response) => Ok(Response::new(response)),
                None => Err(Status::internal("Completed without a response")),
            }),
        }
    }
}

/// The server's `StreamObserver<WriteRequest>`.
trait WriteRequestObserver: Send {
    fn on_next(&mut self, write_request: WriteRequest, stream_observer: &mut WriteResponseObserver);
    fn on_error(&mut self, throwable: &Status);
    fn on_completed(&mut self, stream_observer: &mut WriteResponseObserver);
}

struct FnObserver<S, N, E, C> {
    state: S,
    on_next: N,
    on_error: E,
    on_completed: C,
}

impl<S, N, E, C> WriteRequestObserver for FnObserver<S, N, E, C>
where
    S: Send,
    N: FnMut(&mut S, WriteRequest, &mut WriteResponseObserver) + Send,
    E: FnMut(&mut S, &Status) + Send,
    C: FnMut(&mut S, &mut WriteResponseObserver) + Send,
{
    fn on_next(
        &mut self,
        write_request: WriteRequest,
        stream_observer: &mut WriteResponseObserver,
    ) {
        (self.on_next)(&mut self.state, write_request, stream_observer)
    }

    fn on_error(&mut self, throwable: &Status) {
        (self.on_error)(&mut self.state, throwable)
    }

    fn on_completed(&mut self, stream_observer: &mut WriteResponseObserver) {
        (self.on_completed)(&mut self.state, stream_observer)
    }
}

/// `new StreamObserver<WriteRequest>() { <fields>; onNext; onError; onCompleted }`: `state`
/// holds the fields of the anonymous class.
fn stream_observer<S>(
    state: S,
    on_next: impl FnMut(&mut S, WriteRequest, &mut WriteResponseObserver) + Send + 'static,
    on_error: impl FnMut(&mut S, &Status) + Send + 'static,
    on_completed: impl FnMut(&mut S, &mut WriteResponseObserver) + Send + 'static,
) -> Box<dyn WriteRequestObserver>
where
    S: Send + 'static,
{
    Box::new(FnObserver {
        state,
        on_next,
        on_error,
        on_completed,
    })
}

/// `fail("onError should never be called.")`.
fn fail_on_error<S>(_state: &mut S, _throwable: &Status) {
    panic!("onError should never be called.");
}

/// `new NoopStreamObserver()`.
fn noop_stream_observer() -> Box<dyn WriteRequestObserver> {
    stream_observer((), |_, _, _| {}, |_, _| {}, |_, _| {})
}

type WriteMethod = Box<
    dyn Fn(&MetadataMap, &mut WriteResponseObserver) -> Box<dyn WriteRequestObserver> + Send + Sync,
>;
type QueryWriteStatusMethod =
    Box<dyn Fn(QueryWriteStatusRequest) -> Result<QueryWriteStatusResponse, Status> + Send + Sync>;

/// `ByteStreamImplBase`: a service whose methods are unimplemented until overridden.
///
/// PORT: `write` also receives the request headers, which the Java tests read through server
/// interceptors.
struct ByteStreamImplBase {
    write: WriteMethod,
    query_write_status: QueryWriteStatusMethod,
}

impl ByteStreamImplBase {
    fn new() -> ByteStreamImplBase {
        ByteStreamImplBase {
            // io.grpc.stub.ServerCalls.asyncUnimplementedStreamingCall
            write: Box::new(|_headers, stream_observer| {
                stream_observer.on_error(Status::unimplemented(
                    "Method google.bytestream.ByteStream/Write is unimplemented",
                ));
                noop_stream_observer()
            }),
            // io.grpc.stub.ServerCalls.asyncUnimplementedUnaryCall
            query_write_status: Box::new(|_request| {
                Err(Status::unimplemented(
                    "Method google.bytestream.ByteStream/QueryWriteStatus is unimplemented",
                ))
            }),
        }
    }

    /// `@Override public StreamObserver<WriteRequest> write(StreamObserver<WriteResponse>)`.
    fn write(
        mut self,
        write: impl Fn(&MetadataMap, &mut WriteResponseObserver) -> Box<dyn WriteRequestObserver>
        + Send
        + Sync
        + 'static,
    ) -> ByteStreamImplBase {
        self.write = Box::new(write);
        self
    }

    /// `@Override public void queryWriteStatus(request, response)`.
    fn query_write_status(
        mut self,
        query_write_status: impl Fn(QueryWriteStatusRequest) -> Result<QueryWriteStatusResponse, Status>
        + Send
        + Sync
        + 'static,
    ) -> ByteStreamImplBase {
        self.query_write_status = Box::new(query_write_status);
        self
    }
}

/// `MutableHandlerRegistry`: the service the fake server currently runs, plus the assertion
/// failures of its handlers (Java: an exception thrown by a handler on a server thread).
#[derive(Default)]
struct MutableHandlerRegistry {
    service: Mutex<Option<Arc<ByteStreamImplBase>>>,
    failures: Mutex<Vec<String>>,
}

impl MutableHandlerRegistry {
    fn add_service(&self, service: ByteStreamImplBase) {
        *self.service.lock() = Some(Arc::new(service));
    }

    fn service(&self) -> Result<Arc<ByteStreamImplBase>, Status> {
        self.service
            .lock()
            .clone()
            .ok_or_else(|| Status::unimplemented("Method not found"))
    }

    /// Runs a handler; a failed assertion in it is recorded and closes the call with UNKNOWN,
    /// as gRPC does for an exception thrown by a service method.
    fn guarded<T>(&self, handler: impl FnOnce() -> T) -> Result<T, Status> {
        match catch_unwind(AssertUnwindSafe(handler)) {
            Ok(value) => Ok(value),
            Err(payload) => {
                let message = if let Some(s) = payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "handler panicked".to_string()
                };
                self.failures.lock().push(message);
                Err(Status::unknown("Application error processing RPC"))
            }
        }
    }
}

/// The generated service trait over the registry.
struct RegisteredByteStream(Arc<MutableHandlerRegistry>);

#[async_trait]
impl ByteStream for RegisteredByteStream {
    type ReadStream = futures::stream::Empty<Result<ReadResponse, Status>>;

    async fn read(
        &self,
        _request: Request<ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        Err(Status::unimplemented(
            "Method google.bytestream.ByteStream/Read is unimplemented",
        ))
    }

    async fn write(
        &self,
        request: Request<Streaming<WriteRequest>>,
    ) -> Result<Response<WriteResponse>, Status> {
        let registry = &self.0;
        let service = registry.service()?;
        let (headers, _extensions, mut requests) = request.into_parts();
        let mut stream_observer = WriteResponseObserver::default();
        let mut request_observer =
            registry.guarded(|| (service.write)(&headers, &mut stream_observer))?;
        loop {
            // Once the response observer is closed the call is over: later requests of the
            // client are not delivered.
            if let Some(result) = stream_observer.result() {
                return result;
            }
            match requests.message().await {
                Ok(Some(write_request)) => {
                    registry.guarded(|| {
                        request_observer.on_next(write_request, &mut stream_observer)
                    })?;
                }
                Ok(None) => {
                    registry.guarded(|| request_observer.on_completed(&mut stream_observer))?;
                    return match stream_observer.result() {
                        Some(result) => result,
                        // The service never answers: the call stays open.
                        None => std::future::pending().await,
                    };
                }
                Err(status) => {
                    // The client cancelled the call. What `onError` throws on a cancelled call
                    // reaches nobody in Java either.
                    let _ = catch_unwind(AssertUnwindSafe(|| request_observer.on_error(&status)));
                    return Err(status);
                }
            }
        }
    }

    async fn query_write_status(
        &self,
        request: Request<QueryWriteStatusRequest>,
    ) -> Result<Response<QueryWriteStatusResponse>, Status> {
        let registry = &self.0;
        let service = registry.service()?;
        let request = request.into_inner();
        registry
            .guarded(|| (service.query_write_status)(request))?
            .map(Response::new)
    }
}

/// The fake server (`InProcessServerBuilder...build().start()`); dropping it shuts it down.
struct Server {
    addr: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

fn start_server(service_registry: Arc<MutableHandlerRegistry>) -> Server {
    let listener = block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).expect("bind");
    let addr = listener.local_addr().expect("local address");
    let (shutdown, signal) = tokio::sync::oneshot::channel::<()>();
    runtime().spawn(async move {
        let _ = tonic::transport::Server::builder()
            .serve_with_incoming_shutdown(
                ByteStreamServer::new(RegisteredByteStream(service_registry)),
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

/// `TracingMetadataUtils.ServerHeadersInterceptor` followed by
/// `TracingMetadataUtils.fromCurrentContext()`.
fn request_metadata_from_headers(
    headers: &MetadataMap,
    full_method_name: &str,
) -> Result<RequestMetadata, Status> {
    let meta = headers
        .get_bin(METADATA_KEY)
        .and_then(|value| value.to_bytes().ok())
        .and_then(|bytes| RequestMetadata::decode(bytes).ok());
    meta.ok_or_else(|| {
        Status::invalid_argument(format!(
            "RequestMetadata not received from the client for {full_method_name}"
        ))
    })
}

// ---------------------------------------------------------------------------------------------
// Client side helpers
// ---------------------------------------------------------------------------------------------

/// Counts the `Write` calls that have ended on the client.
///
/// PORT: the in-process transport of the Java tests runs the service's `write()` (and with it an
/// immediate answer) before the client's first `onReady`. Over TCP the two race, so the tests
/// that rely on that order hold the client's input ([`BlobAfterWriteCall`]) until the call has
/// ended.
#[derive(Default)]
struct WriteCalls {
    closed: Mutex<i32>,
    changed: Condvar,
}

impl WriteCalls {
    fn await_closed(&self, count: i32) {
        let mut closed = self.closed.lock();
        while *closed < count {
            let timed_out = self
                .changed
                .wait_for(&mut closed, Duration::from_secs(60))
                .timed_out();
            assert!(!timed_out, "the Write call did not end");
        }
    }
}

struct WriteCallsInterceptor(Arc<WriteCalls>);

struct WriteCallListener(Arc<WriteCalls>);

impl ClientInterceptor for WriteCallsInterceptor {
    fn intercept_call(
        &self,
        method: &Method,
        _headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>> {
        if *method == methods::WRITE {
            Some(Box::new(WriteCallListener(self.0.clone())))
        } else {
            None
        }
    }
}

impl CallListener for WriteCallListener {
    fn on_close(&mut self, _status: &Status) {
        *self.0.closed.lock() += 1;
        self.0.changed.notify_all();
    }
}

/// An input that can only be opened once the first `Write` call has ended (see [`WriteCalls`]).
struct BlobAfterWriteCall {
    data: Vec<u8>,
    write_calls: Arc<WriteCalls>,
}

impl Blob for BlobAfterWriteCall {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        self.write_calls.await_closed(1);
        // The listener runs just before the call hands its result to the uploader.
        std::thread::sleep(Duration::from_millis(100));
        Ok(Box::new(io::Cursor::new(&self.data[..])))
    }
}

/// `CallCredentialsProvider.NO_CREDENTIALS`.
fn no_credentials() -> Arc<dyn CallCredentialsProvider> {
    NO_CREDENTIALS.clone()
}

/// The `CallCredentialsProvider` of the authentication tests: no credentials, counts refreshes.
struct CountingCallCredentialsProvider {
    refresh_times: Arc<AtomicI32>,
}

impl CallCredentialsProvider for CountingCallCredentialsProvider {
    fn get_call_credentials(&self) -> Option<Arc<dyn CallCredentials>> {
        None
    }

    fn refresh(&self) -> io::Result<()> {
        self.refresh_times.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// `@Mock Retrier.Backoff mockBackoff`: answers `0` to everything and records its invocations.
#[derive(Default)]
struct MockBackoff {
    /// One entry per `nextDelayMillis(e)`: whether `e` is a `StatusRuntimeException`.
    next_delay_millis: Mutex<Vec<bool>>,
    get_retry_attempts: AtomicI32,
}

impl Backoff for MockBackoff {
    fn next_delay_millis(&self, e: &io::Error) -> i64 {
        let mut calls = self.next_delay_millis.lock();
        calls.push(errors::downcast_ref::<StatusRuntimeException>(e).is_some());
        // A mock never stops the retries; a test that went wrong must still end.
        assert!(
            calls.len() < 1000,
            "mockBackoff: the upload is retried forever"
        );
        0
    }

    fn get_retry_attempts(&self) -> i32 {
        self.get_retry_attempts.fetch_add(1, Ordering::SeqCst);
        0
    }
}

impl MockBackoff {
    /// `Mockito.verifyNoInteractions(mockBackoff)`.
    fn verify_no_interactions(&self) {
        assert_eq!(self.next_delay_millis.lock().len(), 0);
        assert_eq!(self.get_retry_attempts.load(Ordering::SeqCst), 0);
    }

    /// The number of `nextDelayMillis(any(Exception.class))` invocations.
    fn next_delay_millis_times(&self) -> usize {
        self.next_delay_millis.lock().len()
    }

    /// The number of `nextDelayMillis(any(StatusRuntimeException.class))` invocations.
    fn next_delay_millis_with_status_runtime_exception_times(&self) -> usize {
        self.next_delay_millis.lock().iter().filter(|b| **b).count()
    }

    /// The number of `getRetryAttempts()` invocations.
    fn get_retry_attempts_times(&self) -> i32 {
        self.get_retry_attempts.load(Ordering::SeqCst)
    }
}

/// `ByteStreamUploaderTest.FixedBackoff`.
struct FixedBackoff {
    max_retries: i32,
    delay_millis: i32,

    retries: Mutex<i32>,
}

impl FixedBackoff {
    fn new(max_retries: i32, delay_millis: i32) -> FixedBackoff {
        FixedBackoff {
            max_retries,
            delay_millis,
            retries: Mutex::new(0),
        }
    }
}

impl Backoff for FixedBackoff {
    fn next_delay_millis(&self, _e: &io::Error) -> i64 {
        let mut retries = self.retries.lock();
        if *retries < self.max_retries {
            *retries += 1;
            return self.delay_millis as i64;
        }
        -1
    }

    fn get_retry_attempts(&self) -> i32 {
        *self.retries.lock()
    }
}

/// `() -> new FixedBackoff(maxRetries, delayMillis)`.
fn fixed_backoff(max_retries: i32, delay_millis: i32) -> BackoffSupplier {
    Arc::new(move || -> Arc<dyn Backoff> { Arc::new(FixedBackoff::new(max_retries, delay_millis)) })
}

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`: schedules all tasks with zero delay.
struct ZeroDelayListeningScheduledExecutorService {
    delegate: Arc<dyn RetryScheduler>,
}

#[async_trait]
impl RetryScheduler for ZeroDelayListeningScheduledExecutorService {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        self.delegate.schedule(0).await
    }
}

/// A retry service after `shutdownNow()`.
struct ShutdownRetryService;

#[async_trait]
impl RetryScheduler for ShutdownRetryService {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Err(RejectedExecutionException::default())
    }
}

/// `TestUtils.newRemoteRetrier(backoff, resultClassifier, retryScheduler)`.
fn new_remote_retrier(
    backoff: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
    retry_scheduler: Arc<dyn RetryScheduler>,
) -> Arc<RemoteRetrier> {
    let zero_delay_retry_scheduler: Arc<dyn RetryScheduler> =
        Arc::new(ZeroDelayListeningScheduledExecutorService {
            delegate: retry_scheduler,
        });
    Arc::new(RemoteRetrier::with_sleeper(
        backoff,
        Arc::new(move |e: &io::Error| {
            if errors::status_from_throwable(e).map(|status| status.code())
                == Some(tonic::Code::Cancelled)
            {
                ResultClassifierResult::Success
            } else {
                result_classifier.test(e)
            }
        }),
        zero_delay_retry_scheduler,
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> {
            /* don't wait in tests */
            Ok(())
        }),
    ))
}

/// `(e) -> Result.TRANSIENT_FAILURE`.
fn transient_failure() -> Arc<dyn ResultClassifier> {
    Arc::new(|_e: &io::Error| ResultClassifierResult::TransientFailure)
}

/// `e -> Result.PERMANENT_FAILURE`.
fn permanent_failure() -> Arc<dyn ResultClassifier> {
    Arc::new(|_e: &io::Error| ResultClassifierResult::PermanentFailure)
}

/// The fields of the Java test class, as `setUp()` leaves them.
struct ByteStreamUploaderTest {
    service_registry: Arc<MutableHandlerRegistry>,
    retry_service: Arc<dyn RetryScheduler>,

    server: Server,
    reference_counted_channel: ReferenceCountedChannel,
    context: RemoteActionExecutionContext<'static>,

    mock_backoff: Arc<MockBackoff>,

    /// PORT: see [`WriteCalls`].
    write_calls: Arc<WriteCalls>,
}

/// `Single.just(ServerCapabilities.getDefaultInstance())`.
fn default_server_capabilities() -> CapabilitiesFetcher {
    Arc::new(|_channel| Box::pin(async { Ok::<_, io::Error>(ServerCapabilities::default()) }))
}

impl ByteStreamUploaderTest {
    /// `setUp()`.
    fn set_up() -> ByteStreamUploaderTest {
        tracing_metadata_utils::set_tool_version(TOOL_VERSION);

        let service_registry = Arc::new(MutableHandlerRegistry::default());
        let server = start_server(service_registry.clone());
        let write_calls = Arc::new(WriteCalls::default());
        let interceptors: Vec<Arc<dyn ClientInterceptor>> =
            vec![Arc::new(WriteCallsInterceptor(write_calls.clone()))];
        let reference_counted_channel = channel_to(
            server.addr,
            interceptors,
            Some(default_server_capabilities()),
        );
        let metadata = tracing_metadata_utils::build_metadata(
            "none",
            "none",
            &DIGEST_UTIL
                .as_action_key(Digest::default())
                .digest()
                .hash
                .clone(),
            None,
        );
        let context = RemoteActionExecutionContext::create(metadata);

        let retry_service: Arc<dyn RetryScheduler> = Arc::new(TokioRetryScheduler);

        ByteStreamUploaderTest {
            service_registry,
            retry_service,
            server,
            reference_counted_channel,
            context,
            mock_backoff: Arc::new(MockBackoff::default()),
            write_calls,
        }
    }

    /// `() -> mockBackoff`.
    fn mock_backoff_supplier(&self) -> BackoffSupplier {
        let mock_backoff = self.mock_backoff.clone();
        Arc::new(move || -> Arc<dyn Backoff> { mock_backoff.clone() })
    }
}

impl Drop for ByteStreamUploaderTest {
    /// `tearDown()`.
    fn drop(&mut self) {
        self.reference_counted_channel.release();
        let failures = self.service_registry.failures.lock().clone();
        if std::thread::panicking() {
            if !failures.is_empty() {
                eprintln!("assertions failed in the fake ByteStream service: {failures:?}");
            }
        } else {
            assert!(
                failures.is_empty(),
                "assertions failed in the fake ByteStream service: {failures:?}"
            );
        }
    }
}

/// Uploads a BLOB, as provided by the [`Chunker`], to the remote `ByteStream` service. The call
/// blocks until the upload is complete, or returns an error.
///
/// Uploads are retried according to the specified [`RemoteRetrier`]. Retrying is transparent to
/// the user of this API.
fn upload_blob(
    byte_stream_uploader: &ByteStreamUploader,
    context: &RemoteActionExecutionContext<'_>,
    digest: &Digest,
    chunker: Chunker<'_>,
) -> io::Result<()> {
    get_from_future(byte_stream_uploader.upload_blob_async(context, digest, chunker))
}

/// Uploads a list of BLOBs concurrently to the remote `ByteStream` service. The call blocks
/// until the upload of all BLOBs is complete, or returns a `BulkTransferException` if there are
/// errors.
fn upload_blobs(
    byte_stream_uploader: &ByteStreamUploader,
    context: &RemoteActionExecutionContext<'_>,
    chunkers: Vec<(Digest, Chunker<'_>)>,
) -> io::Result<()> {
    let uploads = chunkers.into_iter().map(|(digest, chunker)| async move {
        byte_stream_uploader
            .upload_blob_async(context, &digest, chunker)
            .await
    });

    wait_for_bulk_transfer(uploads)
}

/// `TestUtils.newNoErrorByteStreamService(blob)`.
fn new_no_error_byte_stream_service(blob: Vec<u8>) -> ByteStreamImplBase {
    let blob = Arc::new(blob);
    ByteStreamImplBase::new().write(move |_headers, _stream_observer| {
        struct State {
            received_data: Vec<u8>,
            next_offset: i64,
        }
        let blob_for_next = blob.clone();
        let blob_for_completed = blob.clone();
        stream_observer(
            State {
                received_data: vec![0u8; blob.len()],
                next_offset: 0,
            },
            move |s, write_request, _stream_observer| {
                let blob = &blob_for_next;
                if s.next_offset == 0 {
                    assert!(!write_request.resource_name.is_empty());
                    assert!(
                        write_request
                            .resource_name
                            .ends_with(&blob.len().to_string())
                    );
                } else {
                    assert!(write_request.resource_name.is_empty());
                }

                assert_eq!(write_request.write_offset, s.next_offset);

                let data = &write_request.data;

                let start = s.next_offset as usize;
                s.received_data[start..start + data.len()].copy_from_slice(data);

                s.next_offset += data.len() as i64;
                let last_write = blob.len() as i64 == s.next_offset;
                assert_eq!(write_request.finish_write, last_write);
            },
            fail_on_error,
            move |s, stream_observer| {
                let blob = &blob_for_completed;
                assert_eq!(s.next_offset, blob.len() as i64);
                assert_eq!(&s.received_data, &**blob);

                let response = WriteResponse {
                    committed_size: s.next_offset,
                };
                stream_observer.on_next(response);
                stream_observer.on_completed();
            },
        )
    })
}

/// An byte stream service where an upload for a given blob may or may not fail on the first
/// attempt but is guaranteed to succeed on the second try.
fn maybe_fail_once_upload_service(blobs_by_hash: HashMap<String, Vec<u8>>) -> ByteStreamImplBase {
    let blobs_by_hash = Arc::new(blobs_by_hash);
    let uploads_failed_once: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let rand = Arc::new(Mutex::new(Random::new()));

    ByteStreamImplBase::new()
        .write(move |_headers, _response| {
            #[derive(Default)]
            struct State {
                digest_hash: Option<String>,
                received_data: Vec<u8>,
                next_offset: i64,
                failed: bool,
            }
            let blobs_for_next = blobs_by_hash.clone();
            let blobs_for_completed = blobs_by_hash.clone();
            let uploads_failed_once = uploads_failed_once.clone();
            let rand = rand.clone();
            stream_observer(
                State::default(),
                move |s, write_request, response| {
                    let blobs_by_hash = &blobs_for_next;
                    if s.next_offset == 0 {
                        let resource_name = &write_request.resource_name;
                        assert!(!resource_name.is_empty());

                        let components: Vec<&str> = resource_name.split('/').collect();
                        assert_eq!(components.len(), 6);
                        s.digest_hash = Some(components[4].to_string());
                        assert!(blobs_by_hash.contains_key(components[4]));
                        s.received_data = vec![0u8; components[5].parse::<usize>().unwrap()];
                    }
                    let digest_hash = s.digest_hash.clone().expect("digestHash is not null");
                    // An upload for a given blob has a 10% chance to fail once during its
                    // lifetime. This is to exercise the retry mechanism a bit.
                    let should_fail = rand.lock().next_int(10) == 0
                        && !uploads_failed_once.lock().contains(&digest_hash);
                    if should_fail {
                        uploads_failed_once.lock().insert(digest_hash);
                        response.on_error(Status::internal(""));
                        s.failed = true;
                        return;
                    }

                    let data = &write_request.data;
                    let start = s.next_offset as usize;
                    s.received_data[start..start + data.len()].copy_from_slice(data);
                    s.next_offset += data.len() as i64;

                    let last_write = s.next_offset == s.received_data.len() as i64;
                    assert_eq!(write_request.finish_write, last_write);
                },
                fail_on_error,
                move |s, response| {
                    if s.failed {
                        return;
                    }
                    let expected_blob = blobs_for_completed
                        .get(s.digest_hash.as_ref().expect("digestHash is not null"))
                        .expect("the blob is known");
                    assert_eq!(&s.received_data, expected_blob);

                    let write_response = WriteResponse {
                        committed_size: s.received_data.len() as i64,
                    };

                    response.on_next(write_response);
                    response.on_completed();
                },
            )
        })
        .query_write_status(|_request| {
            // force the client to reset the write
            Ok(QueryWriteStatusResponse {
                committed_size: 0,
                complete: false,
            })
        })
}

/// `ByteStreamUploaderTest.CustomFileTracker`.
struct CustomFileTracker {
    open_files: AtomicI32,
    max_open_files: i32,
}

impl CustomFileTracker {
    fn new(max_open_files: i32) -> CustomFileTracker {
        CustomFileTracker {
            open_files: AtomicI32::new(0),
            max_open_files,
        }
    }

    fn increment_open_files(&self) {
        self.open_files.fetch_add(1, Ordering::SeqCst);
        assert!(self.open_files.load(Ordering::SeqCst) <= self.max_open_files);
    }

    fn decrement_open_files(&self) {
        self.open_files.fetch_sub(1, Ordering::SeqCst);
        assert!(self.open_files.load(Ordering::SeqCst) >= 0);
    }
}

/// The input of `TestChunker`: a custom Chunker used to track number of open files
/// (`() -> new TestByteArrayInputStream(existingData, customFileTracker)`).
struct TestChunkerInput {
    existing_data: Vec<u8>,
    custom_file_tracker: Arc<CustomFileTracker>,
}

impl Blob for TestChunkerInput {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(TestByteArrayInputStream::new(
            &self.existing_data,
            &self.custom_file_tracker,
        )))
    }
}

/// `ByteStreamUploaderTest.TestByteArrayInputStream`; `close()` is the drop.
struct TestByteArrayInputStream<'a> {
    buf: io::Cursor<&'a [u8]>,
    custom_file_tracker: &'a CustomFileTracker,
}

impl<'a> TestByteArrayInputStream<'a> {
    fn new(buf: &'a [u8], custom_file_tracker: &'a CustomFileTracker) -> Self {
        custom_file_tracker.increment_open_files();
        TestByteArrayInputStream {
            buf: io::Cursor::new(buf),
            custom_file_tracker,
        }
    }
}

impl Read for TestByteArrayInputStream<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.buf.read(b)
    }
}

impl Drop for TestByteArrayInputStream<'_> {
    fn drop(&mut self) {
        self.custom_file_tracker.decrement_open_files();
    }
}

/// The input of a mocked `Chunker` whose `seek(0)` throws `new IOException("Too many open
/// files")`.
struct TooManyOpenFiles;

impl Blob for TooManyOpenFiles {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Err(errors::io_exception("Too many open files"))
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[test]
fn single_blob_upload_should_work() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(new_no_error_byte_stream_service(blob));

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn single_chunk_compressed_upload_already_exists() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = vec![b'A'];

    // Set a chunk size that should have no problem accommodating the compressed
    // blob, even though the blob most likely has a compression ratio >= 1.
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_compressed(true)
        .set_chunk_size(100)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(|_headers, _stream_observer| {
                stream_observer(
                    /* num_chunks_received= */ 0,
                    |num_chunks_received: &mut i32, write_request, stream_observer| {
                        // This should be the first and only chunk written.
                        *num_chunks_received += 1;
                        assert_eq!(*num_chunks_received, 1);
                        let data = &write_request.data;
                        assert!(!data.is_empty());
                        assert!(write_request.finish_write);

                        // On receiving the chunk, respond with a committed size of -1
                        // to indicate that the blob already exists (per the remote API
                        // spec) and close the stream.
                        let response = WriteResponse { committed_size: -1 };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                    fail_on_error,
                    |_, _| {},
                )
            }),
        );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn progressive_upload_should_work() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = Arc::new(random_blob((CHUNK_SIZE * 2 + 1) as usize));

    let chunker = Chunker::builder()
        .set_input_bytes((*blob).clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    struct Service {
        received_data: Vec<u8>,
        received_resource_name: Option<String>,
        received_complete: bool,
        next_offset: i64,
        initial_offset: i64,
        must_query_write_status: bool,
    }
    let service = Arc::new(Mutex::new(Service {
        received_data: vec![0u8; blob.len()],
        received_resource_name: None,
        received_complete: false,
        next_offset: 0,
        initial_offset: 0,
        must_query_write_status: false,
    }));
    let service_for_write = service.clone();
    let blob_for_write = blob.clone();
    let service_for_query = service.clone();
    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(move |_headers, _stream_observer| {
                let service_for_next = service_for_write.clone();
                let service_for_completed = service_for_write.clone();
                let blob_for_next = blob_for_write.clone();
                let blob_for_completed = blob_for_write.clone();
                stream_observer(
                    (),
                    move |_, write_request, stream_observer| {
                        let mut this = service_for_next.lock();
                        let blob = &blob_for_next;
                        assert!(!this.must_query_write_status);

                        let resource_name = write_request.resource_name.clone();
                        if this.next_offset == this.initial_offset {
                            if this.initial_offset == 0 {
                                this.received_resource_name = Some(resource_name.clone());
                            }
                            assert!(resource_name.starts_with(&format!("{INSTANCE_NAME}/uploads")));
                            assert!(resource_name.ends_with(&blob.len().to_string()));
                        } else {
                            assert!(resource_name.is_empty());
                        }

                        assert_eq!(write_request.write_offset, this.next_offset);

                        let data = &write_request.data;

                        let start = this.next_offset as usize;
                        this.received_data[start..start + data.len()].copy_from_slice(data);

                        this.next_offset += data.len() as i64;
                        this.received_complete = blob.len() as i64 == this.next_offset;
                        assert_eq!(write_request.finish_write, this.received_complete);

                        if this.initial_offset == 0 {
                            stream_observer.on_error(Status::deadline_exceeded(""));
                            this.must_query_write_status = true;
                            this.initial_offset = this.next_offset;
                        }
                    },
                    fail_on_error,
                    move |_, stream_observer| {
                        let this = service_for_completed.lock();
                        let blob = &blob_for_completed;
                        assert_eq!(this.next_offset, blob.len() as i64);
                        assert_eq!(&this.received_data, &**blob);

                        let response = WriteResponse {
                            committed_size: this.next_offset,
                        };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                )
            })
            .query_write_status(move |request| {
                let mut this = service_for_query.lock();
                let resource_name = request.resource_name;
                let committed_size;
                let complete;
                if this.received_resource_name.as_deref() == Some(resource_name.as_str()) {
                    assert!(this.must_query_write_status);
                    this.must_query_write_status = false;
                    committed_size = this.next_offset;
                    complete = this.received_complete;
                } else {
                    committed_size = 0;
                    complete = false;
                }
                Ok(QueryWriteStatusResponse {
                    committed_size,
                    complete,
                })
            }),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test triggers one retry.
    assert_eq!(
        t.mock_backoff
            .next_delay_millis_with_status_runtime_exception_times(),
        1
    );
    assert_eq!(t.mock_backoff.get_retry_attempts_times(), 1);
}

#[test]
fn progressive_compressed_upload_should_work() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let chunk_size: i32 = 1024;
    let skip_size: i64 = (chunk_size + 1) as i64;
    let blob = random_blob((chunk_size * 2 + 1) as usize);
    let blob_length = blob.len() as i64;

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_compressed(true)
        .set_chunk_size(chunk_size)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    let output: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    struct Service {
        received_resource_name: Option<String>,
        received_complete: bool,
        next_offset: i64,
        initial_offset: i64,
        must_query_write_status: bool,
    }
    let service = Arc::new(Mutex::new(Service {
        received_resource_name: None,
        received_complete: false,
        next_offset: 0,
        initial_offset: 0,
        must_query_write_status: false,
    }));
    let service_for_write = service.clone();
    let service_for_query = service.clone();
    let output_for_write = output.clone();
    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(move |_headers, _stream_observer| {
                let service_for_next = service_for_write.clone();
                let service_for_completed = service_for_write.clone();
                let output = output_for_write.clone();
                stream_observer(
                    (),
                    move |_, write_request, stream_observer| {
                        let mut this = service_for_next.lock();
                        assert!(!this.must_query_write_status);

                        let resource_name = write_request.resource_name.clone();
                        if this.next_offset == this.initial_offset {
                            if this.initial_offset == 0 {
                                this.received_resource_name = Some(resource_name.clone());
                            }
                            assert!(resource_name.starts_with(&format!("{INSTANCE_NAME}/uploads")));
                            assert!(resource_name.ends_with(&blob_length.to_string()));
                        } else {
                            assert!(resource_name.is_empty());
                        }

                        if this.initial_offset == 0 {
                            stream_observer.on_error(Status::deadline_exceeded(""));
                            this.must_query_write_status = true;
                            this.initial_offset = skip_size;
                            this.next_offset = this.initial_offset;
                        } else {
                            let data = &write_request.data;
                            output.lock().extend_from_slice(data);
                            this.next_offset += data.len() as i64;
                            this.received_complete = write_request.finish_write;
                        }
                    },
                    fail_on_error,
                    move |_, stream_observer| {
                        let this = service_for_completed.lock();
                        let response = WriteResponse {
                            committed_size: this.next_offset,
                        };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                )
            })
            .query_write_status(move |request| {
                let mut this = service_for_query.lock();
                let resource_name = request.resource_name;
                let committed_size;
                let complete;
                if this.received_resource_name.as_deref() == Some(resource_name.as_str()) {
                    assert!(this.must_query_write_status);
                    this.must_query_write_status = false;
                    committed_size = if this.received_complete {
                        blob_length
                    } else {
                        skip_size
                    };
                    complete = this.received_complete;
                } else {
                    committed_size = 0;
                    complete = false;
                }
                Ok(QueryWriteStatusResponse {
                    committed_size,
                    complete,
                })
            }),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
    // Zstd.decompress(output.toByteArray(), blob.length - skipSize)
    let decompressed = zstd::stream::decode_all(&output.lock()[..]).unwrap();
    assert_eq!(decompressed.len() as i64, blob_length - skip_size);
    assert!(decompressed[..] == blob[skip_size as usize..]);

    // This test triggers one retry.
    assert_eq!(t.mock_backoff.next_delay_millis_times(), 1);
    assert_eq!(t.mock_backoff.get_retry_attempts_times(), 1);
}

#[test]
fn progressive_compressed_upload_sees_already_exists_at_the_end() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        Arc::new(|e: &io::Error| {
            if errors::status_from_throwable(e).map(|status| status.code())
                == Some(tonic::Code::Internal)
            {
                ResultClassifierResult::TransientFailure
            } else {
                ResultClassifierResult::Success
            }
        }),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let chunk_size: i32 = 1024;
    let blob = random_blob((chunk_size * 2 + 1) as usize);
    let blob_length = blob.len() as i64;

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_compressed(true)
        .set_chunk_size(chunk_size)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(|_headers, _stream_observer| {
                stream_observer(
                    (),
                    |_, _write_request, _stream_observer| {},
                    fail_on_error,
                    |_, stream_observer| {
                        stream_observer.on_error(Status::internal(""));
                    },
                )
            })
            .query_write_status(move |_request| {
                Ok(QueryWriteStatusResponse {
                    committed_size: blob_length,
                    complete: true,
                })
            }),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
}

#[test]
fn concurrently_completed_upload_is_not_retried() {
    // Test that after an upload has failed and the QueryWriteStatus call returns
    // that the upload has completed that we'll not retry the upload.
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);
    let blob_length = blob.len() as i64;

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    let num_write_calls = Arc::new(AtomicI32::new(0));

    let num_write_calls_for_service = num_write_calls.clone();
    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(move |_headers, stream_observer| {
                num_write_calls_for_service.fetch_add(1, Ordering::SeqCst);
                stream_observer.on_error(Status::deadline_exceeded(""));
                noop_stream_observer()
            })
            .query_write_status(move |_request| {
                Ok(QueryWriteStatusResponse {
                    committed_size: blob_length,
                    complete: true,
                })
            }),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should not have triggered any retries.
    assert_eq!(num_write_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unimplemented_query_should_restart_upload() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    struct Service {
        expire_call: bool,
        saw_reset: bool,
    }
    let service = Arc::new(Mutex::new(Service {
        expire_call: true,
        saw_reset: false,
    }));
    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(move |_headers, _stream_observer| {
                let service = service.clone();
                stream_observer(
                    (),
                    move |_, write_request, stream_observer| {
                        let mut this = service.lock();
                        if this.expire_call {
                            stream_observer.on_error(Status::deadline_exceeded(""));
                            this.expire_call = false;
                        } else if !this.saw_reset && write_request.write_offset != 0 {
                            stream_observer.on_error(Status::invalid_argument(""));
                        } else {
                            this.saw_reset = true;
                            if write_request.finish_write {
                                let committed_size =
                                    write_request.write_offset + write_request.data.len() as i64;
                                stream_observer.on_next(WriteResponse { committed_size });
                                stream_observer.on_completed();
                            }
                        }
                    },
                    fail_on_error,
                    |_, _| {},
                )
            })
            .query_write_status(|_request| Err(Status::unimplemented(""))),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should have triggered a single retry, because it made
    // no progress.
    assert_eq!(t.mock_backoff.next_delay_millis_times(), 1);
}

#[test]
fn early_write_response_should_complete_upload() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        permanent_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);
    let blob_length = blob.len() as i64;
    // provide only enough data to write a single chunk
    // PORT: and only once the server's early response has reached the client (see `WriteCalls`).
    let in_ = BlobAfterWriteCall {
        data: blob[..CHUNK_SIZE as usize].to_vec(),
        write_calls: t.write_calls.clone(),
    };

    let chunker = Chunker::builder()
        .set_input(blob_length, &in_)
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |_headers, stream_observer| {
                stream_observer.on_next(WriteResponse {
                    committed_size: blob_length,
                });
                stream_observer.on_completed();
                noop_stream_observer()
            }),
        );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn incorrect_committed_size_fails_completed_upload() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);
    let blob_length = blob.len() as i64;

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |_headers, _stream_observer| {
                stream_observer(
                    (),
                    |_, _write_request, _stream_observer| {},
                    fail_on_error,
                    move |_, stream_observer| {
                        let response = WriteResponse {
                            committed_size: blob_length + 1,
                        };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                )
            }),
        );

    // fail("Should have thrown an exception.") / catch (IOException e) { /* expected */ }
    let e = upload_blob(&uploader, &t.context, &digest, chunker)
        .expect_err("Should have thrown an exception.");
    // PORT: not in the Java test; the message is the one of
    // ByteStreamUploader.AsyncUpload.checkCommittedSize.
    assert!(
        e.to_string().starts_with(&format!(
            "write incomplete: committed_size {} for {} total - {INSTANCE_NAME}/uploads/",
            blob_length + 1,
            blob_length
        )),
        "{e}"
    );

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn incorrect_committed_size_does_not_fail_incomplete_upload() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        permanent_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);

    // PORT: Java's `setInput(blob)`; the input opens once the server's early response has
    // reached the client (see `WriteCalls`), so that the upload is incomplete at that moment.
    let input = BlobAfterWriteCall {
        data: blob.clone(),
        write_calls: t.write_calls.clone(),
    };
    let chunker = Chunker::builder()
        .set_input(blob.len() as i64, &input)
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry.add_service(
        ByteStreamImplBase::new().write(|_headers, stream_observer| {
            stream_observer.on_next(WriteResponse {
                committed_size: CHUNK_SIZE as i64,
            });
            stream_observer.on_completed();
            noop_stream_observer()
        }),
    );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
}

#[test]
fn multiple_blobs_upload_should_work() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let num_uploads = 10;
    let mut blobs_by_hash: HashMap<String, Vec<u8>> = HashMap::new();
    let mut chunkers: Vec<(Digest, Chunker<'_>)> = Vec::with_capacity(num_uploads);
    let mut rand = Random::new();
    for _ in 0..num_uploads {
        let blob_size = rand.next_int(CHUNK_SIZE * 10) + CHUNK_SIZE;
        let mut blob = vec![0u8; blob_size as usize];
        rand.next_bytes(&mut blob);
        let chunker = Chunker::builder()
            .set_input_bytes(blob.clone())
            .set_chunk_size(CHUNK_SIZE)
            .build();
        let digest = DIGEST_UTIL.compute(&blob);
        blobs_by_hash.insert(digest.hash.clone(), blob);
        chunkers.push((digest, chunker));
    }

    t.service_registry
        .add_service(maybe_fail_once_upload_service(blobs_by_hash));

    upload_blobs(&uploader, &t.context, chunkers).unwrap();
}

#[test]
fn too_many_files_io_exception_advise_maximum_open_files_flag() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );
    let blob = vec![0u8; CHUNK_SIZE as usize];
    let digest = DIGEST_UTIL.compute(&blob);
    // PORT: `Mockito.mock(Chunker.class)` with `seek(0)` throwing and `getUncompressedSize()`
    // stubbed is a real chunker over an input that cannot be opened: `seek` opens it.
    let input = TooManyOpenFiles;
    let chunker = Chunker::builder()
        .set_input(digest.size_bytes, &input)
        .set_chunk_size(CHUNK_SIZE)
        .build();
    t.service_registry
        .add_service(maybe_fail_once_upload_service(HashMap::new()));

    let new_message = "An IOException was thrown because the process opened too many files. We recommend setting \
         --bep_maximum_open_remote_upload_files flag to a number lower than your system \
         default (run 'ulimit -a' for *nix-based operating systems). Original error message: \
         Too many open files";
    let e = upload_blob(&uploader, &t.context, &digest, chunker).unwrap_err();
    assert_eq!(e.to_string(), new_message);
}

#[test]
fn available_permits_open_file_semaphore_fewer_permits_than_uploads_end_with_all_permits() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    // number of permits is less than number of uploads to affirm permit is released
    let maximum_open_files = 999;
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        maximum_open_files,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    assert_eq!(
        uploader
            .get_opened_file_permits()
            .unwrap()
            .available_permits(),
        999
    );

    let custom_file_tracker = Arc::new(CustomFileTracker::new(maximum_open_files));
    let num_uploads = 1000;
    let mut blobs_by_hash: HashMap<String, Vec<u8>> = HashMap::new();
    let mut inputs: Vec<(Digest, TestChunkerInput)> = Vec::with_capacity(num_uploads);
    let mut rand = Random::new();
    for _ in 0..num_uploads {
        let blob_size = rand.next_int(CHUNK_SIZE * 10) + CHUNK_SIZE;
        let mut blob = vec![0u8; blob_size as usize];
        rand.next_bytes(&mut blob);
        let digest = DIGEST_UTIL.compute(&blob);
        blobs_by_hash.insert(digest.hash.clone(), blob.clone());
        inputs.push((
            digest,
            TestChunkerInput {
                existing_data: blob,
                custom_file_tracker: custom_file_tracker.clone(),
            },
        ));
    }
    let chunkers: Vec<(Digest, Chunker<'_>)> = inputs
        .iter()
        .map(|(digest, input)| {
            let chunker = Chunker::builder()
                .set_input(input.existing_data.len() as i64, input)
                .set_chunk_size(CHUNK_SIZE)
                .build();
            (digest.clone(), chunker)
        })
        .collect();

    t.service_registry
        .add_service(maybe_fail_once_upload_service(blobs_by_hash));

    upload_blobs(&uploader, &t.context, chunkers).unwrap();

    assert_eq!(
        uploader
            .get_opened_file_permits()
            .unwrap()
            .available_permits(),
        maximum_open_files as usize
    );
}

#[test]
fn no_maximum_open_files_flags_null_semaphore() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );
    assert!(uploader.get_opened_file_permits().is_none());

    let num_uploads = 10;
    let mut blobs_by_hash: HashMap<String, Vec<u8>> = HashMap::new();
    let mut chunkers: Vec<(Digest, Chunker<'_>)> = Vec::with_capacity(num_uploads);
    let mut rand = Random::new();
    for _ in 0..num_uploads {
        let blob_size = rand.next_int(CHUNK_SIZE * 10) + CHUNK_SIZE;
        let mut blob = vec![0u8; blob_size as usize];
        rand.next_bytes(&mut blob);
        let chunker = Chunker::builder()
            .set_input_bytes(blob.clone())
            .set_chunk_size(CHUNK_SIZE)
            .build();
        let digest = DIGEST_UTIL.compute(&blob);
        blobs_by_hash.insert(digest.hash.clone(), blob);
        chunkers.push((digest, chunker));
    }

    t.service_registry
        .add_service(maybe_fail_once_upload_service(blobs_by_hash));

    upload_blobs(&uploader, &t.context, chunkers).unwrap();
    assert!(uploader.get_opened_file_permits().is_none());
}

#[test]
fn context_should_be_preserved_upon_retries() {
    // We upload blobs with different context, and retry 3 times for each upload.
    // We verify that the correct metadata is passed to the server with every blob.
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(5, 0),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let to_upload = ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"];
    let mut chunkers: Vec<(Digest, Chunker<'_>)> = Vec::with_capacity(to_upload.len());
    let uploads_failed: Arc<Mutex<HashMap<String, i32>>> = Arc::new(Mutex::new(HashMap::new()));
    for s in to_upload {
        let chunker = Chunker::builder()
            .set_input_bytes(s.as_bytes().to_vec())
            .set_chunk_size(3)
            .build();
        let digest = DIGEST_UTIL.compute_as_utf8(s);
        uploads_failed.lock().insert(digest.hash.clone(), 0);
        chunkers.push((digest, chunker));
    }

    let bs_service = ByteStreamImplBase::new()
        .write(move |headers, response| {
            // ServerInterceptors.intercept(bsService, new TracingMetadataUtils.ServerHeadersInterceptor())
            let meta = match request_metadata_from_headers(
                headers,
                "google.bytestream.ByteStream/Write",
            ) {
                Ok(meta) => meta,
                Err(status) => {
                    response.on_error(status);
                    return noop_stream_observer();
                }
            };
            let uploads_failed = uploads_failed.clone();
            stream_observer(
                /* digest_hash= */ None::<String>,
                move |digest_hash, write_request, response| {
                    let resource_name = &write_request.resource_name;
                    if !resource_name.is_empty() {
                        let components: Vec<&str> = resource_name.split('/').collect();
                        assert_eq!(components.len(), 6);
                        *digest_hash = Some(components[4].to_string());
                    }
                    let digest_hash = digest_hash.clone().expect("digestHash is not null");
                    // RequestMetadata meta = TracingMetadataUtils.fromCurrentContext();
                    assert_eq!(meta.correlated_invocations_id, "build-req-id");
                    assert_eq!(meta.tool_invocation_id, "command-id");
                    assert_eq!(meta.action_id, digest_hash);
                    let tool_details = meta.tool_details.clone().unwrap_or_default();
                    assert_eq!(tool_details.tool_name, "bazel");
                    assert_eq!(tool_details.tool_version, TOOL_VERSION);
                    // synchronized (this)
                    let mut uploads_failed = uploads_failed.lock();
                    let num_failures = *uploads_failed.get(&digest_hash).unwrap();
                    if num_failures < 3 {
                        uploads_failed.insert(digest_hash, num_failures + 1);
                        response.on_error(Status::internal(""));
                    }
                },
                fail_on_error,
                |_, response| {
                    response.on_next(WriteResponse { committed_size: 10 });
                    response.on_completed();
                },
            )
        })
        .query_write_status(|_request| {
            Ok(QueryWriteStatusResponse {
                committed_size: 0,
                complete: false,
            })
        });
    t.service_registry.add_service(bs_service);

    let uploader = &uploader;
    let uploads = chunkers.into_iter().map(|(action_digest, chunker)| {
        let metadata = tracing_metadata_utils::build_metadata(
            "build-req-id",
            "command-id",
            &DIGEST_UTIL
                .as_action_key(action_digest.clone())
                .digest()
                .hash
                .clone(),
            None,
        );
        let remote_action_execution_context = RemoteActionExecutionContext::create(metadata);
        async move {
            uploader
                .upload_blob_async(&remote_action_execution_context, &action_digest, chunker)
                .await
        }
    });

    // for (ListenableFuture<Void> upload : uploads) { upload.get(); }
    for upload in block_on(futures::future::join_all(uploads)) {
        upload.unwrap();
    }
}

#[test]
fn custom_headers_are_attached_to_request() {
    let mut t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        transient_failure(),
        t.retry_service.clone(),
    );

    let metadata = [
        ("Key1".to_string(), "Value1".to_string()),
        ("Key2".to_string(), "Value2".to_string()),
    ];

    t.reference_counted_channel.release();
    let interceptors: Vec<Arc<dyn ClientInterceptor>> = vec![
        // MetadataUtils.newAttachHeadersInterceptor(metadata)
        Arc::new(AttachHeadersInterceptor::new(&metadata).unwrap()),
        Arc::new(WriteCallsInterceptor(t.write_calls.clone())),
    ];
    t.reference_counted_channel = channel_to(
        t.server.addr,
        interceptors,
        Some(default_server_capabilities()),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = vec![0u8; CHUNK_SIZE as usize];
    let blob_length = blob.len() as i64;
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |headers, _stream_observer| {
                // The ServerInterceptor of the Java test.
                let get = |name: &str| {
                    headers
                        .get(name)
                        .map(|value| value.to_str().unwrap().to_string())
                };
                assert_eq!(get("key1").as_deref(), Some("Value1"));
                assert_eq!(get("key2").as_deref(), Some("Value2"));
                assert_eq!(get("key3"), None);

                stream_observer(
                    (),
                    |_, _write_request, _stream_observer| {},
                    fail_on_error,
                    move |_, stream_observer| {
                        let response = WriteResponse {
                            committed_size: blob_length,
                        };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                )
            }),
        );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
}

#[test]
fn errors_should_be_reported() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 10),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = vec![0u8; CHUNK_SIZE as usize];
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    t.service_registry
        .add_service(ByteStreamImplBase::new().write(|_headers, response| {
            response.on_error(Status::internal(""));
            noop_stream_observer()
        }));

    let e = upload_blob(&uploader, &t.context, &digest, chunker)
        .expect_err("Should have thrown an exception.");
    assert!(remote_retrier_utils::caused_by_status(
        &e,
        tonic::Code::Internal
    ));
}

#[test]
fn failure_in_retry_executor_should_be_handled() {
    let t = ByteStreamUploaderTest::set_up();
    // retryService.shutdownNow(); retryService.awaitTermination(1, TimeUnit.DAYS);
    let retry_service: Arc<dyn RetryScheduler> = Arc::new(ShutdownRetryService);
    let retrier = new_remote_retrier(fixed_backoff(1, 10), transient_failure(), retry_service);
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    t.service_registry
        .add_service(ByteStreamImplBase::new().write(|_headers, response| {
            // Immediately fail the call, so that it is retried.
            response.on_error(Status::aborted(""));
            noop_stream_observer()
        }));

    let blob = vec![0u8; 1];
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);
    let e = upload_blob(&uploader, &t.context, &digest, chunker)
        .expect_err("Should have thrown an exception.");
    // assertThat(e).hasCauseThat().isInstanceOf(RejectedExecutionException.class)
    assert!(
        errors::downcast_ref::<RejectedExecutionException>(&e).is_some(),
        "{e}"
    );
}

#[test]
fn resource_name_without_instance_name() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        /* instance_name= null */ "",
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    t.service_registry
        .add_service(ByteStreamImplBase::new().write(|_headers, _response| {
            stream_observer(
                (),
                |_, write_request, _response| {
                    // Test that the resource name doesn't start with an instance name.
                    assert!(write_request.resource_name.starts_with("uploads/"));
                },
                |_, _| {},
                |_, response| {
                    response.on_next(WriteResponse { committed_size: 1 });
                    response.on_completed();
                },
            )
        }));

    let blob = vec![0u8; 1];
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
}

#[test]
fn resource_with_new_style_digest_function() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        /* instance_name= null */ "",
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Blake3,
    );

    t.service_registry
        .add_service(ByteStreamImplBase::new().write(|_headers, _response| {
            stream_observer(
                (),
                |_, write_request, _response| {
                    // Test that the resource name contains the digest function.
                    assert!(write_request.resource_name.contains("blobs/blake3/"));
                },
                |_, _| {},
                |_, response| {
                    response.on_next(WriteResponse { committed_size: 1 });
                    response.on_completed();
                },
            )
        }));

    let blob = vec![0u8; 1];
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();
}

#[test]
fn non_retryable_status_should_not_be_retried() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        fixed_backoff(1, 0),
        /* No Status is retriable. */
        permanent_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        /* instance_name= null */ "",
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let num_calls = Arc::new(AtomicI32::new(0));

    let num_calls_for_service = num_calls.clone();
    t.service_registry
        .add_service(ByteStreamImplBase::new().write(move |_headers, response| {
            num_calls_for_service.fetch_add(1, Ordering::SeqCst);
            response.on_error(Status::internal(""));
            noop_stream_observer()
        }));

    let blob = vec![0u8; 1];
    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    upload_blob(&uploader, &t.context, &digest, chunker)
        .expect_err("Should have thrown an exception.");
    assert_eq!(num_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unauthenticated_error_should_not_be_retried() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        t.retry_service.clone(),
    );

    let refresh_times = Arc::new(AtomicI32::new(0));
    let call_credentials_provider: Arc<dyn CallCredentialsProvider> =
        Arc::new(CountingCallCredentialsProvider {
            refresh_times: refresh_times.clone(),
        });
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        call_credentials_provider,
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    let num_uploads = Arc::new(AtomicI32::new(0));
    let num_uploads_for_service = num_uploads.clone();
    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |_headers, stream_observer| {
                num_uploads_for_service.fetch_add(1, Ordering::SeqCst);

                stream_observer.on_error(Status::unauthenticated(""));
                noop_stream_observer()
            }),
        );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap_err();

    assert_eq!(refresh_times.load(Ordering::SeqCst), 1);
    assert_eq!(num_uploads.load(Ordering::SeqCst), 2);

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn should_refresh_credentials_on_authentication_error() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        t.retry_service.clone(),
    );

    let refresh_times = Arc::new(AtomicI32::new(0));
    let call_credentials_provider: Arc<dyn CallCredentialsProvider> =
        Arc::new(CountingCallCredentialsProvider {
            refresh_times: refresh_times.clone(),
        });
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        call_credentials_provider,
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);
    let blob_length = blob.len() as i64;

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    let num_uploads = Arc::new(AtomicI32::new(0));
    let num_uploads_for_service = num_uploads.clone();
    let refresh_times_for_service = refresh_times.clone();
    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |_headers, stream_observer| {
                num_uploads_for_service.fetch_add(1, Ordering::SeqCst);

                if refresh_times_for_service.load(Ordering::SeqCst) == 0 {
                    stream_observer.on_error(Status::unauthenticated(""));
                    return noop_stream_observer();
                }

                self::stream_observer(
                    /* next_offset= */ 0i64,
                    move |next_offset, write_request, _stream_observer| {
                        *next_offset += write_request.data.len() as i64;
                        let last_write = blob_length == *next_offset;
                        assert_eq!(write_request.finish_write, last_write);
                    },
                    fail_on_error,
                    move |next_offset, stream_observer| {
                        assert_eq!(*next_offset, blob_length);

                        let response = WriteResponse {
                            committed_size: *next_offset,
                        };
                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                    },
                )
            }),
        );

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    assert_eq!(refresh_times.load(Ordering::SeqCst), 1);
    assert_eq!(num_uploads.load(Ordering::SeqCst), 2);

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();
}

#[test]
fn failure_after_upload_completes() {
    let t = ByteStreamUploaderTest::set_up();
    let num_uploads = Arc::new(AtomicI32::new(0));
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        Arc::new(|e: &io::Error| {
            // e instanceof StatusRuntimeException ? Result.TRANSIENT_FAILURE : Result.SUCCESS
            if errors::downcast_ref::<StatusRuntimeException>(e).is_some() {
                ResultClassifierResult::TransientFailure
            } else {
                ResultClassifierResult::Success
            }
        }),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE - 1) as usize);
    let blob_length = blob.len() as i64;

    let num_uploads_for_service = num_uploads.clone();
    t.service_registry.add_service(
        ByteStreamImplBase::new()
            .write(move |_headers, _stream_observer| {
                num_uploads_for_service.fetch_add(1, Ordering::SeqCst);
                stream_observer(
                    (),
                    |_, _write_request, _stream_observer| {},
                    fail_on_error,
                    move |_, stream_observer| {
                        stream_observer.on_next(WriteResponse {
                            committed_size: blob_length,
                        });
                        stream_observer.on_error(Status::unavailable(""));
                    },
                )
            })
            .query_write_status(move |_request| {
                Ok(QueryWriteStatusResponse {
                    committed_size: blob_length,
                    complete: true,
                })
            }),
    );

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    assert_eq!(num_uploads.load(Ordering::SeqCst), 1);
}

#[test]
fn test_compressed_uploads() {
    let t = ByteStreamUploaderTest::set_up();
    let retrier = new_remote_retrier(
        t.mock_backoff_supplier(),
        transient_failure(),
        t.retry_service.clone(),
    );
    let uploader = ByteStreamUploader::new(
        INSTANCE_NAME,
        t.reference_counted_channel.clone(),
        no_credentials(),
        retrier,
        /* maximum_open_files= */ -1,
        /* digest_function= */ digest_function::Value::Sha256,
    );

    let blob = random_blob((CHUNK_SIZE * 2 + 1) as usize);
    let blob_length = blob.len();

    let num_uploads = Arc::new(AtomicI32::new(0));

    let num_uploads_for_service = num_uploads.clone();
    t.service_registry
        .add_service(
            ByteStreamImplBase::new().write(move |_headers, _stream_observer| {
                #[derive(Default)]
                struct State {
                    baos: Vec<u8>,
                    resource_name: Option<String>,
                }
                let num_uploads = num_uploads_for_service.clone();
                stream_observer(
                    State::default(),
                    |s, write_request, _stream_observer| {
                        if !write_request.resource_name.is_empty() {
                            match &s.resource_name {
                                Some(resource_name) => {
                                    assert_eq!(resource_name, &write_request.resource_name);
                                }
                                None => {
                                    s.resource_name = Some(write_request.resource_name.clone());
                                    assert!(
                                        write_request
                                            .resource_name
                                            .contains("/compressed-blobs/zstd/")
                                    );
                                }
                            }
                        }
                        s.baos.extend_from_slice(&write_request.data);
                    },
                    fail_on_error,
                    move |s, stream_observer| {
                        let data = s.baos.clone();
                        // try { ... } finally { respond }
                        let checked = catch_unwind(AssertUnwindSafe(|| {
                            let decompressed = zstd::stream::decode_all(&data[..])
                                .expect("Failed decompressing data.");
                            let digest = DIGEST_UTIL.compute(&decompressed);

                            assert_eq!(blob_length, decompressed.len());
                            let resource_name = s.resource_name.as_ref().expect("resourceName");
                            assert!(
                                resource_name
                                    .ends_with(&format!("/{}/{}", digest.hash, digest.size_bytes))
                            );

                            num_uploads.fetch_add(1, Ordering::SeqCst);
                        }));
                        let response = WriteResponse {
                            committed_size: data.len() as i64,
                        };

                        stream_observer.on_next(response);
                        stream_observer.on_completed();
                        if let Err(payload) = checked {
                            std::panic::resume_unwind(payload);
                        }
                    },
                )
            }),
        );

    let chunker = Chunker::builder()
        .set_input_bytes(blob.clone())
        .set_compressed(true)
        .set_chunk_size(CHUNK_SIZE)
        .build();
    let digest = DIGEST_UTIL.compute(&blob);

    upload_blob(&uploader, &t.context, &digest, chunker).unwrap();

    // This test should not have triggered any retries.
    t.mock_backoff.verify_no_interactions();

    assert_eq!(num_uploads.load(Ordering::SeqCst), 1);
}
