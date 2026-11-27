//! Port of `src/test/java/com/google/devtools/build/lib/exec/local/LocalSpawnRunnerTest.java`.
//!
//! Java runs most tests on an `InMemoryFileSystem`; here they run on the real file system under a
//! temporary directory, so Java's absolute paths (`/execroot`, `/out/stdout`) are prefixed with
//! that directory.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::resource_manager::{ResourceManager, get_local_host_capacity};
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, BasicVirtualActionInput, ExecException, FakeOwner,
    FileArtifactValue, FilesetOutputTree, HierarchicalPath, InputMapping, InputMetadataProvider,
    ResourceSet, RunfilesArtifactValue, SimpleSpawn, Spawn, Status, TreeArtifactValue,
    execution_requirements,
};
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::local::local_env_provider::{self, LocalEnvProvider};
use blaze_exec::exec::local::local_spawn_runner::{LocalSpawnRunner, get_cached_short_host_name};
use blaze_exec::exec::runfiles_tree_updater::RunfilesTreeUpdater;
use blaze_exec::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::runtime::ProcessWrapper;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_options::{Converter, Options, RegexPatternConverter};
use blaze_protos::tools::protos::Digest;
use blaze_shell::{
    JavaFile, JavaSubprocessFactory, Subprocess, SubprocessBuilder, SubprocessBuilderStreamAction,
    SubprocessFactory,
};
use blaze_vfs::{DigestHashFunction, FileSystemRef, Path, PathFragment, UnixFileSystem};
use indexmap::IndexMap;
use parking_lot::Mutex;
use regex::Regex;

/// `SubprocessBuilder.setDefaultSubprocessFactory` is process-wide; tests that set it run one at
/// a time.
static FACTORY_LOCK: Mutex<()> = Mutex::new(());

struct TestFs {
    _dir: tempfile::TempDir,
    fs: FileSystemRef,
    root: String,
}

impl TestFs {
    fn new() -> TestFs {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        TestFs {
            _dir: dir,
            fs: UnixFileSystem::create(DigestHashFunction::Sha256),
            root,
        }
    }
    /// `fs.getPath(path)` for a Java test path such as "/execroot".
    fn get_path(&self, path: &str) -> Path {
        Path::create_from_str(&format!("{}{}", self.root, path), &self.fs)
    }
    fn s(&self, path: &str) -> String {
        format!("{}{}", self.root, path)
    }
}

/// `FinishedSubprocess`.
struct FinishedSubprocess {
    exit_code: i32,
}

impl Subprocess for FinishedSubprocess {
    fn destroy(&self) -> bool {
        false
    }
    fn exit_value(&self) -> i32 {
        self.exit_code
    }
    fn finished(&self) -> bool {
        true
    }
    fn is_alive(&self) -> bool {
        false
    }
    fn timedout(&self) -> bool {
        false
    }
    fn wait_for(&self) -> Result<(), blaze_shell::InterruptedException> {
        // Do nothing.
        Ok(())
    }
    fn get_output_stream(&self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(io::sink()))
    }
    fn get_input_stream(&self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(io::empty()))
    }
    fn get_error_stream(&self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(io::empty()))
    }
    fn get_process_id(&self) -> i64 {
        0
    }
    fn close(&self) {
        // Do nothing.
    }
}

/// A `FinishedSubprocess(3)` whose `waitFor` throws `InterruptedException` until destroyed.
struct InterruptedSubprocess {
    destroyed: AtomicBool,
}

impl Subprocess for InterruptedSubprocess {
    fn destroy(&self) -> bool {
        self.destroyed.store(true, Ordering::SeqCst);
        true
    }
    fn exit_value(&self) -> i32 {
        3
    }
    fn finished(&self) -> bool {
        true
    }
    fn is_alive(&self) -> bool {
        false
    }
    fn timedout(&self) -> bool {
        false
    }
    fn wait_for(&self) -> Result<(), blaze_shell::InterruptedException> {
        if !self.destroyed.load(Ordering::SeqCst) {
            return Err(blaze_shell::InterruptedException);
        }
        Ok(())
    }
    fn get_output_stream(&self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(io::sink()))
    }
    fn get_input_stream(&self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(io::empty()))
    }
    fn get_error_stream(&self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(io::empty()))
    }
    fn get_process_id(&self) -> i64 {
        0
    }
    fn close(&self) {}
}

