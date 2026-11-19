// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/RemoteSpawnRunnerTest.java`.
//!
//! The Java test mocks `RemoteExecutionCache`, `RemoteExecutionClient` and the local
//! `SpawnRunner` with Mockito and spies on the `RemoteExecutionService`. Here:
//! - `cache` is a real `RemoteExecutionCache` over [`SpyCacheClient`], a `RemoteCacheClient` that
//!   delegates to an `InMemoryCacheClient`, can be stubbed (`downloadActionResult`) and counts
//!   calls. `when(cache.downloadActionResult(...)).thenReturn(x)` stubs the spy; a verification
//!   of a call on the mock is a check of the spy's counters.
//! - `executor` is [`FakeExecutor`], which records every `ExecuteRequest`.
//! - `localRunner` is [`FakeLocalRunner`].
//! - The service is not a spy (a Rust struct cannot be one). A verification of a call on it is
//!   checked through what that call does to the fakes (the request's `skip_cache_lookup` is
//!   `executeRemotely`'s `acceptCachedResult`, the action cache read is `lookupCache`, the
//!   action cache write is `uploadOutputs`, ...). A stub of the service (`doReturn(...).when(
//!   service).lookupCache(...)`, `doThrow(...).when(service).downloadOutputs(...)`) is replaced by
//!   fakes that make the real method produce the same value or failure; each test says how.
//! - `remoteOutputChecker` ("download nothing by default") is a real `RemoteOutputChecker` in
//!   `MINIMAL` mode with no patterns, and the `RemoteActionFileSystem` the outputs are injected
//!   into is [`RecordingInjector`].
//!
//! Java runs on an `InMemoryFileSystem`; here the tests run on the real file system below a
//! temporary directory.

mod util;

use std::any::Any as StdAny;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use blaze_actions::actions::command_lines::ParamFileActionInput;
use blaze_actions::actions::parameter_file::ParameterFileType;
use blaze_actions::{
    ActionLookupKey, ActionLookupKeyRef, Artifact, ArtifactOwner, ArtifactRoot, RootType,
};
use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::spawn_strategy::SpawnStrategy;
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, ExecException, ExecKind, FakeOwner, InputMapping,
    InputMetadataProvider, ResourceSet, SimpleSpawn, Spawn, SpawnMetrics, SpawnResult,
    SpawnResultBuilder, Status,
};
use blaze_exec::exec::abstract_spawn_strategy::{AbstractSpawnStrategy, exec_io_exception};
use blaze_exec::exec::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::action_execution_context::ShowSubcommands;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::remote_options::RemoteOptions;
use blaze_optdefs::misc::remote_outputs_mode::RemoteOutputsMode;
use blaze_protos::google::protobuf::{Any, Timestamp};
use blaze_remote::api_version::ApiVersion;
use blaze_remote::common::errors::{self, io_exception};
use blaze_remote::common::remote_path_resolver::{
    RemotePathResolver, SiblingRepositoryLayoutResolver, create_default,
};
use blaze_remote::common::{
    ActionKey, Blob, MissingDigestsFinder, OperationObserver, RemoteActionExecutionContext,
    RemoteCacheClient, RemoteExecutionClient,
};
use blaze_remote::execution_status_exception::ExecutionStatusException;
use blaze_remote::hooks::{RemoteHooks, RemoteOutputInjector};
use blaze_remote::reapi::longrunning::Operation;
use blaze_remote::reapi::{
    ActionCacheUpdateCapabilities, ActionResult, CacheCapabilities, Digest,
    ExecuteOperationMetadata, ExecuteRequest, ExecuteResponse, ExecutedActionMetadata,
    ExecutionCapabilities, LogFile, OutputFile, ServerCapabilities, execute_response,
    execution_stage, symlink_absolute_path_strategy,
};
use blaze_remote::remote_execution_cache::{RemoteExecutionCache, RemotePathChecker};
use blaze_remote::remote_execution_service::RemoteExecutionService;
use blaze_remote::remote_output_checker::RemoteOutputChecker;
use blaze_remote::remote_spawn_runner::{RemoteLocalFallbackRegistry, RemoteSpawnRunner};
use blaze_remote::retrier::TokioRetryScheduler;
use blaze_remote::util::DigestUtil;
use blaze_remote::util::async_task_cache::AsyncTaskCacheNoResult;
use blaze_remote::util::temp_path_generator::TempPathGenerator;
use blaze_remote::util::tracing_metadata_utils;
use blaze_util::ExitCode;
use blaze_vfs::{DigestHashFunction, Path, PathFragment, UnixFileSystem};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;
use prost::Message as _;

use util::fake_action_input_file_cache::FakeActionInputFileCache;
use util::fake_spawn_execution_context::FakeSpawnExecutionContext;
use util::in_memory_cache_client::InMemoryCacheClient;

/// `TestConstants.WORKSPACE_NAME`.
const WORKSPACE_NAME: &str = "_main";

// The action key of the Spawn returned by newSimpleSpawn().
const SIMPLE_ACTION_ID: &str = "31aea267dc597b047a9b6993100415b6406f82822318dc8988e4164a535b51ee";

static TOOL_VERSION: Once = Once::new();

/// `ImmutableMap.of(ExecutionRequirements.NO_CACHE, "")`.
fn no_cache() -> IndexMap<String, String> {
    IndexMap::from([("no-cache".to_string(), String::new())])
}

/// `Any.pack(message)`.
fn any_pack(type_name: &str, value: Vec<u8>) -> Any {
    Any {
        type_url: format!("type.googleapis.com/{type_name}"),
        value,
    }
}

/// `remoteExecutorCapabilities`; the cache capabilities are those of `InMemoryCacheClient`
/// (Java stubs `cache.remoteActionCacheSupportsUpdate()` to return true).
fn remote_executor_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        low_api_version: Some(ApiVersion::LOW.to_sem_ver()),
        high_api_version: Some(ApiVersion::HIGH.to_sem_ver()),
        execution_capabilities: Some(ExecutionCapabilities {
            exec_enabled: true,
            ..Default::default()
        }),
        cache_capabilities: Some(CacheCapabilities {
            action_cache_update_capabilities: Some(ActionCacheUpdateCapabilities {
                update_enabled: true,
            }),
            symlink_absolute_path_strategy: symlink_absolute_path_strategy::Value::Allowed as i32,
            ..Default::default()
        }),
        ..Default::default()
    }
}

type DownloadActionResultAnswer = Box<dyn Fn() -> io::Result<Option<ActionResult>> + Send + Sync>;

/// The `RemoteCacheClient` under the mocked `cache`: an `InMemoryCacheClient` with stubs and
/// call counters.
struct SpyCacheClient {
    delegate: InMemoryCacheClient,
    download_action_result_answer: Mutex<Option<DownloadActionResultAnswer>>,
    download_action_result_calls: AtomicI32,
    upload_action_result_calls: AtomicI32,
    download_blob_calls: Mutex<Vec<Digest>>,
    extra_cas: Mutex<HashMap<Digest, Vec<u8>>>,
}

impl SpyCacheClient {
    fn new() -> SpyCacheClient {
        SpyCacheClient {
            delegate: InMemoryCacheClient::new(),
            download_action_result_answer: Mutex::new(None),
            download_action_result_calls: AtomicI32::new(0),
            upload_action_result_calls: AtomicI32::new(0),
            download_blob_calls: Mutex::new(Vec::new()),
            extra_cas: Mutex::new(HashMap::new()),
        }
    }

    /// `when(cache.downloadActionResult(any(), any(), eq(false), eq(ImmutableSet.of())))`.
    fn when_download_action_result(
        &self,
        answer: impl Fn() -> io::Result<Option<ActionResult>> + Send + Sync + 'static,
    ) {
        *self.download_action_result_answer.lock() = Some(Box::new(answer));
    }

    fn add_cas_entry(&self, digest: Digest, data: &[u8]) {
        self.extra_cas.lock().insert(digest, data.to_vec());
    }

