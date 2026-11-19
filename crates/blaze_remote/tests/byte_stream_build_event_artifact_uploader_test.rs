//! Port of `src/test/java/com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem` against an in-process gRPC server; here they
//! run on the real file system under a temporary directory (Java's `/file0` is
//! `<tmp>/file0`) against a fake server on localhost (`util::grpc_server`).
//!
//! Java spies on the `GrpcCacheClient` to replace `findMissingDigests` with a
//! `MissingDigestsFinder`; here [`FindMissingOverride`] wraps the client and overrides that
//! method only. Java's `verify(combinedCache, times(n)).uploadFile(...)` counts the
//! `ByteStream.Write` calls the fake server receives ([`WriteLog`]); where Java stubs
//! `uploadFile` to succeed, the fake server accepts every write.

mod util;

use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use blaze_bep::buildeventstream::{BuildEventArtifactUploader, LocalFile, LocalFileType};
use blaze_events::{EventKind, StoredEventHandler};
use blaze_exec::actions::FileArtifactValue;
use blaze_optdefs::misc::remote_build_event_upload_mode::RemoteBuildEventUploadMode;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_options::Options;
use blaze_remote::authandtls::NO_CREDENTIALS;
use blaze_remote::byte_stream_build_event_artifact_uploader::ByteStreamBuildEventArtifactUploader;
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::errors;
use blaze_remote::common::remote_cache_client::MissingDigestsFinder;
use blaze_remote::common::{ActionKey, Blob, RemoteActionExecutionContext, RemoteCacheClient};
use blaze_remote::grpc::ReferenceCountedChannel;
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc_cache_client::GrpcCacheClient;
use blaze_remote::reapi::bytestream::{QueryWriteStatusResponse, WriteRequest, WriteResponse};
use blaze_remote::reapi::{ActionResult, Digest, ServerCapabilities};
use blaze_remote::remote_retrier::RemoteRetrier;
use blaze_remote::retrier::{
    ALLOW_ALL_CALLS, Backoff, BackoffSupplier, RejectedExecutionException, ResultClassifierResult,
    RetryScheduler,
};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::tracing_metadata_utils;
use blaze_repo_fetch::fs_utils;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, UnixFileSystem};
use futures::future::BoxFuture;
use indexmap::IndexSet;
use parking_lot::Mutex;
use tonic::{Request, Status, Streaming};

use util::grpc_server::{FakeByteStream, FakeServer};

/// `DIGEST_UTIL`.
fn digest_util() -> DigestUtil {
    DigestUtil::new(DigestHashFunction::Sha256)
}

/// `new Random()` (`nextInt`, `nextBytes`): a xorshift generator seeded from the clock.
struct Random(u64);

