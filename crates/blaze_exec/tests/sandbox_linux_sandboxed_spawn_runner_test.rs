//! Port of `LinuxSandboxedSpawnRunnerTest.java` (with `SpawnRunnerTestUtil`).
//!
//! PORT: Java copies the `linux-sandbox` built from source into the exec root; these tests use
//! the oracle's embedded `linux-sandbox` (`~/.cache/blaze/oracle-out/ur/install/<key>`) and skip
//! (pass with a message) when it is missing or cannot run here, as Java's `assume()` would.

use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler};
use blaze_exec::actions::resource_manager::{ResourceManager, get_local_host_capacity};
use blaze_exec::actions::{
    ActionInputRef, BasicActionInput, BasicVirtualActionInput, ExecException, FakeOwner,
    FileArtifactValue, FilesetOutputTree, HierarchicalPath, InputMapping, InputMetadataProvider,
    ResourceSet, RunfilesArtifactValue, SimpleSpawn, Spawn, SpawnResult, Status, TreeArtifactValue,
};
use blaze_exec::exec::bin_tools::{BinTools, PathActionInput};
use blaze_exec::exec::exec_command_environment::ExecCommandEnvironment;
use blaze_exec::exec::spawn_runner::{ProgressStatus, SpawnExecutionContext, SpawnRunner};
use blaze_exec::sandbox::SynchronousTreeDeleter;
use blaze_exec::sandbox::linux_sandboxed_spawn_runner::LinuxSandboxedSpawnRunner;
use blaze_exec::sandbox::linux_sandboxed_strategy::LinuxSandboxedStrategy;
use blaze_exec::sandbox::symlinked_sandboxed_spawn::SymlinkedSandboxedSpawn;
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
/// oracle's linux-sandbox is not available.
fn create_command_environment(sandbox_flags: &[&str]) -> Option<Fixture> {
    let home = std::env::var("HOME").ok()?;
    let install_base = std::path::Path::new(&home).join(ORACLE_INSTALL_BASE);
    if !install_base.join("linux-sandbox").exists() {
        eprintln!("skipped: {} has no linux-sandbox", install_base.display());
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
    };
    Some(Fixture {
        _tmp: tmp,
        test_root,
        env,
    })
}

// Java: LinuxSandboxedSpawnRunnerTest.setupSandboxAndCreateRunner
fn setup_sandbox_and_create_runner(env: &ExecCommandEnvironment) -> LinuxSandboxedSpawnRunner {
    let exec_root = &env.exec_root;
    exec_root.create_directory().unwrap();

    // PORT: SpawnRunnerTestUtil.copyLinuxSandboxIntoPath is replaced by the BinTools of the
    // fixture, which point at the oracle's install base.

    let sandbox_base = exec_root.get_relative_str("sandbox");
    sandbox_base.create_directory().unwrap();

    let mut sandbox_options = Options::<SandboxOptions>::get_defaults();
    sandbox_options.sandbox_block_path = Vec::new();
    LinuxSandboxedStrategy::create(
        env,
        &sandbox_base,
        /* timeout_kill_delay= */ Duration::from_secs(2),
        Arc::new(SynchronousTreeDeleter),
        &sandbox_options,
    )
}

/// `new SpawnBuilder(args...)` with inputs and outputs.
fn spawn(args: &[&str], inputs: Vec<ActionInputRef>, outputs: Vec<ActionInputRef>) -> SimpleSpawn {
    SimpleSpawn::new_simple(
        Arc::new(FakeOwner::new("Mnemonic", Some("progress message"), None)),
        args.iter().map(|a| a.to_string()).collect(),
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::wrap(Order::STABLE_ORDER, inputs),
        outputs,
        Arc::new(ResourceSet::zero()),
    )
}

fn create_spawn_execution_context(
    f: &Fixture,
    spawn: &dyn Spawn,
    stdout: Option<Path>,
) -> SpawnExecutionContextForTesting {
    let stdout = stdout.unwrap_or_else(|| f.test_root.get_child("stdout"));
    let file_out_err = FileOutErr::new(stdout, f.test_root.get_child("stderr"));
    SpawnExecutionContextForTesting::new(spawn, file_out_err, Duration::from_secs(60))
}

fn read_lines(p: &Path) -> Vec<String> {
    let mut s = String::new();
    p.get_input_stream()
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    s.lines().map(str::to_string).collect()
}

fn exec_or_skip(
    runner: &LinuxSandboxedSpawnRunner,
    f: &Fixture,
    spawn: &dyn Spawn,
    context: &SpawnExecutionContextForTesting,
) -> Option<SpawnResult> {
    if !LinuxSandboxedSpawnRunner::is_supported(&f.env) {
        eprintln!("skipped: linux-sandbox is not supported on this machine");
        return None;
    }
    Some(runner.exec(spawn, context).unwrap())
}

#[test]
fn exec_echo_command_executes_successfully() {
    let Some(f) = create_command_environment(&[]) else {
        return;
    };
    let runner = setup_sandbox_and_create_runner(&f.env);
    let spawn = spawn(&["echo", "echolalia"], Vec::new(), Vec::new());
    let stdout = f.test_root.get_child("stdout");
    let policy = create_spawn_execution_context(&f, &spawn, Some(stdout.clone()));

    let Some(spawn_result) = exec_or_skip(&runner, &f, &spawn, &policy) else {
        return;
    };

    assert_eq!(spawn_result.status(), Status::Success);
    assert_eq!(spawn_result.exit_code(), 0);
    assert!(spawn_result.setup_success());
    assert!(spawn_result.get_wall_time_in_ms() > 0);
    assert_eq!(read_lines(&stdout), vec!["echolalia"]);
    // PORT: not in the Java test; checks the progress events the runner reports.
    assert_eq!(
        *policy.reported_status.lock().unwrap(),
        vec![
            ProgressStatus::Scheduling("linux-sandbox".to_string()),
            ProgressStatus::Executing("linux-sandbox".to_string()),
        ]
    );
}

