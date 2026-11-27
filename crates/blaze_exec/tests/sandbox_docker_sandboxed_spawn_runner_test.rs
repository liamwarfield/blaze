//! Runs `DockerSandboxedSpawnRunner` without a Docker daemon: a fake `docker` executable records
//! its argv (and the Dockerfile `docker build` reads from stdin) and runs the `docker run` command
//! itself in the mounted sandbox exec root.
//!
//! PORT: Java has no unit test for `DockerSandboxedSpawnRunner` (only the integration test
//! `bazel_docker_sandboxing_test.sh`, which needs a daemon). The expected argv is the one the
//! oracle (Bazel 9.3.2) passes to the same fake with `--spawn_strategy=docker
//! --experimental_docker_image=fake:img` (recorded by
//! `~/.cache/blaze/work/exec-cluster/docker_probe.sh`): `docker info`, then `docker build
//! --build-arg image=... ... -q -` with the Dockerfile on stdin, then `docker run --rm
//! --network=host|none -e K=V... -v <sandbox>/execroot/_main:/execroot/_main -w /execroot/_main
//! -u uid:gid -l command_id=<id> --name <uuid> <image> <args>`.
//! The tests use the oracle's embedded `process-wrapper` and skip (pass with a message) when it is
//! missing.

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
use blaze_exec::sandbox::docker_sandboxed_spawn_runner::DockerSandboxedSpawnRunner;
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

/// `new SpawnBuilder(args...).withExecutionInfo(...).build()`.
fn spawn(args: &[&str], execution_info: &[(&str, &str)]) -> SimpleSpawn {
    SimpleSpawn::new_simple(
        Arc::new(FakeOwner::new("Mnemonic", Some("progress message"), None)),
        args.iter().map(|a| a.to_string()).collect(),
        IndexMap::new(),
        execution_info
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        Arc::new(ResourceSet::zero()),
    )
}

/// The fake `docker` of `docker_probe.sh`: records every call in `log`, answers `info`, `rm` and
/// `build` (printing an image id), and runs `docker run` itself: `cd` to the source of the mount
/// whose target is the `-w` directory, then `env -i <the -e variables> <command>`.
fn write_fake_docker(dir: &std::path::Path, log: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let docker = dir.join("docker");
    let script = format!(
        r#"#!/bin/bash
LOG='{log}'
{{ printf "%s\n" "== docker"; for a in "$@"; do printf "%s\n" "$a"; done; }} >> "$LOG"
case "$1" in
  info) exit 0 ;;
  rm) exit 0 ;;
  build) {{ echo "-- stdin"; cat; }} >> "$LOG"; echo "sha256:fakeimage"; exit 0 ;;
  run)
    shift
    envs=(); mounts=(); wd=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --rm|--privileged|--network=*) shift ;;
        -e) envs+=("$2"); shift 2 ;;
        -v) mounts+=("$2"); shift 2 ;;
        -w) wd="$2"; shift 2 ;;
        -u|-l|--name) shift 2 ;;
        *) break ;;
      esac
    done
    shift
    dir=""
    for m in "${{mounts[@]}}"; do [ "${{m#*:}}" = "$wd" ] && dir="${{m%%:*}}"; done
    cd "$dir" || exit 125
    exec env -i "${{envs[@]}}" "$@"
    ;;
esac
exit 1
"#,
        log = log.display()
    );
    std::fs::write(&docker, script).unwrap();
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
    docker
}

/// The recorded calls, one `Vec` of arguments per call (`-- stdin` and what follows it stay in the
/// call that read them).
fn recorded_calls(log: &std::path::Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(log).unwrap();
    let mut calls: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        if line == "== docker" {
            calls.push(Vec::new());
        } else {
            calls.last_mut().unwrap().push(line.to_string());
        }
    }
    calls
}