impl Random {
    fn new() -> Random {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        Random(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        (self.next() % bound as u64) as i32
    }

    fn next_bytes(&mut self, bytes: &mut [u8]) {
        for b in bytes {
            *b = self.next() as u8;
        }
    }
}

/// `ByteStreamUploaderTest.FixedBackoff`.
struct FixedBackoff {
    max_retries: i32,
    delay_millis: i32,
    retries: Mutex<i32>,
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

/// `() -> new FixedBackoff(1, 0)`.
fn fixed_backoff(max_retries: i32, delay_millis: i32) -> BackoffSupplier {
    Arc::new(move || -> Arc<dyn Backoff> {
        Arc::new(FixedBackoff {
            max_retries,
            delay_millis,
            retries: Mutex::new(0),
        })
    })
}

/// `TestUtils.ZeroDelayListeningScheduledExecutorService`.
struct ZeroDelayRetryScheduler;

#[async_trait]
impl RetryScheduler for ZeroDelayRetryScheduler {
    async fn schedule(&self, _delay_millis: i64) -> Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// `TestUtils.newRemoteRetrier(() -> new FixedBackoff(1, 0), (e) -> Result.TRANSIENT_FAILURE,
/// retryService)`.
// Java: com/google/devtools/build/lib/remote/util/TestUtils.java:46 TestUtils.newRemoteRetrier
fn new_remote_retrier() -> Arc<RemoteRetrier> {
    Arc::new(RemoteRetrier::with_sleeper(
        fixed_backoff(1, 0),
        Arc::new(|e: &io::Error| {
            if errors::status_from_throwable(e).map(|s| s.code()) == Some(tonic::Code::Cancelled) {
                ResultClassifierResult::Success
            } else {
                ResultClassifierResult::TransientFailure
            }
        }),
        Arc::new(ZeroDelayRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
        Arc::new(|_millis: i64| -> io::Result<()> { Ok(()) }),
    ))
}

/// The resource names of the `ByteStream.Write` calls the fake server received.
#[derive(Default)]
struct WriteLog {
    resource_names: Mutex<Vec<String>>,
}

impl WriteLog {
    fn writes_of(&self, digest: &Digest) -> usize {
        self.resource_names
            .lock()
            .iter()
            .filter(|name| name.contains(&format!("/blobs/{}/", digest.hash)))
            .count()
    }

    fn count(&self) -> usize {
        self.resource_names.lock().len()
    }
}

type WriteFuture = BoxFuture<'static, Result<WriteResponse, Status>>;

/// `ByteStreamUploaderTest.MaybeFailOnceUploadService`: accepts uploads of the known blobs; an
/// upload has a 10% chance to fail once with `INTERNAL` (to exercise the retries).
/// `fail_hash` stands for the anonymous subclass of `someUploadsFail_succeedsWithWarningMessages`:
/// a write of that blob fails with `CANCELLED`.
fn maybe_fail_once_upload_service(
    blobs_by_hash: HashMap<String, Vec<u8>>,
    fail_hash: Option<String>,
    log: Arc<WriteLog>,
) -> FakeByteStream {
    let blobs_by_hash = Arc::new(blobs_by_hash);
    let uploads_failed_once: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let rand = Arc::new(Mutex::new(Random::new()));
    FakeByteStream {
        write: Some(Box::new(
            move |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
                let blobs_by_hash = blobs_by_hash.clone();
                let uploads_failed_once = uploads_failed_once.clone();
                let rand = rand.clone();
                let fail_hash = fail_hash.clone();
                let log = log.clone();
                Box::pin(async move {
                    let mut requests = request.into_inner();
                    let mut digest_hash: Option<String> = None;
                    let mut received_data: Vec<u8> = Vec::new();
                    let mut next_offset = 0usize;
                    while let Some(write_request) = requests.message().await? {
                        if next_offset == 0 {
                            let resource_name = &write_request.resource_name;
                            assert!(!resource_name.is_empty());
                            log.resource_names.lock().push(resource_name.clone());
                            if let Some(fail_hash) = &fail_hash
                                && resource_name.contains(fail_hash.as_str())
                            {
                                return Err(Status::cancelled(""));
                            }

                            let components: Vec<&str> = resource_name.split('/').collect();
                            assert_eq!(components.len(), 6);
                            digest_hash = Some(components[4].to_string());
                            assert!(blobs_by_hash.contains_key(components[4]));
                            received_data = vec![0u8; components[5].parse::<usize>().unwrap()];
                        }
                        let hash = digest_hash.clone().expect("digestHash is not null");
                        // An upload for a given blob has a 10% chance to fail once during its
                        // lifetime. This is to exercise the retry mechanism a bit.
                        let should_fail = rand.lock().next_int(10) == 0
                            && !uploads_failed_once.lock().contains(&hash);
                        if should_fail {
                            uploads_failed_once.lock().insert(hash);
                            return Err(Status::internal(""));
                        }
                        let data = &write_request.data;
                        received_data[next_offset..next_offset + data.len()].copy_from_slice(data);
                        next_offset += data.len();
                        let last_write = next_offset == received_data.len();
                        assert_eq!(write_request.finish_write, last_write);
                    }
                    let expected_blob = blobs_by_hash
                        .get(digest_hash.as_ref().expect("digestHash is not null"))
                        .expect("the blob is known");
                    assert_eq!(&received_data, expected_blob);
                    Ok(WriteResponse {
                        committed_size: received_data.len() as i64,
                    })
                })
            },
        )),
        query_write_status: Some(Box::new(|_request| {
            // force the client to reset the write
            Ok(QueryWriteStatusResponse {
                committed_size: 0,
                complete: false,
            })
        })),
        ..Default::default()
    }
}

/// A server that accepts every write (`doAnswer(.. immediateFuture(null)).when(combinedCache)
/// .uploadFile(..)`) and logs it.
fn accept_all_upload_service(log: Arc<WriteLog>) -> FakeByteStream {
    FakeByteStream {
        write: Some(Box::new(
            move |request: Request<Streaming<WriteRequest>>| -> WriteFuture {
                let log = log.clone();
                Box::pin(async move {
                    let mut requests = request.into_inner();
                    let mut size = 0i64;
                    let mut first = true;
                    while let Some(write_request) = requests.message().await? {
                        if first {
                            log.resource_names
                                .lock()
                                .push(write_request.resource_name.clone());
                            first = false;
                        }
                        size += write_request.data.len() as i64;
                    }
                    Ok(WriteResponse {
                        committed_size: size,
                    })
                })
            },
        )),
        ..Default::default()
    }
}

/// `MissingDigestsFinder` of the tests.
trait TestMissingDigestsFinder: Send + Sync {
    fn find_missing_digests(&self, digests: &[Digest]) -> IndexSet<Digest>;
}

/// `AllMissingDigestsFinder`.
struct AllMissingDigestsFinder;

impl TestMissingDigestsFinder for AllMissingDigestsFinder {
    fn find_missing_digests(&self, digests: &[Digest]) -> IndexSet<Digest> {
        digests.iter().cloned().collect()
    }
}

/// `StaticMissingDigestsFinder(knownDigests)`: the digests not in `known_digests` are missing.
struct StaticMissingDigestsFinder {
    known_digests: HashSet<Digest>,
}

impl TestMissingDigestsFinder for StaticMissingDigestsFinder {
    fn find_missing_digests(&self, digests: &[Digest]) -> IndexSet<Digest> {
        digests
            .iter()
            .filter(|d| !self.known_digests.contains(*d))
            .cloned()
            .collect()
    }
}

/// `spy(new GrpcCacheClient(..))` with `findMissingDigests` answered by a
/// `MissingDigestsFinder`; `find_missing_calls` is `verify(digestQuerier, times(n))`.
struct FindMissingOverride {
    delegate: Arc<GrpcCacheClient>,
    finder: Box<dyn TestMissingDigestsFinder>,
    find_missing_calls: AtomicI32,
}

#[async_trait]
impl MissingDigestsFinder for FindMissingOverride {
    async fn find_missing_digests(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        self.find_missing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.finder.find_missing_digests(digests))
    }
}

#[async_trait]
impl RemoteCacheClient for FindMissingOverride {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        self.delegate.cas_upload_cache()
    }

    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        self.delegate.get_server_capabilities()
    }

