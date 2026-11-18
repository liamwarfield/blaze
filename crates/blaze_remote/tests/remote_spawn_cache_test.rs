#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteSpawnCacheTest.java`.
//!
//! PORT:
//! - Java runs the tests on an `InMemoryFileSystem`; here they run on the real file system under
//!   a temporary directory, so Java's absolute paths (`/exec/root`, `/tmp/stdout`) are prefixed
//!   with that directory.
//! - Java mocks `CombinedCache` and spies on `RemoteExecutionService` with Mockito. Here the
//!   service is the real one, over a `CombinedCache` whose remote client is
//!   [`SpyRemoteCacheClient`] (the stubs of `combinedCache.downloadActionResult` are answers of
//!   its `downloadActionResult`; `hasRemoteCache()` is true because it has a remote client and
//!   `remoteActionCacheSupportsUpdate()` is true because its capabilities say so). The stubs of
//!   the service's own methods are replaced by what makes the real methods behave like the stub:
//!   - `downloadOutputs`: the service runs with `--remote_download_minimal`
//!     (`DUMMY_REMOTE_OUTPUT_CHECKER`), so the real method downloads nothing and injects the
//!     metadata of the outputs through the `RemoteOutputInjector` hook
//!     ([`RecordingRemoteOutputInjector`]); `verify(service).downloadOutputs(...)` is checked on
//!     the recorded injections, and `doThrow(e).when(service).downloadOutputs(...)` is the
//!     injector failing with `e`.
//!   - `uploadOutputs`: the real method uploads the outputs to the in-memory remote cache; the
//!     test writes the spawn's output file first (Java's stub does not need it), waits for the
//!     background upload (`service.shutdown()`), and `verify(service).uploadOutputs(...)` /
//!     `never()` is checked on the action results uploaded to [`SpyRemoteCacheClient`].
//! - The spawn's output is `random/file` (under the exec root) instead of Java's `/random/file`:
//!   Java never lets the path reach the file system (the methods that would use it are stubbed),
//!   while the real `downloadOutputs` requires every output to be under the exec root.
//! - The path-mapped tests need a `PathMapper` other than `NOOP`, which blaze_exec does not have
//!   (`PathMapper` is an enum with the single variant `Noop`), so the in-flight execution
//!   de-duplication of `RemoteSpawnCache` cannot be reached.

mod util;

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use blaze_actions::actions::command_lines::ParamFileActionInput;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_events::{EventKind, StoredEventHandler};
use blaze_exec::actions::execution_requirements;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, BasicActionInput, ExecException, FakeOwner,
    InputMapping, InputMetadataProvider, ResourceSet, SimpleSpawn, Spawn, SpawnResult,
    SpawnResultBuilder, Status,
};
use blaze_exec::exec::action_execution_context::SpawnCache;
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext};
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::action_execution_context::ShowSubcommands;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_options::Foreign;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_remote::combined_cache::CombinedCache;
use blaze_remote::common::errors;
use blaze_remote::common::remote_path_resolver::{self, RemotePathResolver};
use blaze_remote::common::{
    ActionKey, Blob, CacheNotFoundException, MissingDigestsFinder, RemoteActionExecutionContext,
    RemoteCacheClient,
};
use blaze_remote::disk::disk_cache_client::DiskCacheClient;
use blaze_remote::hooks::{MayModifySpawnOutputsAfterExecution, RemoteHooks, RemoteOutputInjector};
use blaze_remote::reapi::{ActionResult, Digest, OutputFile, RequestMetadata, ServerCapabilities};
use blaze_remote::remote_execution_service::RemoteExecutionService;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_remote::remote_spawn_cache::RemoteSpawnCache;
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils;
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use util::fake_action_input_file_cache::FakeActionInputFileCache;
use util::fake_spawn_execution_context::FakeSpawnExecutionContext;
use util::in_memory_cache_client::InMemoryCacheClient;

const BUILD_REQUEST_ID: &str = "build-req-id";
const COMMAND_ID: &str = "command-id";
/// `TestConstants.WORKSPACE_NAME`.
const WORKSPACE_NAME: &str = "_main";

/// The arguments of one `downloadActionResult` call (Mockito's `ArgumentCaptor`s).
#[derive(Clone, Debug)]
struct DownloadActionResultCall {
    request_metadata: RequestMetadata,
    action_key: ActionKey,
    inline_out_err: bool,
    inline_output_files: HashSet<String>,
}

/// The answer of a `downloadActionResult` stub: `thenReturn` / `thenAnswer` / `doThrow`.
type DownloadActionResultAnswer = Box<dyn Fn() -> io::Result<Option<ActionResult>> + Send + Sync>;

/// `mock(RemoteCacheClient.class)` (the remote client behind Java's `mock(CombinedCache.class)`)
/// with the stubs the tests set up: `downloadActionResult` answers as stubbed (null by default,
/// as a Mockito mock) and records its arguments; the other methods go to an
/// [`InMemoryCacheClient`], and every call is counted (`verifyNoMoreInteractions`).
#[derive(Default)]
struct SpyRemoteCacheClient {
    delegate: InMemoryCacheClient,
    download_action_result_answer: Mutex<Option<DownloadActionResultAnswer>>,
    download_action_result_calls: Mutex<Vec<DownloadActionResultCall>>,
    upload_action_result_calls: Mutex<Vec<(ActionKey, ActionResult)>>,
    interactions: AtomicI32,
}