enum FactoryBehavior {
    Finished(i32),
    Throw(&'static str),
    Interrupted,
}

/// The mocked `SubprocessFactory` with an `ArgumentCaptor<SubprocessBuilder>`.
struct CapturingFactory {
    behavior: FactoryBehavior,
    captured: Mutex<Vec<SubprocessBuilder>>,
}

impl CapturingFactory {
    fn new(behavior: FactoryBehavior) -> Arc<CapturingFactory> {
        Arc::new(CapturingFactory {
            behavior,
            captured: Mutex::new(Vec::new()),
        })
    }
    fn captured(&self) -> SubprocessBuilder {
        let c = self.captured.lock();
        assert_eq!(c.len(), 1, "factory.create must be called exactly once");
        c[0].clone()
    }
}

impl SubprocessFactory for CapturingFactory {
    fn create(&self, params: &SubprocessBuilder) -> io::Result<Box<dyn Subprocess>> {
        self.captured.lock().push(params.clone());
        match self.behavior {
            FactoryBehavior::Finished(exit_code) => Ok(Box::new(FinishedSubprocess { exit_code })),
            FactoryBehavior::Throw(msg) => Err(io::Error::other(msg)),
            FactoryBehavior::Interrupted => Ok(Box::new(InterruptedSubprocess {
                destroyed: AtomicBool::new(false),
            })),
        }
    }
}

/// `SubprocessInterceptor`: prevents any subprocess execution at all.
struct SubprocessInterceptor;

impl SubprocessFactory for SubprocessInterceptor {
    fn create(&self, _params: &SubprocessBuilder) -> io::Result<Box<dyn Subprocess>> {
        panic!("UnsupportedOperationException")
    }
}

/// `LocalSpawnRunnerTest::keepLocalEnvUnchanged`.
struct KeepLocalEnvUnchanged;

impl LocalEnvProvider for KeepLocalEnvUnchanged {
    fn rewrite_local_env(
        &self,
        env: &IndexMap<String, String>,
        _bin_tools: &BinTools,
        _fallback_tmp_dir: &str,
    ) -> io::Result<IndexMap<String, String>> {
        Ok(env.clone())
    }
}

/// The mocked `LocalEnvProvider` of `checkLocalEnvProviderCalled`.
#[derive(Default)]
struct RecordingLocalEnvProvider {
    fallback_tmp_dirs: Mutex<Vec<String>>,
}

impl LocalEnvProvider for RecordingLocalEnvProvider {
    fn rewrite_local_env(
        &self,
        env: &IndexMap<String, String>,
        _bin_tools: &BinTools,
        fallback_tmp_dir: &str,
    ) -> io::Result<IndexMap<String, String>> {
        self.fallback_tmp_dirs
            .lock()
            .push(fallback_tmp_dir.to_string());
        Ok(env.clone())
    }
}

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

/// `SpawnRunnerTestUtil.SpawnExecutionContextForTesting`.
struct SpawnExecutionContextForTesting {
    reported_status: Mutex<Vec<ProgressStatus>>,
    prefetch_called: AtomicBool,
    lock_output_files_called: AtomicBool,
    spawn: Arc<SimpleSpawn>,
    timeout: Duration,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
}

impl SpawnExecutionContextForTesting {
    fn new(spawn: Arc<SimpleSpawn>, file_out_err: FileOutErr, timeout: Duration) -> Self {
        SpawnExecutionContextForTesting {
            reported_status: Mutex::new(Vec::new()),
            prefetch_called: AtomicBool::new(false),
            lock_output_files_called: AtomicBool::new(false),
            spawn,
            timeout,
            file_out_err,
            client_env: IndexMap::new(),
        }
    }
}

impl SpawnExecutionContext for SpawnExecutionContextForTesting {
    fn get_id(&self) -> i32 {
        0
    }
    fn set_digest(&self, _digest: Digest) {
        // Intentionally empty.
    }
    fn get_digest(&self) -> Option<Digest> {
        None
    }
    fn prefetch_inputs_and_wait(&self) -> Result<(), ExecException> {
        self.prefetch_called.store(true, Ordering::SeqCst);
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
        self.lock_output_files_called.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn speculating(&self) -> bool {
        false
    }
    fn get_timeout(&self) -> blaze_util::java_duration::JavaDuration {
        self.timeout.into()
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        let mut input_mapping = InputMapping::new();
        for action_input in self.spawn.get_input_files().to_list().iter() {
            input_mapping.insert(
                HierarchicalPath(base_directory.get_relative(action_input.get_exec_path())),
                action_input.clone(),
            );
        }
        Arc::new(input_mapping)
    }
    fn report(&self, progress: ProgressStatus) {
        self.reported_status.lock().push(progress);
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

/// `SpawnBuilder`.
struct SpawnBuilder {
    args: Vec<String>,
    environment: IndexMap<String, String>,
    execution_info: IndexMap<String, String>,
    inputs: Vec<ActionInputRef>,
}

impl SpawnBuilder {
    fn new(args: &[&str]) -> SpawnBuilder {
        SpawnBuilder {
            args: args.iter().map(|s| s.to_string()).collect(),
            environment: IndexMap::new(),
            execution_info: IndexMap::new(),
            inputs: Vec::new(),
        }
    }
    fn with_environment(mut self, key: &str, value: &str) -> SpawnBuilder {
        self.environment.insert(key.to_string(), value.to_string());
        self
    }
    fn with_execution_info(mut self, key: &str, value: &str) -> SpawnBuilder {
        self.execution_info
            .insert(key.to_string(), value.to_string());
        self
    }
    fn with_input(mut self, input: ActionInputRef) -> SpawnBuilder {
        self.inputs.push(input);
        self
    }
    fn build(self) -> Arc<SimpleSpawn> {
        let owner = FakeOwner::new(
            "Mnemonic",
            Some("progress message"),
            Some(Label::parse_canonical_unchecked("//dummy:label")),
        );
        let mut inputs = NestedSetBuilder::stable_order();
        inputs.add_all(self.inputs);
        Arc::new(SimpleSpawn::new_simple(
            Arc::new(owner),
            self.args,
            self.environment,
            self.execution_info,
            inputs.build(),
            vec![],
            Arc::new(ResourceSet::zero()),
        ))
    }
}

/// `SIMPLE_SPAWN`.
fn simple_spawn() -> Arc<SimpleSpawn> {
    SpawnBuilder::new(&["/bin/echo", "Hi!"])
        .with_environment("VARIABLE", "value")
        .build()
}

/// `setupEnvironmentForFakeExecution()`: no subprocess execution at all, and a resource manager
/// with 1MB, 1 CPU, 1 test.
fn setup_environment_for_fake_execution() -> (TestFs, Arc<ResourceManager>) {
    SubprocessBuilder::set_default_subprocess_factory(Some(Arc::new(SubprocessInterceptor)));
    let resource_manager = ResourceManager::new();
    resource_manager.set_available_resources(ResourceSet::create(
        /* memory_mb= */ 1.0, /* cpu= */ 1.0, /* local_test_count= */ 1,
    ));
    (TestFs::new(), resource_manager)
}

/// `makeProcessWrapper(options)`.
fn make_process_wrapper(options: &LocalExecutionOptions) -> ProcessWrapper {
    ProcessWrapper::new(
        PathFragment::create("/process-wrapper"),
        ActionInputRef::new(BasicActionInput::from_path("/process-wrapper")),
        Some(options.get_local_sigkill_grace_seconds()),
        /* graceful_sigterm= */ false,
    )
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `TestedLocalSpawnRunner`: a `LocalSpawnRunner` whose `createActionTemp` makes
/// `<execroot>/tmp<hex>_<hex>` and remembers it.
fn tested_local_spawn_runner(
    exec_root: Path,
    options: LocalExecutionOptions,
    resource_manager: Arc<ResourceManager>,
    process_wrapper: Option<ProcessWrapper>,
    local_env_provider: Arc<dyn LocalEnvProvider>,
) -> (LocalSpawnRunner, Arc<Mutex<Option<Path>>>) {
    let tmp_dir_path: Arc<Mutex<Option<Path>>> = Arc::new(Mutex::new(None));
    let tmp_dir_path2 = tmp_dir_path.clone();
    let bin_tools = Arc::new(BinTools::for_unit_testing(&exec_root, vec![]));
    let exec_root_for_updater = exec_root.clone();
    let runner = LocalSpawnRunner::new(
        exec_root,
        Arc::new(options),
        resource_manager,
        local_env_provider,
        bin_tools,
        process_wrapper,
        Arc::new(RunfilesTreeUpdater::new(exec_root_for_updater)),
    )
    .with_action_temp_creator(Arc::new(move |exec_root: &Path| {
        let temp_dir_path = loop {
            let id_str = format!(
                "{:x}_{:x}",
                std::process::id(),
                TMP_COUNTER.fetch_add(1, Ordering::SeqCst) + 0x1234
            );
            let p = exec_root.get_relative_str(&format!("tmp{id_str}"));
            if !p.exists() {
                break p;
            }
        };
        if !temp_dir_path.create_directory()? {
            return Err(io::Error::other(format!(
                "Could not create temp directory '{temp_dir_path}'"
            )));
        }
        *tmp_dir_path2.lock() = Some(temp_dir_path.clone());
        Ok(temp_dir_path)
    }));
    (runner, tmp_dir_path)
}

fn defaults() -> LocalExecutionOptions {
    Options::<LocalExecutionOptions>::get_defaults()
}

fn file_out_err(t: &TestFs) -> FileOutErr {
    FileOutErr::new(t.get_path("/out/stdout"), t.get_path("/out/stderr"))
}

#[test]
fn vanilla_zero_exit() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let mut options = defaults();
    options.local_sigkill_grace_seconds = 456;
    let pw = make_process_wrapper(&options);
    let (runner, action_temp) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = simple_spawn();
    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );
    assert!(t.get_path("/execroot").create_directory().unwrap());
    let result = runner.exec(&*spawn, &context).unwrap();
    let captured = factory.captured();
    assert_eq!(result.status(), Status::Success);
    assert_eq!(result.exit_code(), 0);
    assert!(result.setup_success());
    assert_eq!(
        result.get_executor_host_name(),
        Some(get_cached_short_host_name().as_str())
    );

    let action_temp = action_temp.lock().clone().unwrap();
    assert_eq!(
        captured.get_argv(),
        &[
            "/process-wrapper".to_string(),
            "--timeout=123".to_string(),
            "--kill_delay=456".to_string(),
            format!("--stats={}", action_temp.get_relative_str("stats.out")),
            "/bin/echo".to_string(),
            "Hi!".to_string(),
        ]
    );
    let mut expected_env = IndexMap::new();
    expected_env.insert("VARIABLE".to_string(), "value".to_string());
    assert_eq!(captured.get_env(), Some(&expected_env));
    assert_eq!(captured.get_timeout_millis(), 0);
    assert_eq!(
        captured.get_stdout(),
        SubprocessBuilderStreamAction::Redirect
    );
    assert_eq!(
        captured.get_stdout_file(),
        Some(&JavaFile::new(&t.s("/out/stdout")))
    );
    assert_eq!(
        captured.get_stderr(),
        SubprocessBuilderStreamAction::Redirect
    );
    assert_eq!(
        captured.get_stderr_file(),
        Some(&JavaFile::new(&t.s("/out/stderr")))
    );

    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
    assert_eq!(
        *context.reported_status.lock(),
        vec![
            ProgressStatus::Scheduling("local".to_string()),
            ProgressStatus::Executing("local".to_string()),
        ]
    );
}

#[test]
#[ignore = "needs CommandLines.ParamFileActionInput (blaze_actions)"]
fn test_param_files() {}

#[test]
fn exec_materializes_virtual_input_as_executable() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();
    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));
    let exec_root = t.get_path("/execroot");
    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        exec_root.clone(),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );
    let virtual_input = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("input1"),
        b"hello".to_vec(),
    ));
    let spawn = SpawnBuilder::new(&["/bin/true"])
        .with_input(virtual_input.clone())
        .build();
    let context =
        SpawnExecutionContextForTesting::new(spawn.clone(), file_out_err(&t), Duration::ZERO);

    let result = runner.exec(&*spawn, &context).unwrap();

    assert_eq!(result.status(), Status::Success);
    // DirectoryListingHelper.leafDirectoryEntries(execRoot): containsExactly(file("input1")).
    let entries: Vec<String> = exec_root
        .get_directory_entries()
        .unwrap()
        .iter()
        .map(|p| p.get_base_name().to_string())
        .collect();
    assert_eq!(entries, vec!["input1".to_string()]);
    let input_path = exec_root.get_relative(virtual_input.get_exec_path());
    assert!(input_path.is_file());
    assert!(input_path.is_executable().unwrap());
    let contents = std::fs::read_to_string(input_path.get_path_string()).unwrap();
    assert_eq!(contents.lines().collect::<Vec<_>>(), vec!["hello"]);
}

