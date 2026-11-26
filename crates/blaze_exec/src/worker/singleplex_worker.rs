//! Ports of `lib/worker/SingleplexWorker.java` and `lib/worker/SandboxedWorker.java` (a
//! `SingleplexWorker` that runs inside a sandboxed execution root).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_shell::{JavaFile, Subprocess, SubprocessBuilder, SubprocessFactory};
use blaze_vfs::{FileSystemExt, Symlinks};
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};
use parking_lot::Mutex;

use super::error_message::JavaException;
use super::recording_input_stream::{PipeInputStream, RecordingInputStream};
use super::shutdown_hooks::{self, ShutdownHookId};
use super::worker::{Worker, WorkerBase, WorkerError, java_sleep};
use super::worker_exec_root::WorkerExecRoot;
use super::worker_key::WorkerKey;
use super::worker_process_status::{Status, WorkerProcessStatus};
use super::worker_protocol::{
    JsonWorkerProtocol, ProtoWorkerProtocol, SharedRecordingStream, WorkerProtocolImpl,
};
use crate::actions::execution_requirements::WorkerProtocolFormat;
use crate::exec::tree_deleter::TreeDeleter;
use crate::sandbox::cgroups::VirtualCgroupFactory;
use crate::sandbox::cgroups_info::CgroupsInfo;
use crate::sandbox::linux_sandbox_command_line_builder::{
    LinuxSandboxCommandLineBuilder, NetworkNamespace,
};
use crate::sandbox::linux_sandbox_util;
use crate::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};

/// Options specific to hardened sandbox (`--experimental_worker_sandbox_hardening`).
///
/// Java's record holds `ImmutableSet`s and an `ImmutableMap`, whose `equals` ignores order; the
/// `PartialEq` below does the same (the `WorkerFactory` comparison of `WorkerModule.buildStarting`
/// reads it).
// Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:49 SandboxedWorker.WorkerSandboxOptions
#[derive(Clone, Debug)]
pub struct WorkerSandboxOptions {
    /// `LinuxSandboxUtil.getLinuxSandbox(workspace)`: null without an embedded `linux-sandbox`.
    pub sandbox_binary: Option<Path>,
    pub fake_hostname: bool,
    pub fake_username: bool,
    pub debug_mode: bool,
    /// `ImmutableSet<PathFragment>`: insertion order, no duplicates.
    pub tmpfs_path: Vec<PathFragment>,
    /// `ImmutableSet<String>`.
    pub writable_paths: Vec<String>,
    pub memory_limit: i32,
    /// `ImmutableSet<Path>`.
    pub inaccessible_paths: Vec<Path>,
    pub additional_mount_paths: IndexMap<String, String>,
}

/// `ImmutableSet.equals`: the same elements, in any order.
fn same_set<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    a.len() == b.len() && a.iter().all(|x| b.contains(x))
}

impl PartialEq for WorkerSandboxOptions {
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:49 SandboxedWorker.WorkerSandboxOptions.equals
    fn eq(&self, o: &Self) -> bool {
        self.sandbox_binary == o.sandbox_binary
            && self.fake_hostname == o.fake_hostname
            && self.fake_username == o.fake_username
            && self.debug_mode == o.debug_mode
            && same_set(&self.tmpfs_path, &o.tmpfs_path)
            && same_set(&self.writable_paths, &o.writable_paths)
            && self.memory_limit == o.memory_limit
            && same_set(&self.inaccessible_paths, &o.inaccessible_paths)
            // ImmutableMap.equals (AbstractMap.equals): the same entries, in any order.
            && self.additional_mount_paths.len() == o.additional_mount_paths.len()
            && self
                .additional_mount_paths
                .iter()
                .all(|(k, v)| o.additional_mount_paths.get(k) == Some(v))
    }
}

impl Eq for WorkerSandboxOptions {}

/// The parts of a `SandboxedWorker` its `SingleplexWorker` superclass does not have.
struct SandboxedParts {
    worker_exec_root: Mutex<WorkerExecRoot>,
    tree_deleter: Option<Arc<dyn TreeDeleter>>,
    /// Options specific to hardened sandbox, null if not using that.
    hardened_sandbox_options: Option<WorkerSandboxOptions>,
    /// `inaccessibleHelperDir` and `inaccessibleHelperFile`, set by `getBindMounts`.
    inaccessible_helpers: Arc<Mutex<InaccessibleHelpers>>,
}