impl SpyRemoteCacheClient {
    /// `when(combinedCache.downloadActionResult(...)).thenAnswer(...)`.
    fn when_download_action_result(
        &self,
        answer: impl Fn() -> io::Result<Option<ActionResult>> + Send + Sync + 'static,
    ) {
        *self.download_action_result_answer.lock() = Some(Box::new(answer));
    }

    fn download_action_result_calls(&self) -> Vec<DownloadActionResultCall> {
        self.download_action_result_calls.lock().clone()
    }

    fn upload_action_result_calls(&self) -> Vec<(ActionKey, ActionResult)> {
        self.upload_action_result_calls.lock().clone()
    }

    fn interactions(&self) -> i32 {
        self.interactions.load(Ordering::SeqCst)
    }

    fn interact(&self) {
        self.interactions.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl MissingDigestsFinder for SpyRemoteCacheClient {
    async fn find_missing_digests(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digests: &[Digest],
    ) -> io::Result<IndexSet<Digest>> {
        self.interact();
        self.delegate.find_missing_digests(context, digests).await
    }
}

#[async_trait]
impl RemoteCacheClient for SpyRemoteCacheClient {
    fn cas_upload_cache(&self) -> &AsyncTaskCacheNoResult<Digest> {
        self.delegate.cas_upload_cache()
    }

    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        self.interact();
        // `when(combinedCache.remoteActionCacheSupportsUpdate()).thenReturn(true)`.
        self.delegate.get_server_capabilities()
    }

    async fn get_authority(&self) -> io::Result<String> {
        self.interact();
        self.delegate.get_authority().await
    }

    async fn download_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        inline_out_err: bool,
        inline_output_files: &HashSet<String>,
    ) -> io::Result<Option<ActionResult>> {
        self.interact();
        self.download_action_result_calls
            .lock()
            .push(DownloadActionResultCall {
                request_metadata: context.get_request_metadata().clone(),
                action_key: action_key.clone(),
                inline_out_err,
                inline_output_files: inline_output_files.clone(),
            });
        match &*self.download_action_result_answer.lock() {
            Some(answer) => answer(),
            None => Ok(None),
        }
    }

    async fn upload_action_result(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        action_key: &ActionKey,
        action_result: &ActionResult,
    ) -> io::Result<()> {
        self.interact();
        self.upload_action_result_calls
            .lock()
            .push((action_key.clone(), action_result.clone()));
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
        self.interact();
        self.delegate.download_blob(context, digest, out).await
    }

    async fn upload_blob_impl(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        blob: &dyn Blob,
    ) -> io::Result<()> {
        self.interact();
        self.delegate.upload_blob_impl(context, digest, blob).await
    }

    fn close(&self) {
        self.interact();
        self.delegate.close();
    }
}

/// The `RemoteActionFileSystem` the real `downloadOutputs` injects the outputs into; records the
/// injections, or fails with the error of `fail_with` (Java: `doThrow(e).when(service)
/// .downloadOutputs(...)`).
#[derive(Default)]
struct RecordingRemoteOutputInjector {
    injected: Mutex<Vec<(PathFragment, Vec<u8>, i64)>>,
    failure: Mutex<Option<Box<dyn Fn() -> io::Error + Send + Sync>>>,
}

impl RecordingRemoteOutputInjector {
    fn fail_with(&self, failure: impl Fn() -> io::Error + Send + Sync + 'static) {
        *self.failure.lock() = Some(Box::new(failure));
    }

    fn injected(&self) -> Vec<(PathFragment, Vec<u8>, i64)> {
        self.injected.lock().clone()
    }
}

impl RemoteOutputInjector for RecordingRemoteOutputInjector {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        digest: Vec<u8>,
        size: i64,
        _expiration_time: SystemTime,
        _in_memory_output: bool,
    ) -> io::Result<()> {
        if let Some(failure) = &*self.failure.lock() {
            return Err(failure());
        }
        self.injected.lock().push((path.clone(), digest, size));
        Ok(())
    }
}

/// The `SpawnExecutionContext` of `createSpawnExecutionContext(spawn, execRoot, fakeFileCache,
/// outErr)` (an anonymous class of the Java test).
///
/// PORT: the parts that are the same as `FakeSpawnExecutionContext` are delegated to one; the
/// reported progress is recorded (Java ignores it) so that a test can see which caches the
/// `CombinedCache` looked into.
struct TestSpawnExecutionContext {
    inner: FakeSpawnExecutionContext,
    digest: Mutex<Option<blaze_protos::tools::protos::Digest>>,
    reported: Mutex<Vec<ProgressStatus>>,
}

impl TestSpawnExecutionContext {
    fn reported(&self) -> Vec<ProgressStatus> {
        self.reported.lock().clone()
    }
}

// Java: RemoteSpawnCacheTest.createSpawnExecutionContext
fn create_spawn_execution_context(
    spawn: Arc<dyn Spawn>,
    exec_root: &Path,
    fake_file_cache: Arc<FakeActionInputFileCache>,
    out_err: Arc<FileOutErr>,
) -> TestSpawnExecutionContext {
    TestSpawnExecutionContext {
        inner: FakeSpawnExecutionContext::new(spawn, fake_file_cache, exec_root.clone(), out_err),
        digest: Mutex::new(None),
        reported: Mutex::new(Vec::new()),
    }
}

impl SpawnExecutionContext for TestSpawnExecutionContext {
    fn get_id(&self) -> i32 {
        0
    }

    fn set_digest(&self, digest: blaze_protos::tools::protos::Digest) {
        let mut this_digest = self.digest.lock();
        blaze_util::check_state!(this_digest.is_none());
        *this_digest = Some(digest);
    }