#[test]
fn no_process_wrapper() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let mut options = defaults();
    options.local_sigkill_grace_seconds = 456;
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        /* process_wrapper= */ None,
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = simple_spawn();
    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );
    assert!(t.get_path("/execroot").create_directory().unwrap());
    let result = runner.exec(&*spawn, &context).unwrap();
    let captured = factory.captured();
    assert_eq!(result.status(), Status::Success);
    assert_eq!(result.exit_code(), 0);
    assert!(result.setup_success());
    assert_eq!(
        result.get_executor_host_name(),
        Some(get_cached_short_host_name().as_str())
    );

    assert_eq!(
        captured.get_argv(),
        &["/bin/echo".to_string(), "Hi!".to_string()]
    );
    let mut expected_env = IndexMap::new();
    expected_env.insert("VARIABLE".to_string(), "value".to_string());
    assert_eq!(captured.get_env(), Some(&expected_env));
    // Without the process wrapper, we use the Command API to enforce the timeout.
    assert_eq!(captured.get_timeout_millis(), 123 * 1000);

    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
}

#[test]
fn non_zero_exit() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(3));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, action_temp) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    assert!(t.get_path("/execroot").create_directory().unwrap());
    let spawn = simple_spawn();
    let context =
        SpawnExecutionContextForTesting::new(spawn.clone(), file_out_err(&t), Duration::ZERO);
    let result = runner.exec(&*spawn, &context).unwrap();
    let captured = factory.captured();
    assert_eq!(result.status(), Status::NonZeroExit);
    assert_eq!(result.exit_code(), 3);
    assert!(result.setup_success());
    assert_eq!(
        result.get_executor_host_name(),
        Some(get_cached_short_host_name().as_str())
    );

    let action_temp = action_temp.lock().clone().unwrap();
    assert_eq!(
        captured.get_argv(),
        &[
            "/process-wrapper".to_string(),
            "--timeout=0".to_string(),
            "--kill_delay=15".to_string(),
            format!("--stats={}", action_temp.get_relative_str("stats.out")),
            "/bin/echo".to_string(),
            "Hi!".to_string(),
        ]
    );
    let mut expected_env = IndexMap::new();
    expected_env.insert("VARIABLE".to_string(), "value".to_string());
    assert_eq!(captured.get_env(), Some(&expected_env));
    assert_eq!(
        captured.get_stdout(),
        SubprocessBuilderStreamAction::Redirect
    );
    assert_eq!(
        captured.get_stdout_file(),
        Some(&JavaFile::new(&t.s("/out/stdout")))
    );
    assert_eq!(
        captured.get_stderr(),
        SubprocessBuilderStreamAction::Redirect
    );
    assert_eq!(
        captured.get_stderr_file(),
        Some(&JavaFile::new(&t.s("/out/stderr")))
    );

    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
}