/// `SandboxedWorker.inaccessibleHelperDir` and `inaccessibleHelperFile`.
#[derive(Default)]
struct InaccessibleHelpers {
    dir: Option<Path>,
    file: Option<Path>,
}

/// The state Java keeps in the worker's fields.
struct SingleplexState {
    process: Option<Arc<dyn Subprocess>>,
    /// Stream for recording the WorkResponse as it's read, so that it can be printed in the case
    /// of parsing failures.
    recording_input_stream: Option<Arc<Mutex<RecordingInputStream>>>,
    /// True if we deliberately destroyed this process.
    was_destroyed: bool,
    /// Shutdown hook to make sure we wait for the process to finish on JVM shutdown, to avoid
    /// creating zombie processes.
    shutdown_hook: Option<ShutdownHookId>,
}

/// What the shutdown hook and the worker share.
struct SingleplexShared {
    state: Mutex<SingleplexState>,
    /// The implementation of the worker protocol (JSON or Proto); `@GuardedBy("this")`.
    worker_protocol: Mutex<Option<Arc<dyn WorkerProtocolImpl>>>,
}

/// Interface to a worker process running as a single child process.
///
/// A worker process must follow this protocol to be usable via this class: The worker process
/// is spawned on demand. The worker process is free to exit whenever necessary, as new instances
/// will be relaunched automatically. Communication happens via the WorkerProtocol protobuf, sent
/// to and received from the worker process via stdin / stdout.
// Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:52 SingleplexWorker
pub struct SingleplexWorker {
    base: WorkerBase,
    /// The execution root of the worker.
    work_dir: Path,
    options: Arc<WorkerOptions>,
    shared: Arc<SingleplexShared>,
    /// `Some` for a `SandboxedWorker`.
    sandboxed: Option<SandboxedParts>,
    /// For tests: the factory `createProcess` starts the process with (Java's tests override
    /// `createProcess` in a subclass, `WorkerTestUtils.TestWorker`).
    subprocess_factory: Mutex<Option<Arc<dyn SubprocessFactory>>>,
    /// `cgroupFactory`: null unless `--incompatible_use_new_cgroup_implementation` (set by the
    /// `WorkerFactory`).
    cgroup_factory: Option<Arc<VirtualCgroupFactory>>,
}

impl SingleplexWorker {
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:85 SingleplexWorker.<init>
    pub fn new(
        worker_key: Arc<WorkerKey>,
        worker_id: i32,
        work_dir: Path,
        log_file: Path,
        options: Arc<WorkerOptions>,
    ) -> SingleplexWorker {
        SingleplexWorker {
            base: WorkerBase::new(
                worker_key,
                worker_id,
                log_file,
                Arc::new(WorkerProcessStatus::new()),
            ),
            work_dir,
            options,
            shared: Arc::new(SingleplexShared {
                state: Mutex::new(SingleplexState {
                    process: None,
                    recording_input_stream: None,
                    was_destroyed: false,
                    shutdown_hook: None,
                }),
                worker_protocol: Mutex::new(None),
            }),
            sandboxed: None,
            subprocess_factory: Mutex::new(None),
            cgroup_factory: None,
        }
    }

    /// The `cgroupFactory` constructor argument.
    pub fn with_cgroup_factory(
        mut self,
        cgroup_factory: Option<Arc<VirtualCgroupFactory>>,
    ) -> Self {
        self.cgroup_factory = cgroup_factory;
        self
    }

    /// For tests: starts the worker's process with `factory` (see `subprocess_factory`).
    pub fn set_process_factory(&self, factory: Arc<dyn SubprocessFactory>) {
        *self.subprocess_factory.lock() = Some(factory);
    }