    fn download_blob_calls(&self) -> Vec<Digest> {
        self.download_blob_calls.lock().clone()
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
        // doReturn(remoteExecutorCapabilities).when(cache).getRemoteServerCapabilities();
        Ok(remote_executor_capabilities())
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
        self.download_action_result_calls
            .fetch_add(1, Ordering::SeqCst);
        let stubbed: Option<io::Result<Option<ActionResult>>> = {
            let answer = self.download_action_result_answer.lock();
            answer.as_ref().map(|answer| answer())
        };
        if let Some(stubbed) = stubbed {
            // `/* inlineOutErr= */ eq(false), /* inlineOutputFiles= */ eq(ImmutableSet.of())`
            assert!(!inline_out_err);
            assert!(inline_output_files.is_empty());
            return stubbed;
        }
        // An unstubbed mock returns null (the in-memory cache has no action result).
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
        self.upload_action_result_calls
            .fetch_add(1, Ordering::SeqCst);
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
        self.download_blob_calls.lock().push(digest.clone());
        let extra = self.extra_cas.lock().get(digest).cloned();
        if let Some(data) = extra {
            out.write_all(&data)?;
            out.flush()?;
            return Ok(());
        }
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

type ExecuteAnswer = Box<
    dyn Fn(&ExecuteRequest, &dyn OperationObserver, i32) -> io::Result<ExecuteResponse>
        + Send
        + Sync,
>;

/// The mocked `RemoteExecutionClient executor`: records the requests; `answer` gets the request,
/// the observer and the index of the call.
struct FakeExecutor {
    requests: Mutex<Vec<ExecuteRequest>>,
    answer: Mutex<Option<ExecuteAnswer>>,
}

impl FakeExecutor {
    fn new() -> FakeExecutor {
        FakeExecutor {
            requests: Mutex::new(Vec::new()),
            answer: Mutex::new(None),
        }
    }

    /// `when(executor.executeRemotely(any(), any(), any())).thenAnswer(...)`.
    fn when_execute_remotely(
        &self,
        answer: impl Fn(&ExecuteRequest, &dyn OperationObserver, i32) -> io::Result<ExecuteResponse>
        + Send
        + Sync
        + 'static,
    ) {
        *self.answer.lock() = Some(Box::new(answer));
    }

    /// `.thenReturn(response)`.
    fn then_return(&self, response: ExecuteResponse) {
        self.when_execute_remotely(move |_, _, _| Ok(response.clone()));
    }

    /// `.thenThrow(e)`: a new error per call (an `io::Error` cannot be cloned).
    fn then_throw(&self, e: impl Fn() -> io::Error + Send + Sync + 'static) {
        self.when_execute_remotely(move |_, _, _| Err(e()));
    }

    fn requests(&self) -> Vec<ExecuteRequest> {
        self.requests.lock().clone()
    }
}

impl RemoteExecutionClient for FakeExecutor {
    fn get_server_capabilities(&self) -> io::Result<ServerCapabilities> {
        Ok(remote_executor_capabilities())
    }

    fn execute_remotely(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        request: ExecuteRequest,
        observer: &dyn OperationObserver,
    ) -> io::Result<ExecuteResponse> {
        let index = {
            let mut requests = self.requests.lock();
            requests.push(request.clone());
            requests.len() as i32 - 1
        };
        let answer = self.answer.lock();
        match &*answer {
            Some(answer) => answer(&request, observer, index),
            // An unstubbed mock returns null, which the service would dereference.
            None => panic!("NullPointerException: executeRemotely is not stubbed"),
        }
    }

    fn close(&self) {}
}

type LocalExecAnswer = Box<dyn Fn() -> Result<SpawnResult, ExecException> + Send + Sync>;

/// The mocked `SpawnRunner localRunner`.
struct FakeLocalRunner {
    exec_calls: AtomicI32,
    answer: Mutex<Option<LocalExecAnswer>>,
}

impl FakeLocalRunner {
    fn new() -> FakeLocalRunner {
        FakeLocalRunner {
            exec_calls: AtomicI32::new(0),
            answer: Mutex::new(None),
        }
    }

    fn when_exec(
        &self,
        answer: impl Fn() -> Result<SpawnResult, ExecException> + Send + Sync + 'static,
    ) {
        *self.answer.lock() = Some(Box::new(answer));
    }

    fn exec_calls(&self) -> i32 {
        self.exec_calls.load(Ordering::SeqCst)
    }
}

impl SpawnRunner for FakeLocalRunner {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn SpawnExecutionContext,
    ) -> Result<SpawnResult, ExecException> {
        self.exec_calls.fetch_add(1, Ordering::SeqCst);
        match &*self.answer.lock() {
            Some(answer) => answer(),
            // Mockito returns null; the only test that leaves this unstubbed does not look at
            // the result.
            None => Ok(success_result("test")),
        }
    }

    fn can_exec(&self, _spawn: &dyn Spawn) -> bool {
        true
    }

    fn handles_caching(&self) -> bool {
        false
    }

    fn get_name(&self) -> String {
        "local".to_string()
    }
}

/// `(spawnInput) -> fakeLocalStrategy`.
struct FakeRemoteLocalFallbackRegistry(Arc<dyn SpawnStrategy>);

impl RemoteLocalFallbackRegistry for FakeRemoteLocalFallbackRegistry {
    fn get_remote_local_fallback_strategy(
        &self,
        _spawn: &dyn Spawn,
    ) -> Option<Arc<dyn SpawnStrategy>> {
        Some(self.0.clone())
    }
}

/// The `RemoteActionFileSystem` of the spawn context: records injected outputs.
#[derive(Default)]
struct RecordingInjector {
    injected: Mutex<Vec<PathFragment>>,
}

impl RemoteOutputInjector for RecordingInjector {
    fn inject_remote_file(
        &self,
        path: &PathFragment,
        _digest: Vec<u8>,
        _size: i64,
        _expiration_time: SystemTime,
        _in_memory_output: bool,
    ) -> io::Result<()> {
        self.injected.lock().push(path.clone());
        Ok(())
    }
}

/// `mock(SpawnExecutionContext.class)` with `getTimeout()` returning `Duration.ZERO`: records
/// the reported progress. The other methods return what the Mockito defaults stand for (an
/// empty input mapping, no client environment).
struct RecordingSpawnExecutionContext {
    reports: Arc<Mutex<Vec<ProgressStatus>>>,
    input_metadata_provider: Arc<dyn InputMetadataProvider>,
    out_err: Arc<FileOutErr>,
    digest: Mutex<Option<blaze_protos::tools::protos::Digest>>,
    client_env: IndexMap<String, String>,
}

impl SpawnExecutionContext for RecordingSpawnExecutionContext {
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
        Ok(())
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &*self.input_metadata_provider
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
        _base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        Arc::new(InputMapping::new())
    }
    fn report(&self, progress: ProgressStatus) {
        self.reports.lock().push(progress);
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

/// `InOrder.verify(policy, times(1)).report(e)` for each of `expected` in turn: each is reported
/// once, in this order.
fn assert_reported_in_order(reports: &[ProgressStatus], expected: &[ProgressStatus]) {
    let mut last: Option<usize> = None;
    for e in expected {
        let positions: Vec<usize> = reports
            .iter()
            .enumerate()
            .filter(|(_, r)| *r == e)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "{e:?} reported {} times in {reports:?}",
            positions.len()
        );
        if let Some(last) = last {
            assert!(positions[0] > last, "{e:?} out of order in {reports:?}");
        }
        last = Some(positions[0]);
    }
}

fn checking_cache() -> ProgressStatus {
    ProgressStatus::CheckingCache("remote".to_string())
}

fn scheduling() -> ProgressStatus {
    ProgressStatus::Scheduling("remote".to_string())
}

fn executing() -> ProgressStatus {
    ProgressStatus::Executing("remote".to_string())
}

/// `new SpawnResult.Builder().setStatus(Status.SUCCESS).setExitCode(0).setRunnerName(name)`.
fn success_result(runner_name: &str) -> SpawnResult {
    let mut builder = SpawnResultBuilder::new();
    builder
        .set_status(Status::Success)
        .set_exit_code(0)
        .set_runner_name(runner_name);
    builder.build()
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
    fn as_any(&self) -> &dyn StdAny {
        self
    }
}

// Java: ActionsTestUtil.createArtifactWithExecPath(ArtifactRoot root, PathFragment execPath)
fn create_artifact_with_exec_path(root: &ArtifactRoot, exec_path: PathFragment) -> Artifact {
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

/// `new FakeOwner("foo", "bar", "//dummy:label")`.
fn fake_owner() -> Arc<dyn ActionExecutionMetadata> {
    Arc::new(FakeOwner::new(
        "foo",
        Some("bar"),
        Some(Label::parse_canonical_unchecked("//dummy:label")),
    ))
}

// Java: RemoteSpawnRunnerTest.newSimpleSpawn
fn new_simple_spawn(outputs: &[Artifact]) -> Arc<dyn Spawn> {
    simple_spawn_with_execution_info(IndexMap::new(), outputs)
}

// Java: RemoteSpawnRunnerTest.simpleSpawnWithExecutionInfo
fn simple_spawn_with_execution_info(
    execution_info: IndexMap<String, String>,
    outputs: &[Artifact],
) -> Arc<dyn Spawn> {
    Arc::new(SimpleSpawn::new_simple(
        fake_owner(),
        /* arguments= */ Vec::new(),
        /* environment= */ IndexMap::new(),
        /* executionInfo= */ execution_info,
        /* inputs= */ NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        /* outputs= */ outputs.iter().map(artifact_input).collect(),
        Arc::new(ResourceSet::zero()),
    ))
}

/// A spawn with one input and no outputs.
fn spawn_with_input(input: ActionInputRef) -> Arc<dyn Spawn> {
    let mut inputs = NestedSetBuilder::stable_order();
    inputs.add(input);
    Arc::new(SimpleSpawn::new_simple(
        fake_owner(),
        /* arguments= */ Vec::new(),
        /* environment= */ IndexMap::new(),
        /* executionInfo= */ IndexMap::new(),
        /* inputs= */ inputs.build(),
        /* outputs= */ Vec::new(),
        Arc::new(ResourceSet::zero()),
    ))
}

/// `Timestamps.add(t, Durations.fromMillis(millis))`.
fn add_millis(t: Timestamp, millis: i64) -> Timestamp {
    let nanos = t.seconds as i128 * 1_000_000_000 + t.nanos as i128 + millis as i128 * 1_000_000;
    Timestamp {
        seconds: nanos.div_euclid(1_000_000_000) as i64,
        nanos: nanos.rem_euclid(1_000_000_000) as i32,
    }
}

/// `new IOException(new ExecutionStatusException(resp.getStatus(), resp))`.
fn execution_status_io_exception(resp: &ExecuteResponse) -> io::Error {
    ExecutionStatusException::new(resp.status.clone().unwrap_or_default(), Some(resp.clone()))
        .into()
}

/// The fields of the Java test class, as `setUp` leaves them.
struct RemoteSpawnRunnerTest {
    _dir: tempfile::TempDir,
    reporter: Arc<blaze_events::StoredEventHandler>,
    exec_root: Path,
    artifact_root: ArtifactRoot,
    temp_path_generator: Arc<TempPathGenerator>,
    log_dir: Path,
    digest_util: DigestUtil,
    fake_file_cache: Arc<FakeActionInputFileCache>,
    out_err: Arc<FileOutErr>,
    remote_options: RemoteOptions,
    cache: Arc<SpyCacheClient>,
    executor: Arc<FakeExecutor>,
    local_runner: Arc<FakeLocalRunner>,
    injector: Arc<RecordingInjector>,
    /// The `RemotePathChecker` given to the `RemoteExecutionCache` of `new_service`
    /// (`setRemotePathChecker`), when set.
    remote_path_checker: Mutex<Option<Arc<dyn RemotePathChecker>>>,
}

impl RemoteSpawnRunnerTest {
    // Java: RemoteSpawnRunnerTest.setUp
    fn set_up() -> RemoteSpawnRunnerTest {
        TOOL_VERSION.call_once(|| tracing_metadata_utils::set_tool_version(""));
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
        let digest_util = DigestUtil::new(DigestHashFunction::Sha256);
        let exec_root = Path::create_from_str(&format!("{root}/exec/root"), &fs);
        exec_root.create_directory_and_parents().unwrap();
        let artifact_root =
            ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["outputs"]);
        artifact_root
            .get_root()
            .as_path()
            .unwrap()
            .create_directory_and_parents()
            .unwrap();
        let temp_path_generator = Arc::new(TempPathGenerator::new(Path::create_from_str(
            &format!("{root}/execroot/_tmp/actions/remote"),
            &fs,
        )));
        let log_dir = Path::create_from_str(&format!("{root}/server-logs"), &fs);
        let fake_file_cache = Arc::new(FakeActionInputFileCache::new(exec_root.clone()));

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
        let out_err = Arc::new(FileOutErr::new(stdout, stderr));

        RemoteSpawnRunnerTest {
            _dir: dir,
            reporter: Arc::new(blaze_events::StoredEventHandler::new()),
            exec_root,
            artifact_root,
            temp_path_generator,
            log_dir,
            digest_util,
            fake_file_cache,
            out_err,
            remote_options: RemoteOptions::default(),
            cache: Arc::new(SpyCacheClient::new()),
            executor: Arc::new(FakeExecutor::new()),
            local_runner: Arc::new(FakeLocalRunner::new()),
            injector: Arc::new(RecordingInjector::default()),
            remote_path_checker: Mutex::new(None),
        }
    }

    // Java: RemoteSpawnRunnerTest.getSpawnContext
    fn get_spawn_context(&self, spawn: &Arc<dyn Spawn>) -> FakeSpawnExecutionContext {
        // PORT: the `RemoteLocalFallbackRegistry` of Java's action context registry is given to
        // the runner (see `new_spawn_runner`); the `RemoteActionFileSystem` is `injector`.
        FakeSpawnExecutionContext::new(
            spawn.clone(),
            self.fake_file_cache.clone(),
            self.exec_root.clone(),
            self.out_err.clone(),
        )
    }

    fn new_service(
        &self,
        remote_options: Arc<RemoteOptions>,
        execution_options: ExecutionOptions,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
    ) -> RemoteExecutionService {
        let cache = RemoteExecutionCache::new(
            self.cache.clone(),
            /* diskCacheClient= */ None,
            /* symlinkTemplate= */ None,
            self.digest_util.clone(),
            /* chunkingFunction= */ None,
        );
        if let Some(remote_path_checker) = self.remote_path_checker.lock().clone() {
            cache.set_remote_path_checker(remote_path_checker);
        }
        RemoteExecutionService::new(
            self.reporter.clone(),
            /* verbose_failures= */ true,
            self.exec_root.clone(),
            remote_path_resolver,
            "build-req-id",
            "command-id",
            WORKSPACE_NAME,
            self.digest_util.clone(),
            remote_options,
            Arc::new(execution_options),
            /* combined_cache= */ None,
            Some(Arc::new(cache)),
            Some(self.executor.clone() as Arc<dyn RemoteExecutionClient>),
            self.temp_path_generator.clone(),
            /* capture_corrupted_outputs_dir= */ None,
            // remoteOutputChecker: download nothing by default.
            Some(Arc::new(RemoteOutputChecker::new(
                "build",
                RemoteOutputsMode::MINIMAL,
                Vec::new(),
            ))),
            Arc::new(Mutex::new(HashSet::new())),
            /* scrubber= */ None,
        )
        .with_hooks(RemoteHooks {
            remote_output_injector: Some(self.injector.clone() as Arc<dyn RemoteOutputInjector>),
        })
    }

    fn set_local_fallback_registry(&self, runner: &RemoteSpawnRunner) {
        let fake_local_strategy: Arc<dyn SpawnStrategy> = Arc::new(AbstractSpawnStrategy::new(
            self.exec_root.clone(),
            self.local_runner.clone(),
            Arc::new(ExecutionOptions::default()),
            "fake",
        ));
        runner.set_remote_local_fallback_registry(Arc::new(FakeRemoteLocalFallbackRegistry(
            fake_local_strategy,
        )));
    }

    // Java: RemoteSpawnRunnerTest.newSpawnRunner
    fn new_spawn_runner(&self) -> RemoteSpawnRunner {
        self.new_spawn_runner_with_resolver(create_default(self.exec_root.clone()))
    }

    // Java: RemoteSpawnRunnerTest.newSpawnRunner
    fn new_spawn_runner_with_resolver(
        &self,
        remote_path_resolver: Arc<dyn RemotePathResolver>,
    ) -> RemoteSpawnRunner {
        let remote_options = Arc::new(self.remote_options.clone());
        let service = self.new_service(
            remote_options.clone(),
            ExecutionOptions::default(),
            remote_path_resolver,
        );
        let runner = RemoteSpawnRunner::new(
            remote_options,
            /* verbose_failures= */ false,
            Some(self.reporter.clone() as Arc<dyn blaze_events::ExtendedEventHandler>),
            Arc::new(TokioRetryScheduler),
            self.log_dir.clone(),
            Arc::new(service),
            self.digest_util.clone(),
        );
        self.set_local_fallback_registry(&runner);
        runner
    }

    /// `mock(SpawnExecutionContext.class)`.
    fn mock_spawn_execution_context(&self) -> RecordingSpawnExecutionContext {
        RecordingSpawnExecutionContext {
            reports: Arc::new(Mutex::new(Vec::new())),
            input_metadata_provider: self.fake_file_cache.clone(),
            out_err: self.out_err.clone(),
            digest: Mutex::new(None),
            client_env: IndexMap::new(),
        }
    }
}

/// `verify(service).uploadOutputs(...)`: the outputs are uploaded in the background; shutting
/// the service down waits for them.
fn wait_for_uploads(runner: &RemoteSpawnRunner) {
    runner.get_remote_execution_service().shutdown();
}

/// `succeeded`: a response with a successful result.
fn succeeded_response() -> ExecuteResponse {
    ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The error of `localRunner.exec` throwing `new IOException("local execution error")`, as the
/// runner returns it (see `RemoteSpawnRunner`'s module comment).
fn local_execution_error() -> ExecException {
    exec_io_exception(&io_exception("local execution error"))
}

#[test]
#[ignore = "needs wasRewound: action rewinding is not ported in RemoteExecutionService (the request never skips the cache lookup for a rewound action)"]
fn rewound_action_skips_cache_lookup() {}

#[test]
fn non_cachable_spawns_should_not_be_cached_remote() {
    // Test that if a spawn is marked "NO_CACHE" then it's not fetched from a remote cache.
    // It should be executed remotely, but marked non-cacheable to remote execution, so that
    // the action result is not saved in the remote cache.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_accept_cached = true;
    t.remote_options.remote_local_fallback = false;
    t.remote_options.remote_upload_local_results = true;
    t.remote_options.remote_result_cache_priority = 1;
    t.remote_options.remote_execution_priority = 2;

    let runner = t.new_spawn_runner();

    t.executor.then_return(succeeded_response());

    let spawn = simple_spawn_with_execution_info(no_cache(), &[]);
    let policy = t.get_spawn_context(&spawn);

    runner.exec(&*spawn, &policy).unwrap();

    let requests = t.executor.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].skip_cache_lookup);
    assert_eq!(
        requests[0]
            .results_cache_policy
            .unwrap_or_default()
            .priority,
        1
    );
    assert_eq!(requests[0].execution_policy.unwrap_or_default().priority, 2);
    // TODO(olaola): verify that the uploaded action has the doNotCache set.