/// `ProcessUtils.getuid()` / `getgid()`.
fn uid_gid() -> (u32, u32) {
    // SAFETY: plain getuid(2) / getgid(2).
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// The `docker run` arguments the oracle passes for a spawn with no environment, up to the image.
fn expected_run_prefix(network: &str, sandbox_exec_root: &str) -> Vec<String> {
    let (uid, gid) = uid_gid();
    let mut v: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        format!("--network={network}"),
        // PosixLocalEnvProvider: TMPDIR is always set.
        "-e".into(),
        "TMPDIR=/tmp".into(),
        "-v".into(),
        format!("{sandbox_exec_root}:/execroot/_main"),
        "-w".into(),
        "/execroot/_main".into(),
    ];
    if uid > 0 {
        v.push("-u".into());
        v.push(if gid > 0 {
            format!("{uid}:{gid}")
        } else {
            uid.to_string()
        });
    }
    v
}

/// The Dockerfile Bazel 9.3.2 sends to `docker build` (recorded from the oracle).
const EXPECTED_DOCKERFILE: &[&str] = &[
    "ARG image",
    "FROM $image",
    "ARG work_dir",
    "RUN mkdir --parents $work_dir",
    "ARG group_name",
    "ARG gid",
    "RUN groupadd --non-unique --gid $gid $group_name",
    "ARG user_name",
    "ARG uid",
    "RUN useradd --non-unique --no-log-init --create-home --gid $gid --home-dir $work_dir --no-user-group --uid $uid $user_name",
    "RUN chown --recursive $uid:$gid $work_dir",
    "USER $user_name:$group_name",
    "ENV HOME $work_dir",
    "ENV USER $user_name",
    "WORKDIR $work_dir",
];

struct DockerFixture {
    f: Fixture,
    docker: Path,
    log: std::path::PathBuf,
    sandbox_base: Path,
}

fn docker_fixture() -> Option<DockerFixture> {
    let mut f = create_command_environment(&[])?;
    f.env.command_id = "1b4e28ba-2fa1-11d2-883f-0016d3cca427".to_string();
    let fake_dir = std::path::PathBuf::from(f.test_root.get_path_string()).join("fakedocker");
    std::fs::create_dir(&fake_dir).unwrap();
    let log = std::path::PathBuf::from(f.test_root.get_path_string()).join("docker-argv.log");
    std::fs::write(&log, "").unwrap();
    let docker_file = write_fake_docker(&fake_dir, &log);
    let docker = f.env.file_system.get_path(docker_file.to_str().unwrap());
    f.env.exec_root.create_directory().unwrap();
    let sandbox_base = f.env.exec_root.get_relative_str("sandbox");
    sandbox_base.create_directory().unwrap();
    Some(DockerFixture {
        f,
        docker,
        log,
        sandbox_base,
    })
}

fn run_spawn(
    d: &DockerFixture,
    runner: &DockerSandboxedSpawnRunner,
    spawn: &SimpleSpawn,
    name: &str,
) -> (blaze_exec::actions::SpawnResult, String, String) {
    let stdout = d.f.test_root.get_child(&format!("{name}.stdout"));
    let stderr = d.f.test_root.get_child(&format!("{name}.stderr"));
    let file_out_err = FileOutErr::new(stdout.clone(), stderr.clone());
    let policy = SpawnExecutionContextForTesting::new(spawn, file_out_err, Duration::from_secs(60));
    let result = runner.exec(spawn, &policy).unwrap();
    let read = |p: &Path| std::fs::read_to_string(p.get_path_string()).unwrap_or_default();
    (result, read(&stdout), read(&stderr))
}