    /// `new SandboxedWorker(...)`.
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:74 SandboxedWorker.<init>
    pub fn new_sandboxed(
        worker_key: Arc<WorkerKey>,
        worker_id: i32,
        work_dir: Path,
        log_file: Path,
        worker_options: Arc<WorkerOptions>,
        hardened_sandbox_options: Option<&WorkerSandboxOptions>,
        tree_deleter: Option<Arc<dyn TreeDeleter>>,
        use_in_memory_tracking: bool,
    ) -> SingleplexWorker {
        let mut worker = SingleplexWorker::new(
            worker_key,
            worker_id,
            work_dir.clone(),
            log_file,
            worker_options,
        );
        let tmp_dir_path = sandbox_helpers::get_tmp_dir_path(&work_dir);
        worker.sandboxed = Some(SandboxedParts {
            worker_exec_root: Mutex::new(WorkerExecRoot::new(
                work_dir,
                if hardened_sandbox_options.is_some() {
                    vec![PathFragment::create(tmp_dir_path.get_path_string())]
                } else {
                    Vec::new()
                },
                use_in_memory_tracking,
            )),
            tree_deleter,
            hardened_sandbox_options: hardened_sandbox_options.cloned(),
            inaccessible_helpers: Arc::new(Mutex::new(InaccessibleHelpers::default())),
        });
        worker
    }

    /// `createProcess(clientEnv)` (and `SandboxedWorker.createProcess`).
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:96 SingleplexWorker.createProcess
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:146 SandboxedWorker.createProcess
    fn create_process(
        &self,
        client_env: &IndexMap<String, String>,
    ) -> Result<Arc<dyn Subprocess>, WorkerError> {
        let mut args = self.make_exec_path_absolute(self.base.worker_key.get_args());
        if let Some(sandboxed) = &self.sandboxed {
            let hardened = sandboxed.hardened_sandbox_options.as_ref();
            // We put the sandbox inside a unique subdirectory using the worker's ID.
            if let Some(factory) = &self.cgroup_factory {
                let cgroup = factory.create(self.base.worker_id, &IndexMap::new())?;
                *self.base.cgroup.lock() = Some(cgroup);
            } else if self.options.use_cgroups_on_linux || hardened.is_some() {
                // In the event that the memory limit is 0, we defer to using Blaze's
                // WorkerLifecycleManager to kill workers rather than cgroup's OOM killer.
                // (The old implementation, --noincompatible_use_new_cgroup_implementation.)
                let cgroup = CgroupsInfo::get_blaze_spawns_cgroup().create_individual_spawn_cgroup(
                    &format!("worker_sandbox_{}", self.base.worker_id),
                    hardened.map_or(0, |h| h.memory_limit),
                );
                *self.base.cgroup.lock() = Some(Arc::new(cgroup));
            }

            // TODO(larsrc): Check that execRoot and outputBase are not under /tmp
            if let Some(hardened) = hardened {
                let sandbox_tmp = sandbox_helpers::get_tmp_dir_path(&self.work_dir);
                sandbox_tmp.create_directory_and_parents()?;

                // Mostly tests require network, and some blaze run commands, but no workers.
                let sandbox_binary = hardened
                    .sandbox_binary
                    .clone()
                    .expect("NullPointerException: sandboxBinary");
                let mut command_line_builder =
                    LinuxSandboxCommandLineBuilder::command_line_builder(sandbox_binary);
                command_line_builder
                    .set_writable_files_and_directories(
                        self.get_writable_dirs(hardened, &self.work_dir)?,
                    )
                    .set_tmpfs_directories(hardened.tmpfs_path.iter().cloned().collect())
                    .set_persistent_process(true)
                    .set_bind_mounts(self.get_bind_mounts(
                        sandboxed,
                        hardened,
                        &self.work_dir,
                        &sandbox_tmp,
                    )?)
                    .set_use_fake_hostname(hardened.fake_hostname)
                    .set_create_network_namespace(NetworkNamespace::Netns);

                let cgroup = self.base.cgroup.lock().clone();
                if let Some(cgroup) = cgroup
                    && cgroup.exists()
                {
                    command_line_builder.set_cgroups_dirs(
                        cgroup
                            .paths()
                            .iter()
                            .map(|p| p.display().to_string())
                            .collect(),
                    );
                }

                if hardened.fake_username {
                    command_line_builder.set_use_fake_username(true);
                }

                args = command_line_builder.build_for_command(&args);
            }

            let process: Arc<dyn Subprocess> =
                Arc::from(self.create_process_builder(args, client_env).start()?);

            // If using hardened sandbox (aka linux-sandbox), the linux-sandbox parent process
            // moves the sandboxed children processes (pid 1, 2) into the cgroup. But we still
            // need to move the linux-sandbox process into the worker cgroup. On the other hand,
            // without linux-sandbox, Blaze needs to do this itself for the spawned worker
            // process.
            self.add_process_to_cgroup(&*process)?;
            return Ok(process);
        }
        let process: Arc<dyn Subprocess> =
            Arc::from(self.create_process_builder(args, client_env).start()?);
        if let Some(factory) = &self.cgroup_factory {
            let cgroup = factory.create(self.base.worker_id, &IndexMap::new())?;
            *self.base.cgroup.lock() = Some(cgroup);
        } else if self.options.use_cgroups_on_linux && CgroupsInfo::is_supported() {
            // The old implementation (--noincompatible_use_new_cgroup_implementation).
            let cgroup = CgroupsInfo::get_blaze_spawns_cgroup().create_individual_spawn_cgroup(
                /* dir_name= */ &format!("worker_{}", self.base.worker_id),
                /* memory_limit_mb= */ 0,
            );
            *self.base.cgroup.lock() = Some(Arc::new(cgroup));
        }
        self.add_process_to_cgroup(&*process)?;
        Ok(process)
    }

    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:98 SandboxedWorker.getWritableDirs
    fn get_writable_dirs(
        &self,
        hardened: &WorkerSandboxOptions,
        sandbox_exec_root: &Path,
    ) -> io::Result<IndexSet<Path>> {
        // We have to make the TEST_TMPDIR directory writable if it is specified.
        let mut writable_dirs: IndexSet<Path> = IndexSet::new();
        writable_dirs.insert(sandbox_exec_root.clone());
        writable_dirs.insert(sandbox_exec_root.get_relative_str("/tmp"));

        let fs = sandbox_exec_root.get_file_system();
        for writable_path in &hardened.writable_paths {
            let path = fs.get_path(writable_path);
            writable_dirs.insert(path.clone());
            if path.is_symbolic_link() {
                writable_dirs.insert(path.resolve_symbolic_links()?);
            }
        }

        writable_dirs.insert(fs.get_path("/dev/shm").resolve_symbolic_links()?);
        writable_dirs.insert(fs.get_path("/tmp"));

        Ok(writable_dirs)
    }