#[test]
fn process_startup_throws() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Throw("I'm sorry, Dave"));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    assert!(t.get_path("/out").create_directory().unwrap());
    assert!(t.get_path("/execroot").create_directory().unwrap());
    let spawn = simple_spawn();
    let context =
        SpawnExecutionContextForTesting::new(spawn.clone(), file_out_err(&t), Duration::ZERO);
    let result = runner.exec(&*spawn, &context).unwrap();
    let _ = factory.captured();
    assert_eq!(result.status(), Status::ExecutionFailed);
    assert_eq!(result.exit_code(), -1);
    assert!(!result.setup_success());
    assert_eq!(result.get_wall_time_in_ms(), 0);
    assert_eq!(result.get_user_time_in_ms(), 0);
    assert_eq!(result.get_system_time_in_ms(), 0);
    assert_eq!(
        result.get_executor_host_name(),
        Some(get_cached_short_host_name().as_str())
    );

    assert_eq!(
        std::fs::read_to_string(t.s("/out/stderr")).unwrap(),
        "Action failed to execute: java.io.IOException: I'm sorry, Dave\n"
    );

    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
}

#[test]
fn disallow_local_execution() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let mut options = defaults();
    options.allowed_local_action = Some(
        RegexPatternConverter
            .convert(Some("none"), None)
            .unwrap()
            .unwrap()
            .get(),
    );
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    assert!(t.get_path("/execroot").create_directory().unwrap());
    // new FileOutErr(): a temporary FileOutErr.
    assert!(t.get_path("/out").create_directory().unwrap());
    let spawn = simple_spawn();
    let context =
        SpawnExecutionContextForTesting::new(spawn.clone(), file_out_err(&t), Duration::ZERO);
    let reply = runner.exec(&*spawn, &context).unwrap();
    assert_eq!(reply.status(), Status::ExecutionDenied);
    assert_eq!(reply.exit_code(), -1);
    assert!(!reply.setup_success());
    assert_eq!(reply.get_wall_time_in_ms(), 0);
    assert_eq!(reply.get_user_time_in_ms(), 0);
    assert_eq!(reply.get_system_time_in_ms(), 0);
    assert_eq!(
        reply.get_executor_host_name(),
        Some(get_cached_short_host_name().as_str())
    );

    // TODO(ulfjack): Maybe we should only lock after checking?
    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
}