    fn get_digest(&self) -> Option<blaze_protos::tools::protos::Digest> {
        self.digest.lock().clone()
    }

    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        // `return immediateVoidFuture();`
        Ok(())
    }

    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        self.inner.get_input_metadata_provider()
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
        self.inner.get_file_out_err()
    }

    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        self.inner
            .get_input_mapping(base_directory, will_access_repeatedly)
    }

    fn report(&self, progress: ProgressStatus) {
        self.reported.lock().push(progress);
    }

    fn is_rewinding_enabled(&self) -> bool {
        false
    }

    fn check_for_lost_inputs(&self) -> Result<(), ExecException> {
        Ok(())
    }

    fn get_client_env(&self) -> &IndexMap<String, String> {
        // `ImmutableMap.of()`
        self.inner.get_client_env()
    }
}

/// `new FakeOwner(mnemonic, progressMessage, ownerLabel)`.
fn fake_owner(mnemonic: &str, progress_message: &str, label: &str) -> Arc<FakeOwner> {
    Arc::new(FakeOwner::new(
        mnemonic,
        Some(progress_message),
        Some(Label::parse_canonical_unchecked(label)),
    ))
}

// Java: RemoteSpawnCacheTest.simpleSpawnWithExecutionInfo
fn simple_spawn_with_execution_info(execution_info: &[(&str, &str)]) -> Arc<SimpleSpawn> {
    let mut inputs = NestedSetBuilder::stable_order();
    inputs.add(ActionInputRef::new(BasicActionInput::from_path("input")));
    Arc::new(SimpleSpawn::new_simple(
        fake_owner("Mnemonic", "Progress Message", "//dummy:label"),
        vec!["/bin/echo".to_string(), "Hi!".to_string()],
        IndexMap::from([("VARIABLE".to_string(), "value".to_string())]),
        execution_info
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        /* inputs= */ inputs.build(),
        // PORT: `random/file` instead of `/random/file` (see the file comment).
        /* outputs= */
        vec![ActionInputRef::new(BasicActionInput::from_path(
            "random/file",
        ))],
        Arc::new(ResourceSet::zero()),
    ))
}

/// `new SpawnResult.Builder().setExitCode(0).setStatus(Status.SUCCESS).setRunnerName("test")
/// .build()`.
fn successful_spawn_result() -> SpawnResult {
    SpawnResultBuilder::new()
        .set_exit_code(0)
        .set_status(Status::Success)
        .set_runner_name("test")
        .build()
}

/// The fields of the Java test class.
struct Fixture {
    _tmp: tempfile::TempDir,
    fs_root: String,
    fs: FileSystemRef,
    digest_util: DigestUtil,
    exec_root: Path,
    temp_path_generator: Arc<TempPathGenerator>,
    simple_spawn: Arc<SimpleSpawn>,
    simple_policy: TestSpawnExecutionContext,
    successful_result: ActionResult,
    /// `@Mock private CombinedCache combinedCache`.
    combined_cache: Arc<CombinedCache>,
    /// The remote client of `combined_cache`, which the tests stub and verify.
    remote_cache_client: Arc<SpyRemoteCacheClient>,
    out_err: Arc<FileOutErr>,
    event_handler: Arc<StoredEventHandler>,
    remote_path_resolver: Arc<dyn RemotePathResolver>,
    remote_output_injector: Arc<RecordingRemoteOutputInjector>,
}