#[test]
fn docker_sandboxed_spawn_runner_runs_spawn_through_fake_docker() {
    let Some(d) = docker_fixture() else {
        return;
    };

    assert!(DockerSandboxedSpawnRunner::is_supported(&d.f.env, &d.docker).unwrap());

    let runner = DockerSandboxedSpawnRunner::new(
        &d.f.env,
        d.docker.clone(),
        d.sandbox_base.clone(),
        "fake:img".to_string(),
        /* use_customized_images= */ true,
        Arc::new(SynchronousTreeDeleter),
    );

    let echo = spawn(&["/bin/sh", "-c", "echo cooee; pwd; echo \"$TMPDIR\""], &[]);
    let (result, stdout, stderr) = run_spawn(&d, &runner, &echo, "echo");
    assert_eq!(stderr, "");
    assert_eq!(result.status(), Status::Success);
    assert_eq!(result.exit_code(), 0);
    let sandbox_exec_root = d
        .sandbox_base
        .get_relative_str("docker/0/execroot/_main")
        .get_path_string()
        .to_string();
    assert_eq!(stdout, format!("cooee\n{sandbox_exec_root}\n/tmp\n"));

    let failing = spawn(&["/bin/sh", "-c", "echo failing >&2; exit 3"], &[]);
    let (result, _, stderr) = run_spawn(&d, &runner, &failing, "failing");
    assert_eq!(result.status(), Status::NonZeroExit);
    assert_eq!(result.exit_code(), 3);
    assert_eq!(stderr, "failing\n");

    let blocked = spawn(&["/bin/sh", "-c", "true"], &[("block-network", "")]);
    let (result, _, _) = run_spawn(&d, &runner, &blocked, "blocked");
    assert_eq!(result.status(), Status::Success);

    let calls = recorded_calls(&d.log);
    let (uid, gid) = uid_gid();
    // `docker info` (isSupported), `docker build` once for the image (cached by the runner), then
    // one `docker run` per spawn. No `docker rm`: the containers ran.
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert_eq!(calls[0], vec!["info".to_string()]);
    let mut build: Vec<String> = [
        "build",
        "--build-arg",
        "image=fake:img",
        "--build-arg",
        "work_dir=/execroot/_main",
        "--build-arg",
        "group_name=bazelbuild",
        "--build-arg",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    build.push(format!("gid={gid}"));
    build.extend(
        ["--build-arg", "user_name=bazelbuild", "--build-arg"]
            .iter()
            .map(|s| s.to_string()),
    );
    build.push(format!("uid={uid}"));
    build.extend(["-q", "-", "-- stdin"].iter().map(|s| s.to_string()));
    build.extend(EXPECTED_DOCKERFILE.iter().map(|s| s.to_string()));
    assert_eq!(calls[1], build);

    for (call, network, args) in [
        (
            &calls[2],
            "host",
            vec!["/bin/sh", "-c", "echo cooee; pwd; echo \"$TMPDIR\""],
        ),
        (
            &calls[3],
            "host",
            vec!["/bin/sh", "-c", "echo failing >&2; exit 3"],
        ),
        (&calls[4], "none", vec!["/bin/sh", "-c", "true"]),
    ] {
        let mut expected = expected_run_prefix(network, &sandbox_exec_root);
        expected.push("-l".into());
        expected.push("command_id=1b4e28ba-2fa1-11d2-883f-0016d3cca427".into());
        expected.push("--name".into());
        // The container name is a fresh random UUID.
        let name = &call[expected.len()];
        assert_eq!(name.len(), 36, "{call:?}");
        expected.push(name.clone());
        expected.push("sha256:fakeimage".into());
        expected.extend(args.iter().map(|s| s.to_string()));
        assert_eq!(*call, expected);
    }
}

#[test]
fn docker_sandboxed_spawn_runner_uses_default_image_without_customized_images() {
    let Some(d) = docker_fixture() else {
        return;
    };
    let runner = DockerSandboxedSpawnRunner::new(
        &d.f.env,
        d.docker.clone(),
        d.sandbox_base.clone(),
        "fake:img".to_string(),
        /* use_customized_images= */ false,
        Arc::new(SynchronousTreeDeleter),
    );
    let echo = spawn(&["/bin/sh", "-c", "echo cooee"], &[]);
    let (result, stdout, _) = run_spawn(&d, &runner, &echo, "echo");
    assert_eq!(result.status(), Status::Success);
    assert_eq!(stdout, "cooee\n");

    let calls = recorded_calls(&d.log);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0][0], "run");
    let image = calls[0].iter().position(|a| a == "fake:img");
    assert!(image.is_some(), "{calls:?}");
    assert_eq!(
        calls[0][image.unwrap() + 1..],
        ["/bin/sh", "-c", "echo cooee"]
    );
}