    // verify(service, never()).lookupCache(any());
    assert_eq!(
        t.cache.download_action_result_calls.load(Ordering::SeqCst),
        0
    );
    // verify(service, never()).uploadOutputs(any(), any(), any(), any());
    wait_for_uploads(&runner);
    assert_eq!(t.cache.upload_action_result_calls.load(Ordering::SeqCst), 0);
    // verifyNoMoreInteractions(localRunner);
    assert_eq!(t.local_runner.exec_calls(), 0);
}

#[test]
fn non_cachable_spawns_should_not_be_cached_local_fallback() {
    // Test that if a non-cachable spawn is executed locally due to the local fallback,
    // that its result is not uploaded to the remote cache.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_accept_cached = true;
    t.remote_options.remote_local_fallback = true;
    t.remote_options.remote_upload_local_results = true;

    let runner = t.new_spawn_runner();

    // Throw an IOException to trigger the local fallback.
    t.executor.then_throw(|| io_exception(""));

    let spawn = simple_spawn_with_execution_info(no_cache(), &[]);
    let policy = t.get_spawn_context(&spawn);

    runner.exec(&*spawn, &policy).unwrap();

    // verify(localRunner).exec(spawn, policy);
    assert_eq!(t.local_runner.exec_calls(), 1);
    // verify(cache).ensureInputsPresent(...): the inputs were uploaded before the execution.
    assert_eq!(t.executor.requests().len(), 1);
    // verifyNoMoreInteractions(cache): no action cache lookup and no upload of the result.
    wait_for_uploads(&runner);
    assert_eq!(
        t.cache.download_action_result_calls.load(Ordering::SeqCst),
        0
    );
    assert_eq!(t.cache.upload_action_result_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cachable_spawns_should_be_cached_local_fallback() {
    // Test that if a cacheable spawn is executed locally due to the local fallback,
    // that its result is uploaded to the remote cache.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_accept_cached = true;
    t.remote_options.remote_local_fallback = true;
    t.remote_options.remote_upload_local_results = true;

    let runner = t.new_spawn_runner();
    // PORT: `doNothing().when(service).uploadOutputs(...)` is not possible; the real upload
    // writes the action result to the cache, which is what the verification below checks.

    // Throw an IOException to trigger the local fallback.
    t.executor.then_throw(|| io_exception(""));

    t.local_runner.when_exec(|| Ok(success_result("test")));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.exit_code(), 0);
    assert_eq!(result.status(), Status::Success);
    // verify(localRunner).exec(eq(spawn), eq(policy));
    assert_eq!(t.local_runner.exec_calls(), 1);
    // verify(runner).execLocallyAndUpload(any(), eq(spawn), eq(policy), eq(true));
    // verify(service).uploadOutputs(any(), eq(res), any(), any());
    wait_for_uploads(&runner);
    assert_eq!(t.cache.upload_action_result_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_local_action_should_not_be_uploaded() {
    // Test that the outputs of a locally executed action that failed are not uploaded.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = true;
    t.remote_options.remote_upload_local_results = true;

    let runner = t.new_spawn_runner();

    // Throw an IOException to trigger the local fallback.
    t.executor.then_throw(|| io_exception(""));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    t.local_runner.when_exec(|| {
        // PORT: Java uses `Mockito.mock(SpawnResult.class)` with only `exitCode()` and
        // `status()` stubbed, which skips `SpawnResult.Builder.build`'s check that a failed
        // spawn has a failure detail; the builder here needs one.
        let mut builder = SpawnResultBuilder::new();
        builder
            .set_status(Status::ExecutionFailed)
            .set_exit_code(1)
            .set_runner_name("mock")
            .set_failure_detail(blaze_protos::failure_details::FailureDetail {
                message: "mock".to_string(),
                category: Some(
                    blaze_protos::failure_details::failure_detail::Category::Spawn(
                        blaze_protos::failure_details::Spawn {
                            code: blaze_protos::failure_details::spawn::Code::ExecutionFailed
                                as i32,
                            ..Default::default()
                        },
                    ),
                ),
            });
        Ok(builder.build())
    });

    // assertThat(runner.exec(spawn, policy)).isSameInstanceAs(res);
    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.status(), Status::ExecutionFailed);
    assert_eq!(result.exit_code(), 1);
    assert_eq!(result.get_runner_name(), "mock");

    assert_eq!(t.local_runner.exec_calls(), 1);
    // verify(service, never()).uploadOutputs(any(), any(), any(), any());
    wait_for_uploads(&runner);
    assert_eq!(t.cache.upload_action_result_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn treat_failed_cached_action_as_cache_miss_local() {
    // Test that bazel treats failed cache action as a cache miss and attempts to execute action
    // locally
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = true;
    t.remote_options.remote_upload_local_results = true;

    t.cache.when_download_action_result(|| {
        Ok(Some(ActionResult {
            exit_code: 1,
            ..Default::default()
        }))
    });

    let runner = t.new_spawn_runner();
    // Throw an IOException to trigger the local fallback.
    t.executor.then_throw(|| io_exception(""));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    t.local_runner.when_exec(|| Ok(success_result("test")));

    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.status(), Status::Success);

    assert_eq!(t.local_runner.exec_calls(), 1);
    // verify(service).uploadOutputs(any(), eq(result), any(), any());
    wait_for_uploads(&runner);
    assert_eq!(t.cache.upload_action_result_calls.load(Ordering::SeqCst), 1);
    // verify(service, never()).downloadOutputs(any(), any()): nothing was downloaded or
    // injected.
    assert!(t.cache.download_blob_calls().is_empty());
    assert!(t.injector.injected.lock().is_empty());
}

#[test]
fn treat_failed_cached_action_as_cache_miss_remote() {
    // Test that bazel treats failed cache action as a cache miss and attempts to execute action
    // remotely
    let t = RemoteSpawnRunnerTest::set_up();

    t.cache.when_download_action_result(|| {
        Ok(Some(ActionResult {
            exit_code: 1,
            ..Default::default()
        }))
    });

    let runner = t.new_spawn_runner();

    t.executor.then_return(succeeded_response());
    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    runner.exec(&*spawn, &policy).unwrap();

    // verify(service).executeRemotely(any(), eq(false), any());
    let requests = t.executor.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].skip_cache_lookup);
}

#[test]
fn treat_cached_action_with_missing_output_as_cache_miss_during_remote_cache_check() {
    // Test that bazel treats a cached action with missing mandatory outputs as a cache miss and
    // attempts to execute the action remotely.
    let t = RemoteSpawnRunnerTest::set_up();

    let runner = t.new_spawn_runner();
    // doReturn(RemoteActionResult.createFromCache(CachedActionResult.remote(
    //     ActionResult.getDefaultInstance()))).when(service).lookupCache(any());
    t.cache
        .when_download_action_result(|| Ok(Some(ActionResult::default())));

    let output =
        create_artifact_with_exec_path(&t.artifact_root, PathFragment::create("outputs/out"));
    let successful_response = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 0,
            output_files: vec![OutputFile {
                path: output.get_exec_path().get_path_string().to_string(),
                digest: Some(t.digest_util.compute_as_utf8("content")),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor.then_return(successful_response);
    let spawn = new_simple_spawn(&[output]);
    let spawn_execution_context = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &spawn_execution_context).unwrap();
    assert_eq!(result.status(), Status::Success);

    // verify(service).executeRemotely(any(), eq(false), any());
    let requests = t.executor.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].skip_cache_lookup);
}

