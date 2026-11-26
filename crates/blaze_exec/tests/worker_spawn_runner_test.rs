//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java.
//!
//! PORT: Java uses an `InMemoryFileSystem`; these tests use the real file system under a fresh
//! temporary directory. The Mockito mocks are fakes:
//! - `@Mock Worker` is [`FakeWorker`], whose `getResponse` answers are given per test (the last
//!   answer repeats, as Mockito's last stubbing does) and which records `putRequest`;
//! - `@Mock SpawnExecutionContext` is [`FakeContext`], which records `report` and
//!   `lockOutputFiles`;
//! - `@Mock ResourceManager` and its `ResourceHandle` are the real ones over
//!   `WorkerTestUtils.createTestWorkerPool(worker)`. `verify(resourceHandle).close()` is the
//!   worker coming back through `returnWorker`; `verify(resourceHandle).invalidateAndClose(e)` is
//!   `invalidateWorker` plus the worker status that `invalidateAndClose` sets for `e`;
//! - `@Mock InputMetadataProvider` is [`FakeInputFileCache`], which records its queries.

mod worker_common;

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{EventHandler, StoredEventHandler};
use blaze_exec::actions::execution_requirements::{self as er, WorkerProtocolFormat};
use blaze_exec::actions::resource_manager::ResourceManager;
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, BasicVirtualActionInput, ExecException, FakeOwner,
    FileArtifactValue, FilesetOutputTree, InputMapping, InputMetadataProvider, PathMapper,
    ResourceSet, RunfilesArtifactValue, SimpleSpawn, SpawnMetrics, TreeArtifactValue,
};
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::local::local_env_provider::NoopLocalEnvProvider;
use blaze_exec::exec::runfiles_tree_updater::RunfilesTreeUpdater;
use blaze_exec::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use blaze_exec::util::FileOutErr;
use blaze_exec::worker::worker::{Worker, WorkerBase, WorkerError};
use blaze_exec::worker::worker_key::WorkerKey;
use blaze_exec::worker::worker_pool::{BorrowError, WorkerPool};
use blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector;
use blaze_exec::worker::worker_process_status::{Status as WorkerStatus, WorkerProcessStatus};
use blaze_exec::worker::worker_spawn_runner::{RunnerError, WorkerSpawnRunner, expand_argument};
use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_protos::tools::protos::Digest;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::{IndexMap, IndexSet};
use parking_lot::{Condvar, Mutex};
use sha2::Digest as _;
use worker_common::sandbox_helper::write_iso_latin1;
use worker_common::*;

struct Fixture {
    _tmp: tempfile::TempDir,
    fs: FileSystemRef,
    /// The root of the test's file system (`fs.getPath("/")` in Java).
    root: Path,
    worker: Arc<FakeWorker>,
    pool: Arc<TestWorkerPool>,
    context: FakeContext,
    input_file_cache: FakeInputFileCache,
    spawn_metrics: SpawnMetrics,
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:99 WorkerSpawnRunnerTest.setUp
fn set_up() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let fs = UnixFileSystem::create(DigestHashFunction::Sha256);
    let root = fs.get_path(tmp.path().to_str().unwrap());
    let worker = Arc::new(FakeWorker::new(&fs, &root));
    let pool = Arc::new(TestWorkerPool {
        worker: Some(worker.clone() as Arc<dyn Worker>),
        returned: Mutex::new(0),
        invalidated: Mutex::new(0),
        cv: Condvar::new(),
    });
    let context = FakeContext::new(&root);
    Fixture {
        _tmp: tmp,
        fs,
        root,
        worker,
        pool,
        context,
        input_file_cache: FakeInputFileCache::default(),
        spawn_metrics: SpawnMetrics::default(),
    }
}

impl Fixture {
    /// `fs.getPath(absolute)`.
    fn path(&self, absolute: &str) -> Path {
        self.root.get_relative_str(absolute.trim_start_matches('/'))
    }