    async fn get_authority(&self) -> io::Result<String> {
        self.delegate.get_authority().await
    }

    async fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &HashSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        self.delegate
            .download_action_result(context, action_key, inline_out_err, inline_output_files)
            .await
    }

    async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        self.delegate
            .upload_action_result(context, action_key, action_result)
            .await
    }

    async fn download_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        out: &mut (dyn Write + Send),
    ) -> io::Result<()> {
        self.delegate.download_blob(context, digest, out).await
    }

    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        self.delegate.upload_blob_impl(context, digest, blob).await
    }

    fn close(&self) {
        self.delegate.close();
    }
}

/// The fields of the Java test class, as `setUp()` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    root: String,
    fs: FileSystemRef,
    event_handler: Arc<StoredEventHandler>,
    server: FakeServer,
    write_log: Arc<WriteLog>,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderTest.java:107 ByteStreamBuildEventArtifactUploaderTest.setUp
    fn set_up() -> Fixture {
        tracing_metadata_utils::set_tool_version("test");
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        Fixture {
            _dir: dir,
            root,
            fs: UnixFileSystem::create(DigestHashFunction::Sha256),
            event_handler: Arc::new(StoredEventHandler::new()),
            server: FakeServer::new(),
            write_log: Arc::new(WriteLog::default()),
        }
    }

    /// `fs.getPath(path)` (under the temporary directory).
    fn path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{path}", self.root), &self.fs)
    }

    /// `new ReferenceCountedChannel(channelConnectionFactory)` (default capabilities).
    fn new_channel(&self) -> ReferenceCountedChannel {
        let capabilities_fetcher: CapabilitiesFetcher = Arc::new(
            |_channel: ReferenceCountedChannel| -> BoxFuture<'static, io::Result<ServerCapabilities>> {
                Box::pin(async { Ok(ServerCapabilities::default()) })
            },
        );
        self.server.channel(Vec::new(), Some(capabilities_fetcher))
    }

    /// `newCombinedCache(channel, retrier)`.
    fn new_combined_cache(
        &self,
        channel: &ReferenceCountedChannel,
    ) -> (Arc<CombinedCache>, Arc<FindMissingOverride>) {
        self.new_combined_cache_with(channel, Box::new(AllMissingDigestsFinder))
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderTest.java:674 ByteStreamBuildEventArtifactUploaderTest.newCombinedCache
    fn new_combined_cache_with(
        &self,
        channel: &ReferenceCountedChannel,
        missing_digests_finder: Box<dyn TestMissingDigestsFinder>,
    ) -> (Arc<CombinedCache>, Arc<FindMissingOverride>) {
        let mut remote_options = Options::<RemoteOptions>::get_defaults();
        remote_options.remote_instance_name = "instance".to_string();
        let cache_client = Arc::new(GrpcCacheClient::new(
            channel.clone(),
            NO_CREDENTIALS.clone(),
            &remote_options,
            new_remote_retrier(),
            digest_util(),
        ));
        let spy = Arc::new(FindMissingOverride {
            delegate: cache_client,
            finder: missing_digests_finder,
            find_missing_calls: AtomicI32::new(0),
        });
        let client: Arc<dyn RemoteCacheClient> = spy.clone();
        let combined_cache = Arc::new(CombinedCache::new(
            Some(client),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            digest_util(),
            /* chunkingFunction= */ None,
        ));
        (combined_cache, spy)
    }

    /// `newArtifactUploader(combinedCache)` (mode `ALL`).
    fn new_artifact_uploader(
        &self,
        combined_cache: &Arc<CombinedCache>,
    ) -> ByteStreamBuildEventArtifactUploader {
        self.new_artifact_uploader_with_mode(combined_cache, RemoteBuildEventUploadMode::ALL)
    }

    // Java: com/google/devtools/build/lib/remote/ByteStreamBuildEventArtifactUploaderTest.java:707 ByteStreamBuildEventArtifactUploaderTest.newArtifactUploader
    fn new_artifact_uploader_with_mode(
        &self,
        combined_cache: &Arc<CombinedCache>,
        remote_build_event_upload_mode: RemoteBuildEventUploadMode,
    ) -> ByteStreamBuildEventArtifactUploader {
        ByteStreamBuildEventArtifactUploader::new(
            self.event_handler.clone(),
            /* verbose_failures= */ true,
            combined_cache.clone(),
            /* remote_instance_name= */ String::new(),
            /* remote_bytestream_uri_prefix= */ Some("localhost/instance".to_string()),
            /* build_request_id= */ "none".to_string(),
            /* command_id= */ "none".to_string(),
            remote_build_event_upload_mode,
        )
    }

    /// Writes `num_uploads` random files `/file<i>` and returns them with their blobs by hash.
    fn random_files(
        &self,
        num_uploads: usize,
    ) -> (HashMap<String, Vec<u8>>, HashMap<Path, LocalFile>) {
        let mut blobs_by_hash = HashMap::new();
        let mut files_to_upload = HashMap::new();
        let mut rand = Random::new();
        for i in 0..num_uploads {
            let file = self.path(&format!("/file{i}"));
            let blob_size = rand.next_int(100) as usize + 1;
            let mut blob = vec![0u8; blob_size];
            rand.next_bytes(&mut blob);
            fs_utils::write_content(&file, &blob).unwrap();
            blobs_by_hash.insert(digest_util().compute_path(&file).unwrap().hash, blob);
            files_to_upload.insert(
                file.clone(),
                LocalFile::new(
                    file,
                    LocalFileType::OutputFile,
                    /* artifact_metadata= */ None,
                ),
            );
        }
        (blobs_by_hash, files_to_upload)
    }
}