#[test]
fn treat_cached_action_with_missing_output_as_cache_miss_during_remote_execution() {
    // Test that bazel treats a cached execute result with missing mandatory outputs as a cache
    // miss and reattempts to execute the action remotely, this time ignoring cached results.
    let t = RemoteSpawnRunnerTest::set_up();

    let runner = t.new_spawn_runner();
    // Ensure that the initial cache lookup doesn't already return a result with missing
    // outputs - this case is covered by the previous test.
    // PORT: `doReturn(null).when(service).lookupCache(...)`: the cache has no result.
    t.cache.when_download_action_result(|| Ok(None));

    let output =
        create_artifact_with_exec_path(&t.artifact_root, PathFragment::create("outputs/out"));
    let response_without_output = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 0,
            ..Default::default()
        }),
        cached_result: true,
        ..Default::default()
    };
    let response_with_output = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 0,
            output_files: vec![OutputFile {
                path: output.get_exec_path().get_path_string().to_string(),
                digest: Some(t.digest_util.compute_as_utf8("content")),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor.when_execute_remotely(move |request, _, _| {
        Ok(if request.skip_cache_lookup {
            response_with_output.clone()
        } else {
            response_without_output.clone()
        })
    });
    let spawn = new_simple_spawn(&[output]);
    let spawn_execution_context = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &spawn_execution_context).unwrap();
    assert_eq!(result.status(), Status::Success);

    // The first execution attempt hits the cache and returns the result with missing outputs,
    // the second attempt forcibly re-executes the action.
    // verify(service).executeRemotely(any(), eq(true), any());
    // verify(service).executeRemotely(any(), eq(false), any());
    let requests = t.executor.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].skip_cache_lookup);
    assert!(requests[1].skip_cache_lookup);
}

