//! Port of `ProcessWrapperSandboxedSpawnRunnerTest.java` (with `SpawnRunnerTestUtil`).
//!
//! PORT: Java copies the `process-wrapper` built from source into the exec root; these tests use
//! the oracle's embedded `process-wrapper` (`~/.cache/blaze/oracle-out/ur/install/<key>`) and skip
//! (pass with a message) when it is missing.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler};
use blaze_exec::actions::resource_manager::{ResourceManager, get_local_host_capacity};
use blaze_exec::actions::{
    ActionInputRef, ExecException, FakeOwner, FileArtifactValue, FilesetOutputTree,
    HierarchicalPath, InputMapping, InputMetadataProvider, ResourceSet, RunfilesArtifactValue,
    SimpleSpawn, Spawn, Status, TreeArtifactValue,
};
use blaze_exec::exec::bin_tools::BinTools;
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::sandbox::process_wrapper_sandboxed_spawn_runner::ProcessWrapperSandboxedSpawnRunner;
use blaze_exec::util::FileOutErr;
use blaze_optdefs::command::execution_options::ExecutionOptions;
use blaze_optdefs::misc::local_execution_options::LocalExecutionOptions;
use blaze_optdefs::misc::sandbox_options::SandboxOptions;
use blaze_options::Options;
use blaze_protos::tools::protos::Digest;
use blaze_vfs::{
    DigestHashFunction, FileSystemExt, FileSystemRef, Path, PathFragment, UnixFileSystem,
};
use indexmap::IndexMap;

const ORACLE_INSTALL_BASE: &str =
    ".cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9";

#[derive(Debug)]
struct NullReporter;

impl EventHandler for NullReporter {
    fn handle(&self, _event: &Event) {}
}

/// `mock(InputMetadataProvider.class)`: knows nothing.
struct NullInputMetadataProvider;