/// `BaseEncoding.base16().lowerCase().encode(file.getDigest())` and `file.getFileSize()`.
fn hash_and_size(file: &Path) -> (String, i64) {
    (
        hex::encode(file.get_digest().unwrap()),
        file.get_file_size().unwrap(),
    )
}

fn uploads_should_work_with(num_uploads: usize) {
    let mut f = Fixture::set_up();
    let (blobs_by_hash, files_to_upload) = f.random_files(num_uploads);
    f.server
        .service_registry
        .add_byte_stream(maybe_fail_once_upload_service(
            blobs_by_hash,
            None,
            f.write_log.clone(),
        ));
    f.server.start();

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader
        .upload_and_wait(files_to_upload.clone())
        .unwrap();
    for file in files_to_upload.keys() {
        let (hash, size) = hash_and_size(file);
        let conversion = path_converter.apply(file);
        assert_eq!(
            conversion.as_deref(),
            Some(format!("bytestream://localhost/instance/blobs/{hash}/{size}").as_str())
        );
    }

    artifact_uploader.release();

    assert_eq!(combined_cache.ref_cnt(), 0);
    assert!(ref_cnt_channel.is_shutdown());
}

#[test]
fn uploads_should_work() {
    uploads_should_work_with(2);
}

#[test]
fn uploads_should_work_fewer_permits_than_uploads() {
    // number of permits is less than number of uploads to affirm permit is released
    uploads_should_work_with(2);
}

