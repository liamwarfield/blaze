//! The fixtures of the dynamic-execution tests: the Mockito mocks of Java's tests
//! (`ActionExecutionContext`, `DynamicStrategyRegistry`, `SandboxedSpawnStrategy`) and
//! `SpawnBuilder`.
#![allow(dead_code)]

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Clock, ExtendedEventHandler, JavaClock, StoredEventHandler};
use blaze_exec::actions::spawn_strategy::{
    SandboxedSpawnStrategy, SpawnStrategy, StopConcurrentSpawns,
};
use blaze_exec::actions::{
    ActionExecutionMetadata, ActionInputRef, BasicActionInput, ExecException, FakeOwner,
    FileArtifactValue, FilesetOutputTree, InputMetadataProvider, ResourceSet,
    RunfilesArtifactValue, SimpleSpawn, Spawn, SpawnResult, TreeArtifactValue,
};
use blaze_exec::exec::action_execution_context::{ActionContextRegistry, ActionExecutionContext};
use blaze_exec::exec::spawn_runner::ProgressStatus;
use blaze_exec::exec::spawn_strategy_registry::{DynamicMode, DynamicStrategyRegistry};
use blaze_exec::util::FileOutErr;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// `new SpawnBuilder(args).withMnemonic(mnemonic).withOwnerPrimaryOutput(output)
/// [.setBuiltForToolConfiguration(tool)]`.
pub fn spawn(args: &[&str], mnemonic: &str, primary_output: &str, tool: bool) -> Arc<SimpleSpawn> {
    let mut owner = FakeOwner::new(mnemonic, None, None);
    owner.primary_output = ActionInputRef::new(BasicActionInput::from_path(primary_output));
    owner.owner.is_build_configuration_for_tool = tool;
    Arc::new(SimpleSpawn::new_simple(
        Arc::new(owner) as Arc<dyn ActionExecutionMetadata>,
        args.iter().map(|s| s.to_string()).collect(),
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        Arc::new(ResourceSet::zero()),
    ))
}

/// `newCustomSpawn(mnemonic, executionInfo)` of `DynamicSpawnStrategyTest`: a spawn of a
/// `NullAction` (which has an execution platform) with the given execution info.
pub fn custom_spawn(mnemonic: &str, execution_info: &[(&str, &str)]) -> Arc<SimpleSpawn> {
    let mut owner = FakeOwner::new(mnemonic, None, None);
    owner.primary_output = ActionInputRef::new(BasicActionInput::from_path("output.txt"));
    owner.platform = Some(blaze_exec::actions::PlatformInfo {
        label: blaze_cmdline::Label::parse_canonical_unchecked(
            "@bazel_tools//tools:internal_platform",
        ),
        exec_properties: IndexMap::new(),
        remote_execution_properties: String::new(),
        os_constraint: None,
    });
    Arc::new(SimpleSpawn::new_simple(
        Arc::new(owner) as Arc<dyn ActionExecutionMetadata>,
        Vec::new(),
        IndexMap::new(),
        execution_info
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        Arc::new(ResourceSet::zero()),
    ))
}

/// The answer of a mocked `exec(spawn, context, stopConcurrentSpawns)`.
pub type ExecAnswer = Arc<
    dyn Fn(&dyn Spawn, Option<&dyn StopConcurrentSpawns>) -> Result<Vec<SpawnResult>, ExecException>
        + Send
        + Sync,
>;

/// `mock(SandboxedSpawnStrategy.class)`: `canExec` returns `can_exec`, `exec` runs `answer`
/// (unstubbed: Mockito's empty list), every call is recorded.
pub struct MockStrategy {
    pub name: String,
    pub can_exec: bool,
    pub answer: Mutex<Option<ExecAnswer>>,
    /// The arguments of each spawn `exec` ran, and whether a `StopConcurrentSpawns` was given.
    pub exec_calls: Mutex<Vec<(Vec<String>, bool)>>,
    pub can_exec_calls: AtomicI32,
}