#[test]
fn interrupted_exception() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Interrupted);
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = simple_spawn();
    let context =
        SpawnExecutionContextForTesting::new(spawn.clone(), file_out_err(&t), Duration::ZERO);
    assert!(t.get_path("/execroot").create_directory().unwrap());
    let result = runner.exec(&*spawn, &context);
    assert!(matches!(result, Err(ExecException::Interrupted)));
    blaze_shell::thread_interrupt::interrupted();
    assert!(context.lock_output_files_called.load(Ordering::SeqCst));
}

#[test]
fn interrupt_waits_for_process_exit() {
    let _lock = FACTORY_LOCK.lock();
    // setupEnvironmentForRealExecution()
    SubprocessBuilder::set_default_subprocess_factory(Some(JavaSubprocessFactory::instance()));
    let resource_manager = ResourceManager::new();
    resource_manager.set_available_resources(get_local_host_capacity());

    let t = TestFs::new();
    let temp_dir = t.get_path("");

    let runner = Arc::new(LocalSpawnRunner::new(
        temp_dir.clone(),
        Arc::new(defaults()),
        resource_manager,
        Arc::new(local_env_provider::for_current_os(&IndexMap::new())),
        Arc::new(BinTools::for_unit_testing(&temp_dir, vec![])),
        /* process_wrapper= */ None,
        Arc::new(RunfilesTreeUpdater::new(temp_dir.clone())),
    ));
    let file_out_err = FileOutErr::new(
        temp_dir.get_relative_str("stdout"),
        temp_dir.get_relative_str("stderr"),
    );

    // This test to exercise a race condition by attempting an operation multiple times. We can
    // get false positives (the test passing without us catching a problem), so try a few times.
    let tries = 10;
    let delay_seconds = 1;

    let content = temp_dir.get_child("content");
    let started = temp_dir.get_child("started");
    // Start a subprocess that blocks until it is killed, and when it is, writes some output to a
    // temporary file after some delay.
    let script = format!(
        "trap 'sleep {delay_seconds}; echo foo >{}; exit 1' TERM; touch {}; while :; do   echo 'waiting to be killed';   sleep 1; done",
        content.get_path_string(),
        started.get_path_string()
    );
    let spawn = SpawnBuilder::new(&["/bin/sh", "-c", &script]).build();

    let context = Arc::new(SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err,
        Duration::ZERO,
    ));

    for _ in 0..tries {
        let _ = content.delete();
        let _ = started.delete();
        let (tx, rx) = std::sync::mpsc::channel();
        let runner2 = runner.clone();
        let spawn2 = spawn.clone();
        let context2 = context.clone();
        let handle = std::thread::spawn(move || {
            tx.send(blaze_shell::thread_interrupt::ThreadInterrupt::current())
                .unwrap();
            let r = runner2.exec(&*spawn2, &*context2);
            matches!(r, Err(ExecException::Interrupted))
        });
        let interrupt = rx.recv().unwrap();
        // Wait until we know the subprocess has started so that delivering a termination signal
        // to it triggers the delayed write to the file.
        while !started.exists() {
            std::thread::sleep(Duration::from_millis(1));
        }
        interrupt.interrupt();
        let interrupt_caught = handle.join().unwrap();
        assert!(interrupt_caught);
        // At this point, the subprocess must have fully stopped so write some content to the
        // file and expect that these contents remain unmodified.
        std::fs::write(content.get_path_string(), "bar").unwrap();
        // Wait for longer than the spawn takes to exit before we check the file contents to
        // ensure that we properly awaited for termination of the subprocess.
        std::thread::sleep(Duration::from_secs(delay_seconds * 2));
        assert_eq!(
            std::fs::read_to_string(content.get_path_string()).unwrap(),
            "bar"
        );
    }
}