#[test]
fn directory_not_uploaded() {
    let mut f = Fixture::set_up();
    f.server.start();
    let dir = f.path("/dir");
    let mut files_to_upload = HashMap::new();
    files_to_upload.insert(
        dir.clone(),
        LocalFile::new(
            dir.clone(),
            LocalFileType::OutputDirectory,
            /* artifact_metadata= */ None,
        ),
    );
    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader.upload_and_wait(files_to_upload).unwrap();
    assert_eq!(path_converter.apply(&dir), None);
    artifact_uploader.release();
}

#[test]
fn symlink_not_uploaded() {
    let mut f = Fixture::set_up();
    f.server.start();
    let sym = f.path("/sym");
    let mut files_to_upload = HashMap::new();
    files_to_upload.insert(
        sym.clone(),
        LocalFile::new(
            sym.clone(),
            LocalFileType::OutputSymlink,
            /* artifact_metadata= */ None,
        ),
    );
    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader.upload_and_wait(files_to_upload).unwrap();
    assert_eq!(path_converter.apply(&sym), None);
    artifact_uploader.release();
}

#[test]
#[ignore = "assumeNotNull(BazelHashFunctions.BLAKE3): Blaze's file systems offer SHA-256 only \
            (blaze_vfs::DigestHashFunction), so Java skips it here too; the URI format of a \
            new-style digest function is covered by the unit test \
            path_converter_names_new_style_digest_functions"]
fn custom_hash_function_uploaded() {}

#[test]
fn test_outputs_uploaded_if_files() {
    let mut f = Fixture::set_up();
    f.server
        .service_registry
        .add_byte_stream(accept_all_upload_service(f.write_log.clone()));
    f.server.start();
    let successful_file = f.path("/test.file.passed");
    fs_utils::write_content(&successful_file, b"").unwrap();
    let successful_dir = f.path("/test.dir.passed");
    successful_dir.create_directory().unwrap();
    let failed_file = f.path("/test.file.failed");
    fs_utils::write_content(&failed_file, b"").unwrap();
    let failed_dir = f.path("/test.dir.failed");
    failed_dir.create_directory().unwrap();
    let mut files_to_upload = HashMap::new();
    for (path, type_) in [
        (&successful_file, LocalFileType::SuccessfulTestOutput),
        (&failed_file, LocalFileType::FailedTestOutput),
        (&successful_dir, LocalFileType::SuccessfulTestOutput),
        (&failed_dir, LocalFileType::FailedTestOutput),
    ] {
        files_to_upload.insert(
            path.clone(),
            LocalFile::new(path.clone(), type_, /* artifact_metadata= */ None),
        );
    }
    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader.upload_and_wait(files_to_upload).unwrap();
    assert!(path_converter.apply(&successful_file).is_some());
    assert!(path_converter.apply(&failed_file).is_some());
    assert_eq!(path_converter.apply(&successful_dir), None);
    assert_eq!(path_converter.apply(&failed_dir), None);
    assert!(f.event_handler.get_events().is_empty());
    artifact_uploader.release();
}

#[test]
fn some_uploads_fail_succeeds_with_warning_messages() {
    // Test that if one of multiple file uploads fails, the upload future succeeds but the
    // error is reported correctly.

    let mut f = Fixture::set_up();
    let (blobs_by_hash, files_to_upload) = f.random_files(10);
    let hash_of_blob_that_should_fail = blobs_by_hash.keys().next().unwrap().clone();
    f.server
        .service_registry
        .add_byte_stream(maybe_fail_once_upload_service(
            blobs_by_hash,
            Some(hash_of_blob_that_should_fail),
            f.write_log.clone(),
        ));
    f.server.start();

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    artifact_uploader.upload_and_wait(files_to_upload).unwrap();

    let events = f.event_handler.get_events();
    assert!(!events.is_empty());
    assert_eq!(events[0].get_kind(), EventKind::Warning);
    assert!(
        events[0].get_message().contains(&format!(
            "Uploading BEP referenced local file {}/file",
            f.root
        )),
        "{}",
        events[0].get_message()
    );

    artifact_uploader.release();

    assert_eq!(combined_cache.ref_cnt(), 0);
    assert!(ref_cnt_channel.is_shutdown());
}

