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
use std::time::Duration;

use async_trait::async_trait;
use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::{
    ActionInputRef, ExecException, FakeOwner, FileArtifactValue, FilesetOutputTree, InputMapping,
    InputMetadataProvider, ResourceSet, RunfilesArtifactValue, SimpleSpawn, TreeArtifactValue,
};
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::errors::{self, io_exception};
use blaze_remote::common::{
    ActionKey, Blob, MissingDigestsFinder, RemoteActionExecutionContext, RemoteCacheClient,
};
use blaze_remote::reapi::{ActionResult, Digest, RequestMetadata, ServerCapabilities};
use blaze_remote::runtime::{block_on, get_from_future};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::tracing_metadata_utils;
use blaze_remote::util::utils::wait_for_bulk_transfer;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use futures::future::BoxFuture;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

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

#[test]
#[ignore = "content-defined chunking is not ported"]
fn upload_file_chunked_upload_deduplicates_remote_upload() {
    blaze_util::unported!(
        "GrpcCacheClient.spliceBlob, ChunkedBlobUploader (content-defined chunking)"
    );
}

#[test]
#[ignore = "content-defined chunking is not ported"]
fn download_blob_chunk_missing_after_partial_write_does_not_restart_into_same_stream() {
    blaze_util::unported!(
        "GrpcCacheClient.splitBlob, ChunkedBlobDownloader (content-defined chunking)"
    );
}

#[test]
#[ignore = "content-defined chunking is not ported"]
fn download_blob_first_chunk_missing_does_not_fall_back_to_whole_blob_download() {
    blaze_util::unported!(
        "GrpcCacheClient.splitBlob, ChunkedBlobDownloader (content-defined chunking)"
    );
}

#[test]
#[ignore = "content-defined chunking is not ported"]
fn download_blob_blob_not_splittable_falls_back_to_whole_blob_download() {
    blaze_util::unported!(
        "GrpcCacheClient.splitBlob, BlobNotSplittableException, ChunkedBlobDownloader (content-defined chunking)"
    );
}
