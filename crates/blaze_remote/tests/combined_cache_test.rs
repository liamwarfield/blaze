// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/CombinedCacheTest.java`.
//!
//! Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under a
//! temporary directory, so Java's absolute paths (`/execroot/main`) are prefixed with that
//! directory. Java futures are eager; where a test starts a transfer without waiting for it, the
//! port polls the future once (`futures::poll!`) and cancels it by dropping it.

mod util;

use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::task::{Context, Poll};

use async_trait::async_trait;
use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::{
    ActionInputRef, ExecException, FakeOwner, FileArtifactValue, FilesetOutputTree, InputMapping,
    InputMetadataProvider, ResourceSet, RunfilesArtifactValue, SimpleSpawn, TreeArtifactValue,
};
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::remote_options::{ChunkingFunctionValue, RemoteOptions};
use blaze_remote::authandtls::NO_CREDENTIALS;
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::CacheNotFoundException;
use blaze_remote::common::errors::{self, io_exception};
use blaze_remote::common::{
    ActionKey, Blob, MissingDigestsFinder, RemoteActionExecutionContext, RemoteCacheClient,
};
use blaze_remote::grpc::ReferenceCountedChannel;
use blaze_remote::grpc::channel::CapabilitiesFetcher;
use blaze_remote::grpc_cache_client::GrpcCacheClient;
use blaze_remote::reapi::{
    ActionResult, CacheCapabilities, Digest, FastCdc2020Params, FindMissingBlobsResponse,
    RequestMetadata, ServerCapabilities, SpliceBlobResponse, SplitBlobResponse,
};
use blaze_remote::remote_retrier::{EXPERIMENTAL_GRPC_RESULT_CLASSIFIER, RemoteRetrier};
use blaze_remote::retrier::{ALLOW_ALL_CALLS, TokioRetryScheduler};
use blaze_remote::runtime::{block_on, get_from_future};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::tracing_metadata_utils;
use blaze_remote::util::utils::wait_for_bulk_transfer;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use futures::future::BoxFuture;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use tonic::Status;
use util::grpc_server::{FakeByteStream, FakeContentAddressableStorage, FakeServer, read_response};
use util::in_memory_cache_client::{InMemoryCacheClient, InMemoryCombinedCache};

/// An `InputMetadataProvider` mock.
struct NoMetadata;

impl InputMetadataProvider for NoMetadata {
    fn get_input_metadata(&self, _input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        Ok(None)
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        None
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, _exec_path: &PathFragment) -> Option<ActionInputRef> {
        None
    }
}

/// `mock(SpawnExecutionContext.class)`: records the reported progress.
struct MockSpawnExecutionContext {
    reported: Mutex<Vec<ProgressStatus>>,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
}