    /// `createWorkerKey(fs, mnemonic, proxied)`.
    fn create_worker_key(&self, mnemonic: &str, proxied: bool) -> Arc<WorkerKey> {
        Arc::new(create_worker_key_full(
            WorkerProtocolFormat::Proto,
            &self.fs,
            mnemonic,
            proxied,
            /* sandboxed= */ false,
            /* dynamic= */ false,
            &[],
        ))
    }

    /// `runner.execInWorker(spawn, key, context, inputs, outputs, ImmutableList.of(),
    /// inputFileCache, spawnMetrics)`.
    fn exec_in_worker(
        &mut self,
        runner: &WorkerSpawnRunner,
        spawn: &SimpleSpawn,
        key: &Arc<WorkerKey>,
        inputs: &SandboxInputs,
        outputs: &SandboxOutputs,
    ) -> Result<WorkResponse, RunnerError> {
        runner.exec_in_worker_for_testing(
            spawn,
            key.clone(),
            &self.context,
            inputs,
            outputs,
            &[],
            &self.input_file_cache,
            &mut self.spawn_metrics,
        )
    }

    /// `verify(resourceHandle).close()`; `times(0)` is `expected == 0`.
    fn assert_closed(&self, expected: usize) {
        assert_eq!(
            *self.pool.returned.lock(),
            expected,
            "resourceHandle.close()"
        );
    }

    /// `verify(resourceHandle).invalidateAndClose(...)`; `times(0)` is `None`.
    fn assert_invalidated(&self, cause: Option<WorkerStatus>) {
        match cause {
            None => assert_eq!(*self.pool.invalidated.lock(), 0, "invalidateAndClose"),
            Some(status) => {
                assert_eq!(*self.pool.invalidated.lock(), 1, "invalidateAndClose");
                assert_eq!(self.worker.base.status.get(), status);
            }
        }
    }
}

type Answer = Arc<dyn Fn(i32) -> Result<Option<WorkResponse>, WorkerError> + Send + Sync>;

/// The `@Mock Worker`.
struct FakeWorker {
    base: WorkerBase,
    sandboxed: Mutex<bool>,
    /// `when(worker.getResponse(...)).thenX(...)...`: the last answer repeats.
    answers: Mutex<Vec<Answer>>,
    get_response_calls: Mutex<usize>,
    put_requests: Mutex<Vec<WorkRequest>>,
    recording_stream_message: Mutex<String>,
    exit_value: Mutex<Option<i32>>,
}

impl FakeWorker {
    fn new(fs: &FileSystemRef, root: &Path) -> FakeWorker {
        let key = Arc::new(create_worker_key_full(
            WorkerProtocolFormat::Proto,
            fs,
            "mock",
            false,
            false,
            false,
            &[],
        ));
        FakeWorker {
            // `when(worker.getLogFile()).thenReturn(logFile)` where a test needs it.
            base: WorkerBase::new(
                key,
                0,
                root.get_relative_str("worker.log"),
                Arc::new(WorkerProcessStatus::new()),
            ),
            sandboxed: Mutex::new(false),
            answers: Mutex::new(Vec::new()),
            get_response_calls: Mutex::new(0),
            put_requests: Mutex::new(Vec::new()),
            recording_stream_message: Mutex::new(String::new()),
            exit_value: Mutex::new(None),
        }
    }

    fn when_get_response(&self, answers: Vec<Answer>) {
        *self.answers.lock() = answers;
    }
}

impl fmt::Display for FakeWorker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mock for Worker")
    }
}