#[test]
fn check_prefetch_called() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = simple_spawn();
    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );
    assert!(t.get_path("/execroot").create_directory().unwrap());
    runner.exec(&*spawn, &context).unwrap();
    assert!(context.prefetch_called.load(Ordering::SeqCst));
}

#[test]
fn check_no_prefetch_called() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = SpawnBuilder::new(&["/bin/echo", "Hi!"])
        .with_execution_info(execution_requirements::DISABLE_LOCAL_PREFETCH, "")
        .build();

    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );

    assert!(t.get_path("/execroot").create_directory().unwrap());
    runner.exec(&*spawn, &context).unwrap();
    assert!(!context.prefetch_called.load(Ordering::SeqCst));
}

#[test]
fn check_local_env_provider_called() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));
    let local_env_provider = Arc::new(RecordingLocalEnvProvider::default());

    let options = defaults();
    let pw = make_process_wrapper(&options);
    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        options,
        resource_manager,
        Some(pw),
        local_env_provider.clone(),
    );

    let spawn = simple_spawn();
    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );
    assert!(t.get_path("/execroot").create_directory().unwrap());

    runner.exec(&*spawn, &context).unwrap();
    let dirs = local_env_provider.fallback_tmp_dirs.lock();
    assert_eq!(dirs.len(), 1);
    let pattern = format!(
        "^{}/execroot/tmp[0-9a-fA-F]+_[0-9a-fA-F]+/work$",
        regex::escape(&t.root)
    );
    assert!(
        Regex::new(&pattern).unwrap().is_match(&dirs[0]),
        "{} does not match {pattern}",
        dirs[0]
    );
}

