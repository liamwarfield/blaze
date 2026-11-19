//! Ports of `lib/worker/SingleplexWorker.java` and `lib/worker/SandboxedWorker.java` (a
//! `SingleplexWorker` that runs inside a sandboxed execution root).

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use blaze_optdefs::misc::worker_options::WorkerOptions;
use blaze_protos::blaze::worker::{WorkRequest, WorkResponse};
use blaze_shell::{JavaFile, Subprocess, SubprocessBuilder};
use blaze_vfs::{Path, PathFragment};
use indexmap::IndexMap;
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
use crate::sandbox::sandbox_helpers::{self, SandboxInputs, SandboxOutputs};

/// Options specific to hardened sandbox (`--experimental_worker_sandbox_hardening`).
///
/// PORT: the hardened sandbox needs `CgroupsInfo` (Java creates a cgroup for every hardened
/// worker), which is not ported; [`WorkerFactory`](super::worker_factory::WorkerFactory) fails
/// loudly before one is used.
// Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:52 SandboxedWorker.WorkerSandboxOptions
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerSandboxOptions {
    pub sandbox_binary: Path,
    pub fake_hostname: bool,
    pub fake_username: bool,
    pub debug_mode: bool,
    pub tmpfs_path: Vec<PathFragment>,
    pub writable_paths: Vec<String>,
    pub memory_limit: i32,
    pub inaccessible_paths: Vec<Path>,
    pub additional_mount_paths: IndexMap<String, String>,
}

/// The parts of a `SandboxedWorker` its `SingleplexWorker` superclass does not have.
struct SandboxedParts {
    worker_exec_root: Mutex<WorkerExecRoot>,
    tree_deleter: Arc<dyn TreeDeleter>,
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
        }
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
        tree_deleter: Arc<dyn TreeDeleter>,
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
        });
        if hardened_sandbox_options.is_some() {
            blaze_util::unported!("SandboxedWorker (--experimental_worker_sandbox_hardening)");
        }
        worker
    }

    /// `createProcess(clientEnv)` (and `SandboxedWorker.createProcess`).
    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:96 SingleplexWorker.createProcess
    // Java: com/google/devtools/build/lib/worker/SandboxedWorker.java:146 SandboxedWorker.createProcess
    fn create_process(
        &self,
        client_env: &IndexMap<String, String>,
    ) -> io::Result<Arc<dyn Subprocess>> {
        let args = self.make_exec_path_absolute(self.base.worker_key.get_args());
        if self.sandboxed.is_some() {
            // We put the sandbox inside a unique subdirectory using the worker's ID.
            if self.options.use_cgroups_on_linux {
                // In the event that the memory limit is 0, we defer to using Blaze's
                // WorkerLifecycleManager to kill workers rather than cgroup's OOM killer.
                blaze_util::unported!("CgroupsInfo.getBlazeSpawnsCgroup");
            }
            let process = self.create_process_builder(args, client_env).start()?;
            return Ok(Arc::from(process));
        }
        let process = self.create_process_builder(args, client_env).start()?;
        if self.options.use_cgroups_on_linux {
            // PORT: CgroupsInfo.isSupported() and the worker's cgroup are not ported.
            blaze_util::unported!("CgroupsInfo.getBlazeSpawnsCgroup");
        }
        Ok(Arc::from(process))
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:114 SingleplexWorker.createProcessBuilder
    fn create_process_builder(
        &self,
        argv: Vec<String>,
        client_env: &IndexMap<String, String>,
    ) -> SubprocessBuilder {
        let mut process_builder = SubprocessBuilder::new(client_env);
        process_builder.set_argv(argv);
        process_builder.set_working_directory(Some(JavaFile::new(self.work_dir.get_path_string())));
        process_builder.set_stderr_file(JavaFile::new(self.base.log_file.get_path_string()));
        process_builder.set_env(Some(self.base.worker_key.get_env().clone()));
        process_builder
    }

    // Java: com/google/devtools/build/lib/worker/SingleplexWorker.java:163 SingleplexWorker.addShutdownHook
    fn add_shutdown_hook(&self, state: &mut SingleplexState) {
        let shared = Arc::downgrade(&self.shared);
        state.shutdown_hook = Some(shutdown_hooks::add_shutdown_hook(Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                shared.destroy_from_shutdown_hook();
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
                &*sandboxed.tree_deleter,
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
        self.base.status.set_killed();
        if self.sandboxed.is_some() {
            // Caught IOException while deleting workdir (logged).
            let _ = self.work_dir.delete_tree();
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