impl Fixture {
    // Java: RemoteSpawnCacheTest.setUp
    fn new() -> Fixture {
        // PORT: Java reads the version from BlazeVersionInfo.
        tracing_metadata_utils::set_tool_version("");
        let tmp = tempfile::tempdir().unwrap();
        let fs_root = std::fs::canonicalize(tmp.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{fs_root}/exec/root"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let temp_path_generator = Arc::new(TempPathGenerator::new(Path::create_from_str(
            &format!("{fs_root}/execroot/_tmp/actions/remote"),
            &fs,
        )));
        let fake_file_cache = Arc::new(FakeActionInputFileCache::new(exec_root.clone()));
        let simple_spawn = simple_spawn_with_execution_info(&[]);

        let stdout = Path::create_from_str(&format!("{fs_root}/tmp/stdout"), &fs);
        let stderr = Path::create_from_str(&format!("{fs_root}/tmp/stderr"), &fs);
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
        let out_err = Arc::new(FileOutErr::new(stdout, stderr));
        let event_handler = Arc::new(StoredEventHandler::new());

        let remote_path_resolver = remote_path_resolver::create_default(exec_root.clone());
        let simple_policy = create_spawn_execution_context(
            simple_spawn.clone(),
            &exec_root,
            fake_file_cache.clone(),
            out_err.clone(),
        );

        fake_file_cache
            .create_scratch_input(simple_spawn.get_input_files().get_singleton(), "xyz")
            .unwrap();

        // `when(combinedCache.hasRemoteCache()).thenReturn(true);
        // when(combinedCache.remoteActionCacheSupportsUpdate()).thenReturn(true);`
        let remote_cache_client = Arc::new(SpyRemoteCacheClient::default());
        let combined_cache = Arc::new(CombinedCache::new(
            Some(remote_cache_client.clone() as Arc<dyn RemoteCacheClient>),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            digest_util.clone(),
            /* chunkingFunction= */ None,
        ));

        let mut fixture = Fixture {
            _tmp: tmp,
            fs_root,
            fs,
            digest_util,
            exec_root,
            temp_path_generator,
            simple_spawn,
            simple_policy,
            successful_result: ActionResult::default(),
            combined_cache,
            remote_cache_client,
            out_err,
            event_handler,
            remote_path_resolver,
            remote_output_injector: Arc::new(RecordingRemoteOutputInjector::default()),
        };
        fixture.successful_result = fixture.create_successful_result(&*fixture.simple_spawn);
        fixture
    }

    /// `fs.getPath(path)`.
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{path}", self.fs_root), &self.fs)
    }

    // Java: RemoteSpawnCacheTest.createSuccessfulResult
    fn create_successful_result(&self, spawn: &dyn Spawn) -> ActionResult {
        let mut result = ActionResult {
            exit_code: 0,
            ..Default::default()
        };
        for output in spawn.get_output_files() {
            if spawn.is_mandatory_output(output) {
                result.output_files.push(OutputFile {
                    // `spawn.getPathMapper().getMappedExecPathString(output)`
                    path: spawn
                        .get_path_mapper()
                        .map(output.get_exec_path())
                        .get_path_string()
                        .to_string(),
                    digest: Some(self.digest_util.compute_as_utf8("content")),
                    ..Default::default()
                });
            }
        }
        result
    }

    // Java: RemoteSpawnCacheTest.createRemoteSpawnCache
    fn create_remote_spawn_cache(&self) -> RemoteSpawnCache {
        self.remote_spawn_cache_with_options(RemoteOptions::default())
    }

    // Java: RemoteSpawnCacheTest.remoteSpawnCacheWithOptions
    fn remote_spawn_cache_with_options(&self, options: RemoteOptions) -> RemoteSpawnCache {
        self.remote_spawn_cache_with_options_and_execution_options(
            options,
            ExecutionOptions::default(),
        )
    }

    // Java: RemoteSpawnCacheTest.remoteSpawnCacheWithOptions
    fn remote_spawn_cache_with_options_and_execution_options(
        &self,
        options: RemoteOptions,
        execution_options: ExecutionOptions,
    ) -> RemoteSpawnCache {
        let options = Arc::new(options);
        let service = RemoteExecutionService::new(
            self.event_handler.clone(),
            /* verbose_failures= */ true,
            self.exec_root.clone(),
            self.remote_path_resolver.clone(),
            BUILD_REQUEST_ID,
            COMMAND_ID,
            WORKSPACE_NAME,
            self.digest_util.clone(),
            options.clone(),
            Arc::new(execution_options),
            Some(self.combined_cache.clone()),
            /* remote_execution_cache= */ None,
            /* remote_executor= */ None,
            self.temp_path_generator.clone(),
            /* capture_corrupted_outputs_dir= */ None,
            // `DUMMY_REMOTE_OUTPUT_CHECKER`
            Some(Arc::new(RemoteOutputChecker::new(
                "build",
                RemoteOutputsMode::MINIMAL,
                vec![],
            ))),
            Arc::new(Mutex::new(HashSet::new())),
            /* scrubber= */ None,
        )
        .with_hooks(RemoteHooks {
            remote_output_injector: Some(
                self.remote_output_injector.clone() as Arc<dyn RemoteOutputInjector>
            ),
            may_modify_spawn_outputs_after_execution: Some(Arc::new(
                |_spawn_owner: &dyn ActionExecutionMetadata| false,
            )
                as MayModifySpawnOutputsAfterExecution),
        });
        RemoteSpawnCache::new(
            options,
            /* verbose_failures= */ true,
            Arc::new(service),
            self.digest_util.clone(),
        )
    }

    /// Creates the output of [`simple_spawn_with_execution_info`], so that the real
    /// `uploadOutputs` finds it (Java stubs `uploadOutputs`).
    fn write_simple_spawn_output(&self) {
        let output = self.exec_root.get_relative_str("random/file");
        output
            .get_parent_directory()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        blaze_repo_fetch::fs_utils::write_content(&output, b"content").unwrap();
    }

    /// The action key of the only `downloadActionResult` call (`actionKeyCaptor.getValue()`).
    fn captured_action_key(&self) -> ActionKey {
        let calls = self.remote_cache_client.download_action_result_calls();
        calls
            .last()
            .expect("no downloadActionResult call was captured")
            .action_key
            .clone()
    }
}

/// `verify(combinedCache).downloadActionResult(any(), any(), /* inlineOutErr= */ eq(false),
/// /* inlineOutputFiles= */ eq(ImmutableSet.of()))`.
fn verify_download_action_result(remote_cache_client: &SpyRemoteCacheClient) {
    let calls = remote_cache_client.download_action_result_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(!calls[0].inline_out_err);
    assert!(calls[0].inline_output_files.is_empty());
}

/// Waits for the background uploads of the service (Java's mocked `uploadOutputs` is
/// synchronous) and returns the action results uploaded to the remote cache.
fn uploaded_action_results(
    cache: &RemoteSpawnCache,
    remote_cache_client: &SpyRemoteCacheClient,
) -> Vec<(ActionKey, ActionResult)> {
    cache.get_remote_execution_service().shutdown();
    remote_cache_client.upload_action_result_calls()
}

/// The number of regular files below `dir` (a `DiskCacheClient` without interactions has
/// written none).
fn count_files(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            count += count_files(&entry.path());
        } else {
            count += 1;
        }
    }
    count
}