#[test]
#[ignore = "needs the process-wrapper and spend_cpu_time test binaries in runfiles"]
fn has_execution_statistics() {}

/// Check that relative paths in the Spawn are absolutized relative to the execroot passed to the
/// LocalSpawnRunner.
#[test]
fn relative_path() {
    let _lock = FACTORY_LOCK.lock();
    let (t, resource_manager) = setup_environment_for_fake_execution();

    let factory = CapturingFactory::new(FactoryBehavior::Finished(0));
    SubprocessBuilder::set_default_subprocess_factory(Some(factory.clone()));

    let (runner, _) = tested_local_spawn_runner(
        t.get_path("/execroot"),
        defaults(),
        resource_manager,
        /* process_wrapper= */ None,
        Arc::new(KeepLocalEnvUnchanged),
    );

    let spawn = SpawnBuilder::new(&["foo/bar", "Hi!"]).build();
    let context = SpawnExecutionContextForTesting::new(
        spawn.clone(),
        file_out_err(&t),
        Duration::from_secs(123),
    );
    assert!(t.get_path("/execroot").create_directory().unwrap());
    runner.exec(&*spawn, &context).unwrap();
    let captured = factory.captured();

    assert_eq!(
        captured.get_argv(),
        &[t.s("/execroot/foo/bar"), "Hi!".to_string()]
    );
}