impl SpawnExecutionContext for MockSpawnExecutionContext {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, _digest: blaze_protos::tools::protos::Digest) {}
    fn get_digest(&self) -> Option<blaze_protos::tools::protos::Digest> {
        None
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        Ok(())
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &NoMetadata
    }
    fn lock_output_files(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException> {
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> blaze_util::java_duration::JavaDuration {
        blaze_util::java_duration::JavaDuration::ZERO
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_input_mapping(
        &self,
        _base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        Arc::new(InputMapping::new())
    }
    fn report(&self, progress: ProgressStatus) {
        self.reported.lock().push(progress);
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

/// What a stubbed method of [`SpyCacheClient`] does for one call: `Some(future)` is Mockito's
/// `doAnswer(...)` result, `None` is `invocationOnMock.callRealMethod()`.
type Answer = Box<dyn Fn(&Digest) -> Option<BoxFuture<'static, io::Result<()>>> + Send + Sync>;

/// The answer of a stub: `future` instead of the real method.
fn answer(
    future: impl Future<Output = io::Result<()>> + Send + 'static,
) -> Option<BoxFuture<'static, io::Result<()>>> {
    Some(Box::pin(future))
}

/// `spy(new InMemoryCacheClient())` / `mock(RemoteCacheClient.class)` with `doAnswer` stubs for
/// `downloadBlob` and `uploadBlobImpl`.
struct SpyCacheClient {
    delegate: InMemoryCacheClient,
    download_blob_answer: Option<Answer>,
    upload_blob_impl_answer: Option<Answer>,
}

impl SpyCacheClient {
    fn new() -> SpyCacheClient {
        SpyCacheClient {
            delegate: InMemoryCacheClient::new(),
            download_blob_answer: None,
            upload_blob_impl_answer: None,
        }
    }

    /// `doAnswer(answer).when(remoteCacheClient).downloadBlob(any(), any(), any())`.
    fn do_answer_download_blob(
        mut self,
        answer: impl Fn(&Digest) -> Option<BoxFuture<'static, io::Result<()>>> + Send + Sync + 'static,
    ) -> SpyCacheClient {
        self.download_blob_answer = Some(Box::new(answer));
        self
    }

    /// `doAnswer(answer).when(remoteCacheClient).uploadBlobImpl(any(), any(), any())`.
    fn do_answer_upload_blob_impl(
        mut self,
        answer: impl Fn(&Digest) -> Option<BoxFuture<'static, io::Result<()>>> + Send + Sync + 'static,
    ) -> SpyCacheClient {
        self.upload_blob_impl_answer = Some(Box::new(answer));
        self
    }
}

#[async_trait]
impl MissingDigestsFinder for SpyCacheClient {
    async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        self.delegate.find_missing_digests(context, digests).await
    }
}

#[async_trait]
impl RemoteCacheClient for SpyCacheClient {
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
        inline_output_files: &IndexSet<String>,
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
        let answer = self.download_blob_answer.as_ref().and_then(|a| a(digest));
        match answer {
            Some(future) => future.await,
            None => self.delegate.download_blob(context, digest, out).await,
        }
    }

    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        let answer = self
            .upload_blob_impl_answer
            .as_ref()
            .and_then(|a| a(digest));
        match answer {
            Some(future) => future.await,
            None => self.delegate.upload_blob_impl(context, digest, blob).await,
        }
    }

    fn close(&self) {
        self.delegate.close();
    }
}

/// `SettableFuture.create()` that is never set: it never completes, and records its
/// cancellation (the drop of the future).
struct SettableFuture {
    cancelled: Arc<AtomicBool>,
}

impl Future for SettableFuture {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl Drop for SettableFuture {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// The fields of the Java test class, as `setUp` leaves them.
struct Fixture {
    _dir: tempfile::TempDir,
    fs: FileSystemRef,
    root: String,
    metadata: RequestMetadata,
    spawn: SimpleSpawn,
    spawn_execution_context: MockSpawnExecutionContext,
    exec_root: Path,
    digest_util: DigestUtil,
}

impl Fixture {
    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:142 CombinedCacheTest.setUp
    fn set_up() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        // PORT: Java reads the version from BlazeVersionInfo.
        tracing_metadata_utils::set_tool_version("test");
        let metadata = tracing_metadata_utils::build_metadata("none", "none", "action-id", None);
        let spawn = SimpleSpawn::new_simple(
            Arc::new(FakeOwner::new(
                "foo",
                Some("bar"),
                Some(Label::parse_canonical_unchecked("//dummy:label")),
            )),
            /* arguments= */ vec![],
            /* environment= */ IndexMap::new(),
            /* executionInfo= */ IndexMap::new(),
            /* inputs= */ NestedSetBuilder::stable_order().build(),
            /* outputs= */ vec![],
            Arc::new(ResourceSet::zero()),
        );
        let exec_root = Path::create_from_str(&format!("{root}/execroot/main"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let spawn_execution_context = MockSpawnExecutionContext {
            reported: Mutex::new(Vec::new()),
            file_out_err: FileOutErr::new(
                exec_root.get_relative_str("mock-stdout"),
                exec_root.get_relative_str("mock-stderr"),
            ),
            client_env: IndexMap::new(),
        };
        // artifactRoot = ArtifactRoot.asDerivedRoot(execRoot, RootType.OUTPUT, "outputs");
        exec_root
            .get_relative_str("outputs")
            .create_directory_and_parents()
            .unwrap();
        Fixture {
            _dir: dir,
            fs,
            root,
            metadata,
            spawn,
            spawn_execution_context,
            exec_root,
            digest_util: DigestUtil::new(DigestHashFunction::Sha256),
        }
    }

    /// `remoteActionExecutionContext`.
    fn remote_action_execution_context(&self) -> RemoteActionExecutionContext<'_> {
        RemoteActionExecutionContext::create_for_spawn(
            &self.spawn,
            &self.spawn_execution_context,
            self.metadata.clone(),
        )
    }