impl Worker for FakeWorker {
    fn base(&self) -> &WorkerBase {
        &self.base
    }
    fn is_sandboxed(&self) -> bool {
        *self.sandboxed.lock()
    }
    fn prepare_execution(
        &self,
        _input_files: &SandboxInputs,
        _outputs: &SandboxOutputs,
        _worker_files: &BTreeSet<PathFragment>,
        _client_env: &IndexMap<String, String>,
    ) -> Result<(), WorkerError> {
        Ok(())
    }
    fn put_request(&self, request: &WorkRequest) -> io::Result<()> {
        self.put_requests.lock().push(request.clone());
        Ok(())
    }
    fn get_response(&self, request_id: i32) -> Result<Option<WorkResponse>, WorkerError> {
        let answer = {
            let mut calls = self.get_response_calls.lock();
            let answers = self.answers.lock();
            let i = (*calls).min(answers.len().saturating_sub(1));
            *calls += 1;
            // An unstubbed mock returns null.
            answers.get(i).cloned()
        };
        match answer {
            Some(answer) => answer(request_id),
            None => Ok(None),
        }
    }
    fn finish_execution(
        &self,
        _exec_root: &Path,
        _outputs: &SandboxOutputs,
    ) -> Result<(), WorkerError> {
        Ok(())
    }
    fn destroy(&self) {}
    fn died_unexpectedly(&self) -> bool {
        false
    }
    fn get_exit_value(&self) -> Option<i32> {
        *self.exit_value.lock()
    }
    fn get_recording_stream_message(&self) -> String {
        self.recording_stream_message.lock().clone()
    }
    fn get_process_id(&self) -> i64 {
        0
    }
}

/// `WorkerTestUtils.createTestWorkerPool(worker)`, which also counts the workers handed back.
// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTestUtils.java:324 WorkerTestUtils.createTestWorkerPool
struct TestWorkerPool {
    worker: Option<Arc<dyn Worker>>,
    returned: Mutex<usize>,
    invalidated: Mutex<usize>,
    cv: Condvar,
}

impl TestWorkerPool {
    /// Blocks until a worker came back through `returnWorker` (at most ten seconds).
    fn await_returned(&self) {
        let mut returned = self.returned.lock();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while *returned == 0 {
            if self.cv.wait_until(&mut returned, deadline).timed_out() {
                break;
            }
        }
    }
}

impl WorkerPool for TestWorkerPool {
    fn get_max_total_per_key(&self, _key: &Arc<WorkerKey>) -> i32 {
        1
    }
    fn get_num_active(&self, _key: &Arc<WorkerKey>) -> i32 {
        0
    }
    fn has_available_quota(&self, _key: &Arc<WorkerKey>) -> bool {
        true
    }
    fn evict_workers(&self, _worker_ids_to_evict: &IndexSet<i32>) -> IndexSet<i32> {
        IndexSet::new()
    }
    fn get_idle_workers(&self) -> IndexSet<i32> {
        IndexSet::new()
    }
    fn borrow_worker(&self, _key: &Arc<WorkerKey>) -> Result<Arc<dyn Worker>, BorrowError> {
        Ok(self.worker.clone().expect("worker"))
    }
    fn return_worker(&self, _key: &Arc<WorkerKey>, _worker: Arc<dyn Worker>) {
        *self.returned.lock() += 1;
        self.cv.notify_all();
    }
    fn invalidate_worker(&self, _worker: &Arc<dyn Worker>) {
        *self.invalidated.lock() += 1;
    }
    fn reset(&self) {}
    fn close(&self) {}
}

/// The `@Mock SpawnExecutionContext`.
struct FakeContext {
    reported: Mutex<Vec<ProgressStatus>>,
    /// `lockOutputFiles(exitCode, errorMessage, outErr == null)`.
    lock_output_files: Mutex<Vec<(i32, String, bool)>>,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
}

impl FakeContext {
    fn new(root: &Path) -> FakeContext {
        FakeContext {
            reported: Mutex::new(Vec::new()),
            lock_output_files: Mutex::new(Vec::new()),
            file_out_err: FileOutErr::new(
                root.get_relative_str("out/stdout"),
                root.get_relative_str("out/stderr"),
            ),
            client_env: IndexMap::new(),
        }
    }

    /// `verify(context).report(SpawnExecutingEvent.create("worker"))`.
    fn assert_reported_executing(&self) {
        assert_eq!(
            *self.reported.lock(),
            vec![ProgressStatus::Executing("worker".to_string())]
        );
    }
}