#[test]
fn cache_hit() {
    // arrange
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    let successful_result = f.successful_result.clone();
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(successful_result.clone())));

    // act
    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();
    assert!(entry.has_result());
    let result = entry.get_result();

    // assert
    // The answer of the stub: the request metadata of the context it receives.
    let calls = f.remote_cache_client.download_action_result_calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].inline_out_err);
    assert!(calls[0].inline_output_files.is_empty());
    assert_eq!(
        calls[0].request_metadata.correlated_invocations_id,
        BUILD_REQUEST_ID
    );
    assert_eq!(calls[0].request_metadata.tool_invocation_id, COMMAND_ID);
    // All other methods on RemoteActionCache have side effects, so we verify all of them.
    assert_eq!(
        f.simple_policy.get_digest(),
        Some(f.digest_util.as_spawn_log_proto(&f.captured_action_key()))
    );
    // `verify(service).downloadOutputs(any(), eq(RemoteActionResult.createFromCache(
    // CachedActionResult.remote(successfulResult))))`: the real method injected the output of
    // `successfulResult`.
    let content_digest = f.digest_util.compute_as_utf8("content");
    assert_eq!(
        f.remote_output_injector.injected(),
        vec![(
            f.exec_root
                .get_relative_str("random/file")
                .as_fragment()
                .clone(),
            DigestUtil::to_binary_digest(&content_digest),
            content_digest.size_bytes,
        )]
    );
    // `verify(service, never()).uploadOutputs(any(), any(), any(), any())`
    assert!(uploaded_action_results(&cache, &f.remote_cache_client).is_empty());
    assert_eq!(
        result.get_digest(),
        Some(&f.digest_util.as_spawn_log_proto(&f.captured_action_key()))
    );
    assert!(result.setup_success());
    assert_eq!(result.exit_code(), 0);
    assert!(result.is_cache_hit());
    // We expect the CachedLocalSpawnRunner to _not_ write to outErr at all.
    assert!(!f.out_err.has_recorded_output());
    assert!(!f.out_err.has_recorded_stderr());
}

#[test]
fn cache_miss() {
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    // `.thenReturn(null)`: the default answer of the stub.

    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();

    assert_eq!(
        f.simple_policy.get_digest(),
        Some(f.digest_util.as_spawn_log_proto(&f.captured_action_key()))
    );
    assert!(!entry.has_result());
    let result = successful_spawn_result();
    // `doNothing().when(service).uploadOutputs(any(), any(), any(), any())`
    f.write_simple_spawn_output();
    entry.store(&result).unwrap();
    // `verify(service).uploadOutputs(any(), any(), any(), any())`
    assert_eq!(
        uploaded_action_results(&cache, &f.remote_cache_client).len(),
        1
    );
}

#[test]
fn no_cache_spawns() {
    // Checks that spawns satisfying Spawns.mayBeCached=false are not looked up in the cache
    // (even if it is a local cache) and that the results/artifacts are not uploaded to the cache.
    let mut f = Fixture::new();

    let mut with_local_cache = RemoteOptions::default();
    with_local_cache.disk_cache = Some(Foreign(PathFragment::create(&format!(
        "{}/etc/something/cache/here",
        f.fs_root
    ))));
    for (remote_options, is_with_local_cache) in
        [(RemoteOptions::default(), false), (with_local_cache, true)]
    {
        let mut disk_cache_client: Option<Arc<DiskCacheClient>> = None;
        let mut remote_cache_client: Option<Arc<SpyRemoteCacheClient>> = None;
        if is_with_local_cache {
            // `mock(DiskCacheClient.class)`: a real disk cache; that it has no interactions is
            // checked on its files.
            disk_cache_client = Some(Arc::new(
                DiskCacheClient::new(
                    f.get_path("/etc/something/cache/here"),
                    &f.digest_util,
                    /* check_action_result_integrity= */ true,
                )
                .unwrap(),
            ));
        } else {
            remote_cache_client = Some(Arc::new(SpyRemoteCacheClient::default()));
        }
        f.combined_cache = Arc::new(CombinedCache::new(
            remote_cache_client
                .clone()
                .map(|c| c as Arc<dyn RemoteCacheClient>),
            disk_cache_client.clone(),
            /* symlinkTemplate= */ None,
            f.digest_util.clone(),
            /* chunkingFunction= */ None,
        ));

        let remote_spawn_cache = f.remote_spawn_cache_with_options(remote_options);
        for requirement in [
            execution_requirements::NO_CACHE,
            execution_requirements::LOCAL,
        ] {
            let uncacheable_spawn = simple_spawn_with_execution_info(&[(requirement, "")]);
            let entry = remote_spawn_cache
                .lookup(&*uncacheable_spawn, &f.simple_policy)
                .unwrap();
            // `verify(remoteSpawnCache.getRemoteExecutionService(), never()).lookupCache(any())`
            if let Some(remote_cache_client) = &remote_cache_client {
                assert!(
                    remote_cache_client
                        .download_action_result_calls()
                        .is_empty()
                );
            }
            assert_eq!(f.simple_policy.get_digest(), None);
            assert!(!entry.has_result());
            let result = successful_spawn_result();
            entry.store(&result).unwrap();
            if is_with_local_cache {
                // `verifyNoMoreInteractions(diskCacheClient)`
                assert_eq!(
                    count_files(std::path::Path::new(
                        f.get_path("/etc/something/cache/here").get_path_string()
                    )),
                    0
                );
            } else {
                // `verifyNoMoreInteractions(remoteCacheClient)`
                assert_eq!(remote_cache_client.as_ref().unwrap().interactions(), 0);
            }
        }
    }
}