impl InputMetadataProvider for NullInputMetadataProvider {
    fn get_input_metadata(
        &self,
        _input: &ActionInputRef,
    ) -> std::io::Result<Option<FileArtifactValue>> {
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
    inputs: Vec<ActionInputRef>,
    timeout: Duration,
    file_out_err: FileOutErr,
    client_env: IndexMap<String, String>,
    input_metadata_provider: NullInputMetadataProvider,
}

impl SpawnExecutionContextForTesting {
    fn new(spawn: &dyn Spawn, file_out_err: FileOutErr, timeout: Duration) -> Self {
        SpawnExecutionContextForTesting {
            reported_status: Mutex::new(Vec::new()),
            inputs: spawn.get_input_files().to_list().iter().cloned().collect(),
            timeout,
            file_out_err,
            client_env: IndexMap::new(),
            input_metadata_provider: NullInputMetadataProvider,
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
        Ok(())
    }
    fn get_input_metadata_provider(&self) -> &dyn InputMetadataProvider {
        &self.input_metadata_provider
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
    fn get_timeout(&self) -> Duration {
        self.timeout
    }
    fn get_file_out_err(&self) -> &FileOutErr {
        &self.file_out_err
    }
    fn get_input_mapping(
        &self,
        base_directory: &PathFragment,
        _will_access_repeatedly: bool,
    ) -> Arc<InputMapping> {
        let mut input_mapping: InputMapping = std::collections::BTreeMap::new();
        for action_input in &self.inputs {
            input_mapping.insert(
                HierarchicalPath(base_directory.get_relative(action_input.get_exec_path())),
                action_input.clone(),
            );
        }
        Arc::new(input_mapping)
    }
    fn report(&self, progress: ProgressStatus) {
        self.reported_status.lock().unwrap().push(progress);
    }
    fn get_client_env(&self) -> &IndexMap<String, String> {
        &self.client_env
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    test_root: Path,
    env: ExecCommandEnvironment,
}

/// `runtimeWrapper.newCommand()` + `setAvailableResources(LocalHostCapacity...)`; `None` when the
/// oracle's process-wrapper is not available.
fn create_command_environment(sandbox_flags: &[&str]) -> Option<Fixture> {
    let home = std::env::var("HOME").ok()?;
    let install_base = std::path::Path::new(&home).join(ORACLE_INSTALL_BASE);
    if !install_base.join("process-wrapper").exists() {
        eprintln!("skipped: {} has no process-wrapper", install_base.display());
        return None;
    }
    let tmp = tempfile::tempdir().unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let test_root = fs.get_path(tmp.path().to_str().unwrap());
    let output_base = test_root.get_relative_str("output_base");
    let exec_root = output_base.get_relative_str("execroot/_main");
    exec_root
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    let bin_tools = BinTools::for_embedded_bin(
        fs.get_path(install_base.to_str().unwrap()),
        vec!["linux-sandbox".to_string(), "process-wrapper".to_string()],
    );
    let resource_manager = ResourceManager::new();
    resource_manager.set_available_resources(get_local_host_capacity());
    let sandbox_options: SandboxOptions = Options::<SandboxOptions>::parse(sandbox_flags)
        .unwrap()
        .get_options()
        .clone();
    let env = ExecCommandEnvironment {
        file_system: fs.clone(),
        exec_root,
        output_base,
        client_env: IndexMap::new(),
        bin_tools: Arc::new(bin_tools),
        sandbox_options: Arc::new(sandbox_options),
        execution_options: Arc::new(Options::<ExecutionOptions>::get_defaults()),
        local_execution_options: Arc::new(Options::<LocalExecutionOptions>::get_defaults()),
        remote_options: Arc::new(Options::get_defaults()),
        dynamic_execution_options: Arc::new(Options::get_defaults()),
        worker_options: Arc::new(Options::get_defaults()),
        resource_manager,
        reporter: Arc::new(NullReporter),
        package_path_entries: Vec::new(),
        repo_contents_cache_path: None,
        test_arguments: Vec::new(),
        jobs: None,
        command_id: String::new(),
    };
    Some(Fixture {
        _tmp: tmp,
        test_root,
        env,
    })
}

/// `new SpawnBuilder(args...).build()`.
fn spawn(args: &[&str]) -> SimpleSpawn {
    SimpleSpawn::new_simple(
        Arc::new(FakeOwner::new("Mnemonic", Some("progress message"), None)),
        args.iter().map(|a| a.to_string()).collect(),
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        Arc::new(ResourceSet::zero()),
    )
}

#[test]
fn process_wrapper_sandboxed_spawn_runner_can_run_echo() {
    let Some(f) = create_command_environment(&[]) else {
        return;
    };
    // TODO(b/62588075) Currently no process-wrapper support in windows.

    let exec_root = &f.env.exec_root;
    exec_root.create_directory().unwrap();

    // PORT: SpawnRunnerTestUtil.copyProcessWrapperIntoPath is replaced by the BinTools of the
    // fixture, which point at the oracle's install base.

    let sandbox_base = exec_root.get_relative_str("sandbox");
    sandbox_base.create_directory().unwrap();

    let policy_timeout = Duration::from_secs(60);

    let runner = ProcessWrapperSandboxedSpawnRunner::new(
        &f.env,
        sandbox_base,
        Arc::new(SynchronousTreeDeleter),
    );

    let spawn = spawn(&["echo", "cooee"]);

    let file_out_err = FileOutErr::new(
        f.test_root.get_child("stdout"),
        f.test_root.get_child("stderr"),
    );
    let policy = SpawnExecutionContextForTesting::new(&spawn, file_out_err, policy_timeout);

    let spawn_result = runner.exec(&spawn, &policy).unwrap();

    assert_eq!(spawn_result.status(), Status::Success);
    assert_eq!(spawn_result.exit_code(), 0);
    assert!(spawn_result.setup_success());
    assert_eq!(
        *policy.reported_status.lock().unwrap(),
        vec![
            ProgressStatus::Scheduling("processwrapper-sandbox".to_string()),
            ProgressStatus::Executing("processwrapper-sandbox".to_string()),
        ]
    );
}

#[test]
#[ignore = "needs the spend_cpu_time test tool (SpawnRunnerTestUtil.copyCpuTimeSpenderIntoPath)"]
fn has_execution_statistics_when_option_is_enabled() {}