/// Port of `PosixLocalEnvProviderTest.testRewriteEnvWithClientTmpdir`: should use the client
/// environment's TMPDIR envvar if specified.
#[test]
fn test_rewrite_env_with_client_tmpdir() {
    let mut client_env = IndexMap::new();
    client_env.insert("TMPDIR".to_string(), "client-env/tmp".to_string());
    let p = local_env_provider::PosixLocalEnvProvider::new(client_env);
    let t = TestFs::new();
    let bin_tools = BinTools::for_unit_testing(&t.get_path("/execroot"), vec![]);
    let env = |pairs: &[(&str, &str)]| -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        p.rewrite_local_env(&env(&[("key1", "value1")]), &bin_tools, "")
            .unwrap(),
        env(&[("key1", "value1"), ("TMPDIR", "client-env/tmp")])
    );
    assert_eq!(
        p.rewrite_local_env(
            &env(&[("key1", "value1"), ("TMPDIR", "ignored")]),
            &bin_tools,
            ""
        )
        .unwrap(),
        env(&[("key1", "value1"), ("TMPDIR", "client-env/tmp")])
    );
}

/// Port of `PosixLocalEnvProviderTest.testRewriteEnvWithDefaultTmpdir`: should use the default
/// temp dir when the client env doesn't define TMPDIR.
#[test]
fn test_rewrite_env_with_default_tmpdir() {
    let p = local_env_provider::PosixLocalEnvProvider::new(IndexMap::new());
    let t = TestFs::new();
    let bin_tools = BinTools::for_unit_testing(&t.get_path("/execroot"), vec![]);
    let env = |pairs: &[(&str, &str)]| -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        p.rewrite_local_env(&env(&[("key1", "value1")]), &bin_tools, "")
            .unwrap(),
        env(&[("key1", "value1"), ("TMPDIR", "/tmp")])
    );
    assert_eq!(
        p.rewrite_local_env(
            &env(&[("key1", "value1"), ("TMPDIR", "ignored")]),
            &bin_tools,
            ""
        )
        .unwrap(),
        env(&[("key1", "value1"), ("TMPDIR", "/tmp")])
    );
}