#[test]
fn no_remote_cache_spawns_remote_cache() {
    // Checks that spawns satisfying Spawns.mayBeCachedRemotely=false are not looked up in the
    // remote cache, and that the results/artifacts are not uploaded to the remote cache.
    let mut f = Fixture::new();

    let mut remote_cache_options = RemoteOptions::default();
    remote_cache_options.remote_cache = Some("https://somecache.com".to_string());
    let remote_cache_client = Arc::new(SpyRemoteCacheClient::default());
    f.combined_cache = Arc::new(CombinedCache::new(
        Some(remote_cache_client.clone() as Arc<dyn RemoteCacheClient>),
        /* diskCacheClient= */ None,
        /* symlinkTemplate= */ None,
        f.digest_util.clone(),
        /* chunkingFunction= */ None,
    ));
    let remote_spawn_cache = f.remote_spawn_cache_with_options(remote_cache_options);
    for requirement in [
        execution_requirements::NO_CACHE,
        execution_requirements::LOCAL,
        execution_requirements::NO_REMOTE_CACHE,
        execution_requirements::NO_REMOTE,
    ] {
        let uncacheable_spawn = simple_spawn_with_execution_info(&[(requirement, "")]);
        let entry = remote_spawn_cache
            .lookup(&*uncacheable_spawn, &f.simple_policy)
            .unwrap();
        // `verify(combinedCache, never()).downloadActionResult(any(), any(), anyBoolean(), any())`
        assert!(
            remote_cache_client
                .download_action_result_calls()
                .is_empty()
        );
        assert_eq!(f.simple_policy.get_digest(), None);
        assert!(!entry.has_result());
        let result = successful_spawn_result();
        entry.store(&result).unwrap();
        // `verifyNoMoreInteractions(remoteCacheClient)`
        assert_eq!(remote_cache_client.interactions(), 0);
    }
}

#[test]
fn no_remote_cache_spawns_combined_cache() {
    // Checks that spawns satisfying Spawns.mayBeCachedRemotely=false are not looked up in the
    // remote cache, and that the results/artifacts are not uploaded to the remote cache.
    // The disk cache part of a combined cache is considered as a local cache hence spawns tagged
    // with NO_REMOTE can sill hit it.
    let mut f = Fixture::new();

    let mut remote_options = RemoteOptions::default();
    remote_options.remote_cache = Some("https://somecache.com".to_string());
    remote_options.disk_cache = Some(Foreign(PathFragment::create(&format!(
        "{}/etc/something/cache/here",
        f.fs_root
    ))));
    // As in Java, the cache is created before `combinedCache` is replaced: it uses the mock.
    let remote_spawn_cache = f.remote_spawn_cache_with_options(remote_options);
    let remote_cache_client = Arc::new(SpyRemoteCacheClient::default());
    let disk_cache_client = Arc::new(
        DiskCacheClient::new(
            f.get_path("/etc/something/cache/here"),
            &f.digest_util,
            /* check_action_result_integrity= */ true,
        )
        .unwrap(),
    );
    f.combined_cache = Arc::new(CombinedCache::new(
        Some(remote_cache_client.clone() as Arc<dyn RemoteCacheClient>),
        Some(disk_cache_client),
        /* symlinkTemplate= */ None,
        f.digest_util.clone(),
        /* chunkingFunction= */ None,
    ));

    for requirement in [
        execution_requirements::NO_CACHE,
        execution_requirements::LOCAL,
    ] {
        let uncacheable_spawn = simple_spawn_with_execution_info(&[(requirement, "")]);
        let entry = remote_spawn_cache
            .lookup(&*uncacheable_spawn, &f.simple_policy)
            .unwrap();
        // `verify(combinedCache, never()).downloadActionResult(...)`
        assert!(
            remote_cache_client
                .download_action_result_calls()
                .is_empty()
        );
        assert_eq!(f.simple_policy.get_digest(), None);
        assert!(!entry.has_result());
        let result = successful_spawn_result();
        entry.store(&result).unwrap();
        // `verifyNoMoreInteractions(remoteCacheClient)`
        assert_eq!(remote_cache_client.interactions(), 0);
    }
}

#[test]
fn no_remote_cache_still_uses_local_cache() {
    let mut f = Fixture::new();
    let mut remote_options = RemoteOptions::default();
    remote_options.disk_cache = Some(Foreign(PathFragment::create(&format!(
        "{}/etc/something/cache/here",
        f.fs_root
    ))));
    // `when(combinedCache.hasRemoteCache()).thenReturn(false);
    // when(combinedCache.hasDiskCache()).thenReturn(true);`: a combined cache with a disk cache
    // only. `downloadActionResult` returns null: the disk cache is empty.
    f.combined_cache = Arc::new(CombinedCache::new(
        /* remoteCacheClient= */ None,
        Some(Arc::new(
            DiskCacheClient::new(
                f.get_path("/etc/something/cache/here"),
                &f.digest_util,
                /* check_action_result_integrity= */ true,
            )
            .unwrap(),
        )),
        /* symlinkTemplate= */ None,
        f.digest_util.clone(),
        /* chunkingFunction= */ None,
    ));
    let cache = f.remote_spawn_cache_with_options(remote_options);
    let cacheable_spawn =
        simple_spawn_with_execution_info(&[(execution_requirements::NO_REMOTE_CACHE, "")]);

    let _ = cache.lookup(&*cacheable_spawn, &f.simple_policy).unwrap();

    // `actionKeyCaptor.getValue()`: the action key the service builds for the spawn.
    let action_key = cache
        .get_remote_execution_service()
        .build_remote_action(&*cacheable_spawn, &f.simple_policy)
        .unwrap()
        .get_action_key()
        .clone();
    assert_eq!(
        f.simple_policy.get_digest(),
        Some(f.digest_util.as_spawn_log_proto(&action_key))
    );
    // `verify(combinedCache).downloadActionResult(any(), any(), eq(false), eq(ImmutableSet.of()))`:
    // the combined cache looked into the disk cache (and only there).
    assert_eq!(
        f.simple_policy.reported(),
        vec![ProgressStatus::CheckingCache("disk-cache".to_string())]
    );
}