impl SpawnExecutionContext for FakeContext {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, _digest: Digest) {}
    fn get_digest(&self) -> Option<Digest> {
        None
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        Ok(())
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        // A mock returns null; expandArtifacts asks it only for tree artifacts.
        &NO_METADATA
    }
    fn lock_output_files(
        &self,
        exit_code: i32,
        error_message: &str,
        out_err: Option<&FileOutErr>,
    ) -> Result<(), ExecException> {
        self.lock_output_files.lock().push((
            exit_code,
            error_message.to_string(),
            out_err.is_none(),
        ));
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> Duration {
        Duration::ZERO
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

static NO_METADATA: FakeInputFileCache = FakeInputFileCache {
    queried: Mutex::new(Vec::new()),
};

/// The `@Mock InputMetadataProvider inputFileCache`: answers null and records the queries.
#[derive(Default)]
struct FakeInputFileCache {
    queried: Mutex<Vec<ActionInputRef>>,
}

impl InputMetadataProvider for FakeInputFileCache {
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        self.queried.lock().push(input.clone());
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

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:521 WorkerSpawnRunnerTest.createWorkerSpawnRunner
fn create_worker_spawn_runner(f: &Fixture, worker_options: WorkerOptions) -> WorkerSpawnRunner {
    create_worker_spawn_runner_at(f, f.path("/execRoot"), worker_options)
}

fn create_worker_spawn_runner_at(
    f: &Fixture,
    exec_root: Path,
    worker_options: WorkerOptions,
) -> WorkerSpawnRunner {
    let resource_manager = ResourceManager::new();
    // `when(resourceManager.acquireResources(any(), any(), any())).thenReturn(resourceHandle)`:
    // the real manager, with room for the spawn.
    resource_manager.set_available_resources(ResourceSet::create_with_ram_cpu(1e6, 100.0));
    WorkerSpawnRunner::new(
        exec_root.clone(),
        f.pool.clone(),
        Arc::new(StoredEventHandler::new()) as Arc<dyn EventHandler>,
        Arc::new(NoopLocalEnvProvider),
        // PORT: Java passes null binTools and runfilesTreeUpdater; Blaze's constructor needs
        // values, and execInWorker and canExec read neither.
        Arc::new(BinTools::for_unit_testing(&exec_root, Vec::new())),
        resource_manager,
        Arc::new(RunfilesTreeUpdater::new(exec_root.clone())),
        Arc::new(worker_options),
        WorkerProcessMetricsCollector::instance(),
    )
}

/// The `@Mock Spawn` with the given execution info, tool files and inputs.
fn spawn_with(
    execution_info: &[(&str, &str)],
    tool_files: &[&str],
    inputs: Vec<ActionInputRef>,
) -> SimpleSpawn {
    let mut tools = NestedSetBuilder::new_builder(Order::STABLE_ORDER);
    for t in tool_files {
        tools.add(ActionInputRef::new(BasicActionInput::from_path(t)));
    }
    let mut input_set = NestedSetBuilder::new_builder(Order::COMPILE_ORDER);
    for i in inputs {
        input_set.add(i);
    }
    SimpleSpawn::new(
        Arc::new(FakeOwner::new("Mnemonic", None, None)),
        Vec::new(),
        IndexMap::new(),
        execution_info
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        input_set.build(),
        tools.build(),
        Vec::new(),
        None,
        Arc::new(ResourceSet::create_with_ram_cpu(100.0, 1.0)),
        PathMapper::NOOP,
    )
}

fn mock_spawn(execution_info: &[(&str, &str)], tool_files: &[&str]) -> SimpleSpawn {
    spawn_with(execution_info, tool_files, Vec::new())
}

fn empty_inputs() -> SandboxInputs {
    SandboxInputs::new(IndexMap::new(), IndexMap::new(), IndexMap::new())
}

fn empty_outputs() -> SandboxOutputs {
    SandboxOutputs::create_from_sets(Vec::new(), Vec::new())
}

fn returns(response: WorkResponse) -> Answer {
    Arc::new(move |_| Ok(Some(response.clone())))
}

fn throws_interrupted() -> Answer {
    Arc::new(|_| Err(WorkerError::Interrupted))
}

fn request(id: i32, cancel: bool) -> WorkRequest {
    WorkRequest {
        request_id: id,
        cancel,
        ..Default::default()
    }
}

fn assert_interrupted(result: Result<WorkResponse, RunnerError>) {
    match result {
        Err(RunnerError::Exec(ExecException::Interrupted)) => {}
        other => panic!("expected InterruptedException, got {other:?}"),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:111 WorkerSpawnRunnerTest.testExecInWorker_happyPath
#[test]
fn test_exec_in_worker_happy_path() {
    let mut f = set_up();
    let runner = create_worker_spawn_runner(&f, worker_options());
    let key = f.create_worker_key("mnem", false);
    let log_file = f.path("/worker.log");
    f.worker.when_get_response(vec![returns(WorkResponse {
        exit_code: 0,
        output: "out".to_string(),
        ..Default::default()
    })]);
    let spawn = mock_spawn(&[], &[]);
    let response = f
        .exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs())
        .unwrap();

    assert_eq!(response.exit_code, 0);
    assert_eq!(response.request_id, 0);
    assert_eq!(response.output, "out");
    assert!(!log_file.exists());
    f.context.assert_reported_executing();
    f.assert_closed(1);
    f.assert_invalidated(None);
    assert_eq!(
        *f.context.lock_output_files.lock(),
        vec![(0, "out".to_string(), true)]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:139 WorkerSpawnRunnerTest.testExecInWorker_virtualInputs_doesntQueryInputFileCache
#[test]
fn test_exec_in_worker_virtual_inputs_doesnt_query_input_file_cache() {
    let mut f = set_up();
    let exec_root = f.path("/execRoot");
    let runner = create_worker_spawn_runner_at(&f, exec_root.clone(), worker_options());
    let key = f.create_worker_key("mnem", false);
    let log_file = f.path("/worker.log");

    // sandboxHelper.addAndCreateVirtualInput("input", "content"): the input with the SHA-256 of
    // its contents.
    let virtual_action_input = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("input"),
        b"content".to_vec(),
    ));
    let mut virtual_inputs = IndexMap::new();
    virtual_inputs.insert(
        virtual_action_input.clone(),
        sha2::Sha256::digest(b"content").to_vec(),
    );
    let inputs = SandboxInputs::new(IndexMap::new(), virtual_inputs, IndexMap::new());

    f.worker.when_get_response(vec![returns(WorkResponse {
        exit_code: 0,
        output: "out".to_string(),
        ..Default::default()
    })]);
    let spawn = spawn_with(&[], &[], vec![virtual_action_input.clone()]);

    let response = f
        .exec_in_worker(&runner, &spawn, &key, &inputs, &empty_outputs())
        .unwrap();

    assert_eq!(response.exit_code, 0);
    assert_eq!(response.request_id, 0);
    assert_eq!(response.output, "out");
    assert!(!log_file.exists());
    assert!(
        !f.input_file_cache
            .queried
            .lock()
            .contains(&virtual_action_input)
    );
    f.assert_closed(1);
    f.assert_invalidated(None);
    let locks = f.context.lock_output_files.lock();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].0, 0);
    assert!(locks[0].1.starts_with("out"));
    assert!(locks[0].2);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:189 WorkerSpawnRunnerTest.testExecInWorker_finishesAsyncOnInterrupt
#[test]
fn test_exec_in_worker_finishes_async_on_interrupt() {
    let mut f = set_up();
    let runner = create_worker_spawn_runner(&f, worker_options());
    let key = f.create_worker_key("mnem", false);
    let log_file = f.path("/worker.log");
    f.worker.when_get_response(vec![
        throws_interrupted(),
        returns(WorkResponse {
            request_id: 2,
            ..Default::default()
        }),
    ]);
    let spawn = mock_spawn(&[], &[]);
    assert_interrupted(f.exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs()));
    assert!(!log_file.exists());
    f.context.assert_reported_executing();
    assert_eq!(*f.worker.put_requests.lock(), vec![request(0, false)]);
    f.assert_closed(0);
    f.assert_invalidated(Some(WorkerStatus::PendingKillDueToInterruptedException));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:217 WorkerSpawnRunnerTest.testExecInWorker_sendsCancelMessageOnInterrupt
#[test]
fn test_exec_in_worker_sends_cancel_message_on_interrupt() {
    let mut f = set_up();
    let mut worker_options = worker_options();
    worker_options.worker_cancellation = true;
    worker_options.worker_sandboxing = true;
    *f.worker.sandboxed.lock() = true;
    let runner = create_worker_spawn_runner(&f, worker_options);
    let key = f.create_worker_key("mnem", false);
    let log_file = f.path("/worker.log");
    // Fake that the getting the regular response gets interrupted and we then answer the cancel.
    f.worker.when_get_response(vec![
        throws_interrupted(),
        Arc::new(|request_id| {
            Ok(Some(WorkResponse {
                request_id,
                was_cancelled: true,
                ..Default::default()
            }))
        }),
    ]);
    let spawn = mock_spawn(&[(er::SUPPORTS_WORKER_CANCELLATION, "1")], &[]);
    assert_interrupted(f.exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs()));
    // secondResponseRequested.acquire(), then waiting until the thread produced by
    // WorkerSpawnRunner.finishWorkAsync is finished and returned resources via resourceHandle.
    f.pool.await_returned();
    assert!(!log_file.exists());
    f.context.assert_reported_executing();
    assert_eq!(
        *f.worker.put_requests.lock(),
        vec![request(0, false), request(0, true)]
    );
    f.assert_closed(1);
    f.assert_invalidated(None);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:268 WorkerSpawnRunnerTest.testExecInWorker_unsandboxedDiesOnInterrupt
#[test]
fn test_exec_in_worker_unsandboxed_dies_on_interrupt() {
    let mut f = set_up();
    let mut worker_options = worker_options();
    worker_options.worker_cancellation = true;
    worker_options.worker_sandboxing = false;
    let runner = create_worker_spawn_runner(&f, worker_options);
    let key = f.create_worker_key("mnem", false);
    let log_file = f.path("/worker.log");
    f.worker.when_get_response(vec![throws_interrupted()]);
    // Since this worker is not sandboxed, it will just get killed on interrupt.
    let spawn = mock_spawn(&[(er::SUPPORTS_WORKER_CANCELLATION, "1")], &[]);
    assert_interrupted(f.exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs()));

    assert!(!log_file.exists());
    f.context.assert_reported_executing();
    assert_eq!(*f.worker.put_requests.lock(), vec![request(0, false)]);
    f.assert_closed(0);
    f.assert_invalidated(Some(WorkerStatus::PendingKillDueToInterruptedException));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:301 WorkerSpawnRunnerTest.testExecInWorker_noMultiplexWithDynamic
#[test]
fn test_exec_in_worker_no_multiplex_with_dynamic() {
    let mut f = set_up();
    let mut worker_options = worker_options();
    worker_options.worker_multiplex = true;
    let runner = create_worker_spawn_runner(&f, worker_options);
    // This worker key just so happens to be multiplex and require sandboxing.
    // createWorkerKey(WorkerProtocolFormat.JSON, fs, /* dynamic= */ true)
    let key = Arc::new(create_worker_key_full(
        WorkerProtocolFormat::Json,
        &f.fs,
        "dummy",
        true,
        true,
        true,
        &["arg1", "arg2", "arg3"],
    ));
    let log_file = f.path("/worker.log");
    f.worker.when_get_response(vec![returns(WorkResponse {
        exit_code: 0,
        request_id: 0,
        output: "out".to_string(),
        ..Default::default()
    })]);
    let spawn = mock_spawn(&[], &[]);
    let response = f
        .exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs())
        .unwrap();

    assert_eq!(response.exit_code, 0);
    assert_eq!(response.request_id, 0);
    assert_eq!(response.output, "out");
    assert!(!log_file.exists());
    f.context.assert_reported_executing();
    f.assert_closed(1);
    f.assert_invalidated(None);
    let locks = f.context.lock_output_files.lock();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].0, 0);
    assert!(locks[0].1.starts_with("out"));
    assert!(locks[0].2);
}