#[test]
fn exec_command_with_param_files_executes_successfully() {
    let Some(f) = create_command_environment(&[]) else {
        return;
    };
    let runner = setup_sandbox_and_create_runner(&f.env);
    // PORT: ParamFileActionInput (UNQUOTED: one argument per line) lives in blaze_actions; a
    // BasicVirtualActionInput with the same content stands in.
    let param_file = ActionInputRef::new(BasicVirtualActionInput::new(
        PathFragment::create("params/param-file"),
        b"--foo\n--bar\n".to_vec(),
    ));
    let spawn = spawn(
        &["cp", "params/param-file", "out"],
        vec![param_file],
        vec![ActionInputRef::new(BasicActionInput::from_path("out"))],
    );
    let policy = create_spawn_execution_context(&f, &spawn, None);

    let Some(spawn_result) = exec_or_skip(&runner, &f, &spawn, &policy) else {
        return;
    };

    assert_eq!(spawn_result.status(), Status::Success);
    let param_file = f.env.exec_root.get_relative_str("out");
    assert!(param_file.exists());
    assert_eq!(read_lines(&param_file), vec!["--foo", "--bar"]);
}

#[test]
fn exec_spawn_running_bin_tool_executes_successfully() {
    let Some(f) = create_command_environment(&[]) else {
        return;
    };
    let runner = setup_sandbox_and_create_runner(&f.env);
    let tool_path = f.test_root.get_relative_str("execRoot/tool");
    tool_path
        .get_parent_directory()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();
    {
        let mut out = tool_path.get_output_stream().unwrap();
        std::io::Write::write_all(&mut out, b"#!/bin/bash\necho hello > $1\n").unwrap();
    }
    let path_action_input = ActionInputRef::new(PathActionInput::new(
        tool_path,
        PathFragment::create("_bin/tool"),
    ));
    let output = ActionInputRef::new(BasicActionInput::from_path("blaze-out/output"));
    let spawn = spawn(
        &["_bin/tool", output.get_exec_path_string().as_str()],
        vec![path_action_input],
        vec![output.clone()],
    );
    let policy = create_spawn_execution_context(&f, &spawn, None);

    let Some(spawn_result) = exec_or_skip(&runner, &f, &spawn, &policy) else {
        return;
    };

    assert_eq!(spawn_result.status(), Status::Success);
    assert_eq!(
        read_lines(&f.env.exec_root.get_relative(output.get_exec_path())),
        vec!["hello"]
    );
}

#[test]
#[ignore = "needs the spend_cpu_time test tool (SpawnRunnerTestUtil.copyCpuTimeSpenderIntoPath)"]
fn exec_collects_execution_statistics() {}

#[test]
fn hermetic_tmp_tmp_created_and_mounted() {
    let Some(f) = create_command_environment(&[]) else {
        return;
    };
    let runner = setup_sandbox_and_create_runner(&f.env);
    let spawn = spawn(&[], Vec::new(), Vec::new());
    let sandboxed_spawn =
        match runner.prepare_spawn(&spawn, &create_spawn_execution_context(&f, &spawn, None)) {
            Ok(s) => s,
            Err(e) => panic!("prepareSpawn failed: {:?}", e),
        };

    let sandbox_path = sandboxed_spawn
        .get_sandbox_exec_root()
        .get_parent_directory()
        .unwrap()
        .get_parent_directory()
        .unwrap();
    let hermetic_tmp_path = sandbox_path.get_relative_str("_hermetic_tmp");
    assert!(hermetic_tmp_path.is_directory());

    assert!(
        sandboxed_spawn
            .as_any()
            .downcast_ref::<SymlinkedSandboxedSpawn>()
            .is_some()
    );
    let args = sandboxed_spawn.get_arguments().join(" ");
    assert!(args.contains("-w /tmp"), "{}", args);
    assert!(
        args.contains(&format!("-M {} -m /tmp", hermetic_tmp_path)),
        "{}",
        args
    );
}

#[test]
fn hermetic_tmp_sandbox_tmpfs_on_tmp_tmp_not_created_or_mounted() {
    let Some(f) = create_command_environment(&["--sandbox_tmpfs_path=/tmp"]) else {
        return;
    };
    let runner = setup_sandbox_and_create_runner(&f.env);
    let spawn = spawn(&[], Vec::new(), Vec::new());
    let sandboxed_spawn =
        match runner.prepare_spawn(&spawn, &create_spawn_execution_context(&f, &spawn, None)) {
            Ok(s) => s,
            Err(e) => panic!("prepareSpawn failed: {:?}", e),
        };

    let sandbox_path = sandboxed_spawn
        .get_sandbox_exec_root()
        .get_parent_directory()
        .unwrap()
        .get_parent_directory()
        .unwrap();
    let hermetic_tmp_path = sandbox_path.get_relative_str("_hermetic_tmp");
    assert!(!hermetic_tmp_path.is_directory());

    assert!(
        sandboxed_spawn
            .as_any()
            .downcast_ref::<SymlinkedSandboxedSpawn>()
            .is_some()
    );
    let args = sandboxed_spawn.get_arguments().join(" ");
    assert!(args.contains("-w /tmp"), "{}", args);
    assert!(args.contains("-e /tmp"), "{}", args);
    assert!(!args.contains("-m /tmp"), "{}", args);
}