#[test]
fn no_remote_exec_still_uses_cache() {
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    let cacheable_spawn =
        simple_spawn_with_execution_info(&[(execution_requirements::NO_REMOTE_EXEC, "")]);
    // `.thenReturn(null)`: the default answer of the stub.

    let _ = cache.lookup(&*cacheable_spawn, &f.simple_policy).unwrap();

    assert_eq!(
        f.simple_policy.get_digest(),
        Some(f.digest_util.as_spawn_log_proto(&f.captured_action_key()))
    );
    verify_download_action_result(&f.remote_cache_client);
}

#[test]
fn failed_actions_are_not_uploaded() {
    // Only successful action results are uploaded to the remote cache.
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();
    verify_download_action_result(&f.remote_cache_client);
    assert!(!entry.has_result());
    let result = SpawnResultBuilder::new()
        .set_exit_code(1)
        .set_status(Status::NonZeroExit)
        .set_failure_detail(FailureDetail {
            category: Some(failure_detail::Category::Spawn(failure_details::Spawn {
                code: failure_details::spawn::Code::NonZeroExit as i32,
                ..Default::default()
            })),
            ..Default::default()
        })
        .set_runner_name("test")
        .build();
    f.write_simple_spawn_output();
    entry.store(&result).unwrap();
    // `verify(service, never()).uploadOutputs(any(), any(), any(), any())`
    assert!(uploaded_action_results(&cache, &f.remote_cache_client).is_empty());
}

#[test]
fn print_warning_if_download_fails() {
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    // `doThrow(new IOException(io.grpc.Status.UNAVAILABLE.asRuntimeException()))`
    f.remote_cache_client.when_download_action_result(|| {
        Err(errors::status_to_io_error(tonic::Status::new(
            tonic::Code::Unavailable,
            "",
        )))
    });

    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();
    assert!(!entry.has_result());
    let result = successful_spawn_result();

    // `doNothing().when(service).uploadOutputs(any(), any(), any(), any())`
    f.write_simple_spawn_output();
    entry.store(&result).unwrap();
    // `verify(service).uploadOutputs(any(), eq(result), any(), any())`
    let uploaded = uploaded_action_results(&cache, &f.remote_cache_client);
    assert_eq!(uploaded.len(), 1);
    assert_eq!(uploaded[0].1.exit_code, result.exit_code());

    let events = f.event_handler.get_events();
    assert_eq!(events.len(), 1, "{events:?}");
    let evt = &events[0];
    assert_eq!(evt.get_kind(), EventKind::Warning);
    assert!(evt.get_message().contains("UNAVAILABLE"), "{evt:?}");
}

#[test]
fn orphaned_cached_result_ignored() {
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    let digest = f.digest_util.compute_as_utf8("bla");
    let action_result = ActionResult {
        // PORT: `random/file` instead of `/random/file` (see the file comment).
        output_files: vec![OutputFile {
            path: "random/file".to_string(),
            digest: Some(digest.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let answer = action_result.clone();
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(answer.clone())));
    // `doThrow(new CacheNotFoundException(digest)).when(service).downloadOutputs(any(),
    // eq(RemoteActionResult.createFromCache(CachedActionResult.remote(actionResult))))`
    let not_found = digest.clone();
    f.remote_output_injector
        .fail_with(move || CacheNotFoundException::new(not_found.clone()).into());

    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();
    assert!(!entry.has_result());
    let result = successful_spawn_result();

    // `doNothing().when(service).uploadOutputs(any(), any(), any(), any())`
    f.write_simple_spawn_output();
    entry.store(&result).unwrap();
    // `verify(service).uploadOutputs(any(), eq(result), any(), any())`
    let uploaded = uploaded_action_results(&cache, &f.remote_cache_client);
    assert_eq!(uploaded.len(), 1);
    assert_eq!(uploaded[0].1.exit_code, result.exit_code());
    let events = f.event_handler.get_events();
    assert!(events.is_empty(), "{events:?}"); // no warning is printed.
}

#[test]
fn failed_cache_action_as_cache_miss() {
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    let action_result = ActionResult {
        exit_code: 1,
        ..Default::default()
    };
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(action_result.clone())));

    let entry = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();

    assert!(!entry.has_result());
}

#[test]
fn test_download_minimal() {
    // arrange
    let f = Fixture::new();
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_outputs_mode = RemoteOutputsMode::MINIMAL;
    let cache = f.remote_spawn_cache_with_options(remote_options);

    let successful_result = f.successful_result.clone();
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(successful_result.clone())));
    // `doReturn(null).when(cache.getRemoteExecutionService()).downloadOutputs(any(), any())`:
    // the real method returns null (no in-memory output).

    // act
    let cache_handle = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();

    // assert
    assert!(cache_handle.has_result());
    assert_eq!(cache_handle.get_result().exit_code(), 0);
    // `verify(cache.getRemoteExecutionService()).downloadOutputs(any(), eq(...))`
    assert_eq!(f.remote_output_injector.injected().len(), 1);
}