    /// The bind mounts (target -> source) in `TreeMap` order.
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:118 SandboxedWorker.getBindMounts
    fn get_bind_mounts(
        &self,
        sandboxed: &SandboxedParts,
        hardened: &WorkerSandboxOptions,
        sandbox_exec_root: &Path,
        sandbox_tmp: &Path,
    ) -> Result<IndexMap<Path, Path>, WorkerError> {
        let fs = sandbox_exec_root.get_file_system();
        let tmp_path = fs.get_path("/tmp");
        let mut bind_mounts: BTreeMap<Path, Path> = BTreeMap::new();
        bind_mounts.insert(tmp_path, sandbox_tmp.clone());
        sandbox_helpers::mount_additional_paths(
            &hardened.additional_mount_paths,
            sandbox_exec_root,
            &mut bind_mounts,
        )?;

        let inaccessible_helper_file =
            linux_sandbox_util::get_inaccessible_helper_file(sandbox_exec_root)?;
        let inaccessible_helper_dir =
            linux_sandbox_util::get_inaccessible_helper_dir(sandbox_exec_root)?;
        {
            let mut helpers = sandboxed.inaccessible_helpers.lock();
            helpers.file = Some(inaccessible_helper_file.clone());
            helpers.dir = Some(inaccessible_helper_dir.clone());
        }
        for inaccessible_path in &hardened.inaccessible_paths {
            if inaccessible_path.is_directory_with(Symlinks::Nofollow) {
                bind_mounts.insert(inaccessible_path.clone(), inaccessible_helper_dir.clone());
            } else {
                bind_mounts.insert(inaccessible_path.clone(), inaccessible_helper_file.clone());
            }
        }
        // TODO(larsrc): Handle hermetic tmp
        linux_sandbox_util::validate_bind_mounts(&bind_mounts)?;
        Ok(bind_mounts.into_iter().collect())
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:114 SingleplexWorker.createProcessBuilder
    fn create_process_builder(
        &self,
        argv: Vec<String>,
        client_env: &IndexMap<String, String>,
    ) -> SubprocessBuilder {
        let mut process_builder = match &*self.subprocess_factory.lock() {
            Some(f) => SubprocessBuilder::with_factory(client_env, f.clone()),
            None => SubprocessBuilder::new(client_env),
        };
        process_builder.set_argv(argv);
        process_builder.set_working_directory(Some(JavaFile::new(self.work_dir.get_path_string())));
        process_builder.set_stderr_file(JavaFile::new(self.base.log_file.get_path_string()));
        process_builder.set_env(Some(self.base.worker_key.get_env().clone()));
        process_builder
    }

    /// `if (cgroup != null && cgroup.exists()) cgroup.addProcess(process.getProcessId())`.
    fn add_process_to_cgroup(&self, process: &dyn Subprocess) -> io::Result<()> {
        let cgroup = self.base.cgroup.lock().clone();
        if let Some(cgroup) = cgroup
            && cgroup.exists()
        {
            cgroup.add_process(process.get_process_id())?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:163 SingleplexWorker.addShutdownHook
    fn add_shutdown_hook(&self, state: &mut SingleplexState) {
        let shared = Arc::downgrade(&self.shared);
        let status = self.base.status.clone();
        // `this.destroy()` is virtual: a SandboxedWorker's also deletes its workDir.
        let sandboxed_work_dir = self.sandboxed.as_ref().map(|_| self.work_dir.clone());
        let hardened = self
            .sandboxed
            .as_ref()
            .is_some_and(|s| s.hardened_sandbox_options.is_some());
        let cgroup_factory = self.cgroup_factory.clone();
        let worker_id = self.base.worker_id;
        state.shutdown_hook = Some(shutdown_hooks::add_shutdown_hook(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.destroy_from_shutdown_hook();
                if let Some(factory) = &cgroup_factory {
                    factory.remove(worker_id);
                }
                status.set_killed();
                if let Some(work_dir) = &sandboxed_work_dir {
                    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:225 SandboxedWorker.destroy
                    // Caught IOException while deleting workdir (logged).
                    if work_dir.delete_tree().is_ok() && hardened {
                        let _ = sandbox_helpers::get_tmp_dir_path(work_dir).delete_tree();
                    }
                }
            }
        })));
    }