#[test]
fn no_remote_executor_fallback_fails() {
    // Errors from the fallback runner should be propagated out of the remote runner.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_upload_local_results = true;
    t.remote_options.remote_local_fallback = true;

    let runner = t.new_spawn_runner();
    // Trigger local fallback
    t.executor.then_throw(|| io_exception(""));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    t.cache.when_download_action_result(|| Ok(None));

    t.local_runner.when_exec(|| Err(local_execution_error()));

    let e = runner.exec(&*spawn, &policy).unwrap_err();
    // assertThat(e).isSameInstanceAs(err);
    assert_eq!(e.get_message(), local_execution_error().get_message());

    assert_eq!(t.local_runner.exec_calls(), 1);
}

#[test]
fn remote_cache_error_fallback_fails() {
    // Errors from the fallback runner should be propagated out of the remote runner.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_upload_local_results = true;
    t.remote_options.remote_local_fallback = true;

    let runner = t.new_spawn_runner();
    // Trigger local fallback
    t.executor.then_throw(|| io_exception(""));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    t.cache
        .when_download_action_result(|| Err(io_exception("")));

    t.local_runner.when_exec(|| Err(local_execution_error()));

    let e = runner.exec(&*spawn, &policy).unwrap_err();
    assert_eq!(e.get_message(), local_execution_error().get_message());

    assert_eq!(t.local_runner.exec_calls(), 1);
}

#[test]
fn test_local_fallback_failure_remote_executor_failure() {
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_local_fallback = true;

    let runner = t.new_spawn_runner();

    t.cache.when_download_action_result(|| Ok(None));
    t.executor.then_throw(|| io_exception(""));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    t.local_runner.when_exec(|| Err(local_execution_error()));

    let e = runner.exec(&*spawn, &policy).unwrap_err();
    assert_eq!(e.get_message(), local_execution_error().get_message());

    assert_eq!(t.local_runner.exec_calls(), 1);
}

/// The response of the server logs tests: one log file named "logname".
fn response_with_server_log(log_digest: &Digest, human_readable: bool) -> ExecuteResponse {
    ExecuteResponse {
        server_logs: vec![execute_response::ServerLogsEntry {
            key: "logname".to_string(),
            value: Some(LogFile {
                digest: Some(log_digest.clone()),
                human_readable,
            }),
        }],
        ..Default::default()
    }
}

#[test]
fn test_human_readable_server_logs_saved_for_failing_action() {
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();
    let log_digest = t.digest_util.compute_as_utf8("bla");
    let log_path = t
        .log_dir
        .get_relative_str(SIMPLE_ACTION_ID)
        .get_relative_str("logname");
    let resp = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 31,
            ..Default::default()
        }),
        ..response_with_server_log(&log_digest, true)
    };
    t.executor.then_return(resp);
    // when(cache.downloadFile(any(), eq(logPath), eq(logDigest))).thenReturn(completed);
    t.cache.add_cas_entry(log_digest.clone(), b"bla");

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::NonZeroExit);

    assert_eq!(t.executor.requests().len(), 1);
    // verify(service).maybeDownloadServerLogs(any(), eq(resp), eq(logDir));
    // verify(cache).downloadFile(any(), eq(logPath), eq(logDigest));
    assert_eq!(t.cache.download_blob_calls(), vec![log_digest]);
    assert!(log_path.exists());
}

#[test]
fn test_human_readable_server_logs_saved_for_failing_action_with_sibling_repository_layout() {
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner_with_resolver(Arc::new(SiblingRepositoryLayoutResolver::new(
        t.exec_root.clone(),
    )));
    let log_digest = t.digest_util.compute_as_utf8("bla");
    let log_path = t
        .log_dir
        .get_relative_str("e0a5a3561464123504c1240b3587779cdfd6adee20f72aa136e388ecfd570c12")
        .get_relative_str("logname");
    let resp = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 31,
            ..Default::default()
        }),
        ..response_with_server_log(&log_digest, true)
    };
    t.executor.then_return(resp);
    t.cache.add_cas_entry(log_digest.clone(), b"bla");

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::NonZeroExit);

    assert_eq!(t.executor.requests().len(), 1);
    assert_eq!(t.cache.download_blob_calls(), vec![log_digest]);
    assert!(log_path.exists());
}