impl MockStrategy {
    pub fn new(name: &str, can_exec: bool) -> Arc<MockStrategy> {
        Arc::new(MockStrategy {
            name: name.to_string(),
            can_exec,
            answer: Mutex::new(None),
            exec_calls: Mutex::new(Vec::new()),
            can_exec_calls: AtomicI32::new(0),
        })
    }

    pub fn when_exec(&self, answer: ExecAnswer) {
        *self.answer.lock() = Some(answer);
    }

    pub fn exec_count(&self) -> usize {
        self.exec_calls.lock().len()
    }

    /// Mockito's `verifyNoInteractions`.
    pub fn no_interactions(&self) -> bool {
        self.exec_count() == 0 && self.can_exec_calls.load(Ordering::SeqCst) == 0
    }
}

impl fmt::Display for MockStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl SpawnStrategy for MockStrategy {
    fn exec(
        &self,
        _spawn: &dyn Spawn,
        _context: &dyn ActionExecutionContext,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        panic!("java.lang.UnsupportedOperationException")
    }
    fn can_exec(&self, _spawn: &dyn Spawn, _registry: &dyn ActionContextRegistry) -> bool {
        self.can_exec_calls.fetch_add(1, Ordering::SeqCst);
        self.can_exec
    }
    fn as_sandboxed_spawn_strategy(&self) -> Option<&dyn SandboxedSpawnStrategy> {
        Some(self)
    }
    fn get_simple_class_name(&self) -> String {
        "MockStrategy".to_string()
    }
}

impl SandboxedSpawnStrategy for MockStrategy {
    fn exec_with_stop_concurrent_spawns(
        &self,
        spawn: &dyn Spawn,
        _context: &dyn ActionExecutionContext,
        stop_concurrent_spawns: Option<&dyn StopConcurrentSpawns>,
    ) -> Result<Vec<SpawnResult>, ExecException> {
        self.exec_calls.lock().push((
            spawn.get_arguments().to_vec(),
            stop_concurrent_spawns.is_some(),
        ));
        let answer = self.answer.lock().clone();
        match answer {
            Some(answer) => answer(spawn, stop_concurrent_spawns),
            None => Ok(Vec::new()),
        }
    }
}

/// The anonymous `DynamicStrategyRegistry` of the tests: a list of strategies per mode, or per
/// mnemonic.
pub struct TestDynamicStrategyRegistry {
    pub local: Arc<dyn Fn(&dyn Spawn) -> Vec<Arc<dyn SpawnStrategy>> + Send + Sync>,
    pub remote: Arc<dyn Fn(&dyn Spawn) -> Vec<Arc<dyn SpawnStrategy>> + Send + Sync>,
}

impl DynamicStrategyRegistry for TestDynamicStrategyRegistry {
    fn get_dynamic_spawn_action_contexts(
        &self,
        spawn: &dyn Spawn,
        dynamic_mode: DynamicMode,
    ) -> Vec<Arc<dyn SpawnStrategy>> {
        match dynamic_mode {
            DynamicMode::Local => (self.local)(spawn),
            DynamicMode::Remote => (self.remote)(spawn),
        }
    }
    fn notify_used_dynamic(&self, _registry: Option<&dyn ActionContextRegistry>) {}
}

/// An `InputMetadataProvider` that knows nothing.
pub struct EmptyInputMetadataProvider;

