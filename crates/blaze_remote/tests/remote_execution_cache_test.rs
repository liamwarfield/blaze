#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of the `ensureInputsPresent_*` tests of
//! `src/test/java/com/google/devtools/build/lib/remote/CombinedCacheTest.java`, which test
//! `RemoteExecutionCache` (the other tests of that class are in `combined_cache_test.rs`).
//!
//! Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under a
//! temporary directory, so Java's `/execroot/main` is below that directory.
//!
//! The Java tests run `ensureInputsPresent` on threads and interrupt them. The port has no thread
//! interruption: a call is the future of
//! `RemoteExecutionCache::ensure_inputs_present_with_additional_inputs_async`, a started thread
//! is that future polled by the test, and `thread.interrupt()` is dropping it (the convention of
//! `combined_cache_test.rs`). A `CountDownLatch` the Java test waits for is a counter the test
//! polls the future up to.

mod util;

use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use blaze_actions::{
    ActionLookupKey, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot, RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, ExecException, FakeOwner, FileArtifactValue,
    FileContentsProxy, FilesetOutputTree, InputMapping, InputMetadataProvider, PathMapper,
    ResourceSet, RunfilesArtifactValue, SimpleSpawn, Spawn, TreeArtifactValue,
};
use blaze_exec::exec::spawn_input_expander::SpawnInputExpander;
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_protos::JavaMessage;
use blaze_remote::common::errors::{self, io_exception};
use blaze_remote::common::remote_path_resolver::create_default;
use blaze_remote::common::{
    ActionKey, Blob, BulkTransferException, DefaultRemotePathResolver, MissingDigestsFinder,
    RemoteActionExecutionContext, RemoteCacheClient,
};
use blaze_remote::merkletree::{BlobPolicy, MerkleTree, MerkleTreeComputer, MerkleTreeUploadable};
use blaze_remote::reapi::{ActionResult, Digest, RequestMetadata, ServerCapabilities};
use blaze_remote::remote_execution_cache::{RemoteExecutionCache, RemotePathChecker};
use blaze_remote::runtime::{block_on, get_from_future};
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::tracing_metadata_utils;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use futures::future::BoxFuture;
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use tokio::sync::watch;

use util::in_memory_cache_client::InMemoryCacheClient;

/// `TestConstants.WORKSPACE_NAME`.
const WORKSPACE_NAME: &str = "_main";

/// `SettableFuture<T>`: completes with the value it is set to, and records whether a future
/// handed out for it was cancelled (dropped before it completed).
#[derive(Clone)]
struct SettableFuture<T: Clone + Send + Sync + 'static> {
    value: Arc<watch::Sender<Option<T>>>,
    cancelled: Arc<AtomicBool>,
}

/// Records the cancellation of a future of a [`SettableFuture`].
struct CancelledOnDrop {
    done: bool,
    cancelled: Arc<AtomicBool>,
}