#[test]
fn test_download_minimal_io_error() {
    // arrange
    let f = Fixture::new();
    let mut remote_options = RemoteOptions::default();
    remote_options.remote_outputs_mode = RemoteOutputsMode::MINIMAL;
    let cache = f.remote_spawn_cache_with_options(remote_options);

    // `IOException downloadFailure = new IOException("downloadMinimal failed")`
    let download_failure_message = "downloadMinimal failed";

    let successful_result = f.successful_result.clone();
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(successful_result.clone())));
    // `doThrow(downloadFailure).when(cache.getRemoteExecutionService()).downloadOutputs(...)`
    f.remote_output_injector
        .fail_with(move || errors::io_exception(download_failure_message));

    // act
    let cache_handle = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();

    // assert
    assert!(!cache_handle.has_result());
    // `verify(cache.getRemoteExecutionService()).downloadOutputs(any(), eq(...))`: the call
    // reached the injection, which failed.
    let events = f.event_handler.get_events();
    assert_eq!(events.len(), 1, "{events:?}");
    let evt = &events[0];
    assert_eq!(evt.get_kind(), EventKind::Warning);
    assert!(
        evt.get_message().contains(download_failure_message),
        "{evt:?}"
    );
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_is_deduplicated() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP) and action rewinding"]
fn rewound_path_mapped_action_is_deduplicated() {
    blaze_util::unported!(
        "SimpleSpawn with a path mapper (PathMapper other than NOOP), RemoteExecutionService wasRewound"
    );
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_is_deduplicated_with_spawn_output_modification() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_with_in_memory_output_is_deduplicated() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn deduplicated_action_with_non_zero_exit_code_is_a_cache_miss() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn deduplicated_action_with_missing_output_is_a_cache_miss() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_with_cache_hit_removes_in_flight_execution() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_not_uploaded_removes_in_flight_execution() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP)"]
fn path_mapped_action_with_cache_io_exception_removes_in_flight_execution() {
    blaze_util::unported!("SimpleSpawn with a path mapper (PathMapper other than NOOP)");
}

#[test]
#[ignore = "needs a non-NOOP PathMapper (blaze_exec's PathMapper has only NOOP) and CredentialHelperException (lib/authandtls/credentialhelper is not ported)"]
fn path_mapped_action_with_cache_credential_helper_exception_removes_in_flight_execution() {
    blaze_util::unported!(
        "SimpleSpawn with a path mapper (PathMapper other than NOOP), CredentialHelperException"
    );
}

#[test]
fn test_materialize_param_files() {
    // `Options.parse(ExecutionOptions.class, "--materialize_param_files")`
    let mut execution_options = ExecutionOptions::default();
    execution_options.materialize_param_files = true;
    test_param_files_are_materialized_for_flag(execution_options);
}

#[test]
fn test_materialize_param_files_is_implied_by_subcommands() {
    // `Options.parse(ExecutionOptions.class, "--subcommands")`
    let mut execution_options = ExecutionOptions::default();
    execution_options.show_subcommands = ShowSubcommands::TRUE;
    test_param_files_are_materialized_for_flag(execution_options);
}

// Java: RemoteSpawnCacheTest.testParamFilesAreMaterializedForFlag
fn test_param_files_are_materialized_for_flag(execution_options: ExecutionOptions) {
    let f = Fixture::new();
    let remote_options = RemoteOptions::default();
    let cache =
        f.remote_spawn_cache_with_options_and_execution_options(remote_options, execution_options);

    let args = vec!["--foo".to_string(), "--bar".to_string()];
    let input = ActionInputRef::new(ParamFileActionInput::new(
        PathFragment::create("out/param_file"),
        args,
        ParameterFileType::UNQUOTED,
    ));
    let mut inputs = NestedSetBuilder::stable_order();
    inputs.add(input);
    let spawn: Arc<SimpleSpawn> = Arc::new(SimpleSpawn::new_simple(
        fake_owner("foo", "bar", "//dummy:label"),
        /* arguments= */ vec![],
        /* environment= */ IndexMap::new(),
        /* executionInfo= */ IndexMap::new(),
        /* inputs= */ inputs.build(),
        /* outputs= */ vec![],
        Arc::new(ResourceSet::zero()),
    ));
    let param_file = f.exec_root.get_relative_str("out/param_file");

    let success = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    f.remote_cache_client
        .when_download_action_result(move || Ok(Some(success.clone())));
    // `doReturn(null).when(cache.getRemoteExecutionService()).downloadOutputs(any(), any())`:
    // the real method returns null (no in-memory output).

    let policy = create_spawn_execution_context(
        spawn.clone(),
        &f.exec_root,
        Arc::new(FakeActionInputFileCache::new(f.exec_root.clone())),
        f.out_err.clone(),
    );
    let second_cache_handle = cache.lookup(&*spawn, &policy).unwrap();
    assert!(second_cache_handle.has_result());
    assert!(param_file.exists());
    let content =
        String::from_utf8(blaze_repo_fetch::fs_utils::read_content(&param_file).unwrap()).unwrap();
    // `new String(...).split("\n")` (trailing empty strings removed).
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines, vec!["--foo", "--bar"]);
    second_cache_handle.close();
}

#[test]
fn missing_mandatory_outputs_no_cache_hit() {
    // Test that an AC which misses mandatory outputs is correctly ignored.
    // arrange
    let f = Fixture::new();
    let cache = f.create_remote_spawn_cache();
    f.remote_cache_client
        .when_download_action_result(|| Ok(Some(ActionResult::default())));

    // act
    let cache_handle = cache.lookup(&*f.simple_spawn, &f.simple_policy).unwrap();

    // assert
    assert!(!cache_handle.has_result());
    assert!(cache_handle.will_store());
}