    /// `fs.getPath(path)` for a Java test path such as "/execroot/file".
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{}", self.root, path), &self.fs)
    }

    /// `fakeFileCache.createScratchInput(ActionInputHelper.fromPath(execPath), content)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java FakeActionInputFileCache.createScratchInput
    fn create_scratch_input(&self, exec_path: &str, content: &str) -> Digest {
        let input_file = self.exec_root.get_relative_str(exec_path);
        input_file
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        blaze_repo_fetch::fs_utils::write_content(&input_file, content.as_bytes()).unwrap();
        self.digest_util.compute_path(&input_file).unwrap()
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1130 CombinedCacheTest.newCombinedCache
    fn new_combined_cache(&self) -> InMemoryCombinedCache {
        InMemoryCombinedCache::new(self.digest_util.clone())
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1134 CombinedCacheTest.newCombinedCache
    fn new_combined_cache_with_symlink_template(
        &self,
        cas_entries: HashMap<Digest, Vec<u8>>,
        digest_util: DigestUtil,
        symlink_template: Option<&str>,
    ) -> InMemoryCombinedCache {
        InMemoryCombinedCache::new_with_symlink_template(
            cas_entries,
            digest_util,
            symlink_template.map(str::to_string),
        )
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1139 CombinedCacheTest.newCombinedCache
    fn new_combined_cache_with_client(
        &self,
        remote_cache_client: Arc<dyn RemoteCacheClient>,
    ) -> CombinedCache {
        CombinedCache::new(
            Some(remote_cache_client),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            self.digest_util.clone(),
            /* chunkingFunction= */ None,
        )
    }
}

#[test]
fn test_download_empty_blob_and_file() {
    // Test that downloading an empty BLOB/file does not try to perform a download.

    // arrange
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let file = f.get_path("/execroot/file");
    let combined_cache = f.new_combined_cache();
    let empty_digest = f.digest_util.compute(&[]);

    // act and assert
    assert!(
        get_from_future(
            combined_cache.download_blob(&remote_action_execution_context, &empty_digest)
        )
        .unwrap()
        .is_empty()
    );

    {
        let _out = file.get_output_stream().unwrap();
        get_from_future(combined_cache.download_file(
            &remote_action_execution_context,
            &file,
            &empty_digest,
        ))
        .unwrap();
    }
    assert!(file.exists());
    assert_eq!(file.get_file_size().unwrap(), 0);
}

#[test]
fn download_action_result_reports_spawn_checking_cache_event() {
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let combined_cache = f.new_combined_cache();
    let _unused = combined_cache.download_action_result(
        &remote_action_execution_context,
        &f.digest_util
            .as_action_key(f.digest_util.compute_as_utf8("key")),
        /* inlineOutErr= */ false,
        /* inlineOutputFiles= */ &indexmap::IndexSet::new(),
    );

    let reported = f.spawn_execution_context.reported.lock();
    assert_eq!(
        reported
            .iter()
            .filter(|p| **p == ProgressStatus::CheckingCache("remote-cache".to_string()))
            .count(),
        1
    );
}

#[test]
fn download_file_cancelled_cancel_download() {
    // Test that if a download future is cancelled, the download itself is also cancelled.

    // arrange
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let future_is_cancelled = Arc::new(AtomicBool::new(false));
    // Return a future that never completes
    let remote_cache_client = SpyCacheClient::new().do_answer_download_blob({
        let future_is_cancelled = future_is_cancelled.clone();
        move |_digest| {
            answer(SettableFuture {
                cancelled: future_is_cancelled.clone(),
            })
        }
    });
    let remote_cache = f.new_combined_cache_with_client(Arc::new(remote_cache_client));
    let digest = f.create_scratch_input("file", "content");
    let file = f.exec_root.get_relative_str("file");

    // act
    block_on(async {
        let mut download =
            Box::pin(remote_cache.download_file(&remote_action_execution_context, &file, &digest));
        assert!(futures::poll!(download.as_mut()).is_pending());
        assert!(!future_is_cancelled.load(Ordering::SeqCst));
        // download.cancel(/* mayInterruptIfRunning= */ true);
        drop(download);
    });

    // assert
    assert!(future_is_cancelled.load(Ordering::SeqCst));
}

#[test]
fn download_out_err_empty_do_not_perform_download() {
    // Test that downloading empty stdout/stderr does not try to perform a download.

    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let combined_cache = f.new_combined_cache();
    let empty_digest = f.digest_util.compute(&[]);
    let result = ActionResult {
        stdout_digest: Some(empty_digest.clone()),
        stderr_digest: Some(empty_digest),
        ..Default::default()
    };

    let out_err = FileOutErr::new(
        f.exec_root.get_relative_str("stdout"),
        f.exec_root.get_relative_str("stderr"),
    );
    wait_for_bulk_transfer(combined_cache.download_out_err(
        &remote_action_execution_context,
        &result,
        &out_err,
    ))
    .unwrap();

    assert_eq!(combined_cache.get_num_successful_downloads(), 0);
    assert_eq!(combined_cache.get_num_failed_downloads(), 0);
}

#[test]
fn test_download_file_with_symlink_template() {
    // Test that when a symlink template is provided, we don't actually download files to disk.
    // Instead, a symbolic link should be created that points to a location where the file may
    // actually be found. That location could, for example, be backed by a FUSE file system that
    // exposes the Content Addressable Storage.

    // arrange
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let mut cas: HashMap<Digest, Vec<u8>> = HashMap::new();

    let hello_digest = f.digest_util.compute_as_utf8("hello-contents");
    cas.insert(hello_digest.clone(), b"hello-contents".to_vec());

    let file = f.get_path("/execroot/symlink-to-file");
    let combined_cache = f.new_combined_cache_with_symlink_template(
        cas,
        f.digest_util.clone(),
        Some("/home/alice/cas/{hash}-{size_bytes}"),
    );

    // act
    get_from_future(combined_cache.download_file(
        &remote_action_execution_context,
        &file,
        &hello_digest,
    ))
    .unwrap();

    // assert
    assert!(file.is_symbolic_link());
    assert_eq!(
        file.read_symbolic_link().unwrap(),
        PathFragment::create(
            "/home/alice/cas/a378b939ad2e1d470a9a28b34b0e256b189e85cb236766edc1d46ec3b6ca82e5-14"
        )
    );
}

#[test]
fn upload_empty_blob_and_file_do_not_perform_upload() {
    // Test that uploading an empty BLOB/file does not try to perform an upload.
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let combined_cache = f.new_combined_cache();
    let empty_digest = f.create_scratch_input("file", "");
    let file = f.exec_root.get_relative_str("file");

    get_from_future(combined_cache.upload_bytes(
        &remote_action_execution_context,
        &empty_digest,
        bytes::Bytes::new(),
    ))
    .unwrap();
    assert_eq!(
        get_from_future(combined_cache.find_missing_digests(
            &remote_action_execution_context,
            std::slice::from_ref(&empty_digest)
        ))
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>(),
        vec![empty_digest.clone()]
    );

    get_from_future(combined_cache.upload_file(
        &remote_action_execution_context,
        &empty_digest,
        &file,
    ))
    .unwrap();
    assert_eq!(
        get_from_future(combined_cache.find_missing_digests(
            &remote_action_execution_context,
            std::slice::from_ref(&empty_digest)
        ))
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>(),
        vec![empty_digest.clone()]
    );
}

#[test]
fn upload_deduplication_works() {
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let times = Arc::new(AtomicI32::new(0));
    let remote_cache_client = SpyCacheClient::new().do_answer_upload_blob_impl({
        let times = times.clone();
        move |_digest| {
            times.fetch_add(1, Ordering::SeqCst);
            // SettableFuture.create()
            answer(futures::future::pending())
        }
    });
    let combined_cache = f.new_combined_cache_with_client(Arc::new(remote_cache_client));
    let digest = f.create_scratch_input("file", "content");
    let file = f.exec_root.get_relative_str("file");

    block_on(async {
        let mut unused1 =
            Box::pin(combined_cache.upload_file(&remote_action_execution_context, &digest, &file));
        let mut unused2 =
            Box::pin(combined_cache.upload_file(&remote_action_execution_context, &digest, &file));
        assert!(futures::poll!(unused1.as_mut()).is_pending());
        assert!(futures::poll!(unused2.as_mut()).is_pending());

        assert_eq!(times.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn upload_failed_uploads_do_not_deduplicate() {
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    let fail_request = Arc::new(AtomicBool::new(true));
    let remote_cache_client = SpyCacheClient::new().do_answer_upload_blob_impl({
        let fail_request = fail_request.clone();
        move |_digest| {
            if fail_request.swap(false, Ordering::SeqCst) {
                return answer(async { Err(io_exception("Failed")) });
            }
            // invocationOnMock.callRealMethod()
            None
        }
    });
    let combined_cache = f.new_combined_cache_with_client(Arc::new(remote_cache_client));
    let digest = f.create_scratch_input("file", "content");
    let file = f.exec_root.get_relative_str("file");
    assert_eq!(
        get_from_future(combined_cache.find_missing_digests(
            &remote_action_execution_context,
            std::slice::from_ref(&digest)
        ))
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>(),
        vec![digest.clone()]
    );

    let thrown: Option<io::Error> = get_from_future(combined_cache.upload_file(
        &remote_action_execution_context,
        &digest,
        &file,
    ))
    .err();
    assert!(thrown.is_some());
    get_from_future(combined_cache.upload_file(&remote_action_execution_context, &digest, &file))
        .unwrap();

    assert!(
        get_from_future(combined_cache.find_missing_digests(
            &remote_action_execution_context,
            std::slice::from_ref(&digest)
        ))
        .unwrap()
        .is_empty()
    );
}

// The seven `ensureInputsPresent_*` tests of CombinedCacheTest.java exercise
// RemoteExecutionCache; they are ported, under the same names, in
// tests/remote_execution_cache_test.rs.

#[test]
fn shutdown_now_cancel_in_progress_uploads() {
    let f = Fixture::set_up();
    let remote_action_execution_context = f.remote_action_execution_context();
    // Return a future that never completes
    let remote_cache_client = Arc::new(
        SpyCacheClient::new()
            .do_answer_upload_blob_impl(|_digest| answer(futures::future::pending())),
    );
    let combined_cache = f.new_combined_cache_with_client(remote_cache_client.clone());
    let digest = f.create_scratch_input("file", "content");
    let file = f.exec_root.get_relative_str("file");

    block_on(async {
        let mut upload =
            Box::pin(combined_cache.upload_file(&remote_action_execution_context, &digest, &file));
        assert!(futures::poll!(upload.as_mut()).is_pending());
        assert!(
            remote_cache_client
                .get_in_progress_uploads()
                .contains(&digest)
        );
        combined_cache.shutdown_now();

        // assertThat(upload.isCancelled()).isTrue();
        match futures::poll!(upload.as_mut()) {
            Poll::Ready(Err(e)) => assert!(errors::is_interrupted(&e)),
            Poll::Ready(Ok(())) => panic!("the upload completed"),
            Poll::Pending => panic!("the upload is not cancelled"),
        }
    });
}

/// `chunkingCapabilities()`: FastCDC 2020 with an average chunk size of 1024 (a chunking
/// threshold of 4 * 1024).
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1157 CombinedCacheTest.chunkingCapabilities
fn chunking_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        cache_capabilities: Some(CacheCapabilities {
            fast_cdc_2020_params: Some(FastCdc2020Params {
                avg_chunk_size_bytes: 1024,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// `newChunkingGrpcCacheClient()`.
///
/// PORT: Java spies on a `GrpcCacheClient` over a mocked channel and stubs `splitBlob`,
/// `spliceBlob`, `findMissingDigests` and `downloadBlob`; here a real client calls a fake server
/// (started with the services registered so far) whose handlers give the same answers, with
/// `--experimental_remote_cache_chunking` set so that `splitBlob` and `spliceBlob` make their
/// calls (the stubs replace those methods whole in Java). The capabilities are
/// `chunkingCapabilities()`, as the stubbed `getServerCapabilities`. No retries (Java's retrier
/// is a mock the stubbed methods never reach).
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1105 CombinedCacheTest.newChunkingGrpcCacheClient
fn new_chunking_grpc_cache_client(server: &mut FakeServer) -> Arc<GrpcCacheClient> {
    server.start();
    let capabilities_fetcher: CapabilitiesFetcher = Arc::new(
        |_channel: ReferenceCountedChannel| -> BoxFuture<'static, io::Result<ServerCapabilities>> {
            Box::pin(async { Ok(chunking_capabilities()) })
        },
    );
    let channel = server.channel(vec![], Some(capabilities_fetcher));
    let mut options = RemoteOptions::default();
    options.experimental_remote_cache_chunking = true;
    options.remote_max_retry_attempts = 0;
    let retrier = Arc::new(RemoteRetrier::new(
        &options,
        EXPERIMENTAL_GRPC_RESULT_CLASSIFIER.clone(),
        Arc::new(TokioRetryScheduler),
        ALLOW_ALL_CALLS.clone(),
    ));
    Arc::new(GrpcCacheClient::new(
        channel,
        NO_CREDENTIALS.clone(),
        &options,
        retrier,
        DigestUtil::new(DigestHashFunction::Sha256),
    ))
}

/// `newChunkingCombinedCache(grpcCacheClient)`.
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1121 CombinedCacheTest.newChunkingCombinedCache
fn new_chunking_combined_cache(grpc_cache_client: &Arc<GrpcCacheClient>) -> CombinedCache {
    let client: Arc<dyn RemoteCacheClient> = grpc_cache_client.clone();
    CombinedCache::new(
        Some(client),
        /* diskCacheClient= */ None,
        /* symlinkTemplate= */ None,
        DigestUtil::new(DigestHashFunction::Sha256),
        /* chunkingFunction= */ Some(ChunkingFunctionValue::FAST_CDC_2020),
    )
}

/// The ByteStream `Read` of the fake server: `reads` answers by blob hash (a missing hash is
/// `NOT_FOUND`, as a `CacheNotFoundException` of the stubbed `downloadBlob`); each read is
/// counted by hash. `wait_for` delays the answer for a hash until the flag is set.
fn chunk_byte_stream(
    reads: HashMap<String, Vec<u8>>,
    read_counts: Arc<Mutex<HashMap<String, usize>>>,
    wait_for: Option<(String, Arc<AtomicBool>)>,
) -> FakeByteStream {
    FakeByteStream {
        read: Some(Box::new(move |request| {
            let resource_name = request.into_inner().resource_name;
            // "blobs/<hash>/<size>"
            let hash = resource_name
                .split('/')
                .nth(1)
                .expect("a blob resource name")
                .to_string();
            *read_counts.lock().entry(hash.clone()).or_insert(0) += 1;
            let answer = match reads.get(&hash) {
                Some(data) => Ok(read_response(data.clone())),
                None => Err(Status::not_found(format!("{hash} not found"))),
            };
            let wait = wait_for
                .as_ref()
                .filter(|(waiting_hash, _)| *waiting_hash == hash)
                .map(|(_, flag)| flag.clone());
            Box::pin(futures::stream::once(async move {
                if let Some(flag) = wait {
                    while !flag.load(Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                }
                answer
            }))
        })),
        ..Default::default()
    }
}

/// `new ContentAddressableStorageImplBase() { splitBlob(...) }` answering `chunk_digests` for
/// `blob_digest`.
fn split_blob_cas(
    blob_digest: Digest,
    chunk_digests: Vec<Digest>,
) -> FakeContentAddressableStorage {
    FakeContentAddressableStorage {
        split_blob: Some(Box::new(move |request| {
            assert_eq!(request.get_ref().blob_digest.as_ref(), Some(&blob_digest));
            Ok(SplitBlobResponse {
                chunk_digests: chunk_digests.clone(),
                ..Default::default()
            })
        })),
        ..Default::default()
    }
}

/// A `ByteArrayOutputStream` that sets `written` once it holds any bytes.
struct SignallingOutputStream {
    data: Arc<Mutex<Vec<u8>>>,
    written: Arc<AtomicBool>,
}

impl Write for SignallingOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.lock().extend_from_slice(buf);
        if !buf.is_empty() {
            self.written.store(true, Ordering::SeqCst);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:895 CombinedCacheTest.uploadFile_chunkedUpload_deduplicatesRemoteUpload
fn upload_file_chunked_upload_deduplicates_remote_upload() {
    let f = Fixture::set_up();
    let context = f.remote_action_execution_context();
    let mut server = FakeServer::new();
    let find_missing_calls = Arc::new(AtomicI32::new(0));
    let splice_calls = Arc::new(AtomicI32::new(0));
    let splice_started = Arc::new(AtomicBool::new(false));
    // spliceFuture: the splice call answers once this is set.
    let splice_released = Arc::new(AtomicBool::new(false));
    {
        let find_missing_calls = find_missing_calls.clone();
        let splice_calls = splice_calls.clone();
        let splice_started = splice_started.clone();
        let splice_released = splice_released.clone();
        server
            .service_registry
            .add_content_addressable_storage(FakeContentAddressableStorage {
                // findMissingDigests: immediateFuture(ImmutableSet.of())
                find_missing_blobs: Some(Box::new(move |_request| {
                    find_missing_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(FindMissingBlobsResponse::default())
                })),
                splice_blob: Some(Box::new(move |_request| {
                    splice_calls.fetch_add(1, Ordering::SeqCst);
                    splice_started.store(true, Ordering::SeqCst);
                    // The handler is synchronous; it holds its server thread until released.
                    tokio::task::block_in_place(|| {
                        while !splice_released.load(Ordering::SeqCst) {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    });
                    Ok(SpliceBlobResponse::default())
                })),
                ..Default::default()
            });
    }
    let grpc_cache_client = new_chunking_grpc_cache_client(&mut server);
    let combined_cache = new_chunking_combined_cache(&grpc_cache_client);
    let data = vec![0u8; 8192];
    let file = f.exec_root.get_relative_str("chunked-output");
    std::fs::write(file.get_path_string(), &data).unwrap();
    let digest = f.digest_util.compute(&data);

    block_on(async {
        let mut first_upload = Box::pin(combined_cache.upload_file(&context, &digest, &file));
        // assertThat(spliceStarted.await(1, TimeUnit.SECONDS)).isTrue()
        // PORT: a Java future runs as soon as it is made; drive this one until the server has the
        // splice call.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !splice_started.load(Ordering::SeqCst) {
            assert!(
                std::time::Instant::now() < deadline,
                "the splice did not start"
            );
            if let Poll::Ready(result) = futures::poll!(first_upload.as_mut()) {
                panic!("the upload finished before the splice was answered: {result:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let mut second_upload = Box::pin(combined_cache.upload_file(&context, &digest, &file));
        assert!(futures::poll!(second_upload.as_mut()).is_pending());

        assert_eq!(grpc_cache_client.get_upload_subscriber_count(&digest), 2);
        assert_eq!(find_missing_calls.load(Ordering::SeqCst), 1);
        assert_eq!(splice_calls.load(Ordering::SeqCst), 1);

        // spliceFuture.set(null)
        splice_released.store(true, Ordering::SeqCst);
        let (first, second) = futures::join!(first_upload, second_upload);
        first.unwrap();
        second.unwrap();
    });
    combined_cache.release();
}

#[test]
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:957 CombinedCacheTest.downloadBlob_chunkMissingAfterPartialWrite_doesNotRestartIntoSameStream
fn download_blob_chunk_missing_after_partial_write_does_not_restart_into_same_stream() {
    let f = Fixture::set_up();
    let context = f.remote_action_execution_context();
    // Larger than the chunking threshold (4 * 1024, derived from the advertised average chunk size
    // of 1024) so that the download is chunked, with chunks small enough not to be chunked again.
    let chunk1_data = vec![1u8; 4096];
    let chunk2_data = vec![2u8; 4096];
    let blob_data = [chunk1_data.clone(), chunk2_data.clone()].concat();
    let blob_digest = f.digest_util.compute(&blob_data);
    let chunk1_digest = f.digest_util.compute(&chunk1_data);
    let chunk2_digest = f.digest_util.compute(&chunk2_data);

    let mut server = FakeServer::new();
    server
        .service_registry
        .add_content_addressable_storage(split_blob_cas(
            blob_digest.clone(),
            vec![chunk1_digest.clone(), chunk2_digest.clone()],
        ));
    let read_counts = Arc::new(Mutex::new(HashMap::new()));
    let written = Arc::new(AtomicBool::new(false));
    // The second chunk was evicted between SplitBlob and the read of the chunk itself. Its
    // answer waits until the first chunk is written (Java's stubs answer synchronously, in
    // order). The whole blob is only reached if the whole-blob fallback is (incorrectly)
    // attempted, in which case `out` ends up holding the first chunk followed by the entire blob.
    server.service_registry.add_byte_stream(chunk_byte_stream(
        HashMap::from([
            (chunk1_digest.hash.clone(), chunk1_data.clone()),
            (blob_digest.hash.clone(), blob_data.clone()),
        ]),
        read_counts.clone(),
        Some((chunk2_digest.hash.clone(), written.clone())),
    ));
    let grpc_cache_client = new_chunking_grpc_cache_client(&mut server);
    let combined_cache = new_chunking_combined_cache(&grpc_cache_client);
    let out_data = Arc::new(Mutex::new(Vec::new()));
    let mut out = SignallingOutputStream {
        data: out_data.clone(),
        written,
    };

    let e =
        block_on(combined_cache.download_blob_to(&context, &blob_digest, &mut out)).unwrap_err();
    assert!(
        errors::downcast_ref::<CacheNotFoundException>(&e).is_some(),
        "{e:?}"
    );

    // Falling back to a whole-blob download here would append the blob to the chunks already
    // written, silently producing a blob longer than its digest claims.
    assert_eq!(read_counts.lock().get(&blob_digest.hash), None);
    assert_eq!(*out_data.lock(), chunk1_data);
    combined_cache.release();
}

#[test]
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1024 CombinedCacheTest.downloadBlob_firstChunkMissing_doesNotFallBackToWholeBlobDownload
fn download_blob_first_chunk_missing_does_not_fall_back_to_whole_blob_download() {
    let f = Fixture::set_up();
    let context = f.remote_action_execution_context();
    let chunk1_data = vec![1u8; 4096];
    let chunk2_data = vec![2u8; 4096];
    let blob_data = [chunk1_data.clone(), chunk2_data.clone()].concat();
    let blob_digest = f.digest_util.compute(&blob_data);
    let chunk1_digest = f.digest_util.compute(&chunk1_data);
    let chunk2_digest = f.digest_util.compute(&chunk2_data);

    let mut server = FakeServer::new();
    server
        .service_registry
        .add_content_addressable_storage(split_blob_cas(
            blob_digest.clone(),
            vec![chunk1_digest.clone(), chunk2_digest.clone()],
        ));
    let read_counts = Arc::new(Mutex::new(HashMap::new()));
    // The first chunk is missing (CacheNotFoundException); the whole blob is there.
    // PORT: Java leaves the second chunk unstubbed (the spy's real method on a mocked channel);
    // the fake server answers NOT_FOUND for it too.
    server.service_registry.add_byte_stream(chunk_byte_stream(
        HashMap::from([(blob_digest.hash.clone(), blob_data.clone())]),
        read_counts.clone(),
        None,
    ));
    let grpc_cache_client = new_chunking_grpc_cache_client(&mut server);
    let combined_cache = new_chunking_combined_cache(&grpc_cache_client);
    let mut out: Vec<u8> = Vec::new();

    // Nothing has been written yet, so restarting would be safe, but the blob is genuinely
    // incomplete in the CAS. Report it as missing and let lost input handling regenerate it
    // rather than papering over it, which also keeps the behavior independent of which chunk
    // happens to be missing.
    let e =
        block_on(combined_cache.download_blob_to(&context, &blob_digest, &mut out)).unwrap_err();
    assert!(
        errors::downcast_ref::<CacheNotFoundException>(&e).is_some(),
        "{e:?}"
    );

    assert_eq!(read_counts.lock().get(&blob_digest.hash), None);
    combined_cache.release();
}

#[test]
// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1078 CombinedCacheTest.downloadBlob_blobNotSplittable_fallsBackToWholeBlobDownload
fn download_blob_blob_not_splittable_falls_back_to_whole_blob_download() {
    let f = Fixture::set_up();
    let context = f.remote_action_execution_context();
    let blob_data = vec![1u8; 8192];
    let blob_digest = f.digest_util.compute(&blob_data);

    let mut server = FakeServer::new();
    // What GrpcCacheClient#splitBlob reports when the server answers NOT_FOUND or UNIMPLEMENTED.
    // PORT: Java stubs splitBlob to fail with BlobNotSplittableException; the fake server answers
    // NOT_FOUND, which the real splitBlob turns into it.
    server
        .service_registry
        .add_content_addressable_storage(FakeContentAddressableStorage {
            split_blob: Some(Box::new(|_request| Err(Status::not_found("no chunks")))),
            ..Default::default()
        });
    server.service_registry.add_byte_stream(chunk_byte_stream(
        HashMap::from([(blob_digest.hash.clone(), blob_data.clone())]),
        Arc::new(Mutex::new(HashMap::new())),
        None,
    ));
    let grpc_cache_client = new_chunking_grpc_cache_client(&mut server);
    let combined_cache = new_chunking_combined_cache(&grpc_cache_client);
    let mut out: Vec<u8> = Vec::new();

    block_on(combined_cache.download_blob_to(&context, &blob_digest, &mut out)).unwrap();

    assert_eq!(out, blob_data);
    combined_cache.release();
}