    /// Makes sure that the executable (first element of argument list) is an absolute path.
    /// Necessary on Windows (https://github.com/bazelbuild/bazel/commit/8efc3ef0)
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:178 SingleplexWorker.makeExecPathAbsolute
    fn make_exec_path_absolute(&self, args: &[String]) -> Vec<String> {
        let executable = JavaFile::new(&args[0]);
        if !executable.is_absolute() && executable.get_parent().is_some() {
            let mut v = Vec::with_capacity(args.len());
            v.push(
                JavaFile::new_child(
                    Some(&JavaFile::new(self.work_dir.get_path_string())),
                    &args[0],
                )
                .get_absolute_path(),
            );
            v.extend(args[1..].iter().cloned());
            v
        } else {
            args.to_vec()
        }
    }

    /// The worker's exec root (`workDir`).
    pub fn get_work_dir(&self) -> &Path {
        &self.work_dir
    }
}

impl SingleplexShared {
    /// `destroy()` run by the shutdown hook.
    ///
    /// PORT: Java's hook calls the synchronized `destroy()`; at exit a thread may still be blocked
    /// in `getResponse` holding that lock, so the hook closes the protocol only if the lock is
    /// free and always destroys the process.
    fn destroy_from_shutdown_hook(&self) {
        if let Some(mut protocol) = self
            .worker_protocol
            .try_lock_for(Duration::from_millis(100))
            && let Some(p) = protocol.take()
        {
            let _ = p.close();
        }
        let process = {
            let mut st = self.state.lock();
            st.shutdown_hook = None;
            st.was_destroyed = true;
            st.process.clone()
        };
        if let Some(process) = process {
            shutdown_hooks::destroy_and_wait_at_exit(&*process);
        }
    }
}