#[test]
#[ignore = "needs a Path on a RemoteActionFileSystem that blaze_remote can recognise: \
            blaze_vfs::FileSystem has no downcast (as_any), so isRemoteFile fails loudly for \
            such a path (ByteStreamBuildEventArtifactUploader::is_remote_file)"]
fn remote_file_should_not_be_uploaded_action_fs() {}

#[test]
fn remote_file_should_not_be_uploaded_find_missing_digests() {
    // Test that findMissingDigests is called to check which files exist remotely
    // and that those are not uploaded.

    // arrange
    let mut f = Fixture::set_up();
    f.server
        .service_registry
        .add_byte_stream(accept_all_upload_service(f.write_log.clone()));
    f.server.start();
    let remote_file = f.path("/remote-file");
    fs_utils::write_content(&remote_file, b"hello world").unwrap();
    let remote_digest = digest_util().compute_path(&remote_file).unwrap();
    let local_file = f.path("/local-file");
    fs_utils::write_content(&local_file, b"foo bar").unwrap();
    let local_digest = digest_util().compute_path(&local_file).unwrap();

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, digest_querier) = f.new_combined_cache_with(
        &ref_cnt_channel,
        Box::new(StaticMissingDigestsFinder {
            known_digests: HashSet::from([remote_digest.clone()]),
        }),
    );
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    // act
    let files = HashMap::from([
        (
            remote_file.clone(),
            LocalFile::new(
                remote_file.clone(),
                LocalFileType::OutputFile,
                /* artifact_metadata= */ None,
            ),
        ),
        (
            local_file.clone(),
            LocalFile::new(
                local_file.clone(),
                LocalFileType::OutputFile,
                /* artifact_metadata= */ None,
            ),
        ),
    ]);
    let path_converter = artifact_uploader.upload_and_wait(files).unwrap();

    // assert
    assert_eq!(digest_querier.find_missing_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.write_log.writes_of(&local_digest), 1);
    assert_eq!(f.write_log.count(), 1);
    assert!(
        path_converter
            .apply(&remote_file)
            .unwrap()
            .contains(&remote_digest.hash)
    );
    assert!(
        path_converter
            .apply(&local_file)
            .unwrap()
            .contains(&local_digest.hash)
    );
}

#[test]
fn file_with_metadata_digest_reused_and_file_not_read() {
    // arrange
    let mut f = Fixture::set_up();
    f.server
        .service_registry
        .add_byte_stream(accept_all_upload_service(f.write_log.clone()));
    f.server.start();
    let blob = "contents of a file that is not present locally".as_bytes();
    let digest = digest_util().compute(blob);
    let file = f.path("/file");
    let metadata = FileArtifactValue::create_for_virtual_action_input(
        hex::decode(&digest.hash).unwrap(),
        digest.size_bytes,
    );

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, digest_querier) = f.new_combined_cache_with(
        &ref_cnt_channel,
        Box::new(StaticMissingDigestsFinder {
            known_digests: HashSet::from([digest.clone()]),
        }),
    );
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    // act
    let path_converter = artifact_uploader
        .upload_and_wait(HashMap::from([(
            file.clone(),
            LocalFile::new(file.clone(), LocalFileType::OutputFile, Some(metadata)),
        )]))
        .unwrap();

    // assert
    assert_eq!(digest_querier.find_missing_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.write_log.count(), 0);
    assert_eq!(
        path_converter.apply(&file).as_deref(),
        Some(
            format!(
                "bytestream://localhost/instance/blobs/{}/{}",
                digest.hash, digest.size_bytes
            )
            .as_str()
        )
    );
    assert!(f.event_handler.get_events().is_empty());
}