#[test]
fn test_human_readable_server_logs_saved_for_failing_action_with_status() {
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();
    let log_digest = t.digest_util.compute_as_utf8("bla");
    let log_path = t
        .log_dir
        .get_relative_str(SIMPLE_ACTION_ID)
        .get_relative_str("logname");
    let timeout_status = blaze_remote::reapi::rpc::Status {
        code: tonic::Code::DeadlineExceeded as i32,
        ..Default::default()
    };
    let resp = ExecuteResponse {
        status: Some(timeout_status),
        ..response_with_server_log(&log_digest, true)
    };
    let thrown = resp.clone();
    t.executor
        .then_throw(move || execution_status_io_exception(&thrown));
    t.cache.add_cas_entry(log_digest.clone(), b"bla");

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Timeout);

    assert_eq!(t.executor.requests().len(), 1);
    assert_eq!(t.cache.download_blob_calls(), vec![log_digest]);
    assert!(log_path.exists());
}

#[test]
fn test_non_human_readable_server_logs_not_saved() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let log_digest = t.digest_util.compute_as_utf8("bla");
    let resp = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 31,
            ..Default::default()
        }),
        ..response_with_server_log(&log_digest, false)
    };
    t.executor.then_return(resp);

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();

    // asset
    assert_eq!(res.status(), Status::NonZeroExit);

    assert_eq!(t.executor.requests().len(), 1);
    // verify(cache, never()).downloadFile(any(), any(Path.class), any(Digest.class));
    assert!(t.cache.download_blob_calls().is_empty());
}

#[test]
fn test_server_logs_not_saved_for_successful_action() {
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let log_digest = t.digest_util.compute_as_utf8("bla");
    let resp = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 0,
            ..Default::default()
        }),
        ..response_with_server_log(&log_digest, true)
    };
    t.executor.then_return(resp);

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    assert_eq!(t.executor.requests().len(), 1);
    // PORT: `verify(service).downloadOutputs(any(), eq(createFromResponse(resp)))` cannot be
    // observed: the result has no outputs.
    // verify(cache, never()).downloadFile(any(), any(Path.class), any(Digest.class));
    assert!(t.cache.download_blob_calls().is_empty());
}

#[test]
fn cache_download_failure_triggers_remote_execution() {
    // If downloading a cached action fails, remote execution should be tried.

    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    // PORT: `doThrow(new BulkTransferException(new CacheNotFoundException(...))).when(service)
    // .downloadOutputs(any(), eq(createFromCache(cachedResult)))`: the cached result names a
    // stdout blob that is not in the cache, so that the real `downloadOutputs` fails with that
    // exception.
    let missing_stdout = t.digest_util.compute_as_utf8("missing stdout");
    t.cache.when_download_action_result(move || {
        Ok(Some(ActionResult {
            exit_code: 0,
            stdout_digest: Some(missing_stdout.clone()),
            ..Default::default()
        }))
    });
    let exec_result = ActionResult {
        exit_code: 31,
        ..Default::default()
    };
    let succeeded = ExecuteResponse {
        result: Some(exec_result),
        ..Default::default()
    };
    t.executor.then_return(succeeded);

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();

    // assert
    assert_eq!(res.status(), Status::NonZeroExit);
    assert_eq!(res.exit_code(), 31);

    assert_eq!(t.executor.requests().len(), 1);
}

#[test]
fn results_download_failure_triggers_remote_execution_with_skip_cache_lookup() {
    // If downloading an action result fails, remote execution should be retried
    // with skip cache lookup enabled

    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    t.cache.when_download_action_result(|| Ok(None));
    // PORT: `doThrow(new BulkTransferException(new CacheNotFoundException(...))).when(service)
    // .downloadOutputs(any(), eq(createFromResponse(cachedResponse)))`: the cached result names
    // a stdout blob that is not in the cache.
    let cached_result = ActionResult {
        exit_code: 0,
        stdout_digest: Some(t.digest_util.compute_as_utf8("missing stdout")),
        ..Default::default()
    };
    let exec_result = ActionResult {
        exit_code: 31,
        ..Default::default()
    };
    let cached_response = ExecuteResponse {
        result: Some(cached_result),
        cached_result: true,
        ..Default::default()
    };
    let executed_response = ExecuteResponse {
        result: Some(exec_result),
        ..Default::default()
    };
    t.executor.when_execute_remotely(move |_, _, index| {
        Ok(if index == 0 {
            cached_response.clone()
        } else {
            executed_response.clone()
        })
    });

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();

    // assert
    assert_eq!(res.status(), Status::NonZeroExit);
    assert_eq!(res.exit_code(), 31);

    let requests = t.executor.requests();
    assert_eq!(requests.len(), 2);
    // first request should have been executed without skip cache lookup
    assert!(!requests[0].skip_cache_lookup);
    // second should have been executed with skip cache lookup
    assert!(requests[1].skip_cache_lookup);
}

/// `RemotePathChecker` that reports one path as not available locally (a remote-only input
/// whose blob the remote cache no longer has).
struct UnavailablePathChecker(Path);

#[async_trait]
impl RemotePathChecker for UnavailablePathChecker {
    async fn is_available_locally(
        &self,
        _context: &RemoteActionExecutionContext<'_>,
        path: &Path,
    ) -> io::Result<bool> {
        Ok(*path != self.0)
    }
}

// Java: com/google/devtools/build/lib/remote/RemoteSpawnRunnerTest.java:1004 RemoteSpawnRunnerTest.lostInputDuringUploadFailsWithoutRetriesOrExecution
#[test]
fn lost_input_during_upload_fails_without_retries_or_execution() {
    // A lost input that is not available locally can only be reliably regenerated by action
    // rewinding or an invocation retry, so the upload should not be retried and remote
    // execution must not be attempted.

    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    t.cache.when_download_action_result(|| Ok(None));
    let lost_input = ActionInputRef::new(blaze_exec::actions::BasicActionInput::from_path(
        "lost_input",
    ));
    let lost_digest = t
        .fake_file_cache
        .create_scratch_input(&lost_input, "lost input content")
        .unwrap();
    // PORT: Java stubs `service.uploadInputsIfNotPresent` to throw
    // `new BulkTransferException(new CacheNotFoundException(lostDigest,
    // lostInput.getExecPath()))`; the service cannot be stubbed here, so the real upload throws
    // that exception: the blob is missing from the (empty) cache and the remote path checker
    // reports the input as not available locally (RemoteExecutionCache.uploadFile).
    *t.remote_path_checker.lock() = Some(Arc::new(UnavailablePathChecker(
        t.exec_root.get_relative(lost_input.get_exec_path()),
    )));
    let runner = t.new_spawn_runner();
    let spawn = spawn_with_input(lost_input.clone());
    let policy = t.get_spawn_context(&spawn);

    // act
    let e = runner
        .exec(&*spawn, &policy)
        .expect_err("expected a LostInputsExecException");

    // assert
    let lost_inputs = e.get_lost_inputs().expect("a LostInputsExecException");
    let expected: IndexMap<String, IndexSet<ActionInputRef>> = IndexMap::from([(
        DigestUtil::to_string(&lost_digest),
        IndexSet::from([lost_input]),
    )]);
    assert_eq!(lost_inputs.by_digest, expected);
    // PORT: `verify(service).uploadInputsIfNotPresent(any(), anyBoolean())` has no spy; the lost
    // input is what that call threw.
    assert!(t.executor.requests().is_empty());
}

#[test]
#[ignore = "needs a stub of RemoteExecutionService.uploadInputsIfNotPresent throwing BulkTransferException(CacheNotFoundException(digest, \"in memory file\")): the real upload cannot fail that way here"]
fn unresolvable_cache_not_found_exception_during_upload_is_retried() {
    // A CacheNotFoundException for a blob that is not an input of the action cannot be
    // recovered from by action rewinding and is thus subject to the regular retry logic.
}