impl Worker for SingleplexWorker {
    fn base(&self) -> &WorkerBase {
        &self.base
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:123 SingleplexWorker.isSandboxed
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:98 SandboxedWorker.isSandboxed
    fn is_sandboxed(&self) -> bool {
        self.sandboxed.is_some()
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:128 SingleplexWorker.prepareExecution
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:202 SandboxedWorker.prepareExecution
    fn prepare_execution(
        &self,
        input_files: &SandboxInputs,
        outputs: &SandboxOutputs,
        worker_files: &BTreeSet<PathFragment>,
        client_env: &IndexMap<String, String>,
    ) -> Result<(), WorkerError> {
        if let Some(sandboxed) = &self.sandboxed {
            // PORT: skipped, profiling.
            sandboxed.worker_exec_root.lock().create_file_system(
                worker_files,
                input_files,
                outputs,
                &**sandboxed
                    .tree_deleter
                    .as_ref()
                    .expect("NullPointerException: treeDeleter"),
            )?;
        }
        let recording = {
            let mut st = self.shared.state.lock();
            if st.process.is_none() {
                self.add_shutdown_hook(&mut st);
                let process = self.create_process(client_env)?;
                // Created worker process %s for worker id %d (logged).
                self.base.status.maybe_update_status(Status::Alive);
                let stdout = process.get_input_stream().expect("worker stdout is a pipe");
                st.recording_input_stream = Some(Arc::new(Mutex::new(RecordingInputStream::new(
                    Box::new(PipeInputStream::new(
                        stdout,
                        &format!("worker-stdout-{}", self.base.worker_id),
                    )),
                ))));
                st.process = Some(process);
            }
            (
                st.process.clone().unwrap(),
                st.recording_input_stream.clone().unwrap(),
            )
        };
        let mut protocol = self.shared.worker_protocol.lock();
        if protocol.is_none() {
            let (process, recording) = recording;
            // PORT: Java's process.getOutputStream() can be asked again; Blaze's moves the stream
            // out once, so a protocol made after the first one (never in Java's flows: it is only
            // dropped by destroy) gets a closed stdin.
            let stdin: Box<dyn io::Write + Send> = process
                .get_output_stream()
                .unwrap_or_else(|| Box::new(io::sink()));
            let stdout = Box::new(SharedRecordingStream(recording));
            *protocol = Some(match self.base.worker_key.get_protocol_format() {
                WorkerProtocolFormat::Json => Arc::new(JsonWorkerProtocol::new(stdin, stdout)),
                WorkerProtocolFormat::Proto => Arc::new(ProtoWorkerProtocol::new(stdin, stdout)),
            });
        }
        Ok(())
    }

    /// PORT: the JDK's process reaper is not ported. Once a worker process has exited, the JDK
    /// replaces its stdin with `ProcessBuilder.NullOutputStream`, so Java's write fails with
    /// `java.io.IOException: Stream closed`; here the write reaches the closed pipe and fails with
    /// `Broken pipe`. Which of the two Java reports depends on whether the reaper ran before the
    /// write (both were seen on the oracle), so no fixed output differs (review exec-2, S3).
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:193 SingleplexWorker.putRequest
    fn put_request(&self, request: &WorkRequest) -> io::Result<()> {
        let protocol = self.shared.worker_protocol.lock();
        match protocol.as_ref() {
            None => Err(JavaException::io("Worker has been destroyed.").into_io_error()),
            Some(p) => p.put_request(request),
        }
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:201 SingleplexWorker.getResponse
    fn get_response(&self, _request_id: i32) -> Result<Option<WorkResponse>, WorkerError> {
        let (process, recording) = {
            let st = self.shared.state.lock();
            (
                st.process.clone().expect("NullPointerException: process"),
                st.recording_input_stream
                    .clone()
                    .expect("NullPointerException: recordingInputStream"),
            )
        };
        recording.lock().start_recording(4096);
        while recording.lock().available()? == 0 {
            if java_sleep(10).is_err() {
                return Err(WorkerError::Interrupted);
            }
            if !process.is_alive() {
                return Err(WorkerError::Io(
                    JavaException::io(format!(
                        "Worker process for {} died while waiting for response",
                        self.base.worker_key.get_mnemonic()
                    ))
                    .into_io_error(),
                ));
            }
        }
        // We only want to synchronize on the getResponse() call, and not in the loop above to
        // avoid locking this worker.
        let protocol = self.shared.worker_protocol.lock();
        match protocol.as_ref() {
            None => Err(WorkerError::Io(
                JavaException::io("Worker has been destroyed.").into_io_error(),
            )),
            Some(p) => Ok(p.get_response()?),
        }
    }

    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:213 SandboxedWorker.finishExecution
    fn finish_execution(
        &self,
        exec_root: &Path,
        outputs: &SandboxOutputs,
    ) -> Result<(), WorkerError> {
        self.base.status.maybe_update_status(Status::Alive);
        if let Some(sandboxed) = &self.sandboxed {
            // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:213 SandboxedWorker.finishExecution
            let cgroup = self.base.cgroup.lock().clone();
            if let Some(cgroup) = cgroup
                && cgroup.exists()
            {
                // This is only to not leave too much behind in the cgroups tree, can ignore errors.
                cgroup.destroy();
            }
            sandboxed
                .worker_exec_root
                .lock()
                .copy_outputs(exec_root, outputs)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:221 SingleplexWorker.destroy
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:225 SandboxedWorker.destroy
    fn destroy(&self) {
        {
            let mut protocol = self.shared.worker_protocol.lock();
            if let Some(p) = protocol.take() {
                // Caught IOException while closing worker protocol (logged).
                let _ = p.close();
            }
        }
        let process = {
            let mut st = self.shared.state.lock();
            if let Some(hook) = st.shutdown_hook.take() {
                // Can only fail if we're already in shutdown, in which case we don't care.
                shutdown_hooks::remove_shutdown_hook(hook);
            }
            if st.process.is_some() {
                st.was_destroyed = true;
            }
            st.process.clone()
        };
        if let Some(process) = process {
            process.destroy_and_wait();
        }
        if let Some(factory) = &self.cgroup_factory {
            factory.remove(self.base.worker_id);
        }
        self.base.status.set_killed();
        if let Some(sandboxed) = &self.sandboxed {
            // SandboxedWorker.destroy: the inaccessible helpers, the cgroup, the work dir and
            // the hardened sandbox's tmp dir. An IOException ends the cleanup (logged).
            let helpers = sandboxed.inaccessible_helpers.lock();
            if let Some(file) = &helpers.file
                && file.delete().is_err()
            {
                return;
            }
            if let Some(dir) = &helpers.dir
                && dir.delete().is_err()
            {
                return;
            }
            drop(helpers);
            let cgroup = self.base.cgroup.lock().clone();
            if let Some(cgroup) = cgroup
                && cgroup.exists()
            {
                // This is only to not leave too much behind in the cgroups tree, can ignore errors.
                cgroup.destroy();
            }
            // Caught IOException while deleting workdir (logged).
            if self.work_dir.delete_tree().is_err() {
                return;
            }
            if sandboxed.hardened_sandbox_options.is_some() {
                let _ = sandbox_helpers::get_tmp_dir_path(&self.work_dir).delete_tree();
            }
        }
    }

    /// Returns true if this process is dead but we didn't deliberately kill it.
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:251 SingleplexWorker.diedUnexpectedly
    fn died_unexpectedly(&self) -> bool {
        let st = self.shared.state.lock();
        st.process
            .as_ref()
            .is_some_and(|p| !st.was_destroyed && !p.is_alive())
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:256 SingleplexWorker.getExitValue
    fn get_exit_value(&self) -> Option<i32> {
        let st = self.shared.state.lock();
        match &st.process {
            Some(p) if !p.is_alive() => Some(p.exit_value()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:263 SingleplexWorker.getRecordingStreamMessage
    fn get_recording_stream_message(&self) -> String {
        let recording = self
            .shared
            .state
            .lock()
            .recording_input_stream
            .clone()
            .expect("NullPointerException: recordingInputStream");
        let mut r = recording.lock();
        r.read_remaining();
        r.get_recorded_data_as_string()
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:274 SingleplexWorker.getProcessId
    fn get_process_id(&self) -> i64 {
        match &self.shared.state.lock().process {
            None => -1,
            Some(p) => p.get_process_id(),
        }
    }
}

impl fmt::Display for SingleplexWorker {
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:269 SingleplexWorker.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} worker #{}",
            self.base.worker_key.get_mnemonic(),
            self.base.worker_id
        )
    }
}