/// `logMarker(text)`.
// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:547 WorkerSpawnRunnerTest.logMarker
fn log_marker(text: &str) -> String {
    format!("---8<---8<--- {text} ---8<---8<---\n")
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:334 WorkerSpawnRunnerTest.assertRecordedResponsethrowsException
fn assert_recorded_responsethrows_exception(recorded_response: &str, exception_text: &str) {
    let mut f = set_up();
    let runner = create_worker_spawn_runner(&f, worker_options());
    let key = f.create_worker_key("mnem", false);
    // when(worker.getLogFile()).thenReturn(logFile): the fake's log file is /worker.log.
    let log_file = f.path("/worker.log");
    assert_eq!(f.worker.get_log_file(), &log_file);
    f.worker.when_get_response(vec![Arc::new(|_| {
        Err(WorkerError::Io(io::Error::other("Bad protobuf")))
    })]);
    *f.worker.recording_stream_message.lock() = recorded_response.to_string();
    *f.worker.exit_value.lock() = Some(2);
    let worker_log = "Log from worker\n";
    write_iso_latin1(&log_file, &["Log from worker"]);
    let spawn = mock_spawn(&[], &[]);
    let exec_exception =
        match f.exec_in_worker(&runner, &spawn, &key, &empty_inputs(), &empty_outputs()) {
            Err(RunnerError::Exec(ExecException::User(e))) => e,
            other => panic!("expected UserExecException, got {other:?}"),
        };

    let message = exec_exception.message.clone();
    assert!(message.contains(exception_text), "{message}");
    if !recorded_response.is_empty() {
        assert!(
            message.contains(&format!(
                "{}java.io.IOException: Bad protobuf",
                log_marker("Exception details")
            )),
            "{message}"
        );
        assert!(
            message.contains(&format!(
                "{}{}{}",
                log_marker("Start of response"),
                recorded_response,
                log_marker("End of response")
            )),
            "{message}"
        );
    }
    assert!(
        message.contains(&format!(
            "{}{}",
            log_marker(&format!(
                "Start of log, file at {}",
                log_file.get_path_string()
            )),
            worker_log
        )),
        "{message}"
    );
    let locks = f.context.lock_output_files.lock();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].0, 2);
    assert!(locks[0].1.contains(exception_text), "{}", locks[0].1);
    assert!(locks[0].2);
    drop(locks);
    f.assert_invalidated(Some(WorkerStatus::PendingKillDueToUserExecException));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:377 WorkerSpawnRunnerTest.testExecInWorker_showsLogFileInException
