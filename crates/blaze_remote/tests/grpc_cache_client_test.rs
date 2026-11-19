// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
// The deprecated ActionResult fields are part of the messages the tests compare.
#![allow(deprecated)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/GrpcCacheClientTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` against an in-process gRPC server; here they
//! run on the real file system under a temporary directory (Java's `/execroot/main` is prefixed
//! with that directory) against a fake server on localhost (`util::grpc_server`). The services
//! of a test are registered first and the server is started (`fake_server.start()`) before the
//! first call.
//!
//! Java creates the client with `GoogleCredentials` read from a JSON file and removes them from
//! the calls again in a `CallCredentialsInterceptor` ("to allow testing with dummy ones").
//! `GoogleAuthUtils.newGoogleCredentialsFromFile` is not ported, so the client gets dummy call
//! credentials that add one marker header, and the port of the interceptor checks that every
//! call carries the marker and removes it.
//!
//! Java futures are eager; where a test starts a transfer and cancels it, the port drives the
//! future until the server has the call and cancels it by dropping it.

mod util;

use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use blaze_events::NullEventHandler;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_protos::JavaMessage;
use blaze_remote::authandtls::{CallCredentialsProvider, new_cache_headers_interceptor};
use blaze_remote::chunker::Chunker;
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::errors;
use blaze_remote::common::remote_path_resolver;
use blaze_remote::common::{
    ActionKey, Blob, OutputDigestMismatchException, RemoteActionExecutionContext,
    RemoteCacheClient, RemotePathResolver,
};
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc::{
    CallCredentials, CallListener, ClientInterceptor, Method, ReferenceCountedChannel,
};
use blaze_remote::grpc_cache_client::GrpcCacheClient;
use blaze_remote::reapi::bytestream::{
    QueryWriteStatusRequest, QueryWriteStatusResponse, ReadRequest, ReadResponse, WriteRequest,
    WriteResponse,
};
use blaze_remote::reapi::{
    Action, ActionResult, Command, Digest, Directory, DirectoryNode, FileNode,
    FindMissingBlobsRequest, FindMissingBlobsResponse, GetActionResultRequest, OutputDirectory,
    OutputFile, ServerCapabilities, Tree, UpdateActionResultRequest, digest_function,
};
use blaze_remote::remote_retrier::{
    EXPERIMENTAL_GRPC_RESULT_CLASSIFIER, ExponentialBackoff, RemoteRetrier,
};
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifier,
    ResultClassifierResult, RetryScheduler,
};
use blaze_remote::runtime::{block_on, get_from_future};
use blaze_remote::upload_manifest::UploadManifest;
use blaze_remote::util::DigestUtil;
use blaze_remote::util::tracing_metadata_utils;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use tonic::metadata::MetadataMap;
use tonic::{Request, Status, Streaming};

use util::grpc_server::{
    FakeActionCache, FakeByteStream, FakeContentAddressableStorage, FakeServer, ResponseStream,
    hanging_response_stream, read_response, response_stream,
};