#[test]
fn test_remote_execution_timeout() {
    // If remote execution times out the SpawnResult status should be TIMEOUT.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = false;

    let runner = t.new_spawn_runner();

    let cached_result = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    t.cache.when_download_action_result(|| Ok(None));
    let resp = ExecuteResponse {
        result: Some(cached_result),
        status: Some(blaze_remote::reapi::rpc::Status {
            code: tonic::Code::DeadlineExceeded as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor
        .then_throw(move || execution_status_io_exception(&resp));

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Timeout);

    assert_eq!(t.executor.requests().len(), 1);
    // PORT: `verify(service).downloadOutputs(any(), eq(createFromResponse(resp)))` cannot be
    // observed: the result has no outputs.
}

#[test]
fn test_remote_execution_timeout_timings() {
    // If remote execution times out the SpawnResult should still have the start and wall times
    // reported correctly.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = false;

    let runner = t.new_spawn_runner();

    let execution_start = Timestamp::default();
    let execution_completed = add_millis(execution_start, 1000);
    let executed_metadata = ExecutedActionMetadata {
        execution_start_timestamp: Some(execution_start),
        execution_completed_timestamp: Some(execution_completed),
        ..Default::default()
    };

    let cached_result = ActionResult {
        exit_code: 0,
        execution_metadata: Some(executed_metadata),
        ..Default::default()
    };
    t.cache.when_download_action_result(|| Ok(None));
    let resp = ExecuteResponse {
        result: Some(cached_result),
        status: Some(blaze_remote::reapi::rpc::Status {
            code: tonic::Code::DeadlineExceeded as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor
        .then_throw(move || execution_status_io_exception(&resp));

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Timeout);
    assert_eq!(res.get_wall_time_in_ms(), 1000);
    assert_eq!(
        res.get_start_time(),
        Some(
            UNIX_EPOCH
                + Duration::new(execution_start.seconds as u64, execution_start.nanos as u32)
        )
    );

    assert_eq!(t.executor.requests().len(), 1);
}

#[test]
fn test_remote_execution_timeout_does_not_trigger_fallback() {
    // If remote execution times out the SpawnResult status should be TIMEOUT, regardess of local
    // fallback option.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = true;

    let runner = t.new_spawn_runner();

    let cached_result = ActionResult {
        exit_code: 0,
        ..Default::default()
    };
    t.cache.when_download_action_result(|| Ok(None));
    let resp = ExecuteResponse {
        result: Some(cached_result),
        status: Some(blaze_remote::reapi::rpc::Status {
            code: tonic::Code::DeadlineExceeded as i32,
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor
        .then_throw(move || execution_status_io_exception(&resp));

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Timeout);

    assert_eq!(t.executor.requests().len(), 1);
    // verify(localRunner, never()).exec(eq(spawn), eq(policy));
    assert_eq!(t.local_runner.exec_calls(), 0);
}

#[test]
fn test_remote_execution_command_failure_does_not_trigger_fallback() {
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_local_fallback = true;

    let runner = t.new_spawn_runner();

    t.cache.when_download_action_result(|| Ok(None));
    let failed = ExecuteResponse {
        result: Some(ActionResult {
            exit_code: 33,
            ..Default::default()
        }),
        ..Default::default()
    };
    t.executor.then_return(failed);

    let spawn = new_simple_spawn(&[]);

    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::NonZeroExit);
    assert_eq!(res.exit_code(), 33);

    assert_eq!(t.executor.requests().len(), 1);
    assert_eq!(t.local_runner.exec_calls(), 0);
}

#[test]
fn test_exit_code_executorfailure() {
    // If we get a failure due to the remote cache not working, the exit code should be
    // ExitCode.REMOTE_ERROR.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = false;

    let runner = t.new_spawn_runner();

    t.cache.when_download_action_result(|| Ok(None));
    t.executor.then_throw(|| io_exception("reasons"));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(
        result.exit_code(),
        ExitCode::REMOTE_ERROR.get_numeric_exit_code()
    );
    assert!(result.get_failure_message().contains("reasons"));
}

#[test]
fn test_exit_code_executionfailure() {
    // If we get a failure due to the remote executor not working, the exit code should be
    // ExitCode.REMOTE_ERROR.
    let mut t = RemoteSpawnRunnerTest::set_up();

    t.remote_options.remote_local_fallback = false;

    let runner = t.new_spawn_runner();

    t.cache
        .when_download_action_result(|| Err(io_exception("reasons")));

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(
        result.exit_code(),
        ExitCode::REMOTE_ERROR.get_numeric_exit_code()
    );
    assert!(result.get_failure_message().contains("reasons"));
}

#[test]
fn test_exit_code_remote_message() {
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_local_fallback = false;

    let runner = t.new_spawn_runner();

    t.cache.when_download_action_result(|| Ok(None));
    t.executor.then_throw(|| {
        let cause = ExecutionStatusException::new(
            blaze_remote::reapi::rpc::Status::default(),
            Some(ExecuteResponse {
                message: "beep and indeed boop".to_string(),
                ..Default::default()
            }),
        );
        errors::io_exception_with_cause("reasons", cause.into())
    });

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(
        result.exit_code(),
        ExitCode::REMOTE_ERROR.get_numeric_exit_code()
    );
    assert!(
        result
            .get_failure_message()
            .contains("beep and indeed boop")
    );
}

#[test]
fn test_materialize_param_files() {
    let mut execution_options = ExecutionOptions::default();
    // --materialize_param_files
    execution_options.materialize_param_files = true;
    test_param_files_are_materialized_for_flag(execution_options);
}

#[test]
fn test_materialize_param_files_is_implied_by_subcommands() {
    let mut execution_options = ExecutionOptions::default();
    // --subcommands
    execution_options.show_subcommands = ShowSubcommands::TRUE;
    test_param_files_are_materialized_for_flag(execution_options);
}

// Java: RemoteSpawnRunnerTest.testParamFilesAreMaterializedForFlag
fn test_param_files_are_materialized_for_flag(execution_options: ExecutionOptions) {
    let t = RemoteSpawnRunnerTest::set_up();
    let remote_options = Arc::new(RemoteOptions::default());
    let remote_execution_service = t.new_service(
        remote_options.clone(),
        execution_options,
        create_default(t.exec_root.clone()),
    );
    let runner = RemoteSpawnRunner::new(
        remote_options,
        /* verbose_failures= */ true,
        /* cmdline_reporter= */ None,
        Arc::new(TokioRetryScheduler),
        t.log_dir.clone(),
        Arc::new(remote_execution_service),
        t.digest_util.clone(),
    );
    t.set_local_fallback_registry(&runner);

    t.executor.then_return(succeeded_response());

    let args = vec!["--foo".to_string(), "--bar".to_string()];
    let input = ActionInputRef::new(ParamFileActionInput::new(
        PathFragment::create("out/param_file"),
        args,
        ParameterFileType::UNQUOTED,
    ));
    let spawn = spawn_with_input(input);
    let policy = t.get_spawn_context(&spawn);
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);
    let param_file = t.exec_root.get_relative_str("out/param_file");
    assert!(param_file.exists());
    let contents = std::fs::read_to_string(param_file.get_path_string()).unwrap();
    // `String.split("\n")` drops the trailing empty strings.
    let mut lines: Vec<&str> = contents.split('\n').collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    assert_eq!(lines, vec!["--foo", "--bar"]);
}

#[test]
fn test_download_minimal_on_cache_hit() {
    // arrange
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_outputs_mode = RemoteOutputsMode::MINIMAL;

    // doReturn(createFromCache(CachedActionResult.remote(succeededAction))).when(service)
    // .lookupCache(any());
    t.cache.when_download_action_result(|| {
        Ok(Some(ActionResult {
            exit_code: 0,
            ..Default::default()
        }))
    });

    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    // act
    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.exit_code(), 0);
    assert_eq!(result.status(), Status::Success);

    // assert
    // verify(service).downloadOutputs(any(), eq(actionResult)): a cache hit, not executed.
    assert!(result.is_cache_hit());
    assert!(t.executor.requests().is_empty());
}

#[test]
fn test_download_minimal_on_cache_miss() {
    // arrange
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_outputs_mode = RemoteOutputsMode::MINIMAL;

    t.executor.then_return(succeeded_response());

    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    // act
    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.exit_code(), 0);
    assert_eq!(result.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert!(!result.is_cache_hit());
}

#[test]
fn test_download_minimal_io_error() {
    // arrange
    let mut t = RemoteSpawnRunnerTest::set_up();
    t.remote_options.remote_outputs_mode = RemoteOutputsMode::MINIMAL;

    // PORT: `doThrow(new IOException("downloadMinimal failed")).when(service).downloadOutputs(
    // any(), eq(cachedActionResult))`: the cached result names a stdout blob whose download
    // fails with that exception.
    let failing_stdout = t.digest_util.compute_as_utf8("stdout");
    t.cache.delegate.add_download_failure(
        failing_stdout.clone(),
        io_exception("downloadMinimal failed"),
    );
    t.cache.when_download_action_result(move || {
        Ok(Some(ActionResult {
            exit_code: 0,
            stdout_digest: Some(failing_stdout.clone()),
            ..Default::default()
        }))
    });

    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    // act
    let result = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(result.get_failure_message(), "downloadMinimal failed");
}

#[test]
fn test_digest() {
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    t.executor.then_return(succeeded_response());

    let spawn = new_simple_spawn(&[]);
    let policy = t.get_spawn_context(&spawn);

    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // verify(service).executeRemotely(requestCaptor.capture(), ...): the action key is the
    // digest of the request.
    let requests = t.executor.requests();
    assert_eq!(requests.len(), 1);
    let action_key = ActionKey::new(requests[0].action_digest.clone().unwrap());

    assert_eq!(
        policy.get_digest(),
        Some(t.digest_util.as_spawn_log_proto(&action_key))
    );

    assert_eq!(
        res.get_digest(),
        Some(&t.digest_util.as_spawn_log_proto(&action_key))
    );
}

#[test]
fn accounting_disabled_without_worker() {
    // `mock(SpawnMetrics.Builder.class)` + `verifyNoMoreInteractions(spawnMetrics)`: the
    // builder is left as it was.
    let mut spawn_metrics = SpawnMetrics::for_exec(ExecKind::Remote);
    let before = spawn_metrics.clone();
    RemoteSpawnRunner::spawn_metrics_accounting(
        &mut spawn_metrics,
        &ExecutedActionMetadata::default(),
    );
    assert_eq!(spawn_metrics, before);
}

#[test]
fn accounting_adds_durations_for_stages() {
    let mut builder = SpawnMetrics {
        queue_time_in_ms: 1000,
        setup_time_in_ms: 2 * 1000,
        execution_wall_time_in_ms: 2 * 1000,
        process_outputs_time_in_ms: 2 * 1000,
        ..SpawnMetrics::for_exec(ExecKind::Remote)
    };
    let queued = Timestamp::default();
    let worker_start = add_millis(queued, 1000);
    let execution_start = add_millis(worker_start, 1000);
    let execution_completed = add_millis(execution_start, 1000);
    let output_upload_start = add_millis(execution_completed, 1000);
    let output_upload_complete = add_millis(output_upload_start, 1000);
    let executed_metadata = ExecutedActionMetadata {
        worker: "test worker".to_string(),
        queued_timestamp: Some(queued),
        worker_start_timestamp: Some(worker_start),
        execution_start_timestamp: Some(execution_start),
        execution_completed_timestamp: Some(execution_completed),
        output_upload_start_timestamp: Some(output_upload_start),
        output_upload_completed_timestamp: Some(output_upload_complete),
        ..Default::default()
    };
    RemoteSpawnRunner::spawn_metrics_accounting(&mut builder, &executed_metadata);
    let spawn_metrics = builder;
    // remote queue time is accumulated
    assert_eq!(spawn_metrics.queue_time_in_ms, 2 * 1000);
    // setup time is substituted
    assert_eq!(spawn_metrics.setup_time_in_ms, 1000);
    // execution time is unspecified, assume substituted
    assert_eq!(spawn_metrics.execution_wall_time_in_ms, 1000);
    // ProcessOutputs time is unspecified, assume substituted
    assert_eq!(spawn_metrics.process_outputs_time_in_ms, 1000);
}

/// `Operation.newBuilder().setMetadata(Any.pack(ExecuteOperationMetadata.newBuilder()
/// .setStage(stage).build())).build()`.
fn operation_with_stage(stage: execution_stage::Value) -> Operation {
    Operation {
        metadata: Some(any_pack(
            "build.bazel.remote.execution.v2.ExecuteOperationMetadata",
            ExecuteOperationMetadata {
                stage: stage as i32,
                ..Default::default()
            }
            .encode_to_vec(),
        )),
        ..Default::default()
    }
}

#[test]
fn should_report_checking_cache_before_scheduling() {
    // Prepare a faked/mocked remote SpawnExecutionContext.
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();

    t.executor.when_execute_remotely(|_, receiver, _| {
        receiver.on_next(&Operation::default())?;
        Ok(succeeded_response())
    });

    // PORT: `doReturn(null).when(service).downloadOutputs(any(), any())`: the result has no
    // outputs, so the real method downloads nothing and returns null.

    // Run the faked spawn.
    let res = runner.exec(&*spawn, &policy).unwrap();

    // Verify expected behavior with mocked remote SpawnExecutionContext.
    assert_eq!(res.status(), Status::Success);
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(
        &policy.reports.lock(),
        &[checking_cache(), scheduling(), executing()],
    );
}

#[test]
fn should_report_executing_status_without_metadata() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();
    let reports = policy.reports.clone();

    t.executor.when_execute_remotely(move |_, receiver, _| {
        // verify(policy, never()).report(SpawnExecutingEvent.create("remote"));
        assert!(!reports.lock().contains(&executing()));
        receiver.on_next(&Operation::default())?;
        Ok(succeeded_response())
    });

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(&policy.reports.lock(), &[scheduling(), executing()]);
}

#[test]
fn should_report_executing_status_after_got_executing_stage_from_metadata() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();
    let reports = policy.reports.clone();

    t.executor.when_execute_remotely(move |_, receiver, _| {
        let queued = operation_with_stage(execution_stage::Value::Queued);
        receiver.on_next(&queued)?;
        // verify(policy, never()).report(SpawnExecutingEvent.create("remote"));
        assert!(!reports.lock().contains(&executing()));

        let executing_operation = operation_with_stage(execution_stage::Value::Executing);
        receiver.on_next(&executing_operation)?;

        Ok(succeeded_response())
    });

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(&policy.reports.lock(), &[scheduling(), executing()]);
}

#[test]
fn should_ignore_invalid_metadata() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();

    t.executor.when_execute_remotely(|_, receiver, _| {
        let operation = Operation {
            // Anything that is not ExecutionOperationMetadata
            metadata: Some(any_pack(
                "google.longrunning.Operation",
                Operation::default().encode_to_vec(),
            )),
            ..Default::default()
        };
        receiver.on_next(&operation)?;
        Ok(succeeded_response())
    });

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(&policy.reports.lock(), &[scheduling(), executing()]);
}

#[test]
fn should_report_executing_status_if_no_executing_status_from_metadata() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();

    t.executor.when_execute_remotely(|_, receiver, _| {
        let completed = operation_with_stage(execution_stage::Value::Completed);
        receiver.on_next(&completed)?;
        Ok(succeeded_response())
    });

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(&policy.reports.lock(), &[scheduling(), executing()]);
}

#[test]
fn should_report_executing_status_even_no_operation_from_server() {
    // arrange
    let t = RemoteSpawnRunnerTest::set_up();
    let runner = t.new_spawn_runner();

    let spawn = new_simple_spawn(&[]);
    let policy = t.mock_spawn_execution_context();

    t.executor.then_return(succeeded_response());

    // act
    let res = runner.exec(&*spawn, &policy).unwrap();
    assert_eq!(res.status(), Status::Success);

    // assert
    assert_eq!(t.executor.requests().len(), 1);
    assert_reported_in_order(&policy.reports.lock(), &[scheduling(), executing()]);
}