#[test]
fn test_exec_in_worker_shows_log_file_in_exception() {
    assert_recorded_responsethrows_exception("Some text", "unparseable WorkResponse!\n");
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:382 WorkerSpawnRunnerTest.testExecInWorker_throwsWithEmptyResponse
#[test]
fn test_exec_in_worker_throws_with_empty_response() {
    assert_recorded_responsethrows_exception("", "did not return a WorkResponse");
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:426 WorkerSpawnRunnerTest.testExpandArgument_expandsArgumentsRecursively
#[test]
fn test_expand_argument_expands_arguments_recursively() {
    let f = set_up();
    let mut request_builder = WorkRequest::default();
    write_iso_latin1(&f.path("/file"), &["arg1\n@file2\nmulti arg\n"]);
    write_iso_latin1(&f.path("/file2"), &["arg2\narg3"]);
    let inputs = SandboxInputs::new(
        [
            (PathFragment::create("file"), Some(f.path("/file"))),
            (PathFragment::create("file2"), Some(f.path("/file2"))),
        ]
        .into_iter()
        .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    expand_argument(&inputs, "@file", &mut request_builder).unwrap();
    // containsExactly (any order).
    let mut actual = request_builder.arguments.clone();
    actual.sort();
    let mut expected = vec!["arg1", "arg2", "arg3", "multi arg", ""];
    expected.sort();
    assert_eq!(actual, expected);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:444 WorkerSpawnRunnerTest.testExpandArgument_expandsOnlyProperArguments
#[test]
fn test_expand_argument_expands_only_proper_arguments() {
    let f = set_up();
    let mut request_builder = WorkRequest::default();
    write_iso_latin1(&f.path("/file"), &["arg1\n@@nonfile\n@foo//bar\narg2"]);
    let inputs = SandboxInputs::new(
        [(PathFragment::create("file"), Some(f.path("/file")))]
            .into_iter()
            .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    expand_argument(&inputs, "@file", &mut request_builder).unwrap();
    let mut actual = request_builder.arguments.clone();
    actual.sort();
    let mut expected = vec!["arg1", "@@nonfile", "@foo//bar", "arg2"];
    expected.sort();
    assert_eq!(actual, expected);
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:457 WorkerSpawnRunnerTest.testExpandArgument_failsOnMissingFile
#[test]
fn test_expand_argument_fails_on_missing_file() {
    let f = set_up();
    let mut request_builder = WorkRequest::default();
    let missing = f.path("/dir/file");
    let inputs = SandboxInputs::new(
        [(PathFragment::create("file"), Some(missing.clone()))]
            .into_iter()
            .collect(),
        IndexMap::new(),
        IndexMap::new(),
    );
    let e = expand_argument(&inputs, "@file", &mut request_builder).unwrap_err();
    let message = e.to_string();
    assert!(message.contains("file"), "{message}");
    assert!(message.contains(missing.get_path_string()), "{message}");
    assert!(message.contains("/dir/file"), "{message}");
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:472 WorkerSpawnRunnerTest.testCanExec_checksRequirements
#[test]
fn test_can_exec_checks_requirements() {
    let f = set_up();
    let runner = create_worker_spawn_runner(&f, worker_options());

    // Missing "supports-workers"
    assert!(!runner.can_exec(&mock_spawn(&[], &[])));

    // Missing toolFiles
    assert!(!runner.can_exec(&mock_spawn(&[(er::SUPPORTS_WORKERS, "1")], &[])));

    // Minimum requirements met
    assert!(runner.can_exec(&mock_spawn(
        &[(er::SUPPORTS_WORKERS, "1")],
        &["myTools/tool1", "myTools/tool2"]
    )));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:496 WorkerSpawnRunnerTest.testCanExec_obeysAllowlist
#[test]
fn test_can_exec_obeys_allowlist() {
    let f = set_up();
    let tools = ["myTools/tool1", "myTools/tool2"];
    let runner = create_worker_spawn_runner(&f, worker_options());

    // Allowed due to no allowlist
    assert!(runner.can_exec(&mock_spawn(
        &[
            (er::SUPPORTS_WORKERS, "1"),
            (er::WORKER_KEY_MNEMONIC, "WKM2")
        ],
        &tools
    )));

    // PORT: Java assigns `workerOptions.allowlist` after creating the runner, which shares the
    // options object; Blaze's options are immutable, so a runner is created with the allowlist.
    let mut options = worker_options();
    options.allowlist = Some(vec!["WKM1".to_string(), "Mnemonic".to_string()]);
    let runner = create_worker_spawn_runner(&f, options);

    // Blocked by allowlist
    assert!(!runner.can_exec(&mock_spawn(
        &[
            (er::SUPPORTS_WORKERS, "1"),
            (er::WORKER_KEY_MNEMONIC, "WKM2")
        ],
        &tools
    )));

    // On allowlist
    assert!(runner.can_exec(&mock_spawn(
        &[
            (er::SUPPORTS_WORKERS, "1"),
            (er::WORKER_KEY_MNEMONIC, "WKM1")
        ],
        &tools
    )));

    // On allowlist
    assert!(runner.can_exec(&mock_spawn(
        &[
            (er::SUPPORTS_WORKERS, "1"),
            (er::WORKER_KEY_MNEMONIC, "WKM1")
        ],
        &tools
    )));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerSpawnRunnerTest.java:535 WorkerSpawnRunnerTest.testExpandArgument_failsOnUndeclaredInput
#[test]
fn test_expand_argument_fails_on_undeclared_input() {
    let mut request_builder = WorkRequest::default();
    let inputs = SandboxInputs::new(IndexMap::new(), IndexMap::new(), IndexMap::new());
    let e = expand_argument(&inputs, "@file", &mut request_builder).unwrap_err();
    let message = e.to_string();
    assert!(message.contains("file"), "{message}");
    assert!(message.contains("declared input"), "{message}");
}