/// The future of a `ByteStream.Write` handler.
type WriteFuture = BoxFuture<'static, Result<WriteResponse, Status>>;

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`: schedules all tasks with zero delay.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:69 TestUtils.ZeroDelayListeningScheduledExecutorService
struct ZeroDelayRetryScheduler;

#[async_trait]
impl RetryScheduler for ZeroDelayRetryScheduler {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// `TestUtils.newRemoteRetrier(backoff, resultClassifier, retryScheduler)`.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:46 TestUtils.newRemoteRetrier
fn new_remote_retrier(
    backoff: BackoffSupplier,
    result_classifier: Arc<dyn ResultClassifier>,
) -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_sleeper(
        backoff,
        Arc::new(move |e: &io::Error| {
            if errors::status_from_throwable(e).map(|s| s.code()) == Some(tonic::Code::Cancelled) {
                ResultClassifierResult::Success
            } else {
                result_classifier.test(e)
            }
        }),
        Arc::new(ZeroDelayRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> {
            /* don't wait in tests */
            Ok(())
        }),
    ))
}

/// `() -> new ExponentialBackoff(remoteOptions)`.
fn exponential_backoff(remote_options: &RemoteOptions) -> BackoffSupplier {
    let remote_options = remote_options.clone();
    Arc::new(move || -> Arc<dyn Backoff> {
        Arc::new(ExponentialBackoff::from_options(&remote_options))
    })
}

/// `Mockito.mock(Backoff.class)`: counts `nextDelayMillis` and answers with the stubbed value
/// (Mockito's default answer is 0).
struct MockBackoff {
    next_delay_millis: AtomicI64,
    next_delay_millis_calls: AtomicI32,
}

impl MockBackoff {
    fn new() -> Arc<MockBackoff> {
        Arc::new(MockBackoff {
            next_delay_millis: AtomicI64::new(0),
            next_delay_millis_calls: AtomicI32::new(0),
        })
    }

    /// `when(mockBackoff.nextDelayMillis(any(Exception.class))).thenReturn(value)`.
    fn then_return(&self, value: i64) {
        self.next_delay_millis.store(value, Ordering::SeqCst);
    }

    /// The number `Mockito.verify(mockBackoff, times(n)).nextDelayMillis(any())` checks.
    fn times(&self) -> i32 {
        self.next_delay_millis_calls.load(Ordering::SeqCst)
    }

    /// `() -> mockBackoff`.
    fn supplier(self: &Arc<MockBackoff>) -> BackoffSupplier {
        let mock = self.clone();
        Arc::new(move || -> Arc<dyn Backoff> { mock.clone() })
    }
}

impl Backoff for MockBackoff {
    fn next_delay_millis(&self, _e: &io::Error) -> i64 {
        self.next_delay_millis_calls.fetch_add(1, Ordering::SeqCst);
        self.next_delay_millis.load(Ordering::SeqCst)
    }

    fn get_retry_attempts(&self) -> i32 {
        0
    }
}

/// The header the dummy call credentials add.
const DUMMY_CREDENTIALS_HEADER: &str = "x-dummy-call-credentials";

/// The dummy call credentials of the test client.
struct DummyCallCredentials;

#[async_trait]
impl CallCredentials for DummyCallCredentials {
    async fn get_request_metadata(&self, _uri: &str) -> io::Result<Vec<(String, Vec<String>)>> {
        Ok(vec![(
            DUMMY_CREDENTIALS_HEADER.to_string(),
            vec!["dummy".to_string()],
        )])
    }
}

struct DummyCallCredentialsProvider {
    credentials: Arc<dyn CallCredentials>,
}

impl CallCredentialsProvider for DummyCallCredentialsProvider {
    fn get_call_credentials(&self) -> Option<Arc<dyn CallCredentials>> {
        Some(self.credentials.clone())
    }

    fn refresh(&self) -> io::Result<()> {
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:262 GrpcCacheClientTest.CallCredentialsInterceptor
struct CallCredentialsInterceptor;

impl ClientInterceptor for CallCredentialsInterceptor {
    fn intercept_call(
        &self,
        _method: &Method,
        headers: &mut MetadataMap,
    ) -> Option<Box<dyn CallListener>> {
        // assertThat(callOptions.getCredentials()).isEqualTo(credentials);
        assert_eq!(
            headers
                .get(DUMMY_CREDENTIALS_HEADER)
                .map(|v| v.to_str().unwrap()),
            Some("dummy")
        );
        // Remove the call credentials to allow testing with dummy ones.
        headers.remove(DUMMY_CREDENTIALS_HEADER);
        None
    }
}

/// `metadata.get(Metadata.Key.of(name, Metadata.ASCII_STRING_MARSHALLER))`.
fn header<'a>(metadata: &'a MetadataMap, name: &str) -> Option<&'a str> {
    metadata
        .get(name.to_ascii_lowercase().as_str())
        .map(|v| v.to_str().unwrap())
}

/// `new ByteStreamImplBase() { public void read(request, responseObserver) {...} }`.
fn byte_stream_read(
    read: impl Fn(ReadRequest) -> ResponseStream<ReadResponse> + Send + Sync + 'static,
) -> FakeByteStream {
    FakeByteStream {
        read: Some(Box::new(move |request: Request<ReadRequest>| {
            read(request.into_inner())
        })),
        ..Default::default()
    }
}

/// `new ContentAddressableStorageImplBase() { public void findMissingBlobs(...) {...} }`.
fn cas_find_missing_blobs(
    find_missing_blobs: impl Fn(FindMissingBlobsRequest) -> Result<FindMissingBlobsResponse, Status>
    + Send
    + Sync
    + 'static,
) -> FakeContentAddressableStorage {
    FakeContentAddressableStorage {
        find_missing_blobs: Some(Box::new(
            move |request: Request<FindMissingBlobsRequest>| {
                find_missing_blobs(request.into_inner())
            },
        )),
        ..Default::default()
    }
}

/// `new ActionCacheImplBase() { public void updateActionResult(...) {...} }`.
fn action_cache_update_action_result(
    update_action_result: impl Fn(UpdateActionResultRequest) -> Result<ActionResult, Status>
    + Send
    + Sync
    + 'static,
) -> FakeActionCache {
    FakeActionCache {
        update_action_result: Some(Box::new(
            move |request: Request<UpdateActionResultRequest>| {
                update_action_result(request.into_inner())
            },
        )),
        ..Default::default()
    }
}

/// A `ByteStream` implementation that serves a fixed set of blobs, each only after a number of
/// errors.
// Java: com/google/devtools/build/lib/remote/FakeImmutableCacheByteStreamImpl.java:30 FakeImmutableCacheByteStreamImpl
fn fake_immutable_cache_byte_stream_impl(contents: Vec<(Digest, &str)>) -> FakeByteStream {
    // Start returning the correct response after this number of errors is reached.
    const MAX_ERRORS: i32 = 3;
    let canned_replies: Vec<(ReadRequest, ReadResponse)> = contents
        .into_iter()
        .map(|(digest, data)| {
            (
                ReadRequest {
                    resource_name: format!("blobs/{}/{}", digest.hash, digest.size_bytes),
                    ..Default::default()
                },
                read_response(data),
            )
        })
        .collect();
    let num_errors: Mutex<Vec<i32>> = Mutex::new(vec![0; canned_replies.len()]);
    byte_stream_read(move |request| {
        // assertThat(cannedReplies.keySet()).contains(request);
        let index = canned_replies
            .iter()
            .position(|(canned, _)| *canned == request)
            .unwrap_or_else(|| panic!("unexpected request {request:?}"));
        let mut num_errors = num_errors.lock();
        if num_errors[index] < MAX_ERRORS {
            num_errors[index] += 1;
            response_stream(vec![Err(Status::unavailable(""))]) // Retriable error.
        } else {
            response_stream(vec![Ok(canned_replies[index].1.clone())])
        }
    })
}

/// `assertThat(actual).containsExactly(expected...)`.
fn assert_contains_exactly(actual: &[Digest], expected: &[Digest]) {
    let key = |d: &Digest| (d.hash.clone(), d.size_bytes);
    let mut actual: Vec<_> = actual.iter().map(key).collect();
    let mut expected: Vec<_> = expected.iter().map(key).collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

/// `assertThat(actual).containsAtLeast(expected...)`.
fn assert_contains_at_least(actual: &[Digest], expected: &[Digest]) {
    for digest in expected {
        assert!(
            actual.contains(digest),
            "{actual:?} does not contain {digest:?}"
        );
    }
}

fn output_file(path: &str, digest: &Digest) -> OutputFile {
    OutputFile {
        path: path.to_string(),
        digest: Some(digest.clone()),
        is_executable: true,
        ..Default::default()
    }
}

fn output_directory(path: &str, tree_digest: &Digest) -> OutputDirectory {
    OutputDirectory {
        path: path.to_string(),
        tree_digest: Some(tree_digest.clone()),
        is_topologically_sorted: true,
        ..Default::default()
    }
}

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    _fs: FileSystemRef,
    exec_root: Path,
    out_err: FileOutErr,
    fake_server: FakeServer,
    context: RemoteActionExecutionContext<'static>,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    channels: Vec<ReferenceCountedChannel>,
    /// `RemoteDownloadIdleTimeoutInterceptor` of the next client (`newClient(.., ImmutableList.of(idleTimeoutInterceptor))`).
    download_idle_timeout: Option<Duration>,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:278 GrpcCacheClientTest.setUp
    fn set_up() -> Fixture {
        // Use a mutable service registry for later registering the service impl for each test
        // case.
        let fake_server = FakeServer::new();
        Chunker::set_default_chunk_size_for_testing(1000); // Enough for everything to be one chunk.
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{root}/execroot/main"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let remote_path_resolver = remote_path_resolver::create_default(exec_root.clone());

        let stdout = Path::create_from_str(&format!("{root}/tmp/stdout"), &fs);
        let stderr = Path::create_from_str(&format!("{root}/tmp/stderr"), &fs);
        stdout
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        stderr
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let out_err = FileOutErr::new(stdout, stderr);
        // PORT: Java reads the version from BlazeVersionInfo.
        tracing_metadata_utils::set_tool_version("test");
        let metadata =
            tracing_metadata_utils::build_metadata("none", "none", &Digest::default().hash, None);
        // PORT: Java passes `mock(Spawn.class)` and `mock(SpawnExecutionContext.class)`, whose
        // methods all answer null; a context without a spawn behaves the same in these tests.
        let context = RemoteActionExecutionContext::create(metadata);
        Fixture {
            _dir: dir,
            _fs: fs,
            exec_root,
            out_err,
            fake_server,
            context,
            remote_path_resolver,
            channels: Vec::new(),
            download_idle_timeout: None,
        }
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:173 GrpcCacheClientTest.newClient
    fn new_client(&mut self) -> Arc<GrpcCacheClient> {
        self.new_client_with_options(&RemoteOptions::default())
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:177 GrpcCacheClientTest.newClient
    fn new_client_with_options(&mut self, remote_options: &RemoteOptions) -> Arc<GrpcCacheClient> {
        self.new_client_with_backoff(remote_options, exponential_backoff(remote_options))
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:181 GrpcCacheClientTest.newClient
    fn new_client_with_backoff(
        &mut self,
        remote_options: &RemoteOptions,
        backoff_supplier: BackoffSupplier,
    ) -> Arc<GrpcCacheClient> {
        self.new_client_with_max_outbound_message_size(
            remote_options,
            backoff_supplier,
            RemoteOptions::MAX_OUTBOUND_MESSAGE_SIZE,
        )
    }

    /// `newClient(remoteOptions, backoffSupplier, ImmutableList.of())` for a test that assigns
    /// `remoteOptions.maxOutboundMessageSize`.
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:186 GrpcCacheClientTest.newClient
    fn new_client_with_max_outbound_message_size(
        &mut self,
        remote_options: &RemoteOptions,
        backoff_supplier: BackoffSupplier,
        max_outbound_message_size: i32,
    ) -> Arc<GrpcCacheClient> {
        // PORT: see the file comment for the credentials.
        let call_credentials_provider: Arc<dyn CallCredentialsProvider> =
            Arc::new(DummyCallCredentialsProvider {
                credentials: Arc::new(DummyCallCredentials),
            });

        let retrier = new_remote_retrier(
            backoff_supplier,
            EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        );
        // InProcessChannelBuilder.forName(fakeServerName).directExecutor()
        //     .intercept(new CallCredentialsInterceptor(creds))
        //     .intercept(TracingMetadataUtils.newCacheHeadersInterceptor(remoteOptions));
        // The interceptor added last runs first.
        let interceptors: Vec<Arc<dyn ClientInterceptor>> = vec![
            Arc::new(new_cache_headers_interceptor(remote_options)),
            Arc::new(CallCredentialsInterceptor),
        ];
        // Single.just(ServerCapabilities.getDefaultInstance())
        let capabilities_fetcher: CapabilitiesFetcher = Arc::new(
            |_channel: ReferenceCountedChannel| -> BoxFuture<'static, io::Result<ServerCapabilities>> {
                Box::pin(async { Ok(ServerCapabilities::default()) })
            },
        );
        let channel = self
            .fake_server
            .channel(interceptors, Some(capabilities_fetcher));
        // The channel-level `RemoteDownloadIdleTimeoutInterceptor` (the innermost interceptor
        // in Java), when the test installs one.
        let channel = match self.download_idle_timeout {
            Some(timeout) => channel.with_download_idle_timeout(timeout),
            None => channel,
        };
        self.channels.push(channel.clone());
        Arc::new(GrpcCacheClient::new_with_max_outbound_message_size(
            channel,
            call_credentials_provider,
            remote_options,
            retrier,
            digest_util(),
            max_outbound_message_size,
        ))
    }

    /// `new CombinedCache(client, null, null, DIGEST_UTIL, null)`.
    fn new_combined_cache(&self, client: &Arc<GrpcCacheClient>) -> CombinedCache {
        let client: Arc<dyn RemoteCacheClient> = client.clone();
        CombinedCache::new(
            Some(client),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            digest_util(),
            /* chunkingFunction= */ None,
        )
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:133 FakeActionInputFileCache.createScratchInput
    fn create_scratch_input(&self, exec_path: &str, content: &str) -> Digest {
        let input_file = self.exec_root.get_relative_str(exec_path);
        input_file
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        fs_utils::write_content(&input_file, content.as_bytes()).unwrap();
        digest_util().compute_path(&input_file).unwrap()
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:146 FakeActionInputFileCache.createScratchInputDirectory
    fn create_scratch_input_directory(&self, exec_path: &str, content: &Tree) -> Digest {
        let input_file = self.exec_root.get_relative_str(exec_path);
        input_file.create_directory_and_parents().unwrap();
        digest_util().compute_message(content)
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:826 GrpcCacheClientTest.upload
    fn upload(
        &self,
        combined_cache: &CombinedCache,
        action_key: ActionKey,
        action: &Action,
        command: &Command,
        outputs: &[Path],
    ) -> ActionResult {
        let upload_manifest = UploadManifest::create(
            &combined_cache.get_remote_cache_capabilities().unwrap(),
            combined_cache.digest_util().clone(),
            self.remote_path_resolver.clone(),
            action_key,
            action,
            command,
            outputs,
            Some(&self.out_err),
            /* exitCode= */ 0,
            /* startTime= */ None,
            /* wallTimeInMs= */ 0,
            /* preserveExecutableBit= */ false,
        )
        .unwrap();
        upload_manifest
            .upload(
                &self.context,
                combined_cache,
                NullEventHandler::INSTANCE,
                /* force= */ false,
            )
            .unwrap()
    }

    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:851 GrpcCacheClientTest.uploadDirectory
    fn upload_directory(&self, combined_cache: &CombinedCache, outputs: &[Path]) -> ActionResult {
        let action = Action::default();
        let action_key = digest_util().compute_action_key(&action);
        let cmd = Command::default();
        self.upload(combined_cache, action_key, &action, &cmd, outputs)
    }
}

impl Drop for Fixture {
    // Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:304 GrpcCacheClientTest.tearDown
    fn drop(&mut self) {
        for channel in &self.channels {
            let _ = channel.release();
        }
        self.fake_server.shutdown_now();
    }
}

// Java: com/google/devtools/build/lib/remote/GrpcCacheClientTest.java:249 GrpcCacheClientTest.downloadBlob
fn download_blob(
    context: &RemoteActionExecutionContext<'_>,
    cache_client: &GrpcCacheClient,
    digest: &Digest,
) -> io::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    get_from_future(cache_client.download_blob(context, digest, &mut out))?;
    Ok(out)
}

/// Waits (up to ten seconds) until `condition` holds.
fn wait_until(condition: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

#[test]
#[ignore = "asserts on JVM heap usage (System.gc, Runtime.totalMemory/freeMemory) around a 64 MB upload of a VirtualActionInput, which has no counterpart; the streaming it guards is covered by ByteStreamUploader's and Chunker's tests"]
fn test_virtual_action_input_support() {
    panic!("not ported: needs a heap usage measurement");
}

#[test]
fn download_blob_cancelled_cancel_request() {
    // Test that if the download future is cancelled, the download itself is also cancelled.

    // arrange
    let mut f = Fixture::set_up();
    let digest = digest_util().compute_as_utf8("abcdefg");
    let cancelled = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    // Mock a byte stream whose read method never finish.
    {
        let cancelled = cancelled.clone();
        let started = started.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |_request| {
                started.store(true, Ordering::SeqCst);
                // setOnCancelHandler(() -> cancelled.set(true))
                hanging_response_stream(vec![], cancelled.clone())
            }));
    }
    f.fake_server.start();
    let cache_client = f.new_client();

    // act
    {
        let mut out: Vec<u8> = Vec::new();
        block_on(async {
            let download = cache_client.download_blob(&f.context, &digest, &mut out);
            tokio::pin!(download);
            // PORT: a Java future runs as soon as it is made; drive this one until the server
            // has the call.
            loop {
                tokio::select! {
                    biased;
                    result = &mut download => panic!("the download finished: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {
                        if started.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                }
            }
            // download.cancel(/* mayInterruptIfRunning= */ true): the future is dropped here.
        });
    }

    // assert
    // PORT: the cancellation travels over a socket, so it is awaited.
    assert!(wait_until(|| cancelled.load(Ordering::SeqCst)));
}

/// `new ByteArrayInputStream(data) { public void close() { super.close(); latch.countDown(); } }`
struct CloseCountingInputStream {
    data: io::Cursor<Vec<u8>>,
    latch: Arc<AtomicBool>,
}

impl Read for CloseCountingInputStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.data.read(buf)
    }
}

impl Drop for CloseCountingInputStream {
    fn drop(&mut self) {
        self.latch.store(true, Ordering::SeqCst);
    }
}

struct CloseCountingBlob {
    data: Vec<u8>,
    latch: Arc<AtomicBool>,
}

impl Blob for CloseCountingBlob {
    fn get(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(CloseCountingInputStream {
            data: io::Cursor::new(self.data.clone()),
            latch: self.latch.clone(),
        }))
    }
}

#[test]
fn test_chunker_reset_after_error() {
    // arrange
    let mut f = Fixture::set_up();
    let client = f.new_client();
    f.fake_server
        .service_registry
        .add_byte_stream(FakeByteStream {
            write: Some(Box::new(
                |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
                    Box::pin(async move {
                        let mut requests = request.into_inner();
                        match requests.message().await {
                            // onNext(request)
                            Ok(Some(_)) => Err(Status::data_loss("")),
                            // onCompleted() {} / onError(t) {}: the call is never answered.
                            _ => futures::future::pending().await,
                        }
                    })
                },
            )),
            ..Default::default()
        });
    f.fake_server.start();
    let data = vec![0u8; 20];
    let digest = digest_util().compute(&data);
    let latch = Arc::new(AtomicBool::new(false));
    let blob = CloseCountingBlob {
        data: data.clone(),
        latch: latch.clone(),
    };
    let chunker = Chunker::new(&blob, data.len() as i64, 2, false);

    // act
    let t = get_from_future(client.upload_chunker(&f.context, &digest, chunker)).unwrap_err();

    // assert
    assert_eq!(
        errors::status_from_throwable(&t).map(|s| s.code()),
        Some(tonic::Code::DataLoss)
    );
    // latch.await()
    assert!(wait_until(|| latch.load(Ordering::SeqCst)));
}

#[test]
fn test_download_empty_blob() {
    let mut f = Fixture::set_up();
    let client = f.new_client();
    let empty_digest = digest_util().compute(&[]);
    // Will not call the mock Bytestream interface at all.
    assert!(
        download_blob(&f.context, &client, &empty_digest)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_download_blob_single_chunk() {
    let mut f = Fixture::set_up();
    let client = f.new_client();
    let digest = digest_util().compute_as_utf8("abcdefg");
    {
        let digest = digest.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                response_stream(vec![Ok(read_response("abcdefg"))])
            }));
    }
    f.fake_server.start();
    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
}

#[test]
fn test_download_blob_multiple_chunks() {
    let mut f = Fixture::set_up();
    let client = f.new_client();
    let digest = digest_util().compute_as_utf8("abcdefg");
    {
        let digest = digest.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                response_stream(vec![
                    Ok(read_response("abc")),
                    Ok(read_response("def")),
                    Ok(read_response("g")),
                ])
            }));
    }
    f.fake_server.start();
    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
}

#[test]
fn test_download_all_results() {
    // arrange
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let foo_digest = digest_util().compute_as_utf8("foo-contents");
    let bar_digest = digest_util().compute_as_utf8("bar-contents");
    let empty_digest = digest_util().compute(&[]);
    f.fake_server
        .service_registry
        .add_byte_stream(fake_immutable_cache_byte_stream_impl(vec![
            (foo_digest.clone(), "foo-contents"),
            (bar_digest.clone(), "bar-contents"),
        ]));
    f.fake_server.start();

    // act
    get_from_future(combined_cache.download_file(
        &f.context,
        &f.exec_root.get_relative_str("a/foo"),
        &foo_digest,
    ))
    .unwrap();
    get_from_future(combined_cache.download_file(
        &f.context,
        &f.exec_root.get_relative_str("b/empty"),
        &empty_digest,
    ))
    .unwrap();
    get_from_future(combined_cache.download_file(
        &f.context,
        &f.exec_root.get_relative_str("a/bar"),
        &bar_digest,
    ))
    .unwrap();

    // assert
    assert_eq!(
        digest_util()
            .compute_path(&f.exec_root.get_relative_str("a/foo"))
            .unwrap(),
        foo_digest
    );
    assert_eq!(
        digest_util()
            .compute_path(&f.exec_root.get_relative_str("b/empty"))
            .unwrap(),
        empty_digest
    );
    assert_eq!(
        digest_util()
            .compute_path(&f.exec_root.get_relative_str("a/bar"))
            .unwrap(),
        bar_digest
    );
}

#[test]
fn test_upload_directory() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let foo_digest = f.create_scratch_input("a/foo", "xyz");
    let qux_digest = f.create_scratch_input("bar/qux", "abc");
    let bar_digest = f.create_scratch_input_directory(
        "bar",
        &Tree {
            root: Some(Directory {
                files: vec![FileNode {
                    is_executable: true,
                    name: "qux".to_string(),
                    digest: Some(qux_digest.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let foo_file = f.exec_root.get_relative_str("a/foo");
    let qux_file = f.exec_root.get_relative_str("bar/qux");
    qux_file.set_executable(true).unwrap();
    let bar_dir = f.exec_root.get_relative_str("bar");
    {
        let expected = vec![foo_digest.clone(), qux_digest.clone(), bar_digest.clone()];
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |request| {
                assert_contains_at_least(&request.blob_digests, &expected);
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    f.fake_server
        .service_registry
        .add_action_cache(action_cache_update_action_result(|request| {
            Ok(request.action_result.unwrap_or_default())
        }));
    f.fake_server.start();

    let result = f.upload_directory(&combined_cache, &[foo_file, bar_dir]);
    // output files will have permission 0555 after action execution regardless the current
    // permission
    let expected_result = ActionResult {
        output_files: vec![output_file("a/foo", &foo_digest)],
        output_directories: vec![output_directory("bar", &bar_digest)],
        ..Default::default()
    };
    assert_eq!(result, expected_result);
}

#[test]
fn test_upload_directory_empty() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let bar_digest = f.create_scratch_input_directory(
        "bar",
        &Tree {
            root: Some(Directory::default()),
            ..Default::default()
        },
    );
    let bar_dir = f.exec_root.get_relative_str("bar");
    {
        let bar_digest = bar_digest.clone();
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |request| {
                assert!(request.blob_digests.contains(&bar_digest));
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    f.fake_server
        .service_registry
        .add_action_cache(action_cache_update_action_result(|request| {
            Ok(request.action_result.unwrap_or_default())
        }));
    f.fake_server.start();

    let result = f.upload_directory(&combined_cache, &[bar_dir]);
    let expected_result = ActionResult {
        output_directories: vec![output_directory("bar", &bar_digest)],
        ..Default::default()
    };
    assert_eq!(result, expected_result);
}

#[test]
fn test_upload_directory_nested() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let wobble_digest = f.create_scratch_input("bar/test/wobble", "xyz");
    let qux_digest = f.create_scratch_input("bar/qux", "abc");
    let test_dir_message = Directory {
        files: vec![FileNode {
            name: "wobble".to_string(),
            digest: Some(wobble_digest.clone()),
            is_executable: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let test_digest = digest_util().compute_message(&test_dir_message);
    let bar_tree = Tree {
        root: Some(Directory {
            files: vec![FileNode {
                name: "qux".to_string(),
                digest: Some(qux_digest.clone()),
                is_executable: true,
                ..Default::default()
            }],
            directories: vec![DirectoryNode {
                name: "test".to_string(),
                digest: Some(test_digest),
            }],
            ..Default::default()
        }),
        children: vec![test_dir_message],
    };
    let bar_digest = f.create_scratch_input_directory("bar", &bar_tree);
    let qux_file = f.exec_root.get_relative_str("bar/qux");
    qux_file.set_executable(true).unwrap();
    let bar_dir = f.exec_root.get_relative_str("bar");
    {
        let expected = vec![
            qux_digest.clone(),
            bar_digest.clone(),
            wobble_digest.clone(),
        ];
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |request| {
                assert_contains_at_least(&request.blob_digests, &expected);
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    f.fake_server
        .service_registry
        .add_action_cache(action_cache_update_action_result(|request| {
            Ok(request.action_result.unwrap_or_default())
        }));
    f.fake_server.start();

    let result = f.upload_directory(&combined_cache, &[bar_dir]);
    let expected_result = ActionResult {
        output_directories: vec![output_directory("bar", &bar_digest)],
        ..Default::default()
    };
    assert_eq!(result, expected_result);
}

#[test]
fn extra_headers() {
    let mut f = Fixture::set_up();
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_headers = vec![
        ("CommonKey1".to_string(), "CommonValue1".to_string()),
        ("CommonKey2".to_string(), "CommonValue2".to_string()),
    ];
    remote_options.remote_exec_headers = vec![
        ("ExecKey1".to_string(), "ExecValue1".to_string()),
        ("ExecKey2".to_string(), "ExecValue2".to_string()),
    ];
    remote_options.remote_cache_headers = vec![
        ("CacheKey1".to_string(), "CacheValue1".to_string()),
        ("CacheKey2".to_string(), "CacheValue2".to_string()),
    ];

    f.fake_server
        .service_registry
        .add_content_addressable_storage(cas_find_missing_blobs(|_request| {
            Ok(FindMissingBlobsResponse::default())
        }));
    let intercepted = Arc::new(AtomicBool::new(false));
    {
        let intercepted = intercepted.clone();
        f.fake_server
            .service_registry
            .add_action_cache(FakeActionCache {
            get_action_result: Some(Box::new(
                move |request: Request<GetActionResultRequest>| -> Result<ActionResult, Status> {
                    // ServerInterceptors.intercept(actionCache, interceptor)
                    let metadata = request.metadata();
                    assert_eq!(header(metadata, "CommonKey1"), Some("CommonValue1"));
                    assert_eq!(header(metadata, "CommonKey2"), Some("CommonValue2"));
                    assert_eq!(header(metadata, "CacheKey1"), Some("CacheValue1"));
                    assert_eq!(header(metadata, "CacheKey2"), Some("CacheValue2"));
                    assert_eq!(header(metadata, "ExecKey1"), None);
                    assert_eq!(header(metadata, "ExecKey2"), None);
                    intercepted.store(true, Ordering::SeqCst);
                    Ok(ActionResult::default())
                },
            )),
            ..Default::default()
        });
    }
    f.fake_server.start();

    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);
    // PORT: Java drops the future (`var unused`), so a failed assertion of the interceptor does
    // not fail its test; the port checks the outcome.
    combined_cache
        .download_action_result(
            &f.context,
            &digest_util().as_action_key(digest_util().compute_as_utf8("key")),
            /* inlineOutErr= */ false,
            /* inlineOutputFiles= */ &indexmap::IndexSet::new(),
        )
        .unwrap();
    assert!(intercepted.load(Ordering::SeqCst));
}

#[test]
fn test_upload() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let foo_digest = f.create_scratch_input("a/foo", "xyz");
    let bar_digest = f.create_scratch_input("bar", "x");
    let foo_file = f.exec_root.get_relative_str("a/foo");
    let bar_file = f.exec_root.get_relative_str("bar");
    bar_file.set_executable(true).unwrap();
    let command = Command {
        output_files: vec!["a/foo".to_string()],
        ..Default::default()
    };
    let cmd_digest = digest_util().compute(&command.to_byte_array());
    let action = Action {
        command_digest: Some(cmd_digest.clone()),
        ..Default::default()
    };
    let action_digest = digest_util().compute(&action.to_byte_array());

    f.out_err.write_out(b"foo out").unwrap();
    f.out_err.write_err(b"foo err").unwrap();

    let stdout_digest = digest_util()
        .compute_path(f.out_err.get_output_path())
        .unwrap();
    let stderr_digest = digest_util()
        .compute_path(f.out_err.get_error_path())
        .unwrap();

    {
        let expected = vec![
            cmd_digest.clone(),
            action_digest.clone(),
            foo_digest.clone(),
            bar_digest.clone(),
            stdout_digest.clone(),
            stderr_digest.clone(),
        ];
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |request| {
                assert_contains_exactly(&request.blob_digests, &expected);
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    f.fake_server
        .service_registry
        .add_action_cache(action_cache_update_action_result(|request| {
            Ok(request.action_result.unwrap_or_default())
        }));
    f.fake_server.start();

    let result = f.upload(
        &combined_cache,
        digest_util().as_action_key(action_digest),
        &action,
        &command,
        &[foo_file, bar_file],
    );
    // output files will have permission 0555 after action execution regardless the current
    // permission
    let expected_result = ActionResult {
        stdout_digest: Some(stdout_digest),
        stderr_digest: Some(stderr_digest),
        output_files: vec![
            output_file("a/foo", &foo_digest),
            output_file("bar", &bar_digest),
        ],
        ..Default::default()
    };
    assert_eq!(result, expected_result);
}

#[test]
fn test_upload_split_missing_digests_call() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    // remoteOptions.maxOutboundMessageSize = 80; // Enough for one digest, but not two.
    let client = f.new_client_with_max_outbound_message_size(
        &remote_options,
        exponential_backoff(&remote_options),
        80,
    );
    let combined_cache = f.new_combined_cache(&client);

    let foo_digest = f.create_scratch_input("a/foo", "xyz");
    let bar_digest = f.create_scratch_input("bar", "x");
    let foo_file = f.exec_root.get_relative_str("a/foo");
    let bar_file = f.exec_root.get_relative_str("bar");
    bar_file.set_executable(true).unwrap();
    let command = Command {
        output_files: vec!["a/foo".to_string()],
        ..Default::default()
    };
    let cmd_digest = digest_util().compute(&command.to_byte_array());
    let action = Action {
        command_digest: Some(cmd_digest),
        ..Default::default()
    };
    let action_digest = digest_util().compute(&action.to_byte_array());
    let num_get_missing_calls = Arc::new(AtomicI32::new(0));
    {
        let num_get_missing_calls = num_get_missing_calls.clone();
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |request| {
                num_get_missing_calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request.blob_digests.len(), 1);
                // Nothing is missing.
                Ok(FindMissingBlobsResponse::default())
            }));
    }
    f.fake_server
        .service_registry
        .add_action_cache(action_cache_update_action_result(|request| {
            Ok(request.action_result.unwrap_or_default())
        }));
    f.fake_server.start();

    let result = f.upload(
        &combined_cache,
        digest_util().as_action_key(action_digest),
        &action,
        &command,
        &[foo_file, bar_file],
    );
    // output files will have permission 0555 after action execution regardless the current
    // permission
    let expected_result = ActionResult {
        output_files: vec![
            output_file("a/foo", &foo_digest),
            output_file("bar", &bar_digest),
        ],
        ..Default::default()
    };
    assert_eq!(result, expected_result);
    assert_eq!(num_get_missing_calls.load(Ordering::SeqCst), 4);
}

#[test]
fn test_upload_cache_misses_with_retries() {
    let mut f = Fixture::set_up();
    let remote_options = RemoteOptions::default();
    let client = f.new_client_with_options(&remote_options);
    let combined_cache = f.new_combined_cache(&client);

    let foo_digest = f.create_scratch_input("a/foo", "xyz");
    let bar_digest = f.create_scratch_input("bar", "x");
    let baz_digest = f.create_scratch_input("baz", "z");
    let foobar_digest = f.create_scratch_input("foobar", "foobar");
    let foo_file = f.exec_root.get_relative_str("a/foo");
    let bar_file = f.exec_root.get_relative_str("bar");
    let baz_file = f.exec_root.get_relative_str("baz");
    let foobar_file = f.exec_root.get_relative_str("foobar");
    let action_key = digest_util().as_action_key(foo_digest.clone()); // Could be any key.
    bar_file.set_executable(true).unwrap();
    {
        let num_errors = AtomicI32::new(4);
        let missing = vec![
            foo_digest.clone(),
            bar_digest.clone(),
            baz_digest.clone(),
            foobar_digest.clone(),
        ];
        f.fake_server
            .service_registry
            .add_content_addressable_storage(cas_find_missing_blobs(move |_request| {
                if num_errors.fetch_sub(1, Ordering::SeqCst) <= 0 {
                    // All outputs are missing.
                    Ok(FindMissingBlobsResponse {
                        missing_blob_digests: missing.clone(),
                    })
                } else {
                    Err(Status::unavailable(""))
                }
            }));
    }
    // output files will have permission 0555 after action execution regardless the current
    // permission
    let result = ActionResult {
        output_files: vec![
            output_file("a/foo", &foo_digest),
            output_file("bar", &bar_digest),
            output_file("baz", &baz_digest),
            output_file("foobar", &foobar_digest),
        ],
        ..Default::default()
    };
    {
        let num_errors = AtomicI32::new(4);
        let result = result.clone();
        let foo_digest = foo_digest.clone();
        f.fake_server
            .service_registry
            .add_action_cache(action_cache_update_action_result(move |request| {
                assert_eq!(
                    request,
                    UpdateActionResultRequest {
                        digest_function: digest_function::Value::Sha256 as i32,
                        action_digest: Some(foo_digest.clone()),
                        action_result: Some(result.clone()),
                        ..Default::default()
                    }
                );
                if num_errors.fetch_sub(1, Ordering::SeqCst) <= 0 {
                    Ok(result.clone())
                } else {
                    Err(Status::unavailable(""))
                }
            }));
    }
    // ByteStreamImplBase mockByteStreamImpl = spy(ByteStreamImplBase.class);
    let write_calls = Arc::new(AtomicI32::new(0));
    {
        let write_calls = write_calls.clone();
        let num_errors = Arc::new(AtomicI32::new(4));
        let foo_digest = foo_digest.clone();
        let bar_digest = bar_digest.clone();
        let baz_digest = baz_digest.clone();
        let foobar_digest = foobar_digest.clone();
        f.fake_server.service_registry.add_byte_stream(FakeByteStream {
            write: Some(Box::new(
                move |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
                write_calls.fetch_add(1, Ordering::SeqCst);
                let num_errors = num_errors.clone();
                let foo_digest = foo_digest.clone();
                let bar_digest = bar_digest.clone();
                let baz_digest = baz_digest.clone();
                let foobar_digest = foobar_digest.clone();
                Box::pin(async move {
                    let mut requests = request.into_inner();
                    let mut response: Option<WriteResponse> = None;
                    loop {
                        let request = match requests.message().await {
                            Ok(Some(request)) => request,
                            // onCompleted(): responseObserver.onCompleted()
                            Ok(None) => break,
                            // onError(t): fail("An error occurred: " + t)
                            Err(t) => panic!("An error occurred: {t}"),
                        };
                        // onNext(request)
                        // Java: `numErrors.decrementAndGet() >= 0`.
                        #[allow(clippy::int_plus_one)]
                        let erroring = num_errors.fetch_sub(1, Ordering::SeqCst) - 1 >= 0;
                        if erroring {
                            return Err(Status::unavailable(""));
                        }
                        assert!(request.finish_write);
                        let resource_name = &request.resource_name;
                        let data_str = String::from_utf8(request.data.clone()).unwrap();
                        let size;
                        if resource_name.contains(&foo_digest.hash) {
                            assert_eq!(data_str, "xyz");
                            size = 3;
                        } else if resource_name.contains(&bar_digest.hash) {
                            assert_eq!(data_str, "x");
                            size = 1;
                        } else if resource_name.contains(&baz_digest.hash) {
                            assert_eq!(data_str, "z");
                            size = 1;
                        } else if resource_name.contains(&foobar_digest.hash) {
                            return Err(Status::already_exists(""));
                        } else {
                            panic!("Unexpected resource name in upload: {resource_name}");
                        }
                        response = Some(WriteResponse {
                            committed_size: size,
                        });
                    }
                    Ok(response.expect("the write was completed without a request"))
                })
            },
            )),
            query_write_status: Some(Box::new(
                |_request: Request<QueryWriteStatusRequest>|
                 -> Result<QueryWriteStatusResponse, Status> {
                    Ok(QueryWriteStatusResponse {
                        committed_size: 0,
                        complete: false,
                    })
                },
            )),
            ..Default::default()
        });
    }
    f.fake_server.start();
    let _unused = f.upload(
        &combined_cache,
        action_key,
        &Action::default(),
        &Command::default(),
        &[foo_file, bar_file, baz_file, foobar_file],
    );
    // 4 times for the errors, 4 times for the successful uploads.
    assert_eq!(write_calls.load(Ordering::SeqCst), 8);
}

#[test]
fn test_get_cached_action_result_with_retries() {
    let mut f = Fixture::set_up();
    let client = f.new_client();
    let action_key = digest_util().as_action_key(digest_util().compute_as_utf8("key"));
    {
        let num_errors = AtomicI32::new(4);
        f.fake_server
            .service_registry
            .add_action_cache(FakeActionCache {
            get_action_result: Some(Box::new(
                move |_request: Request<GetActionResultRequest>| -> Result<ActionResult, Status> {
                    Err(if num_errors.fetch_sub(1, Ordering::SeqCst) <= 0 {
                        Status::not_found("")
                    } else {
                        Status::unavailable("")
                    })
                },
            )),
            ..Default::default()
        });
    }
    f.fake_server.start();
    assert!(
        get_from_future(client.download_action_result(
            &f.context,
            &action_key,
            /* inlineOutErr= */ false,
            /* inlineOutputFiles= */ &indexmap::IndexSet::new(),
        ))
        .unwrap()
        .is_none()
    );
}

#[test]
fn download_blob_is_retried_with_progress() {
    let mut f = Fixture::set_up();
    let mock_backoff = MockBackoff::new();
    let client = f.new_client_with_backoff(&RemoteOptions::default(), mock_backoff.supplier());
    let digest = digest_util().compute_as_utf8("abcdefg");
    {
        let digest = digest.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                let data = b"abcdefg";
                let off = request.read_offset as usize;
                let data = if off == 0 { &data[0..1] } else { &data[off..] };
                let mut responses = vec![Ok(read_response(data))];
                if off == 0 {
                    responses.push(Err(Status::deadline_exceeded("")));
                }
                response_stream(responses)
            }));
    }
    f.fake_server.start();
    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
    assert_eq!(mock_backoff.times(), 0);
}

#[test]
fn download_blob_idle_timeout_is_retried_with_progress() {
    // Unexpected failures without progress should stop immediately. The idle-timeout retry below
    // is allowed only because receiving the prefix resets ProgressiveBackoff.
    let mut f = Fixture::set_up();
    let mock_backoff = MockBackoff::new();
    mock_backoff.then_return(-1);
    // PORT: Java captures the interceptor's timer task and fires it by hand
    // (`new RemoteDownloadIdleTimeoutInterceptor(idleTimeout, scheduler, nanoTime::get)`); the
    // port's idle timeout is a timer of the channel (`with_download_idle_timeout`), so a short
    // real timeout stands in for firing the 60-second one.
    f.download_idle_timeout = Some(Duration::from_millis(300));
    let client = f.new_client_with_backoff(&RemoteOptions::default(), mock_backoff.supplier());
    let digest = digest_util().compute_as_utf8("abcdefg");
    let read_calls = Arc::new(AtomicI32::new(0));
    {
        let digest = digest.clone();
        let read_calls = read_calls.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                let data = b"abcdefg";
                let offset = request.read_offset as usize;
                let read_call = read_calls.fetch_add(1, Ordering::SeqCst);
                if read_call == 0 {
                    // Send a prefix without closing the stream. Only the idle-timeout
                    // interceptor can terminate this attempt and cause GrpcCacheClient to retry.
                    assert_eq!(offset, 0);
                    return hanging_response_stream(
                        vec![Ok(read_response(&data[0..1]))],
                        Arc::new(AtomicBool::new(false)),
                    );
                }
                // GrpcCacheClient must resume after the byte already written to the output
                // stream.
                assert_eq!(read_call, 1);
                assert_eq!(offset, 1);
                response_stream(vec![Ok(read_response(&data[offset..]))])
            }));
    }
    f.fake_server.start();

    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
    assert_eq!(read_calls.load(Ordering::SeqCst), 2);
    assert_eq!(mock_backoff.times(), 0);
}

#[test]
fn download_blob_does_not_retry_zero_length_requests() {
    let mut f = Fixture::set_up();
    let mock_backoff = MockBackoff::new();
    let client = f.new_client_with_backoff(&RemoteOptions::default(), mock_backoff.supplier());
    let digest = digest_util().compute_as_utf8("abcdefg");
    {
        let digest = digest.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                assert_eq!(request.read_offset, 0);
                response_stream(vec![
                    Ok(read_response("abcdefg")),
                    Err(Status::internal("")),
                ])
            }));
    }
    f.fake_server.start();
    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
    assert_eq!(mock_backoff.times(), 0);
}

#[test]
fn download_blob_passes_through_deadline_exceeded_without_progress() {
    let mut f = Fixture::set_up();
    let mock_backoff = MockBackoff::new();
    mock_backoff.then_return(-1);
    let client = f.new_client_with_backoff(&RemoteOptions::default(), mock_backoff.supplier());
    let digest = digest_util().compute_as_utf8("abcdefg");
    {
        let digest = digest.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                let data = b"abcdefg";
                let mut responses = Vec::new();
                if request.read_offset == 0 {
                    responses.push(Ok(read_response(&data[0..2])));
                }
                responses.push(Err(Status::deadline_exceeded("")));
                response_stream(responses)
            }));
    }
    f.fake_server.start();
    let e = download_blob(&f.context, &client, &digest).unwrap_err();
    let st = errors::status_from_throwable(&e);
    assert_eq!(st.map(|s| s.code()), Some(tonic::Code::DeadlineExceeded));
    assert_eq!(mock_backoff.times(), 1);
}

#[test]
fn test_download_fails_on_digest_mismatch() {
    // Test that the download fails when a blob/file has a different content hash than expected.

    let mut f = Fixture::set_up();
    let client = f.new_client();
    let digest = digest_util().compute_as_utf8("foo");
    f.fake_server
        .service_registry
        .add_byte_stream(byte_stream_read(|_request| {
            response_stream(vec![Ok(read_response("bar"))])
        }));
    f.fake_server.start();
    let e = download_blob(&f.context, &client, &digest).unwrap_err();
    assert!(e.to_string().contains(&digest.hash));
    assert!(
        e.to_string()
            .contains(&digest_util().compute_as_utf8("bar").hash)
    );
}

#[test]
fn test_disabling_digest_verification() {
    // Test that when digest verification is disabled a corrupted download works.

    let mut f = Fixture::set_up();
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_verify_downloads = false;

    let client = f.new_client_with_options(&remote_options);
    let digest = digest_util().compute_as_utf8("foo");
    let download_contents = b"bar".to_vec();
    {
        let download_contents = download_contents.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |_request| {
                response_stream(vec![Ok(read_response(download_contents.clone()))])
            }));
    }
    f.fake_server.start();

    assert_eq!(
        download_blob(&f.context, &client, &digest).unwrap(),
        download_contents
    );
}

#[test]
fn download_blob_fails_when_server_completes_before_expected_size() {
    let mut f = Fixture::set_up();
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_verify_downloads = false;
    let client = f.new_client_with_options(&remote_options);
    let digest = digest_util().compute_as_utf8("abcdefg");
    f.fake_server
        .service_registry
        .add_byte_stream(byte_stream_read(|_request| {
            response_stream(vec![Ok(read_response("abc"))])
        }));
    f.fake_server.start();

    let e = download_blob(&f.context, &client, &digest).unwrap_err();
    let e = errors::downcast_ref::<OutputDigestMismatchException>(&e)
        .unwrap_or_else(|| panic!("expected an OutputDigestMismatchException, got {e}"));
    assert!(e.get_message().contains(&digest.hash));
    assert!(e.get_message().contains("received 3 bytes"));
}

/// `Zstd.compress(data)`.
fn zstd_compress(data: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(data, zstd::DEFAULT_COMPRESSION_LEVEL).unwrap()
}

#[test]
fn compressed_download_blob_is_retried_with_progress() {
    let mut f = Fixture::set_up();
    let mut options = RemoteOptions::default();
    options.cache_compression = true;
    options.cache_compression_threshold = 0;
    let client = f.new_client_with_options(&options);
    let digest = digest_util().compute_as_utf8("abcdefg");
    let chunk1 = zstd_compress(b"abc");
    let chunk2 = zstd_compress(b"def");
    let chunk3 = zstd_compress(b"g");
    {
        let digest = digest.clone();
        let first = AtomicBool::new(true);
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                assert!(request.resource_name.contains(&digest.hash));
                if first.swap(false, Ordering::SeqCst) {
                    return response_stream(vec![Err(Status::deadline_exceeded(""))]);
                }
                match request.read_offset {
                    0 => response_stream(vec![
                        Ok(read_response(chunk1.clone())),
                        Err(Status::deadline_exceeded("")),
                    ]),
                    3 => response_stream(vec![
                        Ok(read_response(chunk2.clone())),
                        Err(Status::deadline_exceeded("")),
                    ]),
                    6 => response_stream(vec![Ok(read_response(chunk3.clone()))]),
                    offset => panic!("unexpected offset {offset}"),
                }
            }));
    }
    f.fake_server.start();
    assert_eq!(
        String::from_utf8(download_blob(&f.context, &client, &digest).unwrap()).unwrap(),
        "abcdefg"
    );
}

#[test]
fn compressed_download_fails_when_server_ignores_read_offset() {
    let mut f = Fixture::set_up();
    let mut remote_options = RemoteOptions::default();
    remote_options.cache_compression = true;
    remote_options.cache_compression_threshold = 0;
    remote_options.remote_verify_downloads = false;
    let client = f.new_client_with_options(&remote_options);
    let digest = digest_util().compute_as_utf8("abcdefg");
    let compressed_prefix = zstd_compress(b"abcd");
    let compressed_blob = zstd_compress(b"abcdefg");
    let read_calls = Arc::new(AtomicI32::new(0));
    {
        let read_calls = read_calls.clone();
        f.fake_server
            .service_registry
            .add_byte_stream(byte_stream_read(move |request| {
                let read_call = read_calls.fetch_add(1, Ordering::SeqCst);
                if read_call == 0 {
                    assert_eq!(request.read_offset, 0);
                    return response_stream(vec![
                        Ok(read_response(compressed_prefix.clone())),
                        Err(Status::deadline_exceeded("")),
                    ]);
                }
                // Simulate a noncompliant compressed ByteStream server that ignores the
                // uncompressed read offset and replays the blob from the beginning.
                assert_eq!(read_call, 1);
                assert_eq!(request.read_offset, 4);
                response_stream(vec![Ok(read_response(compressed_blob.clone()))])
            }));
    }
    f.fake_server.start();

    let mut out: Vec<u8> = Vec::new();
    let e = get_from_future(client.download_blob(&f.context, &digest, &mut out)).unwrap_err();
    assert!(
        e.to_string().contains("Received more bytes than expected"),
        "{e}"
    );
    assert!(out.len() as i64 <= digest.size_bytes);
    assert_eq!(read_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn test_compressed_download() {
    // @TestParameter boolean overThreshold
    for over_threshold in [false, true] {
        let mut f = Fixture::set_up();
        let mut options = RemoteOptions::default();
        options.cache_compression = true;
        options.cache_compression_threshold = 100;
        let client = f.new_client_with_options(&options);
        let data: Vec<u8> = if over_threshold {
            "0123456789".repeat(10).into_bytes()
        } else {
            b"0123456789".to_vec()
        };
        let digest = digest_util().compute(&data);
        let bytes: Vec<u8> = if over_threshold {
            zstd_compress(&data)
        } else {
            data.clone()
        };

        {
            let digest = digest.clone();
            f.fake_server
                .service_registry
                .add_byte_stream(byte_stream_read(move |request| {
                    assert!(request.resource_name.contains(&digest.hash));
                    if over_threshold {
                        assert!(request.resource_name.contains("compressed-blobs/zstd"));
                    } else {
                        assert!(!request.resource_name.contains("compressed-blobs/zstd"));
                    }
                    response_stream(vec![
                        Ok(read_response(&bytes[..bytes.len() / 3])),
                        Ok(read_response(&bytes[bytes.len() / 3..bytes.len() / 3 * 2])),
                        Ok(read_response(&bytes[bytes.len() / 3 * 2..])),
                    ])
                }));
        }
        f.fake_server.start();
        assert_eq!(download_blob(&f.context, &client, &digest).unwrap(), data);
    }
}

#[test]
fn is_remote_cache_options_when_grpc_enabled() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("grpc://some-host.com".to_string());

    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_grpc_enabled_upper_case() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("GRPC://some-host.com".to_string());

    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_default_remote_cache_enabled_for_localhost() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("localhost:1234".to_string());

    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_default_remote_cache_enabled() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("some-host.com:1234".to_string());

    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_http_enabled() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("http://some-host.com".to_string());

    assert!(!GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_http_enabled_with_upper_case() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("HTTP://some-host.com".to_string());

    assert!(!GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_https_enabled() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("https://some-host.com".to_string());

    assert!(!GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_unknown_scheme() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("grp://some-host.com".to_string());

    // TODO(ishikhman): add proper vaildation and flip to false
    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_unknown_scheme_starts_as_grpc() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some("grpcsss://some-host.com".to_string());

    // TODO(ishikhman): add proper vaildation and flip to false
    assert!(GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_empty_cache_provided() {
    let mut options = RemoteOptions::default();
    options.remote_cache = Some(String::new());

    assert!(!GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
fn is_remote_cache_options_when_remote_cache_disabled() {
    let options = RemoteOptions::default();

    assert!(!GrpcCacheClient::is_remote_cache_options(&options));
}

#[test]
#[ignore = "GrpcCacheClient.splitBlob (content-defined chunking, --experimental_remote_cache_chunking) and BlobNotSplittableException are not ported"]
fn split_blob_server_cannot_split_fails_with_blob_not_splittable() {
    panic!("not ported: GrpcCacheClient.splitBlob");
}