impl Drop for CancelledOnDrop {
    fn drop(&mut self) {
        if !self.done {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

impl<T: Clone + Send + Sync + 'static> SettableFuture<T> {
    /// `SettableFuture.create()`.
    fn create() -> SettableFuture<T> {
        let (value, _) = watch::channel(None);
        SettableFuture {
            value: Arc::new(value),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `set(value)`.
    fn set(&self, value: T) {
        self.value.send_replace(Some(value));
    }

    /// `isCancelled()`.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// The future, as the code under test gets it.
    fn future(&self) -> BoxFuture<'static, T> {
        let mut value = self.value.subscribe();
        let cancelled_on_drop = CancelledOnDrop {
            done: false,
            cancelled: self.cancelled.clone(),
        };
        Box::pin(async move {
            // The future owns the whole guard (not only the field it assigns).
            let mut cancelled_on_drop = cancelled_on_drop;
            loop {
                let current: Option<T> = value.borrow().clone();
                if let Some(current) = current {
                    cancelled_on_drop.done = true;
                    return current;
                }
                value.changed().await.expect("the SettableFuture is alive");
            }
        })
    }
}

/// What a stubbed `uploadBlobImpl` of [`SpyCacheClient`] does for one call: `Some(future)` is
/// Mockito's `doAnswer(...)` result, `None` is `invocationOnMock.callRealMethod()`.
type UploadBlobImplAnswer =
    Box<dyn Fn(&Digest) -> Option<BoxFuture<'static, io::Result<()>>> + Send + Sync>;

/// The answer of a stub: `future` instead of the real method.
fn answer<T>(
    future: impl Future<Output = io::Result<T>> + Send + 'static,
) -> Option<BoxFuture<'static, io::Result<T>>> {
    Some(Box::pin(future))
}

/// The same for a stubbed `findMissingDigests`.
type FindMissingDigestsAnswer = Box<
    dyn Fn(&[Digest]) -> Option<BoxFuture<'static, io::Result<IndexSet<Digest>>>> + Send + Sync,
>;

/// `spy(new InMemoryCacheClient())` with `doAnswer` stubs for `findMissingDigests` and
/// `uploadBlobImpl`.
struct SpyCacheClient {
    delegate: InMemoryCacheClient,
    find_missing_digests_answer: Option<FindMissingDigestsAnswer>,
    upload_blob_impl_answer: Option<UploadBlobImplAnswer>,
}

impl SpyCacheClient {
    fn new() -> SpyCacheClient {
        SpyCacheClient {
            delegate: InMemoryCacheClient::new(),
            find_missing_digests_answer: None,
            upload_blob_impl_answer: None,
        }
    }

    /// `doAnswer(answer).when(cacheProtocol).findMissingDigests(any(), any())`.
    fn do_answer_find_missing_digests(
        mut self,
        answer: impl Fn(&[Digest]) -> Option<BoxFuture<'static, io::Result<IndexSet<Digest>>>>
        + Send
        + Sync
        + 'static,
    ) -> SpyCacheClient {
        self.find_missing_digests_answer = Some(Box::new(answer));
        self
    }

    /// `doAnswer(answer).when(cacheProtocol).uploadBlobImpl(any(), any(), any())`.
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
        let answer = self
            .find_missing_digests_answer
            .as_ref()
            .and_then(|a| a(digests));
        match answer {
            Some(future) => future.await,
            None => self.delegate.find_missing_digests(context, digests).await,
        }
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

/// A `RemotePathChecker` lambda: `(context, path) -> ...`.
struct LambdaRemotePathChecker<F>(F);

#[async_trait]
impl<F> RemotePathChecker for LambdaRemotePathChecker<F>
where
    F: Fn(&Path) -> BoxFuture<'static, io::Result<bool>> + Send + Sync,
{
    async fn is_available_locally(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        path: &Path,
    ) -> io::Result<bool> {
        (self.0)(path).await
    }
}

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical_unchecked("//null/action:owner"))
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Java: ActionsTestUtil.createArtifact(ArtifactRoot root, String path)
fn create_artifact(root: &ArtifactRoot, path: &str) -> Artifact {
    let exec_path = root.get_exec_path().get_relative_str(path);
    if root.is_source_root() {
        Artifact::new_source_artifact(root.clone(), exec_path, ArtifactOwner::NullOwner)
    } else {
        Artifact::create_derived_artifact(
            root.clone(),
            exec_path,
            ActionLookupKeyRef::new(NullArtifactOwner),
        )
    }
}

// Java: FileSystemUtils.writeContentAsLatin1
fn write_content_as_latin1(path: &Path, content: &str) {
    let mut out = path.get_output_stream().unwrap();
    out.write_all(content.as_bytes()).unwrap();
    out.flush().unwrap();
}

/// A fake implementation of the `InputMetadataProvider` interface.
// Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:41 FakeActionInputFileCache
struct FakeActionInputFileCache {
    exec_root: Path,
    cas: Mutex<HashMap<PathFragment, FileArtifactValue>>,
    inputs: Mutex<HashMap<PathFragment, ActionInputRef>>,
    digest_util: DigestUtil,
}

impl FakeActionInputFileCache {
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:51 FakeActionInputFileCache.<init>
    fn new(exec_root: Path) -> FakeActionInputFileCache {
        FakeActionInputFileCache {
            exec_root,
            cas: Mutex::new(HashMap::new()),
            inputs: Mutex::new(HashMap::new()),
            digest_util: DigestUtil::new(DigestHashFunction::Sha256),
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:105 FakeActionInputFileCache.setMetadata
    fn set_metadata(&self, input: &ActionInputRef, metadata: FileArtifactValue) {
        self.cas
            .lock()
            .insert(input.get_exec_path().clone(), metadata);
        self.inputs
            .lock()
            .insert(input.get_exec_path().clone(), input.clone());
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:133 FakeActionInputFileCache.createScratchInput
    fn create_scratch_input(&self, input: &ActionInputRef, content: &str) -> Digest {
        let input_file = self.exec_root.get_relative(input.get_exec_path());
        input_file
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        write_content_as_latin1(&input_file, content);
        let digest = self.digest_util.compute_path(&input_file).unwrap();
        self.set_metadata(
            input,
            FileArtifactValue::create_for_normal_file(
                Some(DigestUtil::to_binary_digest(&digest)),
                Some(FileContentsProxy::create(&input_file.stat().unwrap())),
                content.len() as i64,
            ),
        );
        digest
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:57 FakeActionInputFileCache.getInputMetadataChecked
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        match self.cas.lock().get(input.get_exec_path()) {
            Some(metadata) => Ok(Some(metadata.clone())),
            None => panic!(
                "NullPointerException: No metadata for input '{}' (exec path: '{}')",
                input,
                input.get_exec_path()
            ),
        }
    }
    fn get_tree_metadata(&self, _input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        None
    }
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        panic!("UnsupportedOperationException")
    }
    fn get_runfiles_metadata(&self, _input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        None
    }
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.inputs.lock().get(exec_path).cloned()
    }
}

/// `FakeSpawnExecutionContext`.
// Java: com/google/devtools/build/lib/remote/util/FakeSpawnExecutionContext.java
struct FakeSpawnExecutionContext<'a> {
    spawn: &'a dyn Spawn,
    input_metadata_provider: &'a dyn InputMetadataProvider,
    out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    digest: Mutex<Option<blaze_protos::tools::protos::Digest>>,
}

impl SpawnExecutionContext for FakeSpawnExecutionContext<'_> {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, digest: blaze_protos::tools::protos::Digest) {
        *self.digest.lock() = Some(digest);
    }
    fn get_digest(&self) -> Option<blaze_protos::tools::protos::Digest> {
        self.digest.lock().clone()
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        panic!("UnsupportedOperationException")
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        self.input_metadata_provider
    }
    fn lock_output_files(
        &self,
        _exit_code: i32,
        _error_message: &str,
        _out_err: &FileOutErr,
    ) -> Result<(), ExecException> {
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> Duration {
        Duration::ZERO
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.out_err
    }
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        Arc::new(SpawnInputExpander::new().get_input_mapping(
            self.spawn,
            self.input_metadata_provider,
            base_directory,
        ))
    }
    fn report(&self, _progress: ProgressStatus) {
        // Intentionally left empty.
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

/// `new SpawnBuilder().withInput(input).build()`.
// Java: com/google/devtools/build/lib/exec/util/SpawnBuilder.java:66 SpawnBuilder.build
fn build_spawn_with_input(input: &Artifact) -> SimpleSpawn {
    let owner = FakeOwner::new(
        "Mnemonic",
        Some("progress message"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    );
    let mut inputs = NestedSetBuilder::stable_order();
    inputs.add(artifact_input(input));
    SimpleSpawn::new(
        Arc::new(owner),
        Vec::new(),
        IndexMap::new(),
        IndexMap::new(),
        inputs.build(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        None,
        Arc::new(ResourceSet::zero()),
        PathMapper::NOOP,
    )
}

/// The fields of the Java test class, as `setUp` leaves them (the ones the
/// `ensureInputsPresent_*` tests use).
struct CombinedCacheTest {
    _dir: tempfile::TempDir,
    metadata: RequestMetadata,
    exec_root: Path,
    artifact_root: ArtifactRoot,
    digest_util: DigestUtil,
    merkle_tree_computer: MerkleTreeComputer,
    fake_file_cache: FakeActionInputFileCache,
}

impl CombinedCacheTest {
    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:142 CombinedCacheTest.setUp
    fn set_up() -> CombinedCacheTest {
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
        let exec_root = Path::create_from_str(&format!("{root}/execroot/main"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let fake_file_cache = FakeActionInputFileCache::new(exec_root.clone());
        let artifact_root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["outputs"]);
        artifact_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
        let merkle_tree_computer = MerkleTreeComputer::new(
            digest_util.clone(),
            /* remoteExecutionCache= */ None,
            "buildRequestId",
            "commandId",
            WORKSPACE_NAME,
            // PORT: the exec root stands for `FakeSpawnExecutionContext.getPathResolver()`
            // (`ArtifactPathResolver.forExecRoot(execRoot)`).
            exec_root.clone(),
        );
        CombinedCacheTest {
            _dir: dir,
            metadata,
            exec_root,
            artifact_root,
            digest_util,
            merkle_tree_computer,
            fake_file_cache,
        }
    }

    // Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:1148 CombinedCacheTest.newRemoteExecutionCache
    fn new_remote_execution_cache(
        &self,
        remote_cache_client: Arc<dyn RemoteCacheClient>,
    ) -> RemoteExecutionCache {
        RemoteExecutionCache::new(
            remote_cache_client,
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            self.digest_util.clone(),
            /* chunkingFunction= */ None,
        )
    }

    /// `new FakeSpawnExecutionContext(spawn, fakeFileCache, execRoot, new FileOutErr(
    /// execRoot.getRelative("stdout"), execRoot.getRelative("stderr")),
    /// ImmutableClassToInstanceMap.of(), /* actionFileSystem= */ null)`.
    fn new_fake_spawn_execution_context<'a>(
        &'a self,
        spawn: &'a dyn Spawn,
    ) -> FakeSpawnExecutionContext<'a> {
        FakeSpawnExecutionContext {
            spawn,
            input_metadata_provider: &self.fake_file_cache,
            out_err: FileOutErr::new(
                self.exec_root.get_relative_str("stdout"),
                self.exec_root.get_relative_str("stderr"),
            ),
            client_env: IndexMap::new(),
            digest: Mutex::new(None),
        }
    }

    /// `(MerkleTree.Uploadable) merkleTreeComputer.buildForSpawn(spawn, ImmutableSet.of(),
    /// /* scrubber= */ null, context, RemotePathResolver.createDefault(execRoot),
    /// MerkleTreeComputer.BlobPolicy.KEEP_AND_REUPLOAD)`.
    fn build_for_spawn(
        &self,
        spawn: &dyn Spawn,
        context: &dyn SpawnExecutionContext,
    ) -> MerkleTreeUploadable {
        let merkle_tree = self
            .merkle_tree_computer
            .build_for_spawn(
                spawn,
                &HashSet::new(),
                /* scrubber= */ None,
                context,
                &*create_default(self.exec_root.clone()),
                BlobPolicy::KeepAndReupload,
            )
            .unwrap();
        match merkle_tree {
            MerkleTree::Uploadable(uploadable) => uploadable,
            MerkleTree::RootOnly(_) => panic!(
                "ClassCastException: MerkleTree.RootOnly cannot be cast to MerkleTree.Uploadable"
            ),
        }
    }
}

/// Runs the "thread" `call` until `condition` holds (Java: `latch.await()` after
/// `thread.start()`). The call must not return before that.
async fn run_until<F: Future>(call: &mut Pin<Box<F>>, condition: impl Fn() -> bool) {
    for _ in 0..10_000 {
        if condition() {
            return;
        }
        assert!(
            futures::poll!(call.as_mut()).is_pending(),
            "ensureInputsPresent returned before the awaited condition held"
        );
        tokio::task::yield_now().await;
    }
    panic!("timed out waiting for the condition");
}

/// The `BulkTransferException` that `result` failed with.
fn assert_bulk_transfer_exception(result: io::Result<()>) -> BulkTransferException {
    let e = result.expect_err("expected a BulkTransferException");
    errors::downcast_ref::<BulkTransferException>(&e)
        .unwrap_or_else(|| panic!("expected a BulkTransferException, got: {e}"))
        .clone()
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:361 CombinedCacheTest.ensureInputsPresent_missingInputs_exceptionHasLostInputs
#[test]
fn ensure_inputs_present_missing_inputs_exception_has_lost_inputs() {
    let t = CombinedCacheTest::set_up();
    let cache_protocol: Arc<dyn RemoteCacheClient> = Arc::new(SpyCacheClient::new());
    let remote_cache = t.new_remote_execution_cache(cache_protocol);
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());
    remote_cache.set_remote_path_checker(Arc::new(LambdaRemotePathChecker({
        let exec_root = t.exec_root.clone();
        move |path: &Path| -> BoxFuture<'static, io::Result<bool>> {
            let available = path.relative_to(&exec_root) != PathFragment::create("foo");
            Box::pin(async move { Ok::<bool, io::Error>(available) })
        }
    })));

    let path = t.exec_root.get_relative_str("foo");
    write_content_as_latin1(&path, "bar");
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    inputs.insert(PathFragment::create("foo"), path.clone());
    let merkle_tree = t.merkle_tree_computer.build_for_files(&inputs).unwrap();
    path.delete().unwrap();

    let e =
        assert_bulk_transfer_exception(remote_cache.ensure_inputs_present_with_additional_inputs(
            &remote_action_execution_context,
            &merkle_tree,
            &IndexMap::new(),
            false,
            Some(&DefaultRemotePathResolver::new(t.exec_root.clone())),
        ));
    // ActionInputHelper::fromPath
    let by_digest = e
        .get_lost_artifacts(&|exec_path: &PathFragment| {
            Some(ActionInputRef::new(BasicActionInput::from_path_fragment(
                exec_path.clone(),
            )))
        })
        .by_digest;
    let expected: IndexMap<String, IndexSet<ActionInputRef>> = IndexMap::from([(
        DigestUtil::to_string(&t.digest_util.compute_as_utf8("bar")),
        IndexSet::from([ActionInputRef::new(BasicActionInput::from_path("foo"))]),
    )]);
    assert_eq!(by_digest, expected);
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:393 CombinedCacheTest.ensureInputsPresent_blobsLostAfterUpload_forceReuploadsDirectoryProtos
#[test]
fn ensure_inputs_present_blobs_lost_after_upload_force_reuploads_directory_protos() {
    let t = CombinedCacheTest::set_up();
    let cache_protocol = Arc::new(InMemoryCacheClient::new());
    let remote_cache = t.new_remote_execution_cache(cache_protocol.clone());
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());

    // A file in a subdirectory ensures that the Merkle tree contains directory protos, which are
    // retained as in-memory byte arrays rather than as action inputs.
    let path = t.exec_root.get_relative_str("dir/foo");
    path.get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    write_content_as_latin1(&path, "bar");
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    inputs.insert(PathFragment::create("dir/foo"), path.clone());
    let merkle_tree = t.merkle_tree_computer.build_for_files(&inputs).unwrap();
    let message = Digest {
        hash: "message standing in for an Action".to_string(),
        ..Default::default()
    };
    let message_digest = t.digest_util.compute_message(&message);
    let additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::from([(
        message_digest.clone(),
        bytes::Bytes::from(message.to_byte_array()),
    )]);
    let all_digests: Vec<Digest> = merkle_tree
        .all_digests()
        .into_iter()
        .chain([message_digest])
        .collect::<IndexSet<Digest>>()
        .into_iter()
        .collect();
    let remote_path_resolver = DefaultRemotePathResolver::new(t.exec_root.clone());

    remote_cache
        .ensure_inputs_present_with_additional_inputs(
            &remote_action_execution_context,
            &merkle_tree,
            &additional_inputs,
            /* force= */ false,
            Some(&remote_path_resolver),
        )
        .unwrap();
    assert!(
        get_from_future(
            remote_cache.find_missing_digests(&remote_action_execution_context, &all_digests)
        )
        .unwrap()
        .is_empty()
    );

    // Simulate a remote cache that loses all blobs right after they have been uploaded.
    for digest in &all_digests {
        cache_protocol.remove_cas_entry(digest);
    }

    remote_cache
        .ensure_inputs_present_with_additional_inputs(
            &remote_action_execution_context,
            &merkle_tree,
            &additional_inputs,
            /* force= */ true,
            Some(&remote_path_resolver),
        )
        .unwrap();
    assert!(
        get_from_future(
            remote_cache.find_missing_digests(&remote_action_execution_context, &all_digests)
        )
        .unwrap()
        .is_empty()
    );
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:441 CombinedCacheTest.ensureInputsPresent_sharedMissingDigest_exceptionsHaveOwnLostInputs
#[test]
fn ensure_inputs_present_shared_missing_digest_exceptions_have_own_lost_inputs() {
    let t = CombinedCacheTest::set_up();

    // CountDownLatch findMissingDigestsCalls = new CountDownLatch(2);
    let find_missing_digests_calls = Arc::new(AtomicI32::new(0));
    let cache_protocol: Arc<dyn RemoteCacheClient> =
        Arc::new(SpyCacheClient::new().do_answer_find_missing_digests({
            let find_missing_digests_calls = find_missing_digests_calls.clone();
            move |_digests| {
                find_missing_digests_calls.fetch_add(1, Ordering::SeqCst);
                // invocationOnMock.callRealMethod()
                None
            }
        }));
    let remote_cache = t.new_remote_execution_cache(cache_protocol);

    let missing_input_available: SettableFuture<bool> = SettableFuture::create();
    // CountDownLatch remotePathChecked = new CountDownLatch(1);
    let remote_path_checked = Arc::new(AtomicI32::new(0));
    remote_cache.set_remote_path_checker(Arc::new(LambdaRemotePathChecker({
        let exec_root = t.exec_root.clone();
        let missing_input_available = missing_input_available.clone();
        let remote_path_checked = remote_path_checked.clone();
        move |path: &Path| -> BoxFuture<'static, io::Result<bool>> {
            let exec_path = path.relative_to(&exec_root);
            if exec_path == PathFragment::create("outputs/foo")
                || exec_path == PathFragment::create("outputs/bar")
            {
                remote_path_checked.fetch_add(1, Ordering::SeqCst);
                let missing_input_available = missing_input_available.future();
                return Box::pin(
                    async move { Ok::<bool, io::Error>(missing_input_available.await) },
                );
            }
            Box::pin(async { Ok::<bool, io::Error>(true) })
        }
    })));

    let foo = create_artifact(&t.artifact_root, "foo");
    let bar = create_artifact(&t.artifact_root, "bar");
    let digest = t
        .fake_file_cache
        .create_scratch_input(&artifact_input(&foo), "same");
    assert_eq!(
        t.fake_file_cache
            .create_scratch_input(&artifact_input(&bar), "same"),
        digest
    );

    let foo_spawn = build_spawn_with_input(&foo);
    let foo_context = t.new_fake_spawn_execution_context(&foo_spawn);
    let foo_remote_context = RemoteActionExecutionContext::create_for_spawn(
        &foo_spawn,
        &foo_context,
        t.metadata.clone(),
    );
    let foo_tree = t.build_for_spawn(&foo_spawn, &foo_context);

    let bar_spawn = build_spawn_with_input(&bar);
    let bar_context = t.new_fake_spawn_execution_context(&bar_spawn);
    let bar_remote_context = RemoteActionExecutionContext::create_for_spawn(
        &bar_spawn,
        &bar_context,
        t.metadata.clone(),
    );
    let bar_tree = t.build_for_spawn(&bar_spawn, &bar_context);

    let no_additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::new();
    let foo_remote_path_resolver = create_default(t.exec_root.clone());
    let bar_remote_path_resolver = create_default(t.exec_root.clone());
    let (foo_failure, bar_failure) = block_on(async {
        let mut foo_thread = Box::pin(
            remote_cache.ensure_inputs_present_with_additional_inputs_async(
                &foo_remote_context,
                &foo_tree,
                &no_additional_inputs,
                /* force= */ false,
                Some(&*foo_remote_path_resolver),
            ),
        );
        // fooThread.start(); remotePathChecked.await(...)
        run_until(&mut foo_thread, || {
            remote_path_checked.load(Ordering::SeqCst) >= 1
        })
        .await;

        let mut bar_thread = Box::pin(
            remote_cache.ensure_inputs_present_with_additional_inputs_async(
                &bar_remote_context,
                &bar_tree,
                &no_additional_inputs,
                /* force= */ false,
                Some(&*bar_remote_path_resolver),
            ),
        );
        // barThread.start(); findMissingDigestsCalls.await(...)
        run_until(&mut bar_thread, || {
            find_missing_digests_calls.load(Ordering::SeqCst) >= 2
        })
        .await;

        missing_input_available.set(false);
        // fooThread.join(); barThread.join();
        futures::join!(foo_thread, bar_thread)
    });

    let foo_input = artifact_input(&foo);
    let foo_failure = assert_bulk_transfer_exception(foo_failure);
    assert_eq!(
        foo_failure
            .get_lost_artifacts(&|exec_path: &PathFragment| {
                if exec_path == foo.get_exec_path() {
                    Some(foo_input.clone())
                } else {
                    None
                }
            })
            .by_digest,
        IndexMap::from([(
            DigestUtil::to_string(&digest),
            IndexSet::from([foo_input.clone()])
        )])
    );

    let bar_input = artifact_input(&bar);
    let bar_failure = assert_bulk_transfer_exception(bar_failure);
    assert_eq!(
        bar_failure
            .get_lost_artifacts(&|exec_path: &PathFragment| {
                if exec_path == bar.get_exec_path() {
                    Some(bar_input.clone())
                } else {
                    None
                }
            })
            .by_digest,
        IndexMap::from([(
            DigestUtil::to_string(&digest),
            IndexSet::from([bar_input.clone()])
        )])
    );
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:576 CombinedCacheTest.ensureInputsPresent_interruptedDuringUploadBlobs_cancelInProgressUploadTasks
#[test]
fn ensure_inputs_present_interrupted_during_upload_blobs_cancel_in_progress_upload_tasks() {
    // arrange
    let t = CombinedCacheTest::set_up();
    let futures: Arc<Mutex<Vec<SettableFuture<()>>>> = Arc::new(Mutex::new(Vec::new()));
    // CountDownLatch uploadBlobCalls = new CountDownLatch(2);
    let upload_blob_calls = Arc::new(AtomicI32::new(0));
    let cache_protocol = Arc::new(SpyCacheClient::new().do_answer_upload_blob_impl({
        let futures = futures.clone();
        let upload_blob_calls = upload_blob_calls.clone();
        move |_digest| {
            let future: SettableFuture<()> = SettableFuture::create();
            futures.lock().push(future.clone());
            upload_blob_calls.fetch_add(1, Ordering::SeqCst);
            let future = future.future();
            answer(async move {
                future.await;
                Ok(())
            })
        }
    }));
    let remote_cache = t.new_remote_execution_cache(cache_protocol.clone());
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());

    let path = t.exec_root.get_relative_str("foo");
    write_content_as_latin1(&path, "bar");
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    inputs.insert(PathFragment::create("foo"), path.clone());
    let merkle_tree = t.merkle_tree_computer.build_for_files(&inputs).unwrap();

    let no_additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::new();
    block_on(async {
        let mut thread = Box::pin(
            remote_cache.ensure_inputs_present_with_additional_inputs_async(
                &remote_action_execution_context,
                &merkle_tree,
                &no_additional_inputs,
                false,
                /* remotePathResolver= */ None,
            ),
        );

        // act
        // thread.start(); uploadBlobCalls.await();
        run_until(&mut thread, || {
            upload_blob_calls.load(Ordering::SeqCst) >= 2
        })
        .await;
        assert_eq!(futures.lock().len(), 2);
        assert!(!cache_protocol.get_in_progress_uploads().is_empty());

        // thread.interrupt(); ensureInputsPresentReturned.await();
        drop(thread);

        // assert
        assert!(cache_protocol.get_in_progress_uploads().is_empty());
        assert!(cache_protocol.get_finished_uploads().is_empty());
        for future in futures.lock().iter() {
            assert!(future.is_cancelled());
        }
    });
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:647 CombinedCacheTest.ensureInputsPresent_multipleConsumers_interruptedOneDuringFindMissingBlobs_keepAndFinishInProgressUploadTasks
#[test]
fn ensure_inputs_present_multiple_consumers_interrupted_one_during_find_missing_blobs_keep_and_finish_in_progress_upload_tasks()
 {
    // arrange
    let t = CombinedCacheTest::set_up();

    let find_missing_digests_future: SettableFuture<IndexSet<Digest>> = SettableFuture::create();
    // CountDownLatch findMissingDigestsCalled = new CountDownLatch(1);
    let find_missing_digests_called = Arc::new(AtomicI32::new(0));
    let futures: Arc<Mutex<Vec<SettableFuture<()>>>> = Arc::new(Mutex::new(Vec::new()));
    // CountDownLatch uploadBlobCalls = new CountDownLatch(2);
    let upload_blob_calls = Arc::new(AtomicI32::new(0));
    let cache_protocol = Arc::new(
        SpyCacheClient::new()
            .do_answer_find_missing_digests({
                let find_missing_digests_future = find_missing_digests_future.clone();
                let find_missing_digests_called = find_missing_digests_called.clone();
                move |_digests| {
                    find_missing_digests_called.fetch_add(1, Ordering::SeqCst);
                    let future = find_missing_digests_future.future();
                    answer(async move { Ok(future.await) })
                }
            })
            .do_answer_upload_blob_impl({
                let futures = futures.clone();
                let upload_blob_calls = upload_blob_calls.clone();
                move |_digest| {
                    let future: SettableFuture<()> = SettableFuture::create();
                    futures.lock().push(future.clone());
                    upload_blob_calls.fetch_add(1, Ordering::SeqCst);
                    let future = future.future();
                    answer(async move {
                        future.await;
                        Ok(())
                    })
                }
            }),
    );
    let remote_cache = t.new_remote_execution_cache(cache_protocol.clone());
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());

    let path = t.exec_root.get_relative_str("foo");
    write_content_as_latin1(&path, "bar");
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    inputs.insert(PathFragment::create("foo"), path.clone());
    let merkle_tree = t.merkle_tree_computer.build_for_files(&inputs).unwrap();

    let no_additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::new();
    block_on(async {
        let work = || {
            Box::pin(
                remote_cache.ensure_inputs_present_with_additional_inputs_async(
                    &remote_action_execution_context,
                    &merkle_tree,
                    &no_additional_inputs,
                    false,
                    /* remotePathResolver= */ None,
                ),
            )
        };
        let mut thread1 = work();
        let mut thread2 = work();
        // thread1.start(); thread2.start(); findMissingDigestsCalled.await();
        run_until(&mut thread1, || {
            find_missing_digests_called.load(Ordering::SeqCst) >= 1
        })
        .await;
        assert!(futures::poll!(thread2.as_mut()).is_pending());

        // act
        // thread1.interrupt(); ensureInterrupted.await();
        drop(thread1);
        find_missing_digests_future.set(merkle_tree.all_digests().into_iter().collect());

        // uploadBlobCalls.await();
        run_until(&mut thread2, || {
            upload_blob_calls.load(Ordering::SeqCst) >= 2
        })
        .await;
        assert_eq!(futures.lock().len(), 2);

        // assert
        assert_eq!(cache_protocol.get_in_progress_uploads().len(), 2);
        assert!(cache_protocol.get_finished_uploads().is_empty());
        for future in futures.lock().iter() {
            assert!(!future.is_cancelled());
        }

        for future in futures.lock().iter() {
            future.set(());
        }
        // ensureInputsPresentReturned.await();
        thread2.await.unwrap();
        assert!(cache_protocol.get_in_progress_uploads().is_empty());
        assert_eq!(cache_protocol.get_finished_uploads().len(), 2);
    });
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:740 CombinedCacheTest.ensureInputsPresent_multipleConsumers_interruptedOneDuringUploadBlobs_keepInProgressUploadTasks
#[test]
fn ensure_inputs_present_multiple_consumers_interrupted_one_during_upload_blobs_keep_in_progress_upload_tasks()
 {
    // arrange
    let t = CombinedCacheTest::set_up();

    let upload_futures: Arc<Mutex<HashMap<Digest, SettableFuture<()>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // 3 unique file digests + 2 unique directory blob digests = 5 uploads total.
    // CountDownLatch uploadCalls = new CountDownLatch(5);
    let upload_calls = Arc::new(AtomicI32::new(0));
    let cache_protocol = Arc::new(SpyCacheClient::new().do_answer_upload_blob_impl({
        let upload_futures = upload_futures.clone();
        let upload_calls = upload_calls.clone();
        move |digest| {
            let future: SettableFuture<()> = SettableFuture::create();
            upload_futures.lock().insert(digest.clone(), future.clone());
            upload_calls.fetch_add(1, Ordering::SeqCst);
            let future = future.future();
            answer(async move {
                future.await;
                Ok(())
            })
        }
    }));
    let remote_cache = t.new_remote_execution_cache(cache_protocol.clone());
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());

    let foo = t.exec_root.get_relative_str("foo");
    write_content_as_latin1(&foo, "foo");
    let bar = t.exec_root.get_relative_str("bar");
    write_content_as_latin1(&bar, "bar");
    let qux = t.exec_root.get_relative_str("qux");
    write_content_as_latin1(&qux, "qux");
    let foo_digest = t.digest_util.compute_as_utf8("foo");
    let bar_digest = t.digest_util.compute_as_utf8("bar");
    let qux_digest = t.digest_util.compute_as_utf8("qux");

    let mut input1: BTreeMap<PathFragment, Path> = BTreeMap::new();
    input1.insert(PathFragment::create("foo"), foo.clone());
    input1.insert(PathFragment::create("bar"), bar.clone());
    let merkle_tree1 = t.merkle_tree_computer.build_for_files(&input1).unwrap();

    let mut input2: BTreeMap<PathFragment, Path> = BTreeMap::new();
    input2.insert(PathFragment::create("bar"), bar.clone());
    input2.insert(PathFragment::create("qux"), qux.clone());
    let merkle_tree2 = t.merkle_tree_computer.build_for_files(&input2).unwrap();

    let no_additional_inputs: IndexMap<Digest, bytes::Bytes> = IndexMap::new();
    block_on(async {
        let mut thread1 = Box::pin(
            remote_cache.ensure_inputs_present_with_additional_inputs_async(
                &remote_action_execution_context,
                &merkle_tree1,
                &no_additional_inputs,
                false,
                /* remotePathResolver= */ None,
            ),
        );
        let mut thread2 = Box::pin(
            remote_cache.ensure_inputs_present_with_additional_inputs_async(
                &remote_action_execution_context,
                &merkle_tree2,
                &no_additional_inputs,
                false,
                /* remotePathResolver= */ None,
            ),
        );

        // act
        // thread1.start(); thread2.start(); uploadCalls.await();
        run_until(&mut thread1, || upload_calls.load(Ordering::SeqCst) >= 3).await;
        run_until(&mut thread2, || upload_calls.load(Ordering::SeqCst) >= 5).await;
        assert_eq!(upload_futures.lock().len(), 5);
        assert_eq!(cache_protocol.get_in_progress_uploads().len(), 5);

        // thread1.interrupt(); ensureInterrupted.await();
        drop(thread1);

        // assert
        assert_eq!(cache_protocol.get_in_progress_uploads().len(), 3);
        assert!(cache_protocol.get_finished_uploads().is_empty());
        // foo is only in tree1, so interrupting thread1 cancels it; bar is shared and qux is only
        // in tree2, so both are kept.
        assert!(
            upload_futures
                .lock()
                .get(&foo_digest)
                .unwrap()
                .is_cancelled()
        );
        assert!(
            !upload_futures
                .lock()
                .get(&bar_digest)
                .unwrap()
                .is_cancelled()
        );
        assert!(
            !upload_futures
                .lock()
                .get(&qux_digest)
                .unwrap()
                .is_cancelled()
        );

        let all_upload_futures: Vec<SettableFuture<()>> =
            upload_futures.lock().values().cloned().collect();
        for future in all_upload_futures {
            future.set(());
        }
        // ensureInputsPresentReturned.await();
        thread2.await.unwrap();
        assert!(cache_protocol.get_in_progress_uploads().is_empty());
        assert_eq!(cache_protocol.get_finished_uploads().len(), 3);
    });
}

// Java: com/google/devtools/build/lib/remote/CombinedCacheTest.java:846 CombinedCacheTest.ensureInputsPresent_uploadFailed_propagateErrors
#[test]
fn ensure_inputs_present_upload_failed_propagate_errors() {
    let t = CombinedCacheTest::set_up();
    let cache_protocol: Arc<dyn RemoteCacheClient> =
        Arc::new(SpyCacheClient::new().do_answer_upload_blob_impl(|_digest| {
            // Futures.immediateFailedFuture(new IOException("upload failed"))
            answer(async { Err(io_exception("upload failed")) })
        }));
    let remote_action_execution_context = RemoteActionExecutionContext::create(t.metadata.clone());
    let remote_cache = t.new_remote_execution_cache(cache_protocol);
    let path = t.exec_root.get_relative_str("foo");
    write_content_as_latin1(&path, "bar");
    let mut inputs: BTreeMap<PathFragment, Path> = BTreeMap::new();
    inputs.insert(PathFragment::create("foo"), path.clone());
    let merkle_tree = t.merkle_tree_computer.build_for_files(&inputs).unwrap();

    let e: io::Error = remote_cache
        .ensure_inputs_present_with_additional_inputs(
            &remote_action_execution_context,
            &merkle_tree,
            &IndexMap::new(),
            false,
            /* remotePathResolver= */ None,
        )
        .expect_err("expected an IOException");

    assert!(e.to_string().contains("upload failed"), "{e}");
}