#[test]
fn file_with_remote_metadata_not_queried_or_uploaded() {
    // arrange
    let mut f = Fixture::set_up();
    f.server
        .service_registry
        .add_byte_stream(accept_all_upload_service(f.write_log.clone()));
    f.server.start();
    let blob = "contents of a remote file".as_bytes();
    let digest = digest_util().compute(blob);
    let file = f.path("/file");
    let metadata = FileArtifactValue::create_for_remote_file(
        hex::decode(&digest.hash).unwrap(),
        digest.size_bytes,
        /* location_index= */ 1,
    );

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, digest_querier) = f.new_combined_cache_with(
        &ref_cnt_channel,
        Box::new(StaticMissingDigestsFinder {
            known_digests: HashSet::new(),
        }),
    );
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    // act
    let path_converter = artifact_uploader
        .upload_and_wait(HashMap::from([(
            file.clone(),
            LocalFile::new(file.clone(), LocalFileType::OutputFile, Some(metadata)),
        )]))
        .unwrap();

    // assert
    assert_eq!(digest_querier.find_missing_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.write_log.count(), 0);
    assert_eq!(
        path_converter.apply(&file).as_deref(),
        Some(
            format!(
                "bytestream://localhost/instance/blobs/{}/{}",
                digest.hash, digest.size_bytes
            )
            .as_str()
        )
    );
    assert!(f.event_handler.get_events().is_empty());
}

#[test]
fn file_with_metadata_minimal_mode_digest_reused_and_file_not_read() {
    // arrange
    let mut f = Fixture::set_up();
    f.server
        .service_registry
        .add_byte_stream(accept_all_upload_service(f.write_log.clone()));
    f.server.start();
    let blob = "contents of a file that is not present locally".as_bytes();
    let digest = digest_util().compute(blob);
    let file = f.path("/file");
    let metadata = FileArtifactValue::create_for_virtual_action_input(
        hex::decode(&digest.hash).unwrap(),
        digest.size_bytes,
    );

    let ref_cnt_channel = f.new_channel();
    let (combined_cache, digest_querier) = f.new_combined_cache_with(
        &ref_cnt_channel,
        Box::new(StaticMissingDigestsFinder {
            known_digests: HashSet::new(),
        }),
    );
    let artifact_uploader =
        f.new_artifact_uploader_with_mode(&combined_cache, RemoteBuildEventUploadMode::MINIMAL);

    // act
    let path_converter = artifact_uploader
        .upload_and_wait(HashMap::from([(
            file.clone(),
            LocalFile::new(file.clone(), LocalFileType::OutputFile, Some(metadata)),
        )]))
        .unwrap();

    // assert
    assert_eq!(digest_querier.find_missing_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.write_log.count(), 0);
    assert_eq!(
        path_converter.apply(&file).as_deref(),
        Some(
            format!(
                "bytestream://localhost/instance/blobs/{}/{}",
                digest.hash, digest.size_bytes
            )
            .as_str()
        )
    );
    assert!(f.event_handler.get_events().is_empty());
}

#[test]
fn file_with_directory_metadata_not_uploaded() {
    let mut f = Fixture::set_up();
    f.server.start();
    let dir = f.path("/dir");
    let metadata = FileArtifactValue::create_for_directory_with_mtime(0);
    let files_to_upload = HashMap::from([(
        dir.clone(),
        LocalFile::new(
            dir.clone(),
            LocalFileType::SuccessfulTestOutput,
            Some(metadata),
        ),
    )]);
    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader.upload_and_wait(files_to_upload).unwrap();
    assert_eq!(path_converter.apply(&dir), None);
    assert!(f.event_handler.get_events().is_empty());
    artifact_uploader.release();
}

#[test]
fn file_with_symlink_metadata_not_uploaded() {
    let mut f = Fixture::set_up();
    f.server.start();
    let sym = f.path("/sym");
    sym.create_symbolic_link_to_fragment(&blaze_vfs::PathFragment::create("target"))
        .unwrap();
    let metadata = FileArtifactValue::create_for_unresolved_symlink(&sym).unwrap();
    sym.delete().unwrap();
    let files_to_upload = HashMap::from([(
        sym.clone(),
        LocalFile::new(sym.clone(), LocalFileType::OutputFile, Some(metadata)),
    )]);
    let ref_cnt_channel = f.new_channel();
    let (combined_cache, _) = f.new_combined_cache(&ref_cnt_channel);
    let artifact_uploader = f.new_artifact_uploader(&combined_cache);

    let path_converter = artifact_uploader.upload_and_wait(files_to_upload).unwrap();
    assert_eq!(path_converter.apply(&sym), None);
    assert!(f.event_handler.get_events().is_empty());
    artifact_uploader.release();
}