impl InputMetadataProvider for EmptyInputMetadataProvider {
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

/// The mocked `ActionExecutionContext` (`createMockActionExecutionContext`): a
/// `TestFileOutErr` (files in a temporary directory) and the given registry; `post` is recorded
/// by the event handler (Java's mocked `reporter`).
pub struct TestContext {
    _tmp: tempfile::TempDir,
    exec_root: Path,
    pub event_handler: StoredEventHandler,
    registry: Arc<dyn DynamicStrategyRegistry>,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    input_metadata_provider: EmptyInputMetadataProvider,
}

impl TestContext {
    pub fn new(registry: Arc<dyn DynamicStrategyRegistry>) -> TestContext {
        let tmp = tempfile::tempdir().unwrap();
        let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
        let base = fs.get_path(tmp.path().to_str().unwrap());
        TestContext {
            exec_root: base.get_child("exec"),
            event_handler: StoredEventHandler::new(),
            registry,
            file_out_err: FileOutErr::new(base.get_child("stdout"), base.get_child("stderr")),
            client_env: IndexMap::new(),
            input_metadata_provider: EmptyInputMetadataProvider,
            _tmp: tmp,
        }
    }

    /// A context over `registry` whose `FileOutErr` is `file_out_err`
    /// (`ActionsTestUtil.createContext(executor, ..., outErr, ...)`).
    pub fn with_out_err(
        registry: Arc<dyn DynamicStrategyRegistry>,
        file_out_err: FileOutErr,
    ) -> TestContext {
        let mut context = TestContext::new(registry);
        context.file_out_err = file_out_err;
        context
    }

    /// `createMockActionExecutionContext(localStrategy, remoteStrategy)`.
    pub fn with_strategies(local: Arc<MockStrategy>, remote: Arc<MockStrategy>) -> TestContext {
        TestContext::new(Arc::new(TestDynamicStrategyRegistry {
            local: Arc::new(move |_| vec![local.clone() as Arc<dyn SpawnStrategy>]),
            remote: Arc::new(move |_| vec![remote.clone() as Arc<dyn SpawnStrategy>]),
        }))
    }
}

impl ActionContextRegistry for TestContext {
    fn get_dynamic_strategy_registry(&self) -> Option<Arc<dyn DynamicStrategyRegistry>> {
        Some(self.registry.clone())
    }
}

impl ActionExecutionContext for TestContext {
    fn get_exec_root(&self) -> &Path {
        &self.exec_root
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &self.input_metadata_provider
    }
    fn get_action_file_system(&self) -> Option<FileSystemRef> {
        None
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
    fn get_event_handler(&self) -> &dyn ExtendedEventHandler {
        &self.event_handler
    }
    fn get_clock(&self) -> &dyn Clock {
        &JavaClock
    }
    fn maybe_report_subcommand(&self, _spawn: &dyn Spawn, _spawn_runner: Option<&str>) {}
    fn post_progress(&self, _action: &dyn ActionExecutionMetadata, _progress: &ProgressStatus) {}
}

/// `java.util.concurrent.Semaphore` for the tests' synchronization.
pub struct TestSemaphore {
    permits: Mutex<i32>,
    cv: parking_lot::Condvar,
}

impl TestSemaphore {
    pub fn new(permits: i32) -> Arc<TestSemaphore> {
        Arc::new(TestSemaphore {
            permits: Mutex::new(permits),
            cv: parking_lot::Condvar::new(),
        })
    }
    pub fn release(&self) {
        *self.permits.lock() += 1;
        self.cv.notify_all();
    }
    /// `acquire()`: `Err` (an `InterruptedException`) if the thread is interrupted.
    pub fn acquire(&self) -> Result<(), ExecException> {
        let mut p = self.permits.lock();
        loop {
            if blaze_shell::thread_interrupt::interrupted() {
                return Err(ExecException::Interrupted);
            }
            if *p > 0 {
                *p -= 1;
                return Ok(());
            }
            self.cv
                .wait_for(&mut p, std::time::Duration::from_millis(10));
        }
    }
}

/// `Thread.sleep(millis)`: `Err` if interrupted.
pub fn sleep(millis: u64) -> Result<(), ExecException> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
    while std::time::Instant::now() < deadline {
        if blaze_shell::thread_interrupt::interrupted() {
            return Err(ExecException::Interrupted);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
}
